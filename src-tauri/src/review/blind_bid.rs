use crate::{
    documents::{read_file_bounded, MAX_DOCUMENT_FILE_BYTES},
    error::AppError,
};
use quick_xml::{events::Event, Reader};
use serde::{Deserialize, Serialize};
use std::{
    io::{Cursor, Read},
    path::Path,
};

const MANUAL_REVIEW_NOTE: &str = "辅助检查，需人工复核";
const MAX_XML_PART_BYTES: u64 = 128 * 1024 * 1024;
const MAX_XML_TOTAL_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlindBidFinding {
    #[serde(default)]
    pub raw_level: String,
    #[serde(default)]
    pub biz_level: String,
    #[serde(default)]
    pub category: String,
    pub rule: String,
    pub location: String,
    pub expected: String,
    pub actual: String,
    #[serde(default)]
    pub snippet: String,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlindBidCheck {
    pub skipped: bool,
    pub message: String,
    pub findings: Vec<BlindBidFinding>,
}

pub fn check_blind_bid(
    path: &Path,
    supplier_keywords: &[String],
) -> Result<BlindBidCheck, AppError> {
    check_blind_bid_with_limits(
        path,
        supplier_keywords,
        MAX_DOCUMENT_FILE_BYTES,
        MAX_XML_PART_BYTES,
        MAX_XML_TOTAL_BYTES,
    )
}

fn check_blind_bid_with_limits(
    path: &Path,
    supplier_keywords: &[String],
    max_file_bytes: u64,
    max_part_bytes: u64,
    max_total_xml_bytes: u64,
) -> Result<BlindBidCheck, AppError> {
    if !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("docx"))
    {
        return Ok(BlindBidCheck {
            skipped: true,
            message: "仅对 DOCX 技术暗标执行辅助检查，已跳过".into(),
            findings: Vec::new(),
        });
    }

    let bytes = read_file_bounded(path, max_file_bytes, "DOCX 文件")?.bytes;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(AppError::invalid_docx)?;
    let mut remaining_xml_bytes = max_total_xml_bytes;
    let document = read_entry(
        &mut archive,
        "word/document.xml",
        max_part_bytes,
        &mut remaining_xml_bytes,
    )?;
    let mut parts = vec![("word/document.xml".to_owned(), document.clone())];
    for index in 0..archive.len() {
        let name = archive
            .by_index(index)
            .map_err(AppError::invalid_docx)?
            .name()
            .to_owned();
        if name != "word/document.xml" && is_checked_part(&name) {
            parts.push((
                name.clone(),
                read_entry(
                    &mut archive,
                    &name,
                    max_part_bytes,
                    &mut remaining_xml_bytes,
                )?,
            ));
        }
    }
    let mut findings = Vec::new();

    for (name, xml) in &parts {
        if name.starts_with("word/header") && name.ends_with(".xml") {
            let text = xml_text(xml)?;
            if !text.trim().is_empty() {
                findings.push(finding(
                    "页眉",
                    "页眉#Section1",
                    "无页眉文字",
                    format!("发现: {}", truncate(&text)),
                ));
            }
        }
        if name.starts_with("word/footer") && name.ends_with(".xml") {
            let text = xml_text(xml)?;
            if !text.trim().is_empty() {
                findings.push(finding(
                    "页脚",
                    "页脚#Section1",
                    "无页脚文字",
                    format!("发现: {}", truncate(&text)),
                ));
            }
            if contains_page_field(xml)? {
                findings.push(finding(
                    "页码",
                    "页脚#Section1",
                    "无页码",
                    "检测到 PAGE 字段",
                ));
            }
        }
        if xml.contains("wp:anchor") {
            findings.push(finding(
                "浮动对象",
                "文档",
                "无浮动对象",
                format!("检测到 wp:anchor（{name}）"),
            ));
        }
    }
    for keyword in supplier_keywords
        .iter()
        .filter(|keyword| !keyword.trim().is_empty())
    {
        if parts
            .iter()
            .any(|(_, xml)| xml_text(xml).is_ok_and(|text| text.contains(keyword)))
        {
            findings.push(finding(
                "供应商关键词",
                "正文",
                "不出现供应商识别信息",
                format!("发现关键词: {keyword}"),
            ));
        }
    }
    let styles = parts
        .iter()
        .find(|(name, _)| name == "word/styles.xml")
        .map(|(_, xml)| xml.as_str());
    if !(has_font(&document) || styles.is_some_and(has_default_font)) {
        findings.push(finding("字体", "正文", "宋体 / SimSun", "未检测到宋体设置"));
    }
    if !(has_size_14pt(&document) || styles.is_some_and(has_default_size_14pt)) {
        findings.push(finding("字号", "正文", "四号（14 pt）", "未检测到四号设置"));
    }
    if !(document.contains("w:w=\"11906\"") && document.contains("w:h=\"16838\"")) {
        findings.push(finding("纸张", "页面设置", "A4", "未检测到 A4 页面尺寸"));
    }
    if !(document.contains("w:top=\"1417\"")
        && document.contains("w:bottom=\"1134\"")
        && document.contains("w:left=\"1134\"")
        && document.contains("w:right=\"1134\""))
    {
        findings.push(finding(
            "页边距",
            "页面设置",
            "上2.5cm、下2cm、左2cm、右2cm",
            "未检测到标准页边距设置",
        ));
    }
    if document.contains("w:before=") || document.contains("w:after=") {
        findings.push(finding(
            "段落空格",
            "正文段落",
            "段前0pt、段后0pt",
            "检测到段前或段后间距设置",
        ));
    }

    Ok(BlindBidCheck {
        skipped: false,
        message: "DOCX 技术暗标辅助检查完成，需人工复核".into(),
        findings,
    })
}

