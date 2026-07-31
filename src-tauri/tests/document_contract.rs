mod fixtures;

use app_lib::documents::{DocumentParser, DocxParser, PdfParser};
use app_lib::error::ErrorCode;
use sha2::{Digest, Sha256};

#[test]
fn docx_uses_lines_and_structure_without_page_numbers() {
    let path = fixtures::write_docx(&["第一段", "第二段"], &[&["参数", "要求值"]]);

    let parsed = DocxParser.parse(path.path(), "投标文件").unwrap();

    assert_eq!(parsed.blocks[0].anchor.line_label, "投标文件行1");
    assert_eq!(parsed.blocks[0].anchor.page, None);
    assert_eq!(
        parsed.blocks.last().unwrap().anchor.structure_path,
        "表格#1/行1列2"
    );
}

#[test]
fn docx_combines_split_runs_and_decodes_xml_entities() {
    let path = fixtures::write_docx_xml(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>技</w:t></w:r><w:r><w:t>术</w:t></w:r><w:r><w:t>&amp;合规</w:t></w:r></w:p></w:body></w:document>"#,
    );

    let parsed = DocxParser.parse(path.path(), "技术标").unwrap();

    assert_eq!(parsed.blocks[0].text, "技术&合规");
    assert_eq!(parsed.blocks[0].anchor.structure_path, "段落#1");
}

#[test]
fn docx_maps_missing_document_xml_to_invalid_docx() {
    let path = fixtures::write_empty_docx_archive();

    let error = DocxParser.parse(path.path(), "投标文件").unwrap_err();

    assert_eq!(error.code, ErrorCode::InvalidDocx);
}

#[test]
fn docx_maps_corrupt_archives_to_unreadable_document() {
    let path = fixtures::write_invalid_docx_bytes();

    let error = DocxParser.parse(path.path(), "投标文件").unwrap_err();

    assert_eq!(error.code, ErrorCode::UnreadableDocument);
}

#[test]
fn docx_rejects_documents_without_text() {
    let path = fixtures::write_docx(&["   "], &[]);

    let error = DocxParser.parse(path.path(), "投标文件").unwrap_err();

    assert_eq!(error.code, ErrorCode::TextNotExtractable);
}

#[test]
fn docx_keeps_outer_table_coordinates_after_a_nested_table() {
    let path = fixtures::write_docx_xml(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:tbl><w:tr><w:tc><w:p><w:r><w:t>外表首项</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>内表项</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>外表后续</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#,
    );

    let parsed = DocxParser.parse(path.path(), "投标文件").unwrap();
    let nested = parsed
        .blocks
        .iter()
        .find(|block| block.text == "内表项")
        .unwrap();
    let outer_following = parsed
        .blocks
        .iter()
        .find(|block| block.text == "外表后续")
        .unwrap();

    assert_eq!(nested.anchor.structure_path, "表格#2/行1列1");
    assert_eq!(outer_following.anchor.structure_path, "表格#1/行2列1");
}

#[test]
fn pdf_preserves_real_page_numbers_and_global_line_evidence() {
    let path = fixtures::write_pdf(&["page one requirement", "page two supporting proof"]);

    let parsed = PdfParser.parse(path.path(), "招标文件").unwrap();

    assert_eq!(parsed.blocks.len(), 2);
    assert_eq!(parsed.blocks[0].text, "page one requirement");
    assert_eq!(parsed.blocks[0].anchor.page, Some(1));
    assert_eq!(parsed.blocks[0].anchor.line_label, "招标文件行1");
    assert_eq!(parsed.blocks[0].anchor.structure_path, "PDF第1页/文本行1");
    assert_eq!(parsed.blocks[1].text, "page two supporting proof");
    assert_eq!(parsed.blocks[1].anchor.page, Some(2));
    assert_eq!(parsed.blocks[1].anchor.line_label, "招标文件行2");
    assert_eq!(parsed.blocks[1].anchor.structure_path, "PDF第2页/文本行1");
    assert_eq!(
        parsed.sha256,
        hex::encode(Sha256::digest(std::fs::read(path.path()).unwrap()))
    );
}

#[test]
fn pdf_numbers_multiple_lines_on_the_same_page_globally() {
    let path = fixtures::write_single_page_multiline_pdf(&["first line", "second line"]);

    let parsed = PdfParser.parse(path.path(), "招标文件").unwrap();

    assert_eq!(parsed.blocks.len(), 2);
    assert_eq!(parsed.blocks[0].anchor.line_label, "招标文件行1");
    assert_eq!(parsed.blocks[1].anchor.line_label, "招标文件行2");
    assert_eq!(parsed.blocks[0].anchor.structure_path, "PDF第1页/文本行1");
    assert_eq!(parsed.blocks[1].anchor.structure_path, "PDF第1页/文本行2");
}

#[test]
fn pdf_rejects_image_only_documents_without_ocr() {
    let path = fixtures::write_image_only_pdf();

    let error = PdfParser.parse(path.path(), "投标文件").unwrap_err();

    assert_eq!(error.code, ErrorCode::TextNotExtractable);
}

#[test]
fn pdf_rejects_encrypted_documents_before_text_extraction() {
    let path = fixtures::write_encrypted_pdf();
    let bytes = std::fs::read(path.path()).unwrap();

    assert!(!bytes
        .windows(fixtures::ENCRYPTED_PDF_PLAINTEXT.len())
        .any(|window| window == fixtures::ENCRYPTED_PDF_PLAINTEXT.as_bytes()));
    assert!(lopdf::Document::load_mem(&bytes).unwrap().is_encrypted());

    let error = PdfParser.parse(path.path(), "投标文件").unwrap_err();

    assert_eq!(error.code, ErrorCode::EncryptedDocument);
}

#[test]
fn pdf_maps_invalid_bytes_to_a_stable_unreadable_error() {
    let path = fixtures::write_invalid_pdf_bytes();

    let error = PdfParser.parse(path.path(), "投标文件").unwrap_err();

    assert_eq!(error.code, ErrorCode::UnreadableDocument);
    assert!(error.message.contains("PDF"));
}
