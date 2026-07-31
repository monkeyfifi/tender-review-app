use crate::{
    error::AppError,
    review::schema::{
        invalid_response, parse_findings, parse_requirements, BidFinding, Requirement,
    },
};
use serde::Serialize;

pub trait StructuredModelClient {
    fn complete(&self, prompt: &str) -> Result<String, AppError>;
}

pub struct ModelReviewClient {
    base_url: String,
    model: String,
    api_key: String,
    timeout_seconds: u64,
}

impl ModelReviewClient {
    pub fn new(base_url: String, model: String, api_key: String, timeout_seconds: u64) -> Self {
        Self {
            base_url,
            model,
            api_key,
            timeout_seconds,
        }
    }
}

impl StructuredModelClient for ModelReviewClient {
    fn complete(&self, prompt: &str) -> Result<String, AppError> {
        tauri::async_runtime::block_on(crate::model_client::complete_model_prompt(
            &self.base_url,
            &self.model,
            &self.api_key,
            self.timeout_seconds,
            prompt,
        ))
    }
}

pub struct ReviewService<C> {
    client: C,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TenderReviewResult {
    pub requirements: Vec<Requirement>,
    pub bid_findings: Vec<Result<Vec<BidFinding>, AppError>>,
}

impl<C: StructuredModelClient> ReviewService<C> {
    pub fn new(client: C) -> Self {
        Self { client }
    }

    pub fn extract_requirements(&self, tender_text: &str) -> Result<Vec<Requirement>, AppError> {
        self.complete_with_retry(requirements_prompt(tender_text), parse_requirements)
    }

    pub fn review_bid(
        &self,
        requirements: &[Requirement],
        bid_text: &str,
    ) -> Result<Vec<BidFinding>, AppError> {
        self.complete_with_retry(findings_prompt(requirements, bid_text), |response| {
            parse_findings(response, requirements)
        })
    }

    /// Runs entirely in memory; callers decide whether any review data is persisted.
    pub fn review_tender_and_bids(
        &self,
        tender_text: &str,
        bid_texts: &[String],
    ) -> Result<TenderReviewResult, AppError> {
        let requirements = self.extract_requirements(tender_text)?;
        let bid_findings = bid_texts
            .iter()
            .map(|bid_text| self.review_bid(&requirements, bid_text))
            .collect();
        Ok(TenderReviewResult {
            requirements,
            bid_findings,
        })
    }

    fn complete_with_retry<T>(
        &self,
        prompt: String,
        parse: impl Fn(&str) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let mut last_error = None;
        for _ in 0..2 {
            match self.client.complete(&prompt) {
                Ok(response) => match parse(&response) {
                    Ok(result) => return Ok(result),
                    Err(error) => last_error = Some(error),
                },
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| invalid_response("未收到模型响应")))
    }
}

fn requirements_prompt(tender_text: &str) -> String {
    format!(
        "从以下招标文本提取要求。只返回 JSON 数组，每项为 id、category、title、evidence。category 只能是 disqualification、scoring、evidence、technical、timeline、contract。evidence 必须是文本中的证据锚点。不要作法律结论。\n\n招标文本：\n{tender_text}"
    )
}

fn findings_prompt(requirements: &[Requirement], bid_text: &str) -> String {
    let requirements = serde_json::to_string(requirements).expect("Requirement is serializable");
    format!(
        "根据要求审核投标文本，使用 tender-review-skill 的商务审查规范。只返回 JSON 数组，每项为 requirementId、status、summary、evidence。status 只能是 matched、missing、risk、manualReview。evidence 必须是投标文本中的证据锚点；无法确定时标记 manualReview。不要作法律结论。\n\n必须按 tender-review-skill 的商务线思路核对：## 商务线·废标、## 商务线·评分、## 证明文件清册、## 关键时间节点、## 合同条款·要点。summary 要写清响应情况、缺失项或需人工复核点，不能用散文替代清单结论。\n\n要求：\n{requirements}\n\n投标文本：\n{bid_text}"
    )
}
