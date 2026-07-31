use crate::{
    documents::preflight::{
        display_name, FileFormat, FileInspection, FileRole, InspectedDocument, PreflightService,
    },
    domain::job::{
        BidInput, FileErrorRecord, JobInput, JobManifest, JobState, StoredFileFormat,
        StoredFileMetadata, StoredFileRole,
    },
    error::{AppError, ErrorCode},
    jobs::store::JobStore,
    reports::{build_report, ReviewReportInput},
    review::{
        schema::{BidFinding, Requirement},
        service::{ReviewService, StructuredModelClient},
        similarity::{compare_blind_documents, DuplicatePair},
        word_format_checker::check_with_word_format_checker,
    },
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedFile {
    pub display_name: String,
    pub role: FileRole,
    pub format: FileFormat,
    pub byte_size: u64,
    pub sha256: String,
    pub block_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedJob {
    pub job_id: String,
    pub state: JobState,
    pub files: Vec<PreparedFile>,
    pub errors: Vec<FileErrorRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSummary {
    pub job_id: String,
    pub updated_at: DateTime<Utc>,
    pub state: JobState,
    pub source_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverableJobError {
    pub job_display: String,
    pub code: crate::error::ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverableJobs {
    pub jobs: Vec<JobSummary>,
    pub errors: Vec<RecoverableJobError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewStatus {
    pub job_id: String,
    pub state: JobState,
    pub stages:
        std::collections::BTreeMap<crate::domain::job::JobStage, crate::domain::job::StageState>,
    pub report_path: Option<String>,
    pub report_markdown: Option<String>,
    pub report_files: Vec<ReviewReportFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewReportFile {
    pub title: String,
    pub kind: String,
    pub path: String,
    pub markdown: String,
}

#[derive(Clone)]
pub struct JobService {
    store: JobStore,
    preflight: PreflightService,
    normalized_writer: Arc<dyn NormalizedDocumentWriter>,
}

trait NormalizedDocumentWriter: Send + Sync {
    fn save_normalized(
        &self,
        job_id: &str,
        artifact_name: &str,
        document: &crate::domain::document::NormalizedDocument,
    ) -> Result<(), AppError>;
}

impl NormalizedDocumentWriter for JobStore {
    fn save_normalized(
        &self,
        job_id: &str,
        artifact_name: &str,
        document: &crate::domain::document::NormalizedDocument,
    ) -> Result<(), AppError> {
        JobStore::save_normalized(self, job_id, artifact_name, document)
    }
}

struct AvailableBid {
    index: usize,
    bid: InspectedDocument,
    blind_bid: Option<InspectedDocument>,
}

impl JobService {
    pub fn new(store: JobStore) -> Self {
        Self {
            normalized_writer: Arc::new(store.clone()),
            store,
            preflight: PreflightService::new(),
        }
    }

    #[cfg(test)]
    fn with_normalized_writer(
        store: JobStore,
        normalized_writer: Arc<dyn NormalizedDocumentWriter>,
    ) -> Self {
        Self {
            store,
            preflight: PreflightService::new(),
            normalized_writer,
        }
    }

    pub fn prepare(&self, input: JobInput) -> Result<PreparedJob, AppError> {
        input.validate()?;
        let mut manifest = JobManifest::new(input);
        self.store.save_atomic(&manifest)?;
        manifest.begin(crate::domain::job::JobStage::Preflight);
        self.store.save_atomic(&manifest)?;

        let tender_path = match canonicalize_selected(Path::new(&manifest.input.tender_path)) {
            Ok(path) => path,
            Err(error) => return self.fail_manifest(&mut manifest, error),
        };
        manifest.input.tender_path = tender_path.to_string_lossy().into_owned();
        if let Err(error) = reject_duplicate_existing_paths(&manifest.input, &tender_path) {
            return self.fail_manifest(&mut manifest, error);
        }

        let tender = match self
            .preflight
            .inspect_document(Path::new(&manifest.input.tender_path), FileRole::Tender)
        {
            Ok(tender) => tender,
            Err(error) => return self.fail_manifest(&mut manifest, error),
        };

        let mut available_bids = Vec::new();
        let mut canonical_paths = HashSet::from([tender_path]);
        for (index, bid_input) in manifest.input.bids.clone().into_iter().enumerate() {
            if let Err(error) = self.inspect_bid(
                index + 1,
                &bid_input,
                &mut canonical_paths,
                &mut manifest,
                &mut available_bids,
            ) {
                return self.fail_manifest(&mut manifest, error);
            }
        }

        manifest.complete(crate::domain::job::JobStage::Preflight);
        self.store.save_atomic(&manifest)?;
        manifest.begin(crate::domain::job::JobStage::Extract);
        self.store.save_atomic(&manifest)?;

        let tender_file =
            match self.persist_document(&manifest.id, "tender", FileRole::Tender, tender) {
                Ok(file) => file,
                Err(error) => return self.fail_manifest(&mut manifest, error),
            };
        manifest
            .completed_files
            .push(stored_file("tender", &tender_file));
        self.store.save_atomic(&manifest)?;
        let mut files = vec![tender_file];
        let mut readable_bid_count = 0usize;
        for available in available_bids {
            match self.persist_document(
                &manifest.id,
                &format!("bid-{}", available.index),
                FileRole::Bid,
                available.bid,
            ) {
                Ok(file) => {
                    readable_bid_count += 1;
                    manifest
                        .completed_files
                        .push(stored_file(&format!("bid-{}", available.index), &file));
                    self.store.save_atomic(&manifest)?;
                    files.push(file);
                }
                Err(error) => {
                    manifest.file_errors.push(file_error(
                        display_name(Path::new(
                            &manifest.input.bids[available.index - 1].bid_path,
                        )),
                        bid_source_key(available.index),
                        error.code,
                    ));
                    continue;
                }
            }

            if let Some(blind_bid) = available.blind_bid {
                match self.persist_document(
                    &manifest.id,
                    &format!("blind-bid-{}", available.index),
                    FileRole::BlindBid,
                    blind_bid,
                ) {
                    Ok(file) => {
                        manifest.completed_files.push(stored_file(
                            &format!("blind-bid-{}", available.index),
                            &file,
                        ));
                        self.store.save_atomic(&manifest)?;
                        files.push(file)
                    }
                    Err(error) => manifest.file_errors.push(file_error(
                        display_name(Path::new(
                            manifest.input.bids[available.index - 1]
                                .blind_bid_path
                                .as_deref()
                                .unwrap_or_default(),
                        )),
                        blind_bid_source_key(available.index),
                        error.code,
                    )),
                }
            }
        }

        if readable_bid_count == 0 {
            manifest.fail();
            self.store.save_atomic(&manifest)?;
            return Err(AppError::no_readable_bids(manifest.file_errors.clone()));
        }

        manifest.complete(crate::domain::job::JobStage::Extract);
        self.store.save_atomic(&manifest)?;
        let state = if manifest.file_errors.is_empty() {
            JobState::Ready
        } else {
            JobState::ReadyWithErrors
        };
        manifest.finish(state);
        self.store.save_atomic(&manifest)?;

        Ok(PreparedJob {
            job_id: manifest.id,
            state: manifest.state,
            files,
            errors: manifest.file_errors,
        })
    }

    pub fn list_recoverable(&self) -> Result<RecoverableJobs, AppError> {
        self.store.scan().map(|scan| RecoverableJobs {
            jobs: scan
                .manifests
                .into_iter()
                .map(|manifest| JobSummary {
                    job_id: manifest.id,
                    updated_at: manifest.updated_at,
                    state: manifest.state,
                    source_names: source_names(&manifest.input),
                })
                .collect(),
            errors: scan
                .errors
                .into_iter()
                .map(|failure| RecoverableJobError {
                    job_display: failure.job_display,
                    code: failure.error.code,
                    message: recovery_error_message(failure.error.code).into(),
                })
                .collect(),
        })
    }

    pub fn clear(&self, job_id: &str) -> Result<(), AppError> {
        self.store.remove(job_id)
    }

    pub fn get_review_status(&self, job_id: &str) -> Result<ReviewStatus, AppError> {
        let manifest = self.store.load(job_id)?;
        let report_files = self.report_files(job_id)?;
        Ok(ReviewStatus {
            job_id: manifest.id,
            state: manifest.state,
            stages: manifest.stages,
            report_path: report_files.first().map(|file| file.path.clone()),
            report_markdown: report_files.first().map(|file| file.markdown.clone()),
            report_files,
        })
    }

    pub fn open_report_folder(&self, job_id: &str) -> Result<std::path::PathBuf, AppError> {
        let report_dir = self.store.report_directory(job_id)?;
        let has_markdown = report_dir
            .is_dir()
            .then(|| std::fs::read_dir(&report_dir))
            .transpose()
            .map_err(AppError::io)?
            .map(|entries| {
                entries.filter_map(Result::ok).any(|entry| {
                    entry
                        .path()
                        .extension()
                        .and_then(|extension| extension.to_str())
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
                })
            })
            .unwrap_or(false);
        if !has_markdown {
            return Err(AppError::new(
                ErrorCode::ReportGenerationFailed,
                "审核结果 Markdown 尚未生成",
            ));
        }
        Ok(report_dir)
    }

    pub fn run_review<C: StructuredModelClient>(
        &self,
        job_id: &str,
        client: C,
    ) -> Result<ReviewStatus, AppError> {
        let mut manifest = self.store.load(job_id)?;
        if !matches!(manifest.state, JobState::Ready | JobState::ReadyWithErrors) {
            return Err(AppError::new(
                ErrorCode::JobPersistenceFailed,
                "仅准备完成的任务可以开始审核",
            ));
        }
        let outcome: Result<bool, AppError> = (|| {
            manifest.begin(crate::domain::job::JobStage::ModelTest);
            self.store.save_atomic(&manifest)?;
            manifest.complete(crate::domain::job::JobStage::ModelTest);
            self.store.save_atomic(&manifest)?;
            manifest.begin(crate::domain::job::JobStage::TenderReview);
            self.store.save_atomic(&manifest)?;
            let tender = self.store.load_normalized(job_id, "tender")?;
            let tender_text = normalized_text(&tender);
            let service = ReviewService::new(client);
            let requirements = service.extract_requirements(&tender_text)?;
            manifest.complete(crate::domain::job::JobStage::TenderReview);
            self.store.save_atomic(&manifest)?;
            manifest.begin(crate::domain::job::JobStage::BidReview);
            self.store.save_atomic(&manifest)?;
            let bid_files: Vec<_> = manifest
                .completed_files
                .iter()
                .filter(|file| file.role == crate::domain::job::StoredFileRole::Bid)
                .cloned()
                .collect();
            let mut bid_results = Vec::new();
            for bid in &bid_files {
                let text =
                    normalized_text(&self.store.load_normalized(job_id, &bid.artifact_name)?);
                bid_results.push(
                    service
                        .review_bid(&requirements, &text)
                        .map_err(|error| error.message),
                );
            }
            let has_unfinished_bid = bid_results.iter().any(Result::is_err);
            manifest.complete(crate::domain::job::JobStage::BidReview);
            self.store.save_atomic(&manifest)?;
            manifest.begin(crate::domain::job::JobStage::DuplicateCheck);
            self.store.save_atomic(&manifest)?;
            let blind_documents = self.blind_bid_documents(job_id, &manifest)?;
            let duplicate_pairs = compare_blind_documents(&blind_documents)?;
            manifest.complete(crate::domain::job::JobStage::DuplicateCheck);
            self.store.save_atomic(&manifest)?;
            manifest.begin(crate::domain::job::JobStage::BlindBidCheck);
            self.store.save_atomic(&manifest)?;
            let blind_bid_checks = manifest
                .completed_files
                .iter()
                .filter(|file| file.role == crate::domain::job::StoredFileRole::BlindBid)
                .map(|file| {
                    let bid_index = file
                        .artifact_name
                        .trim_start_matches("blind-bid-")
                        .parse::<usize>()
                        .map_err(|_| AppError::job_persistence())?;
                    let path = manifest
                        .input
                        .bids
                        .get(bid_index - 1)
                        .and_then(|bid| bid.blind_bid_path.as_deref())
                        .ok_or_else(AppError::job_persistence)?;
                    check_with_word_format_checker(std::path::Path::new(path))
                        .map(|check| (bid_index, check))
                })
                .collect::<Result<Vec<_>, _>>()?;
            manifest.complete(crate::domain::job::JobStage::BlindBidCheck);
            self.store.save_atomic(&manifest)?;
            manifest.begin(crate::domain::job::JobStage::Report);
            self.store.save_atomic(&manifest)?;
            let report = build_report(ReviewReportInput {
                requirements: requirements.clone(),
                bid_results: bid_results.clone(),
                duplicate_pairs: duplicate_pairs.clone(),
                blind_bid_checks,
                file_errors: manifest.file_errors.clone(),
            });
            let report_dir = self.store.report_directory(job_id)?;
            std::fs::create_dir_all(&report_dir).map_err(AppError::io)?;
            write_review_outputs(
                &report_dir,
                &requirements,
                &bid_results,
                &blind_documents,
                &report.blind_bid_checks,
                &duplicate_pairs,
                &manifest.file_errors,
            )?;
            manifest.complete(crate::domain::job::JobStage::Report);
            Ok(has_unfinished_bid)
        })();
        match outcome {
            Ok(has_unfinished_bid) => {
                manifest.finish(if manifest.file_errors.is_empty() && !has_unfinished_bid {
                    JobState::Completed
                } else {
                    JobState::CompletedWithIssues
                })
            }
            Err(error) => {
                manifest.failure_code = Some(error.code);
                manifest.finish(JobState::Failed);
                self.store.save_atomic(&manifest)?;
                return Err(error);
            }
        }
        self.store.save_atomic(&manifest)?;
        self.get_review_status(job_id)
    }

    fn report_files(&self, job_id: &str) -> Result<Vec<ReviewReportFile>, AppError> {
        let report_dir = self.store.report_directory(job_id)?;
        if !report_dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut paths = std::fs::read_dir(report_dir)
            .map_err(AppError::io)?
            .map(|entry| entry.map(|entry| entry.path()).map_err(AppError::io))
            .collect::<Result<Vec<_>, _>>()?;
        paths.retain(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        });
        paths.sort();
        paths
            .into_iter()
            .map(|path| {
                let markdown = std::fs::read_to_string(&path).map_err(AppError::io)?;
                let title = report_title(&path);
                Ok(ReviewReportFile {
                    kind: report_kind(&title).into(),
                    title,
                    path: path.to_string_lossy().into_owned(),
                    markdown,
                })
            })
            .collect()
    }

    fn blind_bid_documents(
        &self,
        job_id: &str,
        manifest: &JobManifest,
    ) -> Result<Vec<(usize, crate::domain::document::NormalizedDocument)>, AppError> {
        manifest
            .completed_files
            .iter()
            .filter(|file| file.role == crate::domain::job::StoredFileRole::BlindBid)
            .map(|file| {
                let bid_index = file
                    .artifact_name
                    .trim_start_matches("blind-bid-")
                    .parse::<usize>()
                    .map_err(|_| AppError::job_persistence())?;
                Ok((
                    bid_index,
                    self.store.load_normalized(job_id, &file.artifact_name)?,
                ))
            })
            .collect()
    }

    fn inspect_bid(
        &self,
        index: usize,
        input: &BidInput,
        canonical_paths: &mut HashSet<PathBuf>,
        manifest: &mut JobManifest,
        available_bids: &mut Vec<AvailableBid>,
    ) -> Result<(), AppError> {
        let bid_path = match canonicalize_selected(Path::new(&input.bid_path)) {
            Ok(path) => path,
            Err(error) => {
                manifest.file_errors.push(file_error(
                    display_name(Path::new(&input.bid_path)),
                    bid_source_key(index),
                    error.code,
                ));
                return Ok(());
            }
        };
        if !canonical_paths.insert(bid_path.clone()) {
            return Err(AppError::duplicate_input_file());
        }
        manifest.input.bids[index - 1].bid_path = bid_path.to_string_lossy().into_owned();
        let bid = match self.preflight.inspect_document(&bid_path, FileRole::Bid) {
            Ok(bid) => bid,
            Err(error) => {
                manifest.file_errors.push(file_error(
                    display_name(Path::new(&input.bid_path)),
                    bid_source_key(index),
                    error.code,
                ));
                return Ok(());
            }
        };

        let blind_bid = match input.blind_bid_path.as_deref() {
            Some(blind_path) => match canonicalize_selected(Path::new(blind_path)) {
                Ok(path) => {
                    if !canonical_paths.insert(path.clone()) {
                        return Err(AppError::duplicate_input_file());
                    }
                    manifest.input.bids[index - 1].blind_bid_path =
                        Some(path.to_string_lossy().into_owned());
                    match self.preflight.inspect_document(&path, FileRole::BlindBid) {
                        Ok(blind_bid) => Some(blind_bid),
                        Err(error) => {
                            manifest.file_errors.push(file_error(
                                display_name(Path::new(blind_path)),
                                blind_bid_source_key(index),
                                error.code,
                            ));
                            None
                        }
                    }
                }
                Err(error) => {
                    manifest.file_errors.push(file_error(
                        display_name(Path::new(blind_path)),
                        blind_bid_source_key(index),
                        error.code,
                    ));
                    None
                }
            },
            None => None,
        };
        available_bids.push(AvailableBid {
            index,
            bid,
            blind_bid,
        });
        Ok(())
    }

    fn persist_document(
        &self,
        job_id: &str,
        artifact_name: &str,
        role: FileRole,
        inspected: InspectedDocument,
    ) -> Result<PreparedFile, AppError> {
        self.normalized_writer
            .save_normalized(job_id, artifact_name, &inspected.document)?;
        Ok(prepared_file(role, inspected.inspection))
    }

    fn fail_manifest<T>(&self, manifest: &mut JobManifest, error: AppError) -> Result<T, AppError> {
        manifest.fail();
        self.store.save_atomic(manifest)?;
        Err(error)
    }
}

fn normalized_text(document: &crate::domain::document::NormalizedDocument) -> String {
    document
        .blocks
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

struct ReportArtifact {
    filename: String,
    title: String,
    markdown: String,
}

fn write_review_outputs(
    report_dir: &Path,
    requirements: &[Requirement],
    bid_results: &[Result<Vec<BidFinding>, String>],
    blind_documents: &[(usize, crate::domain::document::NormalizedDocument)],
    blind_bid_checks: &[crate::reports::BlindBidReport],
    duplicate_pairs: &[DuplicatePair],
    file_errors: &[FileErrorRecord],
) -> Result<(), AppError> {
    let mut artifacts = Vec::new();
    for (index, result) in bid_results.iter().enumerate() {
        let bid_index = index + 1;
        artifacts.push(ReportArtifact {
            filename: format!("{bid_index:02}-商务审核-投标文件{bid_index}.md"),
            title: format!("商务审核-投标文件{bid_index}"),
            markdown: business_review_markdown(bid_index, requirements, result),
        });
    }
    for blind in blind_bid_checks {
        artifacts.push(ReportArtifact {
            filename: format!(
                "1{:02}-技术暗标-投标文件{}.md",
                blind.bid_index, blind.bid_index
            ),
            title: format!("技术暗标-投标文件{}", blind.bid_index),
            markdown: blind_bid_markdown(blind.bid_index, &blind.check),
        });
    }
    artifacts.push(ReportArtifact {
        filename: "90-技术文件比对.md".into(),
        title: "技术文件比对".into(),
        markdown: comparison_markdown(blind_documents, duplicate_pairs),
    });
    let _ = file_errors;
    remove_deprecated_index(report_dir)?;
    for artifact in &artifacts {
        write_report_artifact(report_dir, artifact)?;
    }
    Ok(())
}

fn remove_deprecated_index(report_dir: &Path) -> Result<(), AppError> {
    let path = report_dir.join("00-审核结果索引.md");
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::io(error)),
    }
}

fn write_report_artifact(report_dir: &Path, artifact: &ReportArtifact) -> Result<(), AppError> {
    std::fs::write(report_dir.join(&artifact.filename), &artifact.markdown).map_err(|_| {
        AppError::new(
            ErrorCode::ReportGenerationFailed,
            format!("无法写入审核结果 Markdown：{}", artifact.title),
        )
    })
}

fn business_review_markdown(
    bid_index: usize,
    requirements: &[Requirement],
    result: &Result<Vec<BidFinding>, String>,
) -> String {
    let mut lines = vec![
        format!("# 商务审核-投标文件{bid_index}"),
        String::new(),
        "## 商务审核说明".into(),
        "✅ 本结果按商务审核清单组织，投标文件响应情况单独放入“投标响应核对”。".into(),
        String::new(),
        "## 商务线·废标".into(),
        "| ID | 类别 | 废标条款 | 出处(行号) | 必须材料 | 核实步骤 |".into(),
        "| --- | --- | --- | --- | --- | --- |".into(),
    ];
    append_business_rows(
        &mut lines,
        requirements,
        result,
        crate::review::schema::RequirementCategory::Disqualification,
        "B-D",
    );
    lines.extend([
        String::new(),
        "## 商务线·评分".into(),
        "| ID | 评分项 | 满分 | 评分梯度 | 必须材料 | 出处(行号) |".into(),
        "| --- | --- | --- | --- | --- | --- |".into(),
    ]);
    append_business_rows(
        &mut lines,
        requirements,
        result,
        crate::review::schema::RequirementCategory::Scoring,
        "B-S",
    );
    lines.extend([
        String::new(),
        "## 证明文件清册".into(),
        "| 类别 | 材料名称 | 来源(条款ID/行号) | 备注 |".into(),
        "| --- | --- | --- | --- |".into(),
    ]);
    append_business_rows(
        &mut lines,
        requirements,
        result,
        crate::review::schema::RequirementCategory::Evidence,
        "B-E",
    );
    lines.extend([
        String::new(),
        "## 关键时间节点".into(),
        "| 节点 | 时间 | 出处(行号) | 备注 |".into(),
        "| --- | --- | --- | --- |".into(),
    ]);
    append_business_rows(
        &mut lines,
        requirements,
        result,
        crate::review::schema::RequirementCategory::Timeline,
        "B-T",
    );
    lines.extend([
        String::new(),
        "## 合同条款·要点".into(),
        "| ID | 类别 | 条款要点 | 出处(行号) | 影响 | 备注 |".into(),
        "| --- | --- | --- | --- | --- | --- |".into(),
    ]);
    append_business_rows(
        &mut lines,
        requirements,
        result,
        crate::review::schema::RequirementCategory::Contract,
        "B-C",
    );
    lines.extend([
        String::new(),
        "## 投标响应核对".into(),
        "| 图标 | 要求ID | 状态 | 审核结论 | 投标证据 |".into(),
        "| --- | --- | --- | --- | --- |".into(),
    ]);
    append_finding_rows(&mut lines, requirements, result);
    lines.extend([
        String::new(),
        "⚠️ 本结果由模型语义判断辅助生成，需人工复核。".into(),
        String::new(),
        "保定移动政企部 zhaoyun_bd@he".into(),
    ]);
    lines.join("\n")
}

fn append_business_rows(
    lines: &mut Vec<String>,
    requirements: &[Requirement],
    result: &Result<Vec<BidFinding>, String>,
    category: crate::review::schema::RequirementCategory,
    fallback_prefix: &str,
) {
    let rows = requirements
        .iter()
        .filter(|requirement| requirement.category == category)
        .collect::<Vec<_>>();
    if rows.is_empty() {
        lines.push(empty_business_row(category));
        return;
    }
    for (index, requirement) in rows.iter().enumerate() {
        let finding = result.as_ref().ok().and_then(|findings| {
            findings
                .iter()
                .find(|item| item.requirement_id == requirement.id)
        });
        let id = if requirement.id.trim().is_empty() {
            format!("{fallback_prefix}{:03}", index + 1)
        } else {
            requirement.id.clone()
        };
        let conclusion = finding
            .map(|item| format!("{}：{}", finding_status_label(item.status), item.summary))
            .unwrap_or_else(|| match result {
                Ok(_) => "未返回该项审核发现，需人工复核".into(),
                Err(message) => format!("审核未完成：{message}"),
            });
        let bid_evidence = finding
            .map(|item| item.evidence.as_str())
            .unwrap_or("需人工复核");
        lines.push(business_row(
            category,
            &id,
            &requirement.title,
            &requirement.evidence,
            &conclusion,
            bid_evidence,
        ));
    }
}

fn empty_business_row(category: crate::review::schema::RequirementCategory) -> String {
    match category {
        crate::review::schema::RequirementCategory::Disqualification => {
            "| 未提取 | — | 未提取到商务废标要求 | — | — | — |".into()
        }
        crate::review::schema::RequirementCategory::Scoring => {
            "| 未提取 | 未提取到商务评分要求 | — | — | — | — |".into()
        }
        crate::review::schema::RequirementCategory::Evidence => {
            "| 未提取 | 未提取到证明材料要求 | — | — |".into()
        }
        crate::review::schema::RequirementCategory::Timeline => {
            "| 未提取到关键时间节点 | — | — | — |".into()
        }
        crate::review::schema::RequirementCategory::Contract => {
            "| 未提取 | — | 未提取到合同条款要点 | — | — | — |".into()
        }
        crate::review::schema::RequirementCategory::Technical => {
            "| 未提取 | — | 技术要求不在商务审核文件中输出 | — | — | — |".into()
        }
    }
}

fn business_row(
    category: crate::review::schema::RequirementCategory,
    id: &str,
    title: &str,
    evidence: &str,
    _conclusion: &str,
    _bid_evidence: &str,
) -> String {
    match category {
        crate::review::schema::RequirementCategory::Disqualification => format!(
            "| {} | 商务 | {} | {} | 按招标条款提供 | 逐项核验响应文件 |",
            markdown_cell(id),
            markdown_cell(title),
            markdown_cell(evidence)
        ),
        crate::review::schema::RequirementCategory::Scoring => format!(
            "| {} | {} | 见招标条款 | 逐档核验 | 按评分项提供 | {} |",
            markdown_cell(id),
            markdown_cell(title),
            markdown_cell(evidence)
        ),
        crate::review::schema::RequirementCategory::Evidence => format!(
            "| 证明材料 | {} | {} / {} | 按招标条款提供 |",
            markdown_cell(title),
            markdown_cell(id),
            markdown_cell(evidence)
        ),
        crate::review::schema::RequirementCategory::Timeline => format!(
            "| {} | 见招标条款 | {} | 按时间节点核验 |",
            markdown_cell(title),
            markdown_cell(evidence)
        ),
        crate::review::schema::RequirementCategory::Contract => format!(
            "| {} | 合同约束 | {} | {} | 中标后约束 | 按合同条款核验 |",
            markdown_cell(id),
            markdown_cell(title),
            markdown_cell(evidence)
        ),
        crate::review::schema::RequirementCategory::Technical => format!(
            "| {} | 技术 | {} | {} | 技术要求转入技术审核 | — |",
            markdown_cell(id),
            markdown_cell(title),
            markdown_cell(evidence)
        ),
    }
}

fn append_finding_rows(
    lines: &mut Vec<String>,
    requirements: &[Requirement],
    result: &Result<Vec<BidFinding>, String>,
) {
    match result {
        Ok(findings) if findings.is_empty() => {
            lines.push("| 🔎 | — | 需人工复核 | 模型未返回审核发现 | — |".into());
        }
        Ok(findings) => {
            lines.extend(findings.iter().map(|finding| {
                format!(
                    "| {} | {} | {} | {} | {} |",
                    finding_status_icon(finding.status),
                    markdown_cell(&finding.requirement_id),
                    finding_status_label(finding.status),
                    markdown_cell(&finding.summary),
                    markdown_cell(&finding.evidence)
                )
            }));
            let finding_ids = findings
                .iter()
                .map(|finding| finding.requirement_id.as_str())
                .collect::<std::collections::HashSet<_>>();
            lines.extend(
                requirements
                    .iter()
                    .filter(|requirement| !finding_ids.contains(requirement.id.as_str()))
                    .map(|requirement| {
                        format!(
                            "| 🔎 | {} | 需人工复核 | 未返回该项审核发现 | — |",
                            markdown_cell(&requirement.id)
                        )
                    }),
            );
        }
        Err(message) => lines.push(format!(
            "| ⚠️ | — | 审核未完成 | {} | — |",
            markdown_cell(message)
        )),
    }
}

fn finding_status_icon(status: crate::review::schema::FindingStatus) -> &'static str {
    match status {
        crate::review::schema::FindingStatus::Matched => "✅",
        crate::review::schema::FindingStatus::Missing => "❌",
        crate::review::schema::FindingStatus::Risk => "⚠️",
        crate::review::schema::FindingStatus::ManualReview => "🔎",
    }
}

fn finding_status_label(status: crate::review::schema::FindingStatus) -> &'static str {
    match status {
        crate::review::schema::FindingStatus::Matched => "已响应",
        crate::review::schema::FindingStatus::Missing => "缺失",
        crate::review::schema::FindingStatus::Risk => "风险",
        crate::review::schema::FindingStatus::ManualReview => "需人工复核",
    }
}

fn markdown_cell(value: &str) -> String {
    value
        .replace('|', "\\|")
        .replace('\n', "<br>")
        .trim()
        .to_owned()
}

fn blind_bid_markdown(bid_index: usize, check: &crate::review::blind_bid::BlindBidCheck) -> String {
    let mut lines = vec![
        format!("# 技术暗标-投标文件{bid_index}"),
        String::new(),
        "## 技术暗标格式检查".into(),
        format!("- 状态：{}", check.message),
    ];
    if check.findings.is_empty() {
        lines.push("- 未发现技术暗标格式风险".into());
    } else {
        lines.extend([
            String::new(),
            "| 问题编号范围 | 原始级别 | 处理级别 | 类别 | 规则 | 期望值 | 实际值 | 文本片段 | 修复建议 | 高级定位信息 |".into(),
            "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |".into(),
        ]);
        lines.extend(check.findings.iter().enumerate().map(|(index, finding)| {
            format!(
                "| 问题#{} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                index + 1,
                markdown_cell(&finding.raw_level),
                markdown_cell(&finding.biz_level),
                markdown_cell(&finding.category),
                markdown_cell(&finding.rule),
                markdown_cell(&finding.expected),
                markdown_cell(&finding.actual),
                markdown_cell(&finding.snippet),
                markdown_cell(&finding.note),
                markdown_cell(&finding.location)
            )
        }));
    }
    lines.extend([
        String::new(),
        "本结果为技术暗标格式辅助检查，需结合 Word 原文人工复核。".into(),
        String::new(),
        "保定移动政企部 zhaoyun_bd@he".into(),
    ]);
    lines.join("\n")
}

fn comparison_markdown(
    blind_documents: &[(usize, crate::domain::document::NormalizedDocument)],
    duplicate_pairs: &[DuplicatePair],
) -> String {
    let mut lines = vec![
        "# 技术文件比对".into(),
        String::new(),
        "## 检查范围".into(),
        format!(
            "本次仅比对已关联的技术暗标文件，共 {} 份；商务投标文件不参与文本查重。",
            blind_documents.len()
        ),
        String::new(),
        "## 判定方法".into(),
        "系统对技术暗标的文本块进行连续原文比对，仅输出长度达到阈值的重复片段及双方位置。结果用于提示人工复核，不直接判定围标、串标或废标。".into(),
        String::new(),
        "## 结论摘要".into(),
    ];
    if blind_documents.len() < 2 {
        lines.push("未关联至少两份技术文件，未执行比对。".into());
    } else if duplicate_pairs.is_empty() {
        lines.extend([
            "| 结论 | 说明 |".into(),
            "| --- | --- |".into(),
            "| 未发现需重点复核的连续重复内容 | 仍建议人工抽查关键技术方案、实施计划和项目组织章节。 |"
                .into(),
        ]);
    } else {
        lines.extend([
            "| 对比文件 | 连续重复片段数 | 复核重点 |".into(),
            "| --- | --- | --- |".into(),
        ]);
        for pair in duplicate_pairs {
            lines.push(format!(
                "| 投标文件 {} / 投标文件 {} | {} | 优先核对重复片段是否属于固定模板或投标人自编内容 |",
                pair.left_bid,
                pair.right_bid,
                pair.fragments.len()
            ));
            lines.extend([
                String::new(),
                format!(
                    "## 重复内容位置：投标文件 {} / 投标文件 {}",
                    pair.left_bid, pair.right_bid
                ),
                "| 重复片段 | 技术文件 A 位置 | 技术文件 B 位置 | 人工复核提示 |".into(),
                "| --- | --- | --- | --- |".into(),
            ]);
            lines.extend(pair.fragments.iter().map(|fragment| {
                format!(
                    "| {} | {} | {} | 核对是否为招标固定表述、通用模板或连续实质性重复内容 |",
                    markdown_cell(&fragment.text),
                    markdown_cell(&fragment.left_location),
                    markdown_cell(&fragment.right_location),
                )
            }));
        }
    }
    lines.extend([
        String::new(),
        "## 人工复核提示".into(),
        "建议重点查看相似度较高的文件组合，确认重复片段是否属于招标文件固定格式、通用技术术语、模板内容，还是投标人自编方案中的连续实质性重复。".into(),
        String::new(),
        "保定移动政企部 zhaoyun_bd@he".into(),
    ]);
    lines.join("\n")
}

fn report_title(path: &Path) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .map(|stem| {
            stem.split_once('-')
                .filter(|(prefix, _)| prefix.chars().all(|character| character.is_ascii_digit()))
                .map(|(_, title)| title)
                .unwrap_or(stem)
                .to_owned()
        })
        .unwrap_or_else(|| "审核结果".into())
}

fn report_kind(title: &str) -> &'static str {
    if title.starts_with("商务审核") {
        "business"
    } else if title.starts_with("技术暗标") {
        "blindBid"
    } else if title == "技术文件比对" {
        "comparison"
    } else {
        "other"
    }
}

fn recovery_error_message(code: crate::error::ErrorCode) -> &'static str {
    match code {
        crate::error::ErrorCode::CorruptJobManifest => "任务清单已损坏，原文件已保留",
        _ => "任务清单无法读取，原文件已保留",
    }
}

