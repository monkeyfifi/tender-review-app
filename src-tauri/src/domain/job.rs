use crate::error::{AppError, ErrorCode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BidInput {
    pub bid_path: String,
    pub blind_bid_path: Option<String>,
}

impl BidInput {
    pub fn new(bid_path: String, blind_bid_path: Option<String>) -> Self {
        Self {
            bid_path,
            blind_bid_path,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobInput {
    pub tender_path: String,
    pub bids: Vec<BidInput>,
}

impl JobInput {
    pub fn new(tender_path: String, bids: Vec<BidInput>) -> Result<Self, AppError> {
        if !(1..=4).contains(&bids.len()) {
            return Err(AppError::new(
                ErrorCode::BidCountOutOfRange,
                "投标文件数量必须为 1–4 份",
            ));
        }
        Ok(Self { tender_path, bids })
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if !(1..=4).contains(&self.bids.len()) {
            return Err(AppError::new(
                ErrorCode::BidCountOutOfRange,
                "投标文件数量必须为 1–4 份",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobStage {
    Preflight,
    Extract,
    ModelTest,
    TenderReview,
    BidReview,
    DuplicateCheck,
    BlindBidCheck,
    Report,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StageState {
    Pending,
    Running,
    Complete,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobState {
    Draft,
    Preparing,
    Ready,
    ReadyWithErrors,
    Completed,
    CompletedWithIssues,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobManifest {
    pub id: String,
    pub input: JobInput,
    pub state: JobState,
    pub stages: BTreeMap<JobStage, StageState>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub failure_code: Option<ErrorCode>,
    #[serde(default)]
    pub file_errors: Vec<FileErrorRecord>,
    #[serde(default)]
    pub completed_files: Vec<StoredFileMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileErrorRecord {
    #[serde(alias = "displayName")]
    pub source_name: String,
    #[serde(default)]
    pub source_key: String,
    pub code: ErrorCode,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredFileMetadata {
    pub artifact_name: String,
    pub display_name: String,
    pub role: StoredFileRole,
    pub format: StoredFileFormat,
    pub byte_size: u64,
    pub sha256: String,
    pub block_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StoredFileRole {
    Tender,
    Bid,
    BlindBid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StoredFileFormat {
    Pdf,
    Docx,
}

impl JobManifest {
    pub fn new(input: JobInput) -> Self {
        let now = Utc::now();
        let mut stages = BTreeMap::new();
        stages.insert(JobStage::Preflight, StageState::Pending);
        stages.insert(JobStage::Extract, StageState::Pending);
        stages.insert(JobStage::ModelTest, StageState::Pending);
        stages.insert(JobStage::TenderReview, StageState::Pending);
        stages.insert(JobStage::BidReview, StageState::Pending);
        stages.insert(JobStage::DuplicateCheck, StageState::Pending);
        stages.insert(JobStage::BlindBidCheck, StageState::Pending);
        stages.insert(JobStage::Report, StageState::Pending);
        Self {
            id: Uuid::new_v4().to_string(),
            input,
            state: JobState::Draft,
            stages,
            created_at: now,
            updated_at: now,
            failure_code: None,
            file_errors: Vec::new(),
            completed_files: Vec::new(),
        }
    }

    pub fn begin(&mut self, stage: JobStage) {
        self.state = JobState::Preparing;
        self.stages.insert(stage, StageState::Running);
        self.updated_at = Utc::now();
    }

    pub fn complete(&mut self, stage: JobStage) {
        self.stages.insert(stage, StageState::Complete);
        self.updated_at = Utc::now();
    }

    pub fn fail(&mut self) {
        self.state = JobState::Failed;
        for state in self.stages.values_mut() {
            if matches!(state, StageState::Running) {
                *state = StageState::Failed;
            }
        }
        self.updated_at = Utc::now();
    }

    pub fn finish(&mut self, state: JobState) {
        for stage in self.stages.values_mut() {
            if matches!(stage, StageState::Running) {
                *stage = match state {
                    JobState::Failed => StageState::Failed,
                    JobState::Cancelled => StageState::Cancelled,
                    _ => StageState::Complete,
                };
            }
        }
        self.state = state;
        self.updated_at = Utc::now();
    }
}

impl JobState {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Ready
                | Self::ReadyWithErrors
                | Self::Completed
                | Self::CompletedWithIssues
                | Self::Failed
                | Self::Cancelled
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_more_than_four_bids() {
        let bids = (0..5)
            .map(|i| BidInput::new(format!("bid-{i}.pdf"), None))
            .collect();
        let error = JobInput::new("tender.pdf".into(), bids).unwrap_err();
        assert_eq!(error.code, ErrorCode::BidCountOutOfRange);
    }

    #[test]
    fn finishing_a_job_never_leaves_a_stage_running() {
        let input = JobInput::new(
            "tender.docx".into(),
            vec![BidInput::new("bid.docx".into(), None)],
        )
        .unwrap();
        let mut manifest = JobManifest::new(input);
        manifest.begin(JobStage::TenderReview);
        manifest.finish(JobState::Completed);

        assert!(manifest.state.is_terminal());
        assert_eq!(
            manifest.stages[&JobStage::TenderReview],
            StageState::Complete
        );
    }
}
