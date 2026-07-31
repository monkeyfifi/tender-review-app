use crate::error::{AppError, ErrorCode};
use serde::{Deserialize, Serialize};
use url::Url;

const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";
const DEEPSEEK_MODEL: &str = "deepseek-v4-flash";
const OLD_DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com/v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ModelProvider {
    DeepSeek,
    Custom,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelPreset {
    pub provider: ModelProvider,
    pub base_url: &'static str,
    pub model: &'static str,
}

impl ModelProvider {
    pub fn preset(self) -> Option<ModelPreset> {
        match self {
            Self::DeepSeek => Some(ModelPreset {
                provider: self,
                base_url: DEEPSEEK_BASE_URL,
                model: DEEPSEEK_MODEL,
            }),
            Self::Custom => None,
        }
    }
}

pub fn provider_presets() -> Vec<ModelPreset> {
    [ModelProvider::DeepSeek]
        .into_iter()
        .filter_map(ModelProvider::preset)
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelSettings {
    pub base_url: String,
    pub model: String,
    pub timeout_seconds: u64,
    pub api_key_remembered: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveModelSettingsInput {
    pub base_url: String,
    pub model: String,
    pub timeout_seconds: u64,
    pub api_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestModelConnectionInput {
    pub base_url: String,
    pub model: String,
    pub timeout_seconds: u64,
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PersistedModelSettings {
    pub base_url: String,
    pub model: String,
    pub timeout_seconds: u64,
}

impl PersistedModelSettings {
    pub(crate) fn defaults() -> Self {
        Self {
            base_url: DEEPSEEK_BASE_URL.into(),
            model: DEEPSEEK_MODEL.into(),
            timeout_seconds: 60,
        }
    }

    pub(crate) fn normalized(mut self) -> Self {
        if self.base_url.trim_end_matches('/') == OLD_DEEPSEEK_BASE_URL
            && matches!(self.model.trim(), "deepseek-chat" | "deepseek-reasoner")
        {
            self.base_url = DEEPSEEK_BASE_URL.into();
            self.model = DEEPSEEK_MODEL.into();
        }
        self
    }

    pub(crate) fn response(self, has_saved_key: bool) -> ModelSettings {
        let normalized = self.normalized();
        ModelSettings {
            base_url: normalized.base_url,
            model: normalized.model,
            timeout_seconds: normalized.timeout_seconds,
            api_key_remembered: has_saved_key,
        }
    }
}

impl From<&SaveModelSettingsInput> for PersistedModelSettings {
    fn from(input: &SaveModelSettingsInput) -> Self {
        Self {
            base_url: input.base_url.trim().to_owned(),
            model: input.model.trim().to_owned(),
            timeout_seconds: input.timeout_seconds,
        }
        .normalized()
    }
}

pub fn validate_base_url(base_url: &str) -> Result<(), AppError> {
    let parsed = Url::parse(base_url.trim()).map_err(|_| {
        AppError::new(
            ErrorCode::InvalidEndpoint,
            "模型服务地址必须是有效的 HTTPS 地址或本地 HTTP 地址",
        )
    })?;
    let contains_secrets = !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some();
    let local_http = parsed.scheme() == "http"
        && matches!(parsed.host_str(), Some("localhost") | Some("127.0.0.1"));

    if !contains_secrets && (parsed.scheme() == "https" || local_http) {
        Ok(())
    } else {
        Err(AppError::new(
            ErrorCode::InvalidEndpoint,
            "模型服务地址不得包含凭据、查询或片段，且仅允许 HTTPS 或本地 HTTP",
        ))
    }
}

pub fn validate_model_settings(model: &str, timeout_seconds: u64) -> Result<(), AppError> {
    if model.trim().is_empty() || !(1..=600).contains(&timeout_seconds) {
        return Err(AppError::new(
            ErrorCode::InvalidModelSettings,
            "模型名称不能为空，超时时间必须为 1–600 秒",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    #[test]
    fn provider_presets_supply_openai_compatible_defaults() {
        assert_eq!(
            provider_presets(),
            vec![ModelPreset {
                provider: ModelProvider::DeepSeek,
                base_url: "https://api.deepseek.com",
                model: "deepseek-v4-flash",
            }]
        );
        assert_eq!(ModelProvider::Custom.preset(), None);
    }

    #[test]
    fn defaults_and_old_deepseek_settings_use_the_current_deepseek_model() {
        assert_eq!(
            PersistedModelSettings::defaults().base_url,
            "https://api.deepseek.com"
        );
        assert_eq!(
            PersistedModelSettings::defaults().model,
            "deepseek-v4-flash"
        );

        let settings = PersistedModelSettings {
            base_url: "https://api.deepseek.com/v1".into(),
            model: "deepseek-chat".into(),
            timeout_seconds: 60,
        }
        .normalized();

        assert_eq!(settings.base_url, "https://api.deepseek.com");
        assert_eq!(settings.model, "deepseek-v4-flash");
    }

    #[test]
    fn permits_https_and_local_http_only() {
        assert!(validate_base_url("https://api.example.com/v1").is_ok());
        assert!(validate_base_url("http://127.0.0.1:11434/v1").is_ok());
        assert!(validate_base_url("http://localhost:11434/v1").is_ok());
        assert_eq!(
            validate_base_url("http://api.example.com/v1")
                .unwrap_err()
                .code,
            ErrorCode::InvalidEndpoint
        );
    }

    #[test]
    fn serializes_model_settings_without_an_api_key() {
        let settings = ModelSettings {
            base_url: "https://api.example.com/v1".into(),
            model: "reviewer".into(),
            timeout_seconds: 60,
            api_key_remembered: true,
        };

        let serialized = serde_json::to_string(&settings).unwrap();
        assert!(!serialized.contains("sk-secret-value"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&serialized).unwrap()["apiKeyRemembered"],
            true
        );
        assert!(!serialized.contains("hasSavedKey"));
    }

    #[test]
    fn rejects_endpoint_credentials_query_and_fragment() {
        for endpoint in [
            "https://user@example.com/v1",
            "https://user:password@example.com/v1",
            "https://api.example.com/v1?token=secret",
            "https://api.example.com/v1#secret",
        ] {
            assert_eq!(
                validate_base_url(endpoint).unwrap_err().code,
                ErrorCode::InvalidEndpoint
            );
        }
    }

    #[test]
    fn validates_trimmed_non_empty_model_and_timeout_range() {
        assert_eq!(
            validate_model_settings("   ", 60).unwrap_err().code,
            ErrorCode::InvalidModelSettings
        );
        for timeout in [0, 601] {
            assert_eq!(
                validate_model_settings("reviewer", timeout)
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidModelSettings
            );
        }
        assert!(validate_model_settings(" reviewer ", 1).is_ok());
        assert!(validate_model_settings("reviewer", 600).is_ok());
    }
}