fn canonicalize_selected(path: &Path) -> Result<PathBuf, AppError> {
    std::fs::canonicalize(path).map_err(|_| {
        AppError::new(
            crate::error::ErrorCode::UnreadableDocument,
            "无法读取所选文档",
        )
    })
}

fn reject_duplicate_existing_paths(input: &JobInput, tender_path: &Path) -> Result<(), AppError> {
    let mut seen = HashSet::from([tender_path.to_path_buf()]);
    for bid in &input.bids {
        for path in std::iter::once(&bid.bid_path).chain(bid.blind_bid_path.iter()) {
            if let Ok(canonical_path) = canonicalize_selected(Path::new(path)) {
                if !seen.insert(canonical_path) {
                    return Err(AppError::duplicate_input_file());
                }
            }
        }
    }
    Ok(())
}

fn prepared_file(role: FileRole, inspection: FileInspection) -> PreparedFile {
    PreparedFile {
        display_name: inspection.display_name,
        role,
        format: inspection.format,
        byte_size: inspection.byte_size,
        sha256: inspection.sha256,
        block_count: inspection.block_count,
    }
}

fn file_error(
    source_name: String,
    source_key: String,
    code: crate::error::ErrorCode,
) -> FileErrorRecord {
    FileErrorRecord {
        source_name,
        source_key,
        code,
        message: file_error_message(code).into(),
    }
}

