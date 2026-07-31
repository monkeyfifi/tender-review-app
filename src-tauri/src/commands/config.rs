use crate::config::credential::{CredentialStore, KeyringCredentialStore};
use crate::config::model::{
    provider_presets, validate_base_url, validate_model_settings, ModelPreset, ModelSettings,
    PersistedModelSettings, SaveModelSettingsInput, TestModelConnectionInput,
};
use crate::error::AppError;
use crate::review::service::ModelReviewClient;
#[cfg(unix)]
use std::fs::File;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

const SETTINGS_FILE_NAME: &str = "model-settings.json";

pub struct ModelConfigurationState {
    credential_store: Arc<dyn CredentialStore>,
}

impl Default for ModelConfigurationState {
    fn default() -> Self {
        Self {
            credential_store: Arc::new(KeyringCredentialStore),
        }
    }
}

impl ModelConfigurationState {
    pub fn with_credential_store(credential_store: Arc<dyn CredentialStore>) -> Self {
        Self { credential_store }
    }

    fn save_to_path(
        &self,
        path: &Path,
        input: SaveModelSettingsInput,
    ) -> Result<ModelSettings, AppError> {
        validate_base_url(&input.base_url)?;
        validate_model_settings(&input.model, input.timeout_seconds)?;
        let settings = PersistedModelSettings::from(&input);
        persist_settings_to_path(path, &settings)?;

        let api_key = input
            .api_key
            .map(|key| key.trim().to_owned())
            .filter(|key| !key.is_empty());
        if let Some(api_key) = api_key {
            self.credential_store.save_key(&api_key)?;
        }

        let remembered = self.credential_store.load_key()?.is_some();
        Ok(settings.response(remembered))
    }

    pub fn clear_key(&self) -> Result<(), AppError> {
        self.credential_store.delete_key()?;
        Ok(())
    }

    pub fn effective_key(&self) -> Result<Option<String>, AppError> {
        self.credential_store.load_key()
    }
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, AppError> {
    Ok(app
        .path()
        .app_config_dir()
        .map_err(AppError::configuration_persistence)?
        .join(SETTINGS_FILE_NAME))
}

fn read_settings(app: &AppHandle) -> Result<PersistedModelSettings, AppError> {
    read_settings_from_path(&settings_path(app)?)
}

fn read_settings_from_path(path: &Path) -> Result<PersistedModelSettings, AppError> {
    match fs::read_to_string(path) {
        Ok(contents) => {
            serde_json::from_str(&contents).map_err(AppError::configuration_persistence)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(PersistedModelSettings::defaults())
        }
        Err(error) => Err(AppError::configuration_persistence(error)),
    }
}

fn persist_settings_to_path(
    path: &Path,
    settings: &PersistedModelSettings,
) -> Result<(), AppError> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::configuration_persistence("模型设置目录无效"))?;
    fs::create_dir_all(parent).map_err(AppError::configuration_persistence)?;

    let temporary_path = parent.join(format!(".{SETTINGS_FILE_NAME}.{}.tmp", Uuid::new_v4()));
    let contents = serde_json::to_vec(settings).map_err(AppError::configuration_persistence)?;
    let write_result = (|| -> Result<(), AppError> {
        let mut temporary_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary_path)
            .map_err(AppError::configuration_persistence)?;
        temporary_file
            .write_all(&contents)
            .map_err(AppError::configuration_persistence)?;
        temporary_file
            .sync_all()
            .map_err(AppError::configuration_persistence)?;
        replace_file_atomically(&temporary_path, path)?;
        sync_parent_directory(parent)
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    write_result
}

#[cfg(not(windows))]
fn replace_file_atomically(source: &Path, target: &Path) -> Result<(), AppError> {
    fs::rename(source, target).map_err(AppError::configuration_persistence)
}

