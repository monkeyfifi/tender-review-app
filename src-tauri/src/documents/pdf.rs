#[cfg(test)]
use super::read_stream_bounded;
use super::{read_file_bounded, DocumentParser, MAX_DOCUMENT_FILE_BYTES};
use crate::{
    domain::document::{BlockKind, NormalizedBlock, NormalizedDocument, SourceAnchor},
    error::AppError,
};
use sha2::{Digest, Sha256};
use std::path::Path;

pub struct PdfParser;

const MAX_PDF_FILE_BYTES: u64 = MAX_DOCUMENT_FILE_BYTES;
const MAX_DECOMPRESSED_PAGE_CONTENT_BYTES: usize = 128 * 1024 * 1024;
const MAX_PDF_PAGES: usize = 5_000;
const MAX_PDF_TOTAL_TEXT_BYTES: usize = 128 * 1024 * 1024;
const MAX_PDF_BLOCKS: usize = 250_000;

#[derive(Clone, Copy)]
struct PdfAggregateLimits {
    max_pages: usize,
    max_text_bytes: usize,
    max_blocks: usize,
}

impl Default for PdfAggregateLimits {
    fn default() -> Self {
        Self {
            max_pages: MAX_PDF_PAGES,
            max_text_bytes: MAX_PDF_TOTAL_TEXT_BYTES,
            max_blocks: MAX_PDF_BLOCKS,
        }
    }
}

struct PdfAggregateBudget {
    limits: PdfAggregateLimits,
    pages: usize,
    text_bytes: usize,
    blocks: usize,
}

impl PdfAggregateBudget {
    fn new(limits: PdfAggregateLimits) -> Self {
        Self {
            limits,
            pages: 0,
            text_bytes: 0,
            blocks: 0,
        }
    }

    fn record_page(&mut self, text_bytes: usize, blocks: usize) -> Result<(), AppError> {
        let next_pages = self.pages.saturating_add(1);
        let next_text_bytes = self.text_bytes.saturating_add(text_bytes);
        let next_blocks = self.blocks.saturating_add(blocks);
        if next_pages > self.limits.max_pages {
            return Err(AppError::unreadable_pdf("PDF 页数超过 5000 页限制"));
        }
        if next_text_bytes > self.limits.max_text_bytes {
            return Err(AppError::unreadable_pdf("PDF 总文本 UTF-8 字节数超过限制"));
        }
        if next_blocks > self.limits.max_blocks {
            return Err(AppError::unreadable_pdf("PDF 文本块数量超过限制"));
        }
        self.pages = next_pages;
        self.text_bytes = next_text_bytes;
        self.blocks = next_blocks;
        Ok(())
    }
}

impl DocumentParser for PdfParser {
    fn parse(&self, path: &Path, source_label: &str) -> Result<NormalizedDocument, AppError> {
        let snapshot = read_file_bounded(path, MAX_PDF_FILE_BYTES, "PDF 文件")?;
        validate_pdf_file_size(snapshot.byte_size)?;
        let bytes = snapshot.bytes;
        let pdf = load_pdf_document(&bytes)?;
        if pdf.is_encrypted() {
            return Err(AppError::encrypted_document(path));
        }

        let mut blocks = Vec::new();
        let mut aggregate_budget = PdfAggregateBudget::new(PdfAggregateLimits::default());
        for page_number in pdf.get_pages().keys().copied() {
            let page_text = pdf
                .extract_text_with_limit(&[page_number], MAX_DECOMPRESSED_PAGE_CONTENT_BYTES)
                .map_err(AppError::unreadable_pdf)?;
            let mut page_blocks = Vec::new();
            for (page_line, raw_text) in page_text.lines().enumerate() {
                let text = raw_text.trim();
                if text.is_empty() {
                    continue;
                }
                let line_number = blocks.len() + page_blocks.len() + 1;
                page_blocks.push(NormalizedBlock {
                    text: text.to_owned(),
                    kind: BlockKind::Paragraph,
                    anchor: SourceAnchor::new(
                        format!("{source_label}行{line_number}"),
                        Some(page_number),
                        format!("PDF第{page_number}页/文本行{}", page_line + 1),
                    ),
                });
            }
            aggregate_budget.record_page(page_text.len(), page_blocks.len())?;
            blocks.extend(page_blocks);
        }
        if blocks.is_empty() {
            return Err(AppError::text_not_extractable(path));
        }

        Ok(NormalizedDocument {
            source_path: path.to_string_lossy().into_owned(),
            sha256: sha256_hex(&bytes),
            blocks,
        })
    }
}

fn load_pdf_document(bytes: &[u8]) -> Result<lopdf::Document, AppError> {
    load_pdf_document_with_limit(bytes, MAX_DECOMPRESSED_PAGE_CONTENT_BYTES)
}