fn file_error_message(code: crate::error::ErrorCode) -> &'static str {
    use crate::error::ErrorCode;
    match code {
        ErrorCode::UnsupportedExtension => "仅支持 PDF 或 DOCX 文件",
        ErrorCode::BlindBidMustBeDocx => "技术暗标附件仅支持 DOCX 文件",
        ErrorCode::TextNotExtractable => "文档不包含可提取文本",
        ErrorCode::EncryptedDocument => "文档已加密，无法提取",
        ErrorCode::InvalidDocx => "DOCX 文件无效",
        ErrorCode::DocumentChangedDuringRead => "文档在读取期间发生变化",
        ErrorCode::JobPersistenceFailed => "无法保存本地任务数据",
        _ => "文档无法读取",
    }
}

fn bid_source_key(index: usize) -> String {
    format!("bid:{}", index - 1)
}

fn blind_bid_source_key(index: usize) -> String {
    format!("blind:{}", index - 1)
}

fn stored_file(artifact_name: &str, file: &PreparedFile) -> StoredFileMetadata {
    StoredFileMetadata {
        artifact_name: artifact_name.to_owned(),
        display_name: file.display_name.clone(),
        role: match file.role {
            FileRole::Tender => StoredFileRole::Tender,
            FileRole::Bid => StoredFileRole::Bid,
            FileRole::BlindBid => StoredFileRole::BlindBid,
        },
        format: match file.format {
            FileFormat::Pdf => StoredFileFormat::Pdf,
            FileFormat::Docx => StoredFileFormat::Docx,
        },
        byte_size: file.byte_size,
        sha256: file.sha256.clone(),
        block_count: file.block_count,
    }
}