fn read_entry(
    archive: &mut zip::ZipArchive<Cursor<Vec<u8>>>,
    name: &str,
    max_part_bytes: u64,
    remaining_xml_bytes: &mut u64,
) -> Result<String, AppError> {
    let entry = archive.by_name(name).map_err(AppError::invalid_docx)?;
    let declared_size = entry.size();
    if declared_size > max_part_bytes || declared_size > *remaining_xml_bytes {
        return Err(xml_too_large());
    }
    let limit = max_part_bytes.min(*remaining_xml_bytes);
    let mut bytes = Vec::new();
    entry
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(AppError::unreadable_document)?;
    if bytes.len() as u64 > limit {
        return Err(xml_too_large());
    }
    *remaining_xml_bytes -= bytes.len() as u64;
    String::from_utf8(bytes).map_err(AppError::invalid_docx)
}

fn is_checked_part(name: &str) -> bool {
    name == "word/styles.xml"
        || (name.starts_with("word/header") && name.ends_with(".xml"))
        || (name.starts_with("word/footer") && name.ends_with(".xml"))
}

fn xml_too_large() -> AppError {
    AppError::unreadable_document("DOCX XML 超过安全大小限制")
}

fn has_font(xml: &str) -> bool {
    xml.contains("w:rFonts") && (xml.contains("宋体") || xml.contains("SimSun"))
}

fn has_size_14pt(xml: &str) -> bool {
    xml.contains("w:sz") && xml.contains("w:val=\"28\"")
}

fn has_default_font(styles: &str) -> bool {
    default_style_sections(styles).into_iter().any(has_font)
}

fn has_default_size_14pt(styles: &str) -> bool {
    default_style_sections(styles)
        .into_iter()
        .any(has_size_14pt)
}

fn default_style_sections(styles: &str) -> Vec<&str> {
    let mut sections = section(styles, "<w:docDefaults", "</w:docDefaults>")
        .into_iter()
        .collect::<Vec<_>>();
    let mut remainder = styles;
    while let Some(start) = remainder.find("<w:style") {
        let style = &remainder[start..];
        let Some(end) = style.find("</w:style>") else {
            break;
        };
        let style = &style[..end + "</w:style>".len()];
        if style.contains("w:default=\"1\"") || style.contains("w:styleId=\"Normal\"") {
            sections.push(style);
        }
        remainder = &remainder[start + style.len()..];
    }
    sections
}

fn section<'a>(xml: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let start = xml.find(start)?;
    let end = xml[start..].find(end)? + start + end.len();
    Some(&xml[start..end])
}

fn contains_page_field(xml: &str) -> Result<bool, AppError> {
    let mut reader = Reader::from_str(xml);
    let mut in_instruction = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                let event_name = event.name();
                let name = local_name(event_name.as_ref());
                if name == b"fldSimple"
                    && event.attributes().flatten().any(|attribute| {
                        local_name(attribute.key.as_ref()) == b"instr"
                            && page_instruction(&String::from_utf8_lossy(attribute.value.as_ref()))
                    })
                {
                    return Ok(true);
                }
                in_instruction = name == b"instrText";
            }
            Ok(Event::Empty(event)) if local_name(event.name().as_ref()) == b"fldSimple" => {
                if event.attributes().flatten().any(|attribute| {
                    local_name(attribute.key.as_ref()) == b"instr"
                        && page_instruction(&String::from_utf8_lossy(attribute.value.as_ref()))
                }) {
                    return Ok(true);
                }
            }
            Ok(Event::Text(value)) if in_instruction => {
                if page_instruction(&value.xml10_content().map_err(AppError::invalid_docx)?) {
                    return Ok(true);
                }
            }
            Ok(Event::End(event)) if local_name(event.name().as_ref()) == b"instrText" => {
                in_instruction = false
            }
            Ok(Event::Eof) => return Ok(false),
            Err(error) => return Err(AppError::invalid_docx(error)),
            _ => {}
        }
    }
}

