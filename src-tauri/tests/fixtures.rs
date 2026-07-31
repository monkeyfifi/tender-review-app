#![allow(dead_code)]

use app_lib::domain::job::{BidInput, JobInput, JobManifest, JobStage, JobState, StageState};
use chrono::{DateTime, Utc};
use lopdf::{Document, EncryptionState, EncryptionVersion, Object, Permissions};
use std::collections::BTreeMap;
use std::io::Write;

pub const ENCRYPTED_PDF_PLAINTEXT: &str = "secret protected tender text";

pub fn manifest(id: &str, state: JobState, updated_at: DateTime<Utc>) -> JobManifest {
    let mut stages = BTreeMap::new();
    stages.insert(JobStage::Preflight, StageState::Pending);

    JobManifest {
        id: id.into(),
        input: JobInput::new(
            "tender.pdf".into(),
            vec![BidInput::new("bid.pdf".into(), None)],
        )
        .unwrap(),
        state,
        stages,
        created_at: updated_at,
        updated_at,
        failure_code: None,
        file_errors: Vec::new(),
        completed_files: Vec::new(),
    }
}

pub fn write_docx(paragraphs: &[&str], table_rows: &[&[&str]]) -> tempfile::NamedTempFile {
    let mut body = String::new();
    for paragraph in paragraphs {
        body.push_str(&format!("<w:p><w:r><w:t>{paragraph}</w:t></w:r></w:p>"));
    }
    if !table_rows.is_empty() {
        body.push_str("<w:tbl>");
        for row in table_rows {
            body.push_str("<w:tr>");
            for cell in *row {
                body.push_str(&format!(
                    "<w:tc><w:p><w:r><w:t>{cell}</w:t></w:r></w:p></w:tc>"
                ));
            }
            body.push_str("</w:tr>");
        }
        body.push_str("</w:tbl>");
    }
    write_docx_xml(&format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    ))
}

pub fn write_docx_xml(document_xml: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::Builder::new().suffix(".docx").tempfile().unwrap();
    {
        let mut archive = zip::ZipWriter::new(file.as_file_mut());
        archive
            .start_file(
                "word/document.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        archive.write_all(document_xml.as_bytes()).unwrap();
        archive.finish().unwrap();
    }
    file.flush().unwrap();
    file
}

pub fn write_empty_docx_archive() -> tempfile::NamedTempFile {
    let mut file = tempfile::Builder::new().suffix(".docx").tempfile().unwrap();
    {
        let archive = zip::ZipWriter::new(file.as_file_mut());
        archive.finish().unwrap();
    }
    file.flush().unwrap();
    file
}

pub fn write_invalid_docx_bytes() -> tempfile::NamedTempFile {
    let mut file = tempfile::Builder::new().suffix(".docx").tempfile().unwrap();
    file.write_all(b"this is not a ZIP archive").unwrap();
    file.flush().unwrap();
    file
}

pub fn write_pdf(page_text: &[&str]) -> tempfile::NamedTempFile {
    write_pdf_bytes(build_text_pdf(page_text))
}

pub fn write_single_page_multiline_pdf(lines: &[&str]) -> tempfile::NamedTempFile {
    let content = lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let show = format!("({}) Tj", escape_pdf_literal(line));
            if index == 0 {
                show
            } else {
                format!("T* {show}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 5 0 R >> >> /MediaBox [0 0 612 792] /Contents 4 0 R >>".to_owned(),
        pdf_stream(&format!("BT /F1 12 Tf 14 TL 72 720 Td {content} ET")),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    write_pdf_bytes(build_pdf(&objects, ""))
}

pub fn write_image_only_pdf() -> tempfile::NamedTempFile {
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /Resources << /XObject << /Im0 5 0 R >> >> /MediaBox [0 0 612 792] /Contents 4 0 R >>".to_owned(),
        pdf_stream("q 1 0 0 1 72 720 cm /Im0 Do Q"),
        "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 1 >>\nstream\n\0\nendstream".to_owned(),
    ];

    write_pdf_bytes(build_pdf(&objects, ""))
}

pub fn write_encrypted_pdf() -> tempfile::NamedTempFile {
    let mut document = Document::load_mem(&build_text_pdf(&[ENCRYPTED_PDF_PLAINTEXT])).unwrap();
    document.trailer.set(
        "ID",
        Object::Array(vec![
            Object::string_literal("fixture-file-id-01"),
            Object::string_literal("fixture-file-id-01"),
        ]),
    );
    let state = EncryptionState::try_from(EncryptionVersion::V1 {
        document: &document,
        owner_password: "owner-password",
        user_password: "user-password",
        permissions: Permissions::default(),
    })
    .unwrap();
    document.encrypt(&state).unwrap();

    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    write_pdf_bytes(bytes)
}

fn build_text_pdf(page_text: &[&str]) -> Vec<u8> {
    let page_count = page_text.len();
    let font_id = 3 + page_count * 2;
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {page_count} >>",
            (0..page_count)
                .map(|index| format!("{} 0 R", 3 + index * 2))
                .collect::<Vec<_>>()
                .join(" ")
        ),
    ];

    for (index, text) in page_text.iter().enumerate() {
        let page_id = 3 + index * 2;
        let content_id = page_id + 1;
        let content = format!(
            "BT /F1 12 Tf 72 720 Td ({}) Tj ET",
            escape_pdf_literal(text)
        );
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 {font_id} 0 R >> >> /MediaBox [0 0 612 792] /Contents {content_id} 0 R >>"
        ));
        objects.push(pdf_stream(&content));
    }
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned());

    build_pdf(&objects, "")
}

pub fn write_invalid_pdf_bytes() -> tempfile::NamedTempFile {
    let mut file = tempfile::Builder::new().suffix(".pdf").tempfile().unwrap();
    file.write_all(b"this is not a PDF").unwrap();
    file.flush().unwrap();
    file
}

fn escape_pdf_literal(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

fn pdf_stream(content: &str) -> String {
    format!(
        "<< /Length {} >>\nstream\n{content}\nendstream",
        content.len()
    )
}

fn build_pdf(objects: &[String], trailer_entries: &str) -> Vec<u8> {
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref_offset = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R {trailer_entries} >>\nstartxref\n{xref_offset}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

fn write_pdf_bytes(bytes: Vec<u8>) -> tempfile::NamedTempFile {
    let mut file = tempfile::Builder::new().suffix(".pdf").tempfile().unwrap();
    file.write_all(&bytes).unwrap();
    file.flush().unwrap();
    file
}