fn source_names(input: &JobInput) -> Vec<String> {
    let mut names = vec![display_name(Path::new(&input.tender_path))];
    for bid in &input.bids {
        names.push(display_name(Path::new(&bid.bid_path)));
        if let Some(blind_bid_path) = &bid.blind_bid_path {
            names.push(display_name(Path::new(blind_bid_path)));
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::job::{BidInput, JobInput, JobState},
        error::ErrorCode,
        jobs::store::JobStore,
    };
    use std::{
        io::Write,
        sync::{Arc, Mutex},
    };

    #[test]
    fn rejects_five_bids_at_the_service_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let bids = (1..=5)
            .map(|number| BidInput::new(format!("bid-{number}.pdf"), None))
            .collect();

        let error = JobService::new(JobStore::new(temp.path().join("jobs")))
            .prepare(JobInput {
                tender_path: "tender.pdf".into(),
                bids,
            })
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::BidCountOutOfRange);
    }

    #[test]
    fn rejects_duplicate_canonical_source_paths_before_parsing() {
        let temp = tempfile::tempdir().unwrap();
        let shared = temp.path().join("same.docx");
        std::fs::write(&shared, b"not parsed because it is duplicated").unwrap();
        let path = shared.to_string_lossy().into_owned();
        let input = JobInput::new(path.clone(), vec![BidInput::new(path, None)]).unwrap();

        let error = JobService::new(JobStore::new(temp.path().join("jobs")))
            .prepare(input)
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::DuplicateInputFile);
    }

    #[test]
    fn keeps_a_readable_bid_when_another_bid_cannot_be_extracted() {
        let temp = tempfile::tempdir().unwrap();
        let tender = write_docx(temp.path().join("tender.docx"), "招标要求");
        let readable_bid = write_docx(temp.path().join("bid-1.docx"), "投标响应");
        let unreadable_bid = temp.path().join("bid-2.pdf");
        std::fs::write(&unreadable_bid, b"not a PDF").unwrap();
        let input = JobInput::new(
            tender.to_string_lossy().into_owned(),
            vec![
                BidInput::new(readable_bid.to_string_lossy().into_owned(), None),
                BidInput::new(unreadable_bid.to_string_lossy().into_owned(), None),
            ],
        )
        .unwrap();

        let prepared = JobService::new(JobStore::new(temp.path().join("jobs")))
            .prepare(input)
            .unwrap();

        assert_eq!(prepared.state, JobState::ReadyWithErrors);
        assert!(prepared
            .files
            .iter()
            .any(|file| file.display_name == "bid-1.docx"));
        assert!(prepared.errors.iter().any(|error| {
            error.source_name == "bid-2.pdf"
                && error.source_key == "bid:1"
                && error.code == ErrorCode::UnreadableDocument
        }));
    }

    #[test]
    fn keeps_a_valid_bid_when_a_second_bid_path_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let tender = write_docx(temp.path().join("tender.docx"), "招标要求");
        let readable_bid = write_docx(temp.path().join("bid-1.docx"), "投标响应");
        let missing_bid = temp.path().join("missing-bid.docx");
        let input = JobInput::new(
            tender.to_string_lossy().into_owned(),
            vec![
                BidInput::new(readable_bid.to_string_lossy().into_owned(), None),
                BidInput::new(missing_bid.to_string_lossy().into_owned(), None),
            ],
        )
        .unwrap();

        let prepared = JobService::new(JobStore::new(temp.path().join("jobs")))
            .prepare(input)
            .unwrap();

        assert_eq!(prepared.state, JobState::ReadyWithErrors);
        assert!(prepared
            .files
            .iter()
            .any(|file| file.display_name == "bid-1.docx"));
        assert!(prepared.errors.iter().any(|error| {
            error.source_name == "missing-bid.docx" && error.code == ErrorCode::UnreadableDocument
        }));
    }

    #[test]
    fn keeps_a_valid_main_bid_when_its_blind_bid_path_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let tender = write_docx(temp.path().join("tender.docx"), "招标要求");
        let bid = write_docx(temp.path().join("bid.docx"), "投标响应");
        let missing_blind = temp.path().join("missing-blind.docx");
        let input = JobInput::new(
            tender.to_string_lossy().into_owned(),
            vec![BidInput::new(
                bid.to_string_lossy().into_owned(),
                Some(missing_blind.to_string_lossy().into_owned()),
            )],
        )
        .unwrap();

        let prepared = JobService::new(JobStore::new(temp.path().join("jobs")))
            .prepare(input)
            .unwrap();

        assert_eq!(prepared.state, JobState::ReadyWithErrors);
        assert!(prepared
            .files
            .iter()
            .any(|file| file.display_name == "bid.docx"));
        assert!(prepared.errors.iter().any(|error| {
            error.source_name == "missing-blind.docx" && error.code == ErrorCode::UnreadableDocument
        }));
    }

    #[test]
    fn prepares_one_tender_four_bids_and_four_blind_bids_with_manifest_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let tender = write_docx(temp.path().join("tender.docx"), "招标要求");
        let mut bids = Vec::new();
        for index in 1..=4 {
            let bid = write_docx(
                temp.path().join(format!("bid-{index}.docx")),
                &format!("投标响应 {index}"),
            );
            let blind_bid = write_docx(
                temp.path().join(format!("blind-{index}.docx")),
                &format!("技术暗标 {index}"),
            );
            bids.push(BidInput::new(
                bid.to_string_lossy().into_owned(),
                Some(blind_bid.to_string_lossy().into_owned()),
            ));
        }
        let store = JobStore::new(temp.path().join("jobs"));
        let input = JobInput::new(tender.to_string_lossy().into_owned(), bids).unwrap();

        let prepared = JobService::new(store.clone()).prepare(input).unwrap();

        assert_eq!(prepared.state, JobState::Ready);
        assert!(prepared.errors.is_empty());
        assert_eq!(prepared.files.len(), 9);
        assert_eq!(
            prepared
                .files
                .iter()
                .filter(|file| file.role == FileRole::Tender)
                .count(),
            1
        );
        assert_eq!(
            prepared
                .files
                .iter()
                .filter(|file| file.role == FileRole::Bid)
                .count(),
            4
        );
        assert_eq!(
            prepared
                .files
                .iter()
                .filter(|file| file.role == FileRole::BlindBid)
                .count(),
            4
        );
        assert!(prepared.files.iter().all(|file| {
            file.format == FileFormat::Docx
                && file.byte_size > 0
                && file.block_count == 1
                && file.sha256.len() == 64
        }));

        let manifest = store.load(&prepared.job_id).unwrap();
        assert_eq!(manifest.state, JobState::Ready);
        assert_eq!(manifest.completed_files.len(), 9);
        assert_eq!(manifest.completed_files[0].artifact_name, "tender");
        for index in 1..=4 {
            let bid = &manifest.input.bids[index - 1];
            assert!(bid.bid_path.ends_with(&format!("bid-{index}.docx")));
            assert!(bid
                .blind_bid_path
                .as_deref()
                .is_some_and(|path| path.ends_with(&format!("blind-{index}.docx"))));
            assert!(manifest
                .completed_files
                .iter()
                .any(|file| file.artifact_name == format!("bid-{index}")));
            assert!(manifest
                .completed_files
                .iter()
                .any(|file| file.artifact_name == format!("blind-bid-{index}")));
        }
    }

    #[test]
    fn review_outputs_separate_markdown_files_and_compares_only_blind_bids() {
        let temp = tempfile::tempdir().unwrap();
        let tender = write_docx(temp.path().join("tender.docx"), "营业执照要求");
        let bid_1 = write_docx(temp.path().join("bid-1.docx"), "商务共同段 营业执照响应");
        let bid_2 = write_docx(temp.path().join("bid-2.docx"), "商务共同段 营业执照响应");
        let blind_1 = write_docx(temp.path().join("blind-1.docx"), "技术暗标共同段 方案甲");
        let blind_2 = write_docx(temp.path().join("blind-2.docx"), "技术暗标共同段 方案乙");
        let store = JobStore::new(temp.path().join("jobs"));
        let prepared = JobService::new(store.clone())
            .prepare(
                JobInput::new(
                    tender.to_string_lossy().into_owned(),
                    vec![
                        BidInput::new(
                            bid_1.to_string_lossy().into_owned(),
                            Some(blind_1.to_string_lossy().into_owned()),
                        ),
                        BidInput::new(
                            bid_2.to_string_lossy().into_owned(),
                            Some(blind_2.to_string_lossy().into_owned()),
                        ),
                    ],
                )
                .unwrap(),
            )
            .unwrap();
        let client = FixedResponses::new(vec![
            r#"[{"id":"R1","category":"evidence","title":"营业执照","evidence":"招标文件行1"}]"#,
            r#"[{"requirementId":"R1","status":"matched","summary":"已响应","evidence":"投标文件行1"}]"#,
            r#"[{"requirementId":"R1","status":"matched","summary":"已响应","evidence":"投标文件行1"}]"#,
        ]);

        let status = JobService::new(store.clone())
            .run_review(&prepared.job_id, client)
            .unwrap();

        assert_eq!(
            status.stages[&crate::domain::job::JobStage::ModelTest],
            crate::domain::job::StageState::Complete
        );
        let titles: Vec<_> = status
            .report_files
            .iter()
            .map(|file| file.title.as_str())
            .collect();
        assert!(!titles.contains(&"审核结果索引"));
        assert!(titles.contains(&"商务审核-投标文件1"));
        assert!(titles.contains(&"商务审核-投标文件2"));
        assert!(titles.contains(&"技术暗标-投标文件1"));
        assert!(titles.contains(&"技术暗标-投标文件2"));
        assert!(titles.contains(&"技术文件比对"));

        let business = status
            .report_files
            .iter()
            .find(|file| file.title == "商务审核-投标文件1")
            .unwrap();
        assert!(business.markdown.contains("## 商务审核说明"));
        assert!(!business.markdown.contains("tender-review-skill"));
        assert!(business
            .markdown
            .contains("| ID | 类别 | 废标条款 | 出处(行号) | 必须材料 | 核实步骤 |"));
        assert!(business.markdown.contains("## 证明文件清册"));
        assert!(business.markdown.contains("## 投标响应核对"));
        let blind_bid = status
            .report_files
            .iter()
            .find(|file| file.title == "技术暗标-投标文件1")
            .unwrap();
        assert!(blind_bid.markdown.contains("## 技术暗标格式检查"));
        assert!(!blind_bid.markdown.contains("word-format-checker"));
        assert!(blind_bid.markdown.contains("问题编号范围"));
        assert!(blind_bid.markdown.contains("处理级别"));
        assert!(blind_bid.markdown.contains("文本片段"));

        let comparison = status
            .report_files
            .iter()
            .find(|file| file.title == "技术文件比对")
            .unwrap();
        assert!(comparison
            .markdown
            .contains("未发现需重点复核的连续重复内容"));
        assert!(!comparison.markdown.contains("技术暗标共同段"));
        assert!(comparison.markdown.contains("## 检查范围"));
        assert!(comparison.markdown.contains("## 判定方法"));
        assert!(comparison.markdown.contains("## 人工复核提示"));
        assert!(!comparison.markdown.contains("商务共同段"));
    }

    #[test]
    fn marks_the_manifest_failed_when_all_main_bid_paths_are_missing() {
        let temp = tempfile::tempdir().unwrap();
        let tender = write_docx(temp.path().join("tender.docx"), "招标要求");
        let store = JobStore::new(temp.path().join("jobs"));
        let input = JobInput::new(
            tender.to_string_lossy().into_owned(),
            vec![BidInput::new(
                temp.path()
                    .join("missing-bid.docx")
                    .to_string_lossy()
                    .into_owned(),
                None,
            )],
        )
        .unwrap();

        let error = JobService::new(store.clone()).prepare(input).unwrap_err();

        assert_eq!(error.code, ErrorCode::NoReadableBids);
        assert_eq!(error.file_errors.len(), 1);
        assert_eq!(error.file_errors[0].source_key, "bid:0");
        assert_eq!(error.file_errors[0].source_name, "missing-bid.docx");
        assert_eq!(error.file_errors[0].code, ErrorCode::UnreadableDocument);
        assert!(!error.file_errors[0].message.is_empty());
        let serialized = serde_json::to_value(&error).unwrap();
        assert_eq!(serialized["fileErrors"][0]["sourceKey"], "bid:0");
        assert_eq!(
            serialized["fileErrors"][0]["sourceName"],
            "missing-bid.docx"
        );
        let manifest = store.list().unwrap().pop().unwrap();
        assert_eq!(manifest.state, JobState::Failed);
    }

    #[test]
    fn recoverable_jobs_keep_valid_summaries_when_another_manifest_is_corrupt() {
        let temp = tempfile::tempdir().unwrap();
        let jobs_root = temp.path().join("jobs");
        let store = JobStore::new(jobs_root.clone());
        let mut valid = JobManifest::new(
            JobInput::new(
                "safe-tender.docx".into(),
                vec![BidInput::new("safe-bid.docx".into(), None)],
            )
            .unwrap(),
        );
        valid.id = "valid-job".into();
        valid.finish(JobState::Failed);
        store.save_atomic(&valid).unwrap();
        let corrupt_dir = jobs_root.join("corrupt-job");
        std::fs::create_dir_all(&corrupt_dir).unwrap();
        std::fs::write(corrupt_dir.join("manifest.json"), "not-json").unwrap();

        let recovered = JobService::new(store).list_recoverable().unwrap();

        assert_eq!(recovered.jobs.len(), 1);
        assert_eq!(recovered.jobs[0].job_id, "valid-job");
        assert_eq!(recovered.errors.len(), 1);
        assert_eq!(recovered.errors[0].job_display, "corrupt-job");
        assert_eq!(recovered.errors[0].code, ErrorCode::CorruptJobManifest);
        assert!(!recovered.errors[0]
            .message
            .contains(&temp.path().display().to_string()));
        assert!(corrupt_dir.join("manifest.json").exists());
    }

    #[test]
    fn missing_manifest_io_uses_job_persistence_error() {
        let temp = tempfile::tempdir().unwrap();
        let error = JobStore::new(temp.path().join("jobs"))
            .load("missing-job")
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::JobPersistenceFailed);
    }

    #[test]
    fn marks_the_manifest_failed_when_tender_normalization_cannot_be_written() {
        let temp = tempfile::tempdir().unwrap();
        let tender = write_docx(temp.path().join("tender.docx"), "招标要求");
        let bid = write_docx(temp.path().join("bid.docx"), "投标响应");
        let store = JobStore::new(temp.path().join("jobs"));
        let input = JobInput::new(
            tender.to_string_lossy().into_owned(),
            vec![BidInput::new(bid.to_string_lossy().into_owned(), None)],
        )
        .unwrap();

        let error = JobService::with_normalized_writer(store.clone(), Arc::new(FailingWriter))
            .prepare(input)
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::JobPersistenceFailed);
        let manifest = store.list().unwrap().pop().unwrap();
        assert_eq!(manifest.state, JobState::Failed);
    }

    struct FailingWriter;

    impl NormalizedDocumentWriter for FailingWriter {
        fn save_normalized(
            &self,
            _job_id: &str,
            _artifact_name: &str,
            _document: &crate::domain::document::NormalizedDocument,
        ) -> Result<(), AppError> {
            Err(AppError::job_persistence())
        }
    }

    struct FixedResponses {
        responses: Arc<Mutex<Vec<String>>>,
    }

    impl FixedResponses {
        fn new(responses: Vec<&str>) -> Self {
            Self {
                responses: Arc::new(Mutex::new(
                    responses.into_iter().map(str::to_owned).collect(),
                )),
            }
        }
    }

    impl StructuredModelClient for FixedResponses {
        fn complete(&self, _prompt: &str) -> Result<String, AppError> {
            Ok(self.responses.lock().unwrap().remove(0))
        }
    }

    fn write_docx(path: std::path::PathBuf, text: &str) -> std::path::PathBuf {
        let file = std::fs::File::create(&path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file::<_, ()>("[Content_Types].xml", zip::write::FileOptions::default())
            .unwrap();
        write!(
            archive,
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#
        )
        .unwrap();
        archive
            .start_file::<_, ()>("_rels/.rels", zip::write::FileOptions::default())
            .unwrap();
        write!(
            archive,
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#
        )
        .unwrap();
        archive
            .start_file::<_, ()>("word/document.xml", zip::write::FileOptions::default())
            .unwrap();
        write!(
            archive,
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:body></w:document>"#
        )
        .unwrap();
        archive.finish().unwrap();
        path
    }
}