fn load_pdf_document_with_limit(
    bytes: &[u8],
    max_decompressed_size: usize,
) -> Result<lopdf::Document, AppError> {
    let document = lopdf::Document::load_mem_with_options(
        bytes,
        lopdf::LoadOptions::with_max_decompressed_size(max_decompressed_size),
    )
    .map_err(AppError::unreadable_pdf)?;
    if !document.is_encrypted() {
        validate_loaded_pdf_root(&document)?;
    }
    Ok(document)
}

fn validate_loaded_pdf_root(document: &lopdf::Document) -> Result<(), AppError> {
    let root = document
        .trailer
        .get(b"Root")
        .and_then(lopdf::Object::as_reference)
        .map_err(AppError::unreadable_pdf)?;
    document
        .get_dictionary(root)
        .map_err(AppError::unreadable_pdf)?;
    Ok(())
}

fn validate_pdf_file_size(file_size: u64) -> Result<(), AppError> {
    if file_size > MAX_PDF_FILE_BYTES {
        return Err(AppError::unreadable_pdf(format!(
            "PDF 文件大小超过 {} MiB 限制",
            MAX_PDF_FILE_BYTES / (1024 * 1024)
        )));
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Document, Object, SaveOptions};
    use std::io::Cursor;

    #[test]
    fn rejects_pdf_file_larger_than_the_input_limit_before_reading() {
        let error = validate_pdf_file_size(MAX_PDF_FILE_BYTES + 1).unwrap_err();

        assert_eq!(error.code, crate::error::ErrorCode::UnreadableDocument);
        assert!(error.message.contains("PDF 文件"));
    }

    #[test]
    fn permits_pdf_file_at_the_input_limit() {
        assert!(validate_pdf_file_size(MAX_PDF_FILE_BYTES).is_ok());
    }

    #[test]
    fn parser_rejects_an_oversized_sparse_pdf_without_reading_or_leaking_path() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sensitive.pdf");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_PDF_FILE_BYTES + 1).unwrap();

        let error = PdfParser.parse(&path, "投标文件").unwrap_err();

        assert_eq!(error.code, crate::error::ErrorCode::UnreadableDocument);
        assert!(!error.message.contains(&path.display().to_string()));
    }

    #[test]
    fn rejects_a_compressed_object_stream_that_exceeds_the_load_limit() {
        let bytes = compressed_object_stream_pdf(4 * 1024);
        assert!(String::from_utf8_lossy(&bytes).contains("/ObjStm"));
        let unrestricted = Document::load_mem(&bytes).unwrap();
        let root = unrestricted
            .trailer
            .get(b"Root")
            .and_then(lopdf::Object::as_reference)
            .unwrap();
        assert!(unrestricted.get_dictionary(root).is_ok());

        let error = load_pdf_document_with_limit(&bytes, 1024).unwrap_err();

        assert_eq!(error.code, crate::error::ErrorCode::UnreadableDocument);
        assert!(error.message.contains("PDF"));
    }

    #[test]
    fn rejects_a_stream_that_exceeds_the_read_limit_after_metadata_preflight() {
        let error = read_stream_bounded(Cursor::new(vec![0_u8; 4]), 3, "PDF 文件").unwrap_err();

        assert_eq!(error.code, crate::error::ErrorCode::UnreadableDocument);
        assert!(error.message.contains("PDF 文件"));
    }

    #[test]
    fn aggregate_budget_rejects_page_count_over_limit() {
        let mut budget = PdfAggregateBudget::new(PdfAggregateLimits {
            max_pages: 1,
            max_text_bytes: 100,
            max_blocks: 100,
        });
        budget.record_page(5, 1).unwrap();

        let error = budget.record_page(5, 1).unwrap_err();

        assert_eq!(error.code, crate::error::ErrorCode::UnreadableDocument);
        assert!(error.message.contains("页数"));
    }

    #[test]
    fn aggregate_budget_rejects_total_utf8_text_bytes_over_limit() {
        let mut budget = PdfAggregateBudget::new(PdfAggregateLimits {
            max_pages: 10,
            max_text_bytes: 5,
            max_blocks: 100,
        });

        let error = budget.record_page("中文".len(), 1).unwrap_err();

        assert_eq!(error.code, crate::error::ErrorCode::UnreadableDocument);
        assert!(error.message.contains("文本"));
    }

    #[test]
    fn aggregate_budget_rejects_total_blocks_over_limit() {
        let mut budget = PdfAggregateBudget::new(PdfAggregateLimits {
            max_pages: 10,
            max_text_bytes: 100,
            max_blocks: 1,
        });

        let error = budget.record_page(5, 2).unwrap_err();

        assert_eq!(error.code, crate::error::ErrorCode::UnreadableDocument);
        assert!(error.message.contains("文本块"));
    }

    fn compressed_object_stream_pdf(payload_size: usize) -> Vec<u8> {
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.add_object(Object::string_literal("x".repeat(payload_size)));
        document.trailer.set("Root", catalog_id);

        let mut output = Vec::new();
        document
            .save_with_options(
                &mut output,
                SaveOptions::builder()
                    .use_object_streams(true)
                    .use_xref_streams(true)
                    .build(),
            )
            .unwrap();
        output
    }
}
