use crate::{
    domain::{
        document::NormalizedDocument,
        job::{JobManifest, JobState, StageState},
    },
    error::AppError,
    jobs::cleanup::is_expired,
};
use chrono::{DateTime, Utc};
use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};

const MANIFEST_FILE: &str = "manifest.json";
const TEMPORARY_MANIFEST_FILE: &str = "manifest.json.tmp";

#[derive(Debug, Clone)]
pub struct JobStore {
    root: PathBuf,
}

pub struct JobScanError {
    pub job_display: String,
    pub error: AppError,
}

pub struct JobScan {
    pub manifests: Vec<JobManifest>,
    pub errors: Vec<JobScanError>,
}

impl JobStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn create(&self, manifest: &JobManifest) -> Result<(), AppError> {
        self.save_atomic(manifest)
    }

    pub fn load(&self, job_id: &str) -> Result<JobManifest, AppError> {
        let path = self.manifest_path(job_id)?;
        let bytes = fs::read(path).map_err(AppError::io)?;
        serde_json::from_slice(&bytes).map_err(AppError::corrupt_job_manifest)
    }

    pub fn save_atomic(&self, manifest: &JobManifest) -> Result<(), AppError> {
        let dir = self.job_directory(&manifest.id)?;
        fs::create_dir_all(&dir).map_err(AppError::io)?;
        let target = dir.join(MANIFEST_FILE);
        let temporary = dir.join(TEMPORARY_MANIFEST_FILE);
        let bytes = serde_json::to_vec_pretty(manifest).map_err(AppError::serialization)?;
        let mut file = fs::File::create(&temporary).map_err(AppError::io)?;
        file.write_all(&bytes).map_err(AppError::io)?;
        file.sync_all().map_err(AppError::io)?;
        drop(file);
        fs::rename(&temporary, &target).map_err(AppError::io)
    }

    pub fn list(&self) -> Result<Vec<JobManifest>, AppError> {
        Ok(self.scan()?.manifests)
    }

    pub fn scan(&self) -> Result<JobScan, AppError> {
        if !self.root.exists() {
            return Ok(JobScan {
                manifests: Vec::new(),
                errors: Vec::new(),
            });
        }

        let mut scan = JobScan {
            manifests: Vec::new(),
            errors: Vec::new(),
        };
        for entry in fs::read_dir(&self.root).map_err(AppError::io)? {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    scan.errors.push(JobScanError {
                        job_display: "无法识别的任务".into(),
                        error: AppError::io(error),
                    });
                    continue;
                }
            };
            let job_display = entry.file_name().to_string_lossy().into_owned();
            let is_directory = match entry.file_type() {
                Ok(file_type) => file_type.is_dir(),
                Err(error) => {
                    scan.errors.push(JobScanError {
                        job_display,
                        error: AppError::io(error),
                    });
                    continue;
                }
            };
            if !is_directory {
                continue;
            }
            match self.load_entry(&entry.path()) {
                Ok(manifest) if manifest.id == job_display => scan.manifests.push(manifest),
                Ok(_) => scan.errors.push(JobScanError {
                    job_display,
                    error: AppError::corrupt_job_manifest("任务目录与清单 ID 不一致"),
                }),
                Err(error) => scan.errors.push(JobScanError { job_display, error }),
            }
        }
        Ok(scan)
    }

    pub fn mark_interrupted(&self, now: DateTime<Utc>) -> Result<(), AppError> {
        for mut manifest in self.scan()?.manifests {
            if !manifest.state.is_terminal() {
                manifest.state = JobState::Failed;
                manifest.failure_code = Some(crate::error::ErrorCode::JobInterrupted);
                for stage in manifest.stages.values_mut() {
                    if matches!(stage, StageState::Running) {
                        *stage = StageState::Failed;
                    }
                }
                manifest.updated_at = now;
                self.save_atomic(&manifest)?;
            }
        }
        Ok(())
    }

    pub fn remove(&self, job_id: &str) -> Result<(), AppError> {
        let directory = self.job_directory(job_id)?;
        if directory.exists() {
            fs::remove_dir_all(directory).map_err(AppError::io)?;
        }
        Ok(())
    }

    pub fn save_normalized(
        &self,
        job_id: &str,
        artifact_name: &str,
        document: &NormalizedDocument,
    ) -> Result<(), AppError> {
        if !is_safe_artifact_name(artifact_name) {
            return Err(AppError::job_persistence());
        }
        let extracted = self.job_directory(job_id)?.join("extracted");
        fs::create_dir_all(&extracted).map_err(|_| AppError::job_persistence())?;
        let target = extracted.join(format!("{artifact_name}.json"));
        let temporary = extracted.join(format!("{artifact_name}.json.tmp"));
        let bytes = serde_json::to_vec(document).map_err(|_| AppError::job_persistence())?;
        let mut file = fs::File::create(&temporary).map_err(|_| AppError::job_persistence())?;
        file.write_all(&bytes)
            .map_err(|_| AppError::job_persistence())?;
        file.sync_all().map_err(|_| AppError::job_persistence())?;
        drop(file);
        fs::rename(&temporary, &target).map_err(|_| AppError::job_persistence())
    }

    pub fn load_normalized(
        &self,
        job_id: &str,
        artifact_name: &str,
    ) -> Result<NormalizedDocument, AppError> {
        if !is_safe_artifact_name(artifact_name) {
            return Err(AppError::job_persistence());
        }
        let path = self
            .job_directory(job_id)?
            .join("extracted")
            .join(format!("{artifact_name}.json"));
        serde_json::from_slice(&fs::read(path).map_err(AppError::io)?)
            .map_err(AppError::corrupt_job_manifest)
    }

    pub fn report_path(&self, job_id: &str) -> Result<PathBuf, AppError> {
        Ok(self
            .job_directory(job_id)?
            .join("report")
            .join("01-商务审核-投标文件1.md"))
    }

    pub fn report_directory(&self, job_id: &str) -> Result<PathBuf, AppError> {
        Ok(self.job_directory(job_id)?.join("report"))
    }

    pub fn remove_expired(&self, now: DateTime<Utc>) -> Result<(), AppError> {
        if !self.root.exists() {
            return Ok(());
        }

        for entry in fs::read_dir(&self.root).map_err(AppError::io)? {
            let entry = entry.map_err(AppError::io)?;
            if !entry.file_type().map_err(AppError::io)?.is_dir() {
                continue;
            }
            let directory = entry.path();
            let manifest = match self.load_entry(&directory) {
                Ok(manifest) => manifest,
                Err(_) => continue,
            };
            let entry_id = match directory.file_name().and_then(|name| name.to_str()) {
                Some(id) => id,
                None => continue,
            };
            if manifest.id != entry_id {
                continue;
            }
            if is_expired(&manifest, now) {
                fs::remove_dir_all(directory).map_err(AppError::io)?;
            }
        }
        Ok(())
    }

    fn manifest_path(&self, job_id: &str) -> Result<PathBuf, AppError> {
        Ok(self.job_directory(job_id)?.join(MANIFEST_FILE))
    }

    fn job_directory(&self, job_id: &str) -> Result<PathBuf, AppError> {
        if is_safe_job_id(job_id) {
            Ok(self.root.join(job_id))
        } else {
            Err(AppError::invalid_job_id(job_id))
        }
    }

    fn load_entry(&self, directory: &Path) -> Result<JobManifest, AppError> {
        let path = directory.join(MANIFEST_FILE);
        let bytes = fs::read(path).map_err(AppError::io)?;
        serde_json::from_slice(&bytes).map_err(AppError::corrupt_job_manifest)
    }
}

fn is_safe_job_id(job_id: &str) -> bool {
    !job_id.is_empty()
        && !job_id.contains(['/', '\\'])
        && matches!(
            Path::new(job_id).components().next(),
            Some(Component::Normal(_))
        )
        && Path::new(job_id).components().count() == 1
}

fn is_safe_artifact_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['/', '\\'])
        && matches!(
            Path::new(name).components().next(),
            Some(Component::Normal(_))
        )
        && Path::new(name).components().count() == 1
}
