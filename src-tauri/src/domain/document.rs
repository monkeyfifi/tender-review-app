use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceAnchor {
    pub line_label: String,
    pub page: Option<u32>,
    pub structure_path: String,
}

impl SourceAnchor {
    pub fn new(
        line_label: impl Into<String>,
        page: Option<u32>,
        structure_path: impl Into<String>,
    ) -> Self {
        Self {
            line_label: line_label.into(),
            page,
            structure_path: structure_path.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BlockKind {
    Paragraph,
    TableCell,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedBlock {
    pub text: String,
    pub kind: BlockKind,
    pub anchor: SourceAnchor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedDocument {
    pub source_path: String,
    pub sha256: String,
    pub blocks: Vec<NormalizedBlock>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_pdf_anchor_without_inventing_docx_page() {
        let pdf = SourceAnchor::new("招标文件行12", Some(3), "第二章/表格1/行2列3");
        let docx = SourceAnchor::new("投标文件行7", None, "段落#4");
        assert_eq!(serde_json::to_value(&pdf).unwrap()["page"], 3);
        assert!(serde_json::to_value(&docx).unwrap()["page"].is_null());
    }
}
