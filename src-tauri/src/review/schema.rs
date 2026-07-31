use crate::error::{AppError, ErrorCode};
use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Requirement {
    #[serde(deserialize_with = "deserialize_model_id")]
    pub id: String,
    pub category: RequirementCategory,
    pub title: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RequirementCategory {
    #[serde(alias = "废标项", alias = "否决项", alias = "无效标")]
    Disqualification,
    #[serde(alias = "评分项", alias = "评分标准", alias = "分值")]
    Scoring,
    #[serde(alias = "证明材料", alias = "资格证明", alias = "材料要求")]
    Evidence,
    #[serde(alias = "技术参数", alias = "技术要求", alias = "参数要求")]
    Technical,
    #[serde(alias = "时间节点", alias = "截止时间", alias = "工期")]
    Timeline,
    #[serde(alias = "合同条款", alias = "合同约束", alias = "商务条款")]
    Contract,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BidFinding {
    #[serde(deserialize_with = "deserialize_model_id")]
    pub requirement_id: String,
    pub status: FindingStatus,
    pub summary: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FindingStatus {
    #[serde(alias = "已响应", alias = "符合", alias = "满足")]
    Matched,
    #[serde(alias = "缺失", alias = "未响应", alias = "不满足")]
    Missing,
    #[serde(alias = "风险", alias = "存疑", alias = "偏离")]
    Risk,
    #[serde(alias = "人工复核", alias = "需人工复核", alias = "无法确定")]
    ManualReview,
}

pub fn parse_requirements(json: &str) -> Result<Vec<Requirement>, AppError> {
    let requirements: Vec<Requirement> = parse_model_array(json, &["requirements"])?;
    if requirements.iter().any(|requirement| {
        requirement.id.trim().is_empty()
            || requirement.title.trim().is_empty()
            || requirement.evidence.trim().is_empty()
    }) {
        return Err(invalid_response("招标要求缺少 ID、标题或证据锚点"));
    }
    Ok(requirements)
}

pub fn parse_findings(
    json: &str,
    requirements: &[Requirement],
) -> Result<Vec<BidFinding>, AppError> {
    let findings: Vec<BidFinding> = parse_model_array(json, &["findings"])?;
    let requirement_ids: HashSet<&str> = requirements
        .iter()
        .map(|requirement| requirement.id.as_str())
        .collect();
    if findings.iter().any(|finding| {
        !requirement_ids.contains(finding.requirement_id.as_str())
            || finding.summary.trim().is_empty()
            || finding.evidence.trim().is_empty()
    }) {
        return Err(invalid_response("投标发现包含未知要求或缺少摘要、证据锚点"));
    }
    Ok(findings)
}

pub(crate) fn invalid_response(error: impl std::fmt::Display) -> AppError {
    AppError::new(
        ErrorCode::InvalidModelResponse,
        format!("模型结构化响应无效：{error}"),
    )
}

fn parse_model_array<T: DeserializeOwned>(
    response: &str,
    wrapper_keys: &[&str],
) -> Result<Vec<T>, AppError> {
    let trimmed = response.trim();
    match parse_array_candidate(trimmed, wrapper_keys) {
        Ok(items) => Ok(items),
        Err(error) => {
            let Some(start) = trimmed.find('[') else {
                return Err(invalid_response(error));
            };
            let Some(end) = trimmed.rfind(']') else {
                return Err(invalid_response(error));
            };
            parse_array_candidate(&trimmed[start..=end], wrapper_keys).map_err(invalid_response)
        }
    }
}

fn parse_array_candidate<T: DeserializeOwned>(
    candidate: &str,
    wrapper_keys: &[&str],
) -> Result<Vec<T>, serde_json::Error> {
    match serde_json::from_str(candidate) {
        Ok(items) => Ok(items),
        Err(array_error) => {
            let value: serde_json::Value = serde_json::from_str(candidate)?;
            for key in wrapper_keys {
                if let Some(items) = value.get(*key) {
                    return serde_json::from_value(items.clone());
                }
            }
            Err(array_error)
        }
    }
}

fn deserialize_model_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::String(value) => Ok(value),
        serde_json::Value::Number(value) => Ok(value.to_string()),
        _ => Err(serde::de::Error::custom("expected a string or number")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn requirement_json() -> &'static str {
        r#"{"id":"R1","category":"technical","title":"技术参数","evidence":"技术参数"}"#
    }

    #[test]
    fn parses_requirements_from_fenced_model_json() {
        let response = format!("```json\n[{}]\n```", requirement_json());

        let requirements = parse_requirements(&response).unwrap();

        assert_eq!(requirements[0].id, "R1");
    }

    #[test]
    fn parses_requirements_from_wrapped_model_json() {
        let response = format!(r#"{{"requirements":[{}]}}"#, requirement_json());

        let requirements = parse_requirements(&response).unwrap();

        assert_eq!(requirements[0].title, "技术参数");
    }

    #[test]
    fn parses_chinese_requirement_categories_from_model_json() {
        let requirements = parse_requirements(
            r#"[{"id":"R1","category":"技术参数","title":"参数要求","evidence":"技术参数"}]"#,
        )
        .unwrap();

        assert_eq!(requirements[0].category, RequirementCategory::Technical);
    }

    #[test]
    fn parses_chinese_finding_statuses_from_model_json() {
        let requirements = parse_requirements(&format!("[{}]", requirement_json())).unwrap();
        let findings = parse_findings(
            r#"[{"requirementId":"R1","status":"已响应","summary":"已响应","evidence":"响应技术参数"}]"#,
            &requirements,
        )
        .unwrap();

        assert_eq!(findings[0].status, FindingStatus::Matched);
    }

    #[test]
    fn parses_numeric_model_ids_as_strings() {
        let requirements = parse_requirements(
            r#"[{"id":1,"category":"technical","title":"参数要求","evidence":"技术参数"}]"#,
        )
        .unwrap();
        let findings = parse_findings(
            r#"[{"requirementId":1,"status":"matched","summary":"已响应","evidence":"响应技术参数"}]"#,
            &requirements,
        )
        .unwrap();

        assert_eq!(requirements[0].id, "1");
        assert_eq!(findings[0].requirement_id, "1");
    }

    #[test]
    fn parses_findings_from_model_json_with_surrounding_text() {
        let requirements = parse_requirements(&format!("[{}]", requirement_json())).unwrap();
        let response = r#"以下是审核结果：
        [{"requirementId":"R1","status":"matched","summary":"已响应","evidence":"响应技术参数"}]
        请复核。"#;

        let findings = parse_findings(response, &requirements).unwrap();

        assert_eq!(findings[0].summary, "已响应");
    }
}