fn local_name(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
}

fn page_instruction(value: &str) -> bool {
    value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .any(|word| word.eq_ignore_ascii_case("PAGE"))
}

fn xml_text(xml: &str) -> Result<String, AppError> {
    let mut reader = Reader::from_str(xml);
    let mut text = String::new();
    let mut in_text = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                in_text = event.name().as_ref().rsplit(|byte| *byte == b':').next() == Some(b"t")
            }
            Ok(Event::Text(value)) if in_text => {
                text.push_str(&value.xml10_content().map_err(AppError::invalid_docx)?)
            }
            Ok(Event::End(event))
                if event.name().as_ref().rsplit(|byte| *byte == b':').next() == Some(b"t") =>
            {
                in_text = false
            }
            Ok(Event::Eof) => return Ok(text),
            Err(error) => return Err(AppError::invalid_docx(error)),
            _ => {}
        }
    }
}

fn finding(
    rule: impl Into<String>,
    location: impl Into<String>,
    expected: impl Into<String>,
    actual: impl Into<String>,
) -> BlindBidFinding {
    BlindBidFinding {
        raw_level: "warn".into(),
        biz_level: "必改".into(),
        category: "暗标辅助".into(),
        rule: rule.into(),
        location: location.into(),
        expected: expected.into(),
        actual: actual.into(),
        snippet: String::new(),
        note: MANUAL_REVIEW_NOTE.into(),
    }
}

