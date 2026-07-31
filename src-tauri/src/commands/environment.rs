use crate::{
    error::{AppError, ErrorCode},
    review::word_format_checker::{
        technical_environment_status, word_format_checker_setup_script, LocalEnvironmentStatus,
    },
};

#[tauri::command]
pub fn get_local_environment_status() -> LocalEnvironmentStatus {
    technical_environment_status()
}

#[tauri::command]
pub fn open_local_environment_setup() -> Result<(), AppError> {
    open_setup_script()
}

#[cfg(windows)]
fn open_setup_script() -> Result<(), AppError> {
    let script = word_format_checker_setup_script()?;
    std::process::Command::new("cmd")
        .arg("/C")
        .arg("start")
        .arg("")
        .arg(&script)
        .spawn()
        .map(|_| ())
        .map_err(|error| {
            AppError::new(
                ErrorCode::LocalEnvironmentUnavailable,
                format!("无法打开环境安装脚本：{error}"),
            )
        })
}

#[cfg(not(windows))]
fn open_setup_script() -> Result<(), AppError> {
    let _ = word_format_checker_setup_script()?;
    Err(AppError::new(
        ErrorCode::LocalEnvironmentUnavailable,
        "环境安装脚本仅用于 Windows 交付环境；Mac 开发环境请继续使用本机 Python 依赖。",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn setup_script_action_explains_that_the_bundled_script_is_windows_only() {
        let error = open_setup_script().unwrap_err();

        assert_eq!(error.code, ErrorCode::LocalEnvironmentUnavailable);
        assert!(error.message.contains("Windows"));
    }
}
