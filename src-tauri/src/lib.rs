// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
pub mod commands;
pub mod config;
pub mod documents;
pub mod domain;
pub mod error;
pub mod jobs;
pub mod model_client;
pub mod reports;
pub mod review;

use chrono::{DateTime, Utc};
use jobs::{service::JobService, store::JobStore};
use std::path::PathBuf;
use tauri::Manager;

pub struct AppState {
    pub jobs: JobService,
}

fn initialize_job_service(jobs_directory: PathBuf) -> Result<JobService, crate::error::AppError> {
    initialize_job_service_at(jobs_directory, Utc::now())
}

fn initialize_job_service_at(
    jobs_directory: PathBuf,
    now: DateTime<Utc>,
) -> Result<JobService, crate::error::AppError> {
    std::fs::create_dir_all(&jobs_directory).map_err(crate::error::AppError::io)?;
    let store = JobStore::new(jobs_directory);
    store.mark_interrupted(now)?;
    store.remove_expired(now)?;
    Ok(JobService::new(store))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let jobs_directory = app.path().app_local_data_dir()?.join("jobs");
            app.manage(AppState {
                jobs: initialize_job_service(jobs_directory)?,
            });
            Ok(())
        })
        .manage(commands::config::ModelConfigurationState::default())
        .invoke_handler(tauri::generate_handler![
            commands::config::save_model_settings,
            commands::config::get_model_settings,
            commands::config::get_model_provider_presets,
            commands::config::clear_model_key,
            commands::config::test_model_connection,
            commands::environment::get_local_environment_status,
            commands::environment::open_local_environment_setup,
            commands::jobs::prepare_job,
            commands::jobs::list_recoverable_jobs,
            commands::jobs::clear_job,
            commands::jobs::run_review,
            commands::jobs::get_review_status,
            commands::jobs::open_report_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::job::{BidInput, JobInput, JobManifest, JobState};
    use chrono::{Duration, Utc};

    #[test]
    fn startup_cleanup_removes_only_expired_terminal_jobs_and_keeps_corrupt_data() {
        let temp = tempfile::tempdir().unwrap();
        let jobs_directory = temp.path().join("jobs");
        let store = JobStore::new(jobs_directory.clone());
        let now = Utc::now();

        for (id, state, age_hours) in [
            ("expired-failed", JobState::Failed, 25),
            ("expired-cancelled", JobState::Cancelled, 25),
            ("recent-failed", JobState::Failed, 23),
            ("ready-with-errors", JobState::ReadyWithErrors, 25),
        ] {
            store
                .save_atomic(&manifest(id, state, now - Duration::hours(age_hours)))
                .unwrap();
        }
        let corrupt_directory = jobs_directory.join("corrupt-job");
        std::fs::create_dir_all(&corrupt_directory).unwrap();
        std::fs::write(corrupt_directory.join("manifest.json"), "not valid json").unwrap();

        let initialized = initialize_job_service_at(jobs_directory.clone(), now);

        assert!(initialized.is_ok());
        assert!(!jobs_directory.join("expired-failed").exists());
        assert!(!jobs_directory.join("expired-cancelled").exists());
        assert!(jobs_directory.join("recent-failed").exists());
        assert!(jobs_directory.join("ready-with-errors").exists());
        assert!(corrupt_directory.exists());
        assert!(corrupt_directory.join("manifest.json").exists());
    }

    #[test]
    fn startup_marks_interrupted_preparing_job_failed_then_expires_it_after_24_hours() {
        let temp = tempfile::tempdir().unwrap();
        let jobs_directory = temp.path().join("jobs");
        let store = JobStore::new(jobs_directory.clone());
        let started_at = Utc::now() - Duration::hours(48);
        let mut interrupted = manifest("interrupted", JobState::Preparing, started_at);
        interrupted.stages.insert(
            crate::domain::job::JobStage::Preflight,
            crate::domain::job::StageState::Running,
        );
        store.save_atomic(&interrupted).unwrap();
        store
            .save_atomic(&manifest("interrupted-draft", JobState::Draft, started_at))
            .unwrap();

        initialize_job_service_at(jobs_directory.clone(), Utc::now()).unwrap();
        let recovered = store.load("interrupted").unwrap();
        assert_eq!(recovered.state, JobState::Failed);
        assert_eq!(
            recovered.failure_code,
            Some(crate::error::ErrorCode::JobInterrupted)
        );
        assert!(matches!(
            recovered.stages[&crate::domain::job::JobStage::Preflight],
            crate::domain::job::StageState::Failed
        ));
        assert!(recovered.updated_at > started_at);
        let recovered_draft = store.load("interrupted-draft").unwrap();
        assert_eq!(recovered_draft.state, JobState::Failed);
        assert_eq!(
            recovered_draft.failure_code,
            Some(crate::error::ErrorCode::JobInterrupted)
        );

        initialize_job_service_at(
            jobs_directory.clone(),
            recovered.updated_at + Duration::hours(24),
        )
        .unwrap();
        assert!(!jobs_directory.join("interrupted").exists());
        assert!(!jobs_directory.join("interrupted-draft").exists());
    }

    fn manifest(id: &str, state: JobState, updated_at: chrono::DateTime<Utc>) -> JobManifest {
        let mut manifest = JobManifest::new(
            JobInput::new(
                "tender.docx".into(),
                vec![BidInput::new("bid.docx".into(), None)],
            )
            .unwrap(),
        );
        manifest.id = id.into();
        manifest.state = state;
        manifest.created_at = updated_at;
        manifest.updated_at = updated_at;
        manifest
    }
}
