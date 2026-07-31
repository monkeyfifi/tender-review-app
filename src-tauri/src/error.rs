use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    BidCountOutOfRange,
    UnsupportedExtension,
    BlindBidMustBeDocx,
    DuplicateInputFile,
    InvalidFileSelection,
    NoReadableBids,
    JobPersistenceFailed,
    JobInterrupted,
    InvalidDocx,
    UnreadableDocument,
    TextNotExtractable,
    EncryptedDocument,
    InsecureRemoteEndpoint,
    CorruptJobManifest,
    InvalidJobId,
    CredentialUnavailable,
    ConfigurationPersistenceFailed,
    InvalidEndpoint,
    InvalidModelSettings,
    ModelApiKeyMissing,
    ModelConnectionHttpFailed,
    ModelConnectionTimeout,
    ModelConnectionInvalidResponse,
    InvalidModelResponse,
    DocumentChangedDuringRead,
    ReportGenerationFailed,
    LocalEnvironmentUnavailable,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_io_and_serialization_errors_use_job_persistence_code() {
        assert_eq!(
            AppError::io("disk full").code,
            ErrorCode::JobPersistenceFailed
        );
        assert_eq!(
            AppError::serialization("json failed").code,
            ErrorCode::JobPersistenceFailed
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[error("{message}")]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    pub details: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_errors: Vec<crate::domain::job::FileErrorRecord>,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
            file_errors: Vec::new(),
        }
    }

    pub fn credential(error: impl std::fmt::Display) -> Self {
        Self::new(
            ErrorCode::CredentialUnavailable,
            format!("无法访问模型凭据：{error}"),
        )
    }

    pub fn configuration_persistence(error: impl std::fmt::Display) -> Self {
        Self::new(
            ErrorCode::ConfigurationPersistenceFailed,
            format!("无法保存模型设置：{error}"),
        )
    }

    pub fn io(error: impl std::fmt::Display) -> Self {
        Self::new(
            ErrorCode::JobPersistenceFailed,
            format!("无法持久化任务清单：{error}"),
        )
    }

    pub fn serialization(error: impl std::fmt::Display) -> Self {
        Self::new(
            ErrorCode::JobPersistenceFailed,
            format!("无法序列化任务清单：{error}"),
        )
    }

    pub fn corrupt_job_manifest(error: impl std::fmt::Display) -> Self {
        Self::new(
            ErrorCode::CorruptJobManifest,
            format!("任务清单已损坏：{error}"),
        )
    }

    pub fn invalid_job_id(job_id: &str) -> Self {
        Self::new(
            ErrorCode::InvalidJobId,
            format!("任务 ID 必须是安全的单一路径段：{job_id}"),
        )
    }

    pub fn invalid_docx(error: impl std::fmt::Display) -> Self {
        Self::new(ErrorCode::InvalidDocx, format!("DOCX 文件无效：{error}"))
    }

    pub fn unreadable_document(error: impl std::fmt::Display) -> Self {
        Self::new(
            ErrorCode::UnreadableDocument,
            format!("无法读取文档：{error}"),
        )
    }

    pub fn unsupported_extension() -> Self {
        Self::new(ErrorCode::UnsupportedExtension, "仅支持 PDF 或 DOCX 文件")
    }

    pub fn blind_bid_must_be_docx() -> Self {
        Self::new(
            ErrorCode::BlindBidMustBeDocx,
            "技术暗标附件仅支持 DOCX 文件",
        )
    }

    pub fn invalid_file_selection() -> Self {
        Self::new(
            ErrorCode::InvalidFileSelection,
            "所选文件不是可读取的非空普通文件",
        )
    }

    pub fn duplicate_input_file() -> Self {
        Self::new(
            ErrorCode::DuplicateInputFile,
            "同一文件不能重复作为任务输入",
        )
    }

    pub fn no_readable_bids(file_errors: Vec<crate::domain::job::FileErrorRecord>) -> Self {
        let mut error = Self::new(ErrorCode::NoReadableBids, "全部投标文件均无法读取");
        error.file_errors = file_errors;
        error
    }

    pub fn job_persistence() -> Self {
        Self::new(ErrorCode::JobPersistenceFailed, "无法保存本地任务数据")
    }

    pub fn unreadable_pdf(error: impl std::fmt::Display) -> Self {
        Self::new(
            ErrorCode::UnreadableDocument,
            format!("无法读取 PDF 文档：{error}"),
        )
    }

    pub fn text_not_extractable(path: &std::path::Path) -> Self {
        Self::new(
            ErrorCode::TextNotExtractable,
            format!("文档不包含可提取文本：{}", path.display()),
        )
    }

    pub fn encrypted_document(path: &std::path::Path) -> Self {
        Self::new(
            ErrorCode::EncryptedDocument,
            format!("文档已加密，无法提取文本：{}", path.display()),
        )
    }
}
