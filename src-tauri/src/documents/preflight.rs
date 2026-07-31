use super::{read_file_bounded, DocumentParser, DocxParser, PdfParser, MAX_DOCUMENT_FILE_BYTES};
use crate::{domain::document::NormalizedDocument, error::AppError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileRole {
    Tender,
    Bid,
    BlindBid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileFormat {
    Pdf,
    Docx,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileInspection {
    pub display_name: String,
    pub format: FileFormat,
    pub byte_size: u64,
    pub sha256: String,
    pub block_count: usize,
}

pub trait PreflightDocumentParser: Send + Sync {
    fn parse(
        &self,
        format: FileFormat,
        path: &Path,
        source_label: &str,
    ) -> Result<NormalizedDocument, AppError>;
}

struct DefaultPreflightDocumentParser;

impl PreflightDocumentParser for DefaultPreflightDocumentParser {
    fn parse(
        &self,
        format: FileFormat,
        path: &Path,
        source_label: &str,
    ) -> Result<NormalizedDocument, AppError> {
        match format {
            FileFormat::Pdf => PdfParser.parse(path, source_label),
            FileFormat::Docx => DocxParser.parse(path, source_label),
        }
    }
}

#[derive(Clone)]
pub struct PreflightService {
    parser: Arc<dyn PreflightDocumentParser>,
}

pub(crate) struct InspectedDocument {
    pub inspection: FileInspection,
    pub document: NormalizedDocument,
}

impl PreflightService {
    pub fn new() -> Self {
        Self {
            parser: Arc::new(DefaultPreflightDocumentParser),
        }
    }

    pub fn with_parser(parser: Arc<dyn PreflightDocumentParser>) -> Self {
        Self { parser }
    }

    pub fn inspect(&self, path: &Path, role: FileRole) -> Result<FileInspection, AppError> {
        self.inspect_document(path, role)
            .map(|document| document.inspection)
    }

    pub(crate) fn inspect_document(
        &self,
        path: &Path,
        role: FileRole,
    ) -> Result<InspectedDocument, AppError> {
        let canonical_path = std::fs::canonicalize(path).map_err(|_| unreadable_selection())?;
        let format = file_format(&canonical_path, role)?;
        let snapshot = read_file_bounded(&canonical_path, MAX_DOCUMENT_FILE_BYTES, "所选文档")?;
        let bytes = snapshot.bytes;
        let display_name = display_name(&canonical_path);
        let snapshot_sha256 = hex::encode(Sha256::digest(&bytes));
        let document = self
            .parser
            .parse(format, &canonical_path, &display_name)
            .map_err(sanitize_document_error)?;
        if document.sha256 != snapshot_sha256 {
            return Err(AppError::new(
                crate::error::ErrorCode::DocumentChangedDuringRead,
                "文档在读取期间发生变化，请重新选择后重试",
            ));
        }

        Ok(InspectedDocument {
            inspection: FileInspection {
                display_name,
                format,
                byte_size: snapshot.byte_size,
                sha256: snapshot_sha256,
                block_count: document.blocks.len(),
            },
            document,
        })
    }
}

impl Default for PreflightService {
    fn default() -> Self {
        Self::new()
    }
}

fn file_format(path: &Path, role: FileRole) -> Result<FileFormat, AppError> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    match (role, extension.as_deref()) {
        (FileRole::BlindBid, Some("docx")) => Ok(FileFormat::Docx),
        (FileRole::BlindBid, _) => Err(AppError::blind_bid_must_be_docx()),
        (_, Some("pdf")) => Ok(FileFormat::Pdf),
        (_, Some("docx")) => Ok(FileFormat::Docx),
        _ => Err(AppError::unsupported_extension()),
    }
}

pub(crate) fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("已选择文件")
        .to_owned()
}

fn unreadable_selection() -> AppError {
    AppError::new(
        crate::error::ErrorCode::UnreadableDocument,
        "无法读取所选文档",
    )
}

fn sanitize_document_error(error: AppError) -> AppError {
    use crate::error::ErrorCode;

    let message = match error.code {
        ErrorCode::InvalidDocx => "DOCX 文件无效",
        ErrorCode::UnreadableDocument => "文档无法读取",
        ErrorCode::TextNotExtractable => "文档不包含可提取文本",
        ErrorCode::EncryptedDocument => "文档已加密，无法提取",
        _ => "文档预检失败",
    };
    AppError::new(error.code, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::document::{BlockKind, NormalizedBlock, SourceAnchor};
    use crate::error::ErrorCode;
    use std::sync::Arc;

    #[test]
    fn rejects_legacy_doc_extensions_case_insensitively() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("legacy.DOC");
        std::fs::write(&path, b"not a supported document").unwrap();

        let error = PreflightService::new()
            .inspect(&path, FileRole::Tender)
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::UnsupportedExtension);
    }

    #[test]
    fn rejects_pdf_as_a_blind_bid_attachment() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("blind.PDF");
        std::fs::write(&path, b"not a blind bid").unwrap();

        let error = PreflightService::new()
            .inspect(&path, FileRole::BlindBid)
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::BlindBidMustBeDocx);
    }

    #[test]
    fn default_preflight_service_matches_new() {
        fn default_value<T: Default>() -> T {
            T::default()
        }

        let from_default: PreflightService = default_value();
        let from_new = PreflightService::new();

        assert_eq!(
            std::mem::size_of_val(&from_default),
            std::mem::size_of_val(&from_new)
        );
    }

    struct ChangedDocumentParser;

    impl PreflightDocumentParser for ChangedDocumentParser {
        fn parse(
            &self,
            _format: FileFormat,
            path: &Path,
            source_label: &str,
        ) -> Result<NormalizedDocument, AppError> {
            Ok(NormalizedDocument {
                source_path: path.to_string_lossy().into_owned(),
                sha256: "different-snapshot".into(),
                blocks: vec![NormalizedBlock {
                    text: "content".into(),
                    kind: BlockKind::Paragraph,
                    anchor: SourceAnchor::new(format!("{source_label}行1"), None, "段落#1"),
                }],
            })
        }
    }

    #[test]
    fn rejects_a_document_that_changes_between_snapshot_and_parse() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("changing.docx");
        std::fs::write(&path, b"first snapshot").unwrap();
        let service = PreflightService::with_parser(Arc::new(ChangedDocumentParser));

        let error = service.inspect(&path, FileRole::Bid).unwrap_err();

        assert_eq!(error.code, ErrorCode::DocumentChangedDuringRead);
    }

    #[test]
    fn preflight_rejects_an_oversized_sparse_file_without_reading_or_leaking_path() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sensitive.docx");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(super::super::MAX_DOCUMENT_FILE_BYTES + 1)
            .unwrap();

        let error = PreflightService::new()
            .inspect(&path, FileRole::Bid)
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::UnreadableDocument);
        assert!(!error.message.contains(&path.display().to_string()));
    }
}
