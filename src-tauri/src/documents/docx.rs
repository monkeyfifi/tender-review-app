use super::{read_file_bounded, DocumentParser, MAX_DOCUMENT_FILE_BYTES};
use crate::{
    domain::document::{BlockKind, NormalizedBlock, NormalizedDocument, SourceAnchor},
    error::AppError,
};
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::{events::Event, Reader};
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Read},
    path::Path,
};
use zip::result::ZipError;

pub struct DocxParser;

const MAX_DOCX_FILE_BYTES: u64 = MAX_DOCUMENT_FILE_BYTES;
const MAX_DOCUMENT_XML_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug)]
struct TableContext {
    index: usize,
    row_index: usize,
    cell_index: usize,
}

impl DocumentParser for DocxParser {
    fn parse(&self, path: &Path, source_label: &str) -> Result<NormalizedDocument, AppError> {
        let snapshot = read_file_bounded(path, MAX_DOCX_FILE_BYTES, "DOCX 文件")?;
        validate_docx_file_size(snapshot.byte_size)?;
        let bytes = snapshot.bytes;
        let mut archive =
            zip::ZipArchive::new(Cursor::new(&bytes)).map_err(AppError::unreadable_document)?;
        let document = archive
            .by_name("word/document.xml")
            .map_err(map_document_xml_error)?;
        let declared_size = document.size();
        let xml = read_document_xml_limited(document, declared_size, MAX_DOCUMENT_XML_BYTES)?;

        let blocks = extract_wordprocessing_blocks(&xml, source_label)?;
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

fn map_document_xml_error(error: ZipError) -> AppError {
    match error {
        ZipError::FileNotFound => AppError::invalid_docx("缺少 word/document.xml"),
        error => AppError::unreadable_document(error),
    }
}

fn extract_wordprocessing_blocks(
    xml: &str,
    source_label: &str,
) -> Result<Vec<NormalizedBlock>, AppError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut blocks = Vec::new();
    let mut paragraph_text = String::new();
    let mut in_paragraph = false;
    let mut in_text = false;
    let mut table_contexts = Vec::new();
    let mut next_table_index = 0usize;
    let mut paragraph_index = 0usize;

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => match event.name().as_ref() {
                name if is_word_tag(name, b"tbl") => {
                    next_table_index += 1;
                    table_contexts.push(TableContext {
                        index: next_table_index,
                        row_index: 0,
                        cell_index: 0,
                    });
                }
                name if is_word_tag(name, b"tr") => {
                    if let Some(table) = table_contexts.last_mut() {
                        table.row_index += 1;
                        table.cell_index = 0;
                    }
                }
                name if is_word_tag(name, b"tc") => {
                    if let Some(table) = table_contexts.last_mut() {
                        table.cell_index += 1;
                    }
                }
                name if is_word_tag(name, b"p") => {
                    in_paragraph = true;
                    paragraph_text.clear();
                    if table_contexts.is_empty() {
                        paragraph_index += 1;
                    }
                }
                name if is_word_tag(name, b"t") && in_paragraph => in_text = true,
                _ => {}
            },
            Ok(Event::Text(text)) if in_text => {
                let decoded = text
                    .xml10_content()
                    .map_err(AppError::unreadable_document)?;
                paragraph_text.push_str(&decoded);
            }
            Ok(Event::GeneralRef(reference)) if in_text => {
                let reference = reference
                    .xml10_content()
                    .map_err(AppError::unreadable_document)?;
                paragraph_text.push(decode_xml_reference(&reference)?);
            }
            Ok(Event::End(event)) => match event.name().as_ref() {
                name if is_word_tag(name, b"t") => in_text = false,
                name if is_word_tag(name, b"p") => {
                    if !paragraph_text.trim().is_empty() {
                        let (kind, structure_path) = if let Some(table) = table_contexts.last() {
                            (
                                BlockKind::TableCell,
                                format!(
                                    "表格#{}/行{}列{}",
                                    table.index, table.row_index, table.cell_index
                                ),
                            )
                        } else {
                            (BlockKind::Paragraph, format!("段落#{paragraph_index}"))
                        };
                        let line_number = blocks.len() + 1;
                        blocks.push(NormalizedBlock {
                            text: paragraph_text.clone(),
                            kind,
                            anchor: SourceAnchor::new(
                                format!("{source_label}行{line_number}"),
                                None,
                                structure_path,
                            ),
                        });
                    }
                    in_paragraph = false;
                    in_text = false;
                    paragraph_text.clear();
                }
                name if is_word_tag(name, b"tbl") => {
                    table_contexts.pop();
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(error) => return Err(AppError::unreadable_document(error)),
            _ => {}
        }
    }

    Ok(blocks)
}

fn is_word_tag(name: &[u8], local_name: &[u8]) -> bool {
    name.rsplit(|byte| *byte == b':').next() == Some(local_name)
}

fn decode_xml_reference(reference: &str) -> Result<char, AppError> {
    if let Some(value) = resolve_predefined_entity(reference) {
        return value
            .chars()
            .next()
            .ok_or_else(|| AppError::unreadable_document("空 XML 实体"));
    }

    let number = reference
        .strip_prefix("#x")
        .or_else(|| reference.strip_prefix("#X"))
        .map(|value| u32::from_str_radix(value, 16))
        .or_else(|| {
            reference
                .strip_prefix('#')
                .map(|value| value.parse::<u32>())
        })
        .ok_or_else(|| AppError::unreadable_document(format!("未定义 XML 实体：{reference}")))?
        .map_err(AppError::unreadable_document)?;
    char::from_u32(number)
        .ok_or_else(|| AppError::unreadable_document(format!("无效 XML 字符实体：{reference}")))
}

fn validate_docx_file_size(file_size: u64) -> Result<(), AppError> {
    validate_size_at_most(file_size, MAX_DOCX_FILE_BYTES, "DOCX 文件")
}

fn read_document_xml_limited<R: Read>(
    reader: R,
    declared_size: u64,
    max_size: u64,
) -> Result<String, AppError> {
    validate_size_at_most(declared_size, max_size, "document.xml")?;
    let mut bytes = Vec::new();
    reader
        .take(max_size.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(AppError::unreadable_document)?;
    validate_size_at_most(bytes.len() as u64, max_size, "document.xml")?;
    String::from_utf8(bytes).map_err(AppError::unreadable_document)
}

fn validate_size_at_most(size: u64, max_size: u64, subject: &str) -> Result<(), AppError> {
    if size > max_size {
        return Err(AppError::unreadable_document(format!(
            "{subject} 大小超过 {} MiB 限制",
            max_size / (1024 * 1024)
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
    use std::io::Cursor;

    #[test]
    fn rejects_docx_file_larger_than_the_compressed_input_limit() {
        let error = validate_docx_file_size(MAX_DOCX_FILE_BYTES + 1).unwrap_err();

        assert_eq!(error.code, crate::error::ErrorCode::UnreadableDocument);
        assert!(error.message.contains("DOCX 文件"));
    }

    #[test]
    fn permits_docx_file_at_the_compressed_input_limit() {
        assert!(validate_docx_file_size(MAX_DOCX_FILE_BYTES).is_ok());
    }

    #[test]
    fn parser_rejects_an_oversized_sparse_docx_without_reading_or_leaking_path() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sensitive.docx");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_DOCX_FILE_BYTES + 1).unwrap();

        let error = DocxParser.parse(&path, "投标文件").unwrap_err();

        assert_eq!(error.code, crate::error::ErrorCode::UnreadableDocument);
        assert!(!error.message.contains(&path.display().to_string()));
    }

    #[test]
    fn rejects_document_xml_when_actual_read_exceeds_limit_even_if_declared_size_does_not() {
        let error = read_document_xml_limited(Cursor::new(b"four"), 0, 3).unwrap_err();

        assert_eq!(error.code, crate::error::ErrorCode::UnreadableDocument);
        assert!(error.message.contains("document.xml"));
    }

    #[test]
    fn rejects_document_xml_when_declared_size_exceeds_limit_before_reading() {
        let error = read_document_xml_limited(Cursor::new(Vec::new()), 4, 3).unwrap_err();

        assert_eq!(error.code, crate::error::ErrorCode::UnreadableDocument);
        assert!(error.message.contains("document.xml"));
    }
}
