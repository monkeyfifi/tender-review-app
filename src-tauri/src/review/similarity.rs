use crate::{
    domain::document::{NormalizedBlock, NormalizedDocument},
    error::{AppError, ErrorCode},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use unicode_normalization::UnicodeNormalization;

const MIN_FRAGMENT_CHARS: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateFragment {
    pub text: String,
    pub left_location: String,
    pub right_location: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicatePair {
    pub left_bid: usize,
    pub right_bid: usize,
    pub fragments: Vec<DuplicateFragment>,
}

pub fn compare_blind_documents(
    documents: &[(usize, NormalizedDocument)],
) -> Result<Vec<DuplicatePair>, AppError> {
    if documents.len() > 4 {
        return Err(AppError::new(
            ErrorCode::BidCountOutOfRange,
            "投标文件数量必须为 1–4 份",
        ));
    }
    let mut pairs = Vec::new();
    for left_index in 0..documents.len() {
        for right_index in left_index + 1..documents.len() {
            let fragments =
                duplicate_fragments(&documents[left_index].1, &documents[right_index].1);
            if !fragments.is_empty() {
                pairs.push(DuplicatePair {
                    left_bid: documents[left_index].0,
                    right_bid: documents[right_index].0,
                    fragments,
                });
            }
        }
    }
    Ok(pairs)
}

fn duplicate_fragments(
    left: &NormalizedDocument,
    right: &NormalizedDocument,
) -> Vec<DuplicateFragment> {
    let mut fragments = BTreeMap::new();
    for left_block in &left.blocks {
        for right_block in &right.blocks {
            for fragment in block_fragments(left_block, right_block) {
                fragments.entry(fragment.text.clone()).or_insert(fragment);
            }
        }
    }
    let mut fragments: Vec<_> = fragments.into_values().collect();
    fragments.sort_by_key(|fragment| std::cmp::Reverse(fragment.text.chars().count()));
    let mut selected = Vec::new();
    for candidate in fragments {
        if !selected
            .iter()
            .any(|other: &DuplicateFragment| other.text.contains(&candidate.text))
        {
            selected.push(candidate);
        }
    }
    selected
}

fn block_fragments(left: &NormalizedBlock, right: &NormalizedBlock) -> Vec<DuplicateFragment> {
    let left_characters = normalized_characters(&left.text);
    let right_characters = normalized_characters(&right.text);
    if left_characters.len() < MIN_FRAGMENT_CHARS || right_characters.len() < MIN_FRAGMENT_CHARS {
        return Vec::new();
    }
    let mut windows: HashMap<Vec<char>, Vec<usize>> = HashMap::new();
    for start in 0..=left_characters.len() - MIN_FRAGMENT_CHARS {
        windows
            .entry(left_characters[start..start + MIN_FRAGMENT_CHARS].to_vec())
            .or_default()
            .push(start);
    }
    let mut fragments = BTreeMap::new();
    for right_start in 0..=right_characters.len() - MIN_FRAGMENT_CHARS {
        let key = right_characters[right_start..right_start + MIN_FRAGMENT_CHARS].to_vec();
        let Some(left_starts) = windows.get(&key) else {
            continue;
        };
        for left_start in left_starts {
            let (left_start, _right_start, left_end, _right_end) = expand_match(
                &left_characters,
                &right_characters,
                *left_start,
                right_start,
            );
            let text: String = left_characters[left_start..left_end].iter().collect();
            fragments
                .entry(text.clone())
                .or_insert_with(|| DuplicateFragment {
                    text,
                    left_location: location(left),
                    right_location: location(right),
                });
        }
    }
    fragments.into_values().collect()
}

fn expand_match(
    left: &[char],
    right: &[char],
    mut left_start: usize,
    mut right_start: usize,
) -> (usize, usize, usize, usize) {
    while left_start > 0 && right_start > 0 && left[left_start - 1] == right[right_start - 1] {
        left_start -= 1;
        right_start -= 1;
    }
    let mut left_end = left_start;
    let mut right_end = right_start;
    while left_end < left.len() && right_end < right.len() && left[left_end] == right[right_end] {
        left_end += 1;
        right_end += 1;
    }
    (left_start, right_start, left_end, right_end)
}

fn normalized_characters(text: &str) -> Vec<char> {
    text.nfc()
        .collect::<String>()
        .to_lowercase()
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn location(block: &NormalizedBlock) -> String {
    format!(
        "{} {}",
        block.anchor.line_label, block.anchor.structure_path
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::document::{BlockKind, NormalizedBlock, NormalizedDocument, SourceAnchor};

    #[test]
    fn locates_continuous_duplicate_fragments_in_anchored_blocks() {
        let pairs = compare_blind_documents(&[
            (1, document("投标文件1行8", "项目实施组织与质量保障措施。")),
            (
                2,
                document("投标文件2行3", "项目实施组织与质量保障措施。其他内容"),
            ),
        ])
        .unwrap();

        assert_eq!(pairs.len(), 1);
        assert_eq!((pairs[0].left_bid, pairs[0].right_bid), (1, 2));
        assert!(pairs[0].fragments[0]
            .text
            .contains("项目实施组织与质量保障措施"));
        assert!(pairs[0].fragments[0].left_location.contains("投标文件1行8"));
        assert!(pairs[0].fragments[0]
            .right_location
            .contains("投标文件2行3"));
    }

    #[test]
    fn ignores_short_common_terms() {
        let pairs = compare_blind_documents(&[
            (1, document("投标文件1行1", "技术方案甲")),
            (2, document("投标文件2行1", "技术方案乙")),
        ])
        .unwrap();

        assert!(pairs.is_empty());
    }

    #[test]
    fn rejects_more_than_four_documents() {
        let error = compare_blind_documents(&vec![(1, document("行1", "内容")); 5]).unwrap_err();

        assert_eq!(error.code, crate::error::ErrorCode::BidCountOutOfRange);
    }

    fn document(line_label: &str, text: &str) -> NormalizedDocument {
        NormalizedDocument {
            source_path: "fixture.docx".into(),
            sha256: "a".repeat(64),
            blocks: vec![NormalizedBlock {
                text: text.into(),
                kind: BlockKind::Paragraph,
                anchor: SourceAnchor::new(line_label, None, "段落#1"),
            }],
        }
    }
}