#[cfg(windows)]
fn replace_file_atomically(source: &Path, target: &Path) -> Result<(), AppError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: both paths are null-terminated UTF-16 buffers that remain alive
    // for the duration of this synchronous Win32 call.
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(AppError::configuration_persistence(
            std::io::Error::last_os_error(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> Result<(), AppError> {
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(AppError::configuration_persistence)
}

#[cfg(windows)]
fn sync_parent_directory(_parent: &Path) -> Result<(), AppError> {
    // Opening a directory with std::fs::File fails on Windows. The temporary
    // file itself is already synced before the atomic rename.
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn sync_parent_directory(_parent: &Path) -> Result<(), AppError> {
    Ok(())
}

#[tauri::command]
pub fn save_model_settings(
    app: AppHandle,
    state: State<'_, ModelConfigurationState>,
    input: SaveModelSettingsInput,
) -> Result<ModelSettings, AppError> {
    state.save_to_path(&settings_path(&app)?, input)
}

#[tauri::command]
pub fn clear_model_key(state: State<'_, ModelConfigurationState>) -> Result<(), AppError> {
    state.clear_key()
}

#[tauri::command]
pub fn get_model_settings(
    app: AppHandle,
    state: State<'_, ModelConfigurationState>,
) -> Result<ModelSettings, AppError> {
    let has_saved_key = state.credential_store.load_key()?.is_some();
    Ok(read_settings(&app)?.response(has_saved_key))
}

#[tauri::command]
pub fn get_model_provider_presets() -> Vec<ModelPreset> {
    provider_presets()
}

pub(crate) fn review_client(
    app: &AppHandle,
    state: &ModelConfigurationState,
) -> Result<ModelReviewClient, AppError> {
    let settings = read_settings(app)?;
    validate_base_url(&settings.base_url)?;
    validate_model_settings(&settings.model, settings.timeout_seconds)?;
    let api_key = state
        .effective_key()?
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| {
            AppError::new(
                crate::error::ErrorCode::ModelApiKeyMissing,
                "请先配置模型 API Key",
            )
        })?;
    Ok(ModelReviewClient::new(
        settings.base_url,
        settings.model,
        api_key,
        settings.timeout_seconds,
    ))
}

#[tauri::command]
pub async fn test_model_connection(input: TestModelConnectionInput) -> Result<(), AppError> {
    validate_base_url(&input.base_url)?;
    validate_model_settings(&input.model, input.timeout_seconds)?;
    crate::model_client::test_model_connection(
        &input.base_url,
        &input.model,
        &input.api_key,
        input.timeout_seconds,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::credential::MemoryCredentialStore;

    fn input(api_key: Option<&str>) -> SaveModelSettingsInput {
        SaveModelSettingsInput {
            base_url: "https://api.example.com/v1".into(),
            model: " reviewer ".into(),
            timeout_seconds: 60,
            api_key: api_key.map(str::to_owned),
        }
    }

    #[test]
    fn current_platform_settings_directory_sync_succeeds() {
        let temp = tempfile::tempdir().unwrap();
        sync_parent_directory(temp.path()).unwrap();
    }

    #[test]
    fn current_platform_atomic_replace_overwrites_existing_settings() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("model-settings.json");
        let replacement = temp.path().join("model-settings.json.tmp");
        std::fs::write(&target, "old").unwrap();
        std::fs::write(&replacement, "new").unwrap();

        replace_file_atomically(&replacement, &target).unwrap();

        assert_eq!(std::fs::read_to_string(target).unwrap(), "new");
        assert!(!replacement.exists());
    }

    #[test]
    fn saving_a_key_persists_it_until_explicit_clear() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("model-settings.json");
        let credentials = Arc::new(MemoryCredentialStore::new());
        let state = ModelConfigurationState::with_credential_store(credentials.clone());

        let first = state
            .save_to_path(&path, input(Some("sk-secret-value")))
            .unwrap();
        assert!(first.api_key_remembered);
        let second = state.save_to_path(&path, input(None)).unwrap();

        assert!(second.api_key_remembered);
        assert_eq!(
            credentials.load_key().unwrap().as_deref(),
            Some("sk-secret-value")
        );
        assert!(!std::fs::read_to_string(path)
            .unwrap()
            .contains("sk-secret-value"));
    }

    #[test]
    fn explicit_clear_removes_a_saved_key() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("model-settings.json");
        let credentials = Arc::new(MemoryCredentialStore::new());
        let state = ModelConfigurationState::with_credential_store(credentials.clone());
        state
            .save_to_path(&path, input(Some("second-secret")))
            .unwrap();
        state.clear_key().unwrap();
        assert_eq!(credentials.load_key().unwrap(), None);
    }
}
