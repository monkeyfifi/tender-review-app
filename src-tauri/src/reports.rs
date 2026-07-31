use crate::{
    domain::job::FileErrorRecord,
    review::{
        blind_bid::BlindBidCheck,
        schema::{BidFinding, Requirement},
        similarity::DuplicatePair,
    },
};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct ReviewReportInput {
    pub requirements: Vec<Requirement>,
    pub bid_results: Vec<Result<Vec<BidFinding>, String>>,
    pub duplicate_pairs: Vec<DuplicatePair>,
    pub blind_bid_checks: Vec<(usize, BlindBidCheck)>,
    pub file_errors: Vec<FileErrorRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewReport {
    pub summary: String,
    pub requirements: Vec<Requirement>,
    pub bids: Vec<BidReport>,
    pub duplicate_pairs: Vec<DuplicatePair>,
    pub blind_bid_checks: Vec<BlindBidReport>,
    pub file_errors: Vec<FileErrorRecord>,
    pub manual_review_note: String,
    pub footer: String,
    pub sections: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BidReport {
    pub bid_index: usize,
    pub completed: bool,
    pub findings: Vec<BidFinding>,
    pub failure_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlindBidReport {
    pub bid_index: usize,
    pub check: BlindBidCheck,
}

pub fn build_report(input: ReviewReportInput) -> ReviewReport {
    let bids: Vec<_> = input
        .bid_results
        .into_iter()
        .enumerate()
        .map(|(index, result)| match result {
            Ok(findings) => BidReport {
                bid_index: index + 1,
                completed: true,
                findings,
                failure_message: None,
            },
            Err(message) => BidReport {
                bid_index: index + 1,
                completed: false,
                findings: Vec::new(),
                failure_message: Some(message),
            },
        })
        .collect();
    let unfinished = bids.iter().filter(|bid| !bid.completed).count();
    let mut sections = vec![format!(
        "摘要：{} 份投标文件，{} 份未完成",
        bids.len(),
        unfinished
    )];
    sections.extend(bids.iter().map(|bid| match &bid.failure_message {
        Some(message) => format!("投标文件 {}：未完成（{}）", bid.bid_index, message),
        None => format!("投标文件 {}：审核完成", bid.bid_index),
    }));
    ReviewReport {
        summary: sections[0].clone(),
        requirements: input.requirements,
        bids,
        duplicate_pairs: input.duplicate_pairs,
        blind_bid_checks: input
            .blind_bid_checks
            .into_iter()
            .map(|(bid_index, check)| BlindBidReport { bid_index, check })
            .collect(),
        file_errors: input.file_errors,
        manual_review_note:
            "本报告为辅助审核结果，模型分析和格式检查均需人工复核，不构成法律结论。".into(),
        footer: "保定移动政企部 zhaoyun_bd@he".into(),
        sections,
    }
}

pub fn write_markdown(report: &ReviewReport, output: &Path) -> Result<(), crate::error::AppError> {
    std::fs::write(output, to_markdown(report)).map_err(|_| {
        crate::error::AppError::new(
            crate::error::ErrorCode::ReportGenerationFailed,
            "无法写入审核汇总 Markdown",
        )
    })
}

pub fn to_markdown(report: &ReviewReport) -> String {
    let mut lines = vec![
        "# 标书审核汇总".into(),
        String::new(),
        report.summary.clone(),
        String::new(),
        "## 招标要求".into(),
    ];
    lines.extend(report.requirements.iter().map(|requirement| {
        format!(
            "- {}：{}（{}）",
            requirement.id, requirement.title, requirement.evidence
        )
    }));
    lines.extend([String::new(), "## 逐份审核".into()]);
    for bid in &report.bids {
        lines.push(match &bid.failure_message {
            Some(message) => format!("- 投标文件 {}：未完成（{}）", bid.bid_index, message),
            None => format!("- 投标文件 {}：审核完成", bid.bid_index),
        });
        lines.extend(bid.findings.iter().map(|finding| {
            format!(
                "  - {}：{}（{}）",
                finding.requirement_id, finding.summary, finding.evidence
            )
        }));
    }
    lines.extend([String::new(), "## 两两查重".into()]);
    lines.extend(report.duplicate_pairs.iter().map(|pair| {
        format!(
            "- 投标 {} 与投标 {}：发现 {} 个连续重复片段",
            pair.left_bid,
            pair.right_bid,
            pair.fragments.len()
        )
    }));
    lines.extend([String::new(), "## 技术暗标辅助检查".into()]);
    lines.extend(report.blind_bid_checks.iter().flat_map(|blind| {
        blind.check.findings.iter().map(move |finding| {
            format!(
                "- 投标 {}：{}，{}",
                blind.bid_index, finding.rule, finding.actual
            )
        })
    }));
    lines.extend([String::new(), "## 文件失败项".into()]);
    if report.file_errors.is_empty() {
        lines.push("- 无".into());
    } else {
        lines.extend(
            report
                .file_errors
                .iter()
                .map(|error| format!("- {}：{}", error.source_name, error.message)),
        );
    }
    lines.extend([
        String::new(),
        report.manual_review_note.clone(),
        String::new(),
        report.footer.clone(),
    ]);
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    #[test]
    fn report_lists_failed_bid_without_marking_it_passed() {
        let report = super::build_report(super::ReviewReportInput {
            bid_results: vec![Err("模型连接超时".into())],
            ..super::ReviewReportInput::default()
        });

        assert!(report
            .sections
            .iter()
            .any(|section| section.contains("未完成")));
        assert!(!report
            .sections
            .iter()
            .any(|section| section.contains("通过")));
    }

    #[test]
    fn renders_markdown_report_with_footer_contact() {
        let markdown =
            super::to_markdown(&super::build_report(super::ReviewReportInput::default()));

        assert!(markdown.contains("# 标书审核汇总"));
        assert!(markdown.contains("保定移动政企部 zhaoyun_bd@he"));
    }
}
