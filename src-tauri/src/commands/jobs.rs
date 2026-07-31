use crate::{
    domain::job::JobInput,
    error::{AppError, ErrorCode},
    jobs::service::{PreparedJob, RecoverableJobs, ReviewStatus},
    AppState,
};
use std::{ffi::OsString, path::Path};
use tauri::State;

#[tauri::command]
pub async fn prepare_job(
    state: State<'_, AppState>,
    input: JobInput,
) -> Result<PreparedJob, AppError> {
    let service = state.jobs.clone();
    tauri::async_runtime::spawn_blocking(move || service.prepare(input))
        .await
        .map_err(|_| AppError::new(ErrorCode::JobPersistenceFailed, "任务准备意外中断"))?
}

#[tauri::command]
pub fn list_recoverable_jobs(state: State<'_, AppState>) -> Result<RecoverableJobs, AppError> {
    state.jobs.list_recoverable()
}

#[tauri::command]
pub fn clear_job(state: State<'_, AppState>, job_id: String) -> Result<(), AppError> {
    state.jobs.clear(&job_id)
}

#[tauri::command]
pub async fn run_review(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    config: State<'_, crate::commands::config::ModelConfigurationState>,
    job_id: String,
) -> Result<ReviewStatus, AppError> {
    let client = crate::commands::config::review_client(&app, &config)?;
    let service = state.jobs.clone();
    tauri::async_runtime::spawn_blocking(move || service.run_review(&job_id, client))
        .await
        .map_err(|_| AppError::new(ErrorCode::JobPersistenceFailed, "审核任务意外中断"))?
}

#[tauri::command]
pub fn get_review_status(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<ReviewStatus, AppError> {
    state.jobs.get_review_status(&job_id)
}

#[tauri::command]
pub fn open_report_folder(state: State<'_, AppState>, job_id: String) -> Result<String, AppError> {
    let path = state.jobs.open_report_folder(&job_id)?;
    open_folder(&path)?;
    Ok(path.to_string_lossy().into_owned())
}

fn open_folder(path: &Path) -> Result<(), AppError> {
    let (program, args) = folder_open_command(path);
    std::process::Command::new(program)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|_| AppError::new(ErrorCode::ReportGenerationFailed, "无法打开结果目录"))
}

fn folder_open_command(path: &Path) -> (&'static str, Vec<OsString>) {
    #[cfg(target_os = "macos")]
    {
        ("open", vec![path.as_os_str().to_owned()])
    }
    #[cfg(target_os = "windows")]
    {
        ("explorer", vec![path.as_os_str().to_owned()])
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        ("xdg-open", vec![path.as_os_str().to_owned()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn open_report_folder_uses_the_platform_file_manager() {
        let path = Path::new("/tmp/report dir");

        let (program, args) = folder_open_command(path);

        #[cfg(target_os = "macos")]
        assert_eq!(program, "open");
        #[cfg(target_os = "windows")]
        assert_eq!(program, "explorer");
        #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
        assert_eq!(program, "xdg-open");
        assert_eq!(args, vec![path.as_os_str().to_owned()]);
    }
}