fn truncate(text: &str) -> String {
    text.chars().take(40).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn skips_non_docx_instead_of_failing() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("blind.pdf");
        std::fs::write(&path, b"pdf").unwrap();

        let result = check_blind_bid(&path, &[]).unwrap();

        assert!(result.skipped);
        assert!(result.findings.is_empty());
        assert!(result.message.contains("DOCX"));
    }

    #[test]
    fn reports_header_as_assisted_check_requiring_manual_review() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("blind.docx");
        write_docx(
            &path,
            &[(
                "word/header1.xml",
                "<w:hdr xmlns:w=\"w\"><w:p><w:r><w:t>公司名称</w:t></w:r></w:p></w:hdr>",
            )],
        );

        let result = check_blind_bid(&path, &[]).unwrap();
        let finding = result
            .findings
            .iter()
            .find(|item| item.rule == "页眉")
            .unwrap();
        assert_eq!(finding.location, "页眉#Section1");
        assert_eq!(finding.expected, "无页眉文字");
        assert!(finding.actual.contains("公司名称"));
        assert!(finding.note.contains("辅助检查"));
        assert!(finding.note.contains("人工复核"));
    }

    #[test]
    fn reports_footer_text_page_number_and_floating_objects_from_all_xml_parts() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("blind.docx");
        write_docx(
            &path,
            &[
                (
                    "word/document.xml",
                    r#"<w:document xmlns:w="w"><w:body><w:p><w:pPr><w:spacing w:before="1"/></w:pPr><w:r><w:t>投标人有限公司</w:t></w:r></w:p></w:body></w:document>"#,
                ),
                (
                    "word/footer1.xml",
                    r#"<w:ftr xmlns:w="w" xmlns:wp="wp"><w:p><w:r><w:t>某某有限公司</w:t></w:r></w:p><w:instrText> PAGE </w:instrText><wp:anchor/></w:ftr>"#,
                ),
                (
                    "word/header1.xml",
                    r#"<w:hdr xmlns:w="w" xmlns:wp="wp"><wp:anchor/></w:hdr>"#,
                ),
            ],
        );

        let result = check_blind_bid(&path, &["有限公司".into()]).unwrap();
        assert!(result
            .findings
            .iter()
            .any(|item| item.rule == "页脚" && item.actual.contains("某某有限公司")));
        assert!(result.findings.iter().any(|item| item.rule == "页码"));
        assert!(result
            .findings
            .iter()
            .any(|item| item.rule == "浮动对象" && item.actual.contains("footer1.xml")));
        for rule in ["供应商关键词", "字体", "字号", "纸张", "页边距", "段落空格"]
        {
            assert!(
                result.findings.iter().any(|item| item.rule == rule),
                "missing {rule}"
            );
        }
    }

    #[test]
    fn returns_controlled_invalid_docx_error_for_a_corrupt_docx() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("broken.docx");
        std::fs::write(&path, b"not a zip archive").unwrap();

        let error = check_blind_bid(&path, &[]).unwrap_err();

        assert_eq!(error.code, crate::error::ErrorCode::InvalidDocx);
    }

    #[test]
    fn uses_doc_defaults_for_font_and_size_when_body_has_no_direct_formatting() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("styled.docx");
        write_docx(
            &path,
            &[
                (
                    "word/document.xml",
                    r#"<w:document xmlns:w="w"><w:body><w:p><w:r><w:t>正文</w:t></w:r></w:p></w:body></w:document>"#,
                ),
                (
                    "word/styles.xml",
                    r#"<w:styles xmlns:w="w"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:eastAsia="宋体"/><w:sz w:val="28"/></w:rPr></w:rPrDefault></w:docDefaults></w:styles>"#,
                ),
            ],
        );

        let result = check_blind_bid(&path, &[]).unwrap();

        assert!(!result.findings.iter().any(|item| item.rule == "字体"));
        assert!(!result.findings.iter().any(|item| item.rule == "字号"));
    }

    #[test]
    fn uses_normal_style_when_doc_defaults_do_not_set_font_or_size() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("normal-style.docx");
        write_docx(
            &path,
            &[
                (
                    "word/document.xml",
                    r#"<w:document xmlns:w="w"><w:body><w:p><w:r><w:t>正文</w:t></w:r></w:p></w:body></w:document>"#,
                ),
                (
                    "word/styles.xml",
                    r#"<w:styles xmlns:w="w"><w:style w:styleId="Other"><w:rPr/></w:style><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:rPr><w:rFonts w:eastAsia="宋体"/><w:sz w:val="28"/></w:rPr></w:style></w:styles>"#,
                ),
            ],
        );

        let result = check_blind_bid(&path, &[]).unwrap();

        assert!(!result.findings.iter().any(|item| item.rule == "字体"));
        assert!(!result.findings.iter().any(|item| item.rule == "字号"));
    }

    #[test]
    fn ignores_regular_page_word_but_detects_page_field_instruction() {
        let temp = tempfile::tempdir().unwrap();
        let plain = temp.path().join("plain.docx");
        write_docx(
            &plain,
            &[(
                "word/footer1.xml",
                r#"<w:ftr xmlns:w="w"><w:p><w:r><w:t>PAGE</w:t></w:r></w:p></w:ftr>"#,
            )],
        );
        assert!(!check_blind_bid(&plain, &[])
            .unwrap()
            .findings
            .iter()
            .any(|item| item.rule == "页码"));

        let field = temp.path().join("field.docx");
        write_docx(
            &field,
            &[(
                "word/footer1.xml",
                r#"<w:ftr xmlns:w="w"><w:instrText> PAGE </w:instrText></w:ftr>"#,
            )],
        );
        assert!(check_blind_bid(&field, &[])
            .unwrap()
            .findings
            .iter()
            .any(|item| item.rule == "页码"));
    }

    #[test]
    fn rejects_compressed_xml_that_exceeds_part_limit() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("large.docx");
        write_docx(
            &path,
            &[(
                "word/document.xml",
                &format!("<w:document xmlns:w=\"w\">{}</w:document>", "x".repeat(64)),
            )],
        );

        let error = check_blind_bid_with_limits(&path, &[], 1024, 32, 64).unwrap_err();

        assert_eq!(error.code, crate::error::ErrorCode::UnreadableDocument);
        assert!(error.message.contains("XML"));
    }

    fn write_docx(path: &std::path::Path, entries: &[(&str, &str)]) {
        let file = std::fs::File::create(path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        let document = entries
            .iter()
            .find(|(name, _)| *name == "word/document.xml")
            .map(|(_, xml)| *xml)
            .unwrap_or("<w:document xmlns:w=\"w\"/>");
        archive
            .start_file::<_, ()>(
                "word/document.xml",
                zip::write::FileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
        archive.write_all(document.as_bytes()).unwrap();
        for (name, xml) in entries {
            if *name == "word/document.xml" {
                continue;
            }
            archive
                .start_file::<_, ()>(
                    *name,
                    zip::write::FileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated),
                )
                .unwrap();
            archive.write_all(xml.as_bytes()).unwrap();
        }
        archive.finish().unwrap();
    }
}
