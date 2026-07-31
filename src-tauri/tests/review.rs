use app_lib::{
    error::{AppError, ErrorCode},
    review::{
        schema::{
            parse_findings, parse_requirements, FindingStatus, Requirement, RequirementCategory,
        },
        service::{ReviewService, StructuredModelClient},
    },
};
use std::sync::{Arc, Mutex};

#[test]
fn rejects_requirement_without_evidence() {
    let error = parse_requirements(r#"[{"id":"R-1","category":"evidence","title":"营业执照"}]"#)
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::InvalidModelResponse);
}

#[test]
fn rejects_requirements_missing_any_required_field() {
    for json in [
        r#"[{"category":"evidence","title":"营业执照","evidence":"第 1 页"}]"#,
        r#"[{"id":"R-1","title":"营业执照","evidence":"第 1 页"}]"#,
        r#"[{"id":"R-1","category":"evidence","evidence":"第 1 页"}]"#,
        r#"[{"id":"R-1","category":"evidence","title":"营业执照"}]"#,
    ] {
        assert_eq!(
            parse_requirements(json).unwrap_err().code,
            ErrorCode::InvalidModelResponse
        );
    }
}

#[test]
fn parses_valid_structured_json() {
    let requirements = parse_requirements(
        r#"[{"id":"R-1","category":"technical","title":"提供参数表","evidence":"招标文件第 3 章"}]"#,
    )
    .unwrap();
    let findings = parse_findings(
        r#"[{"requirementId":"R-1","status":"matched","summary":"已提供","evidence":"投标文件第 2 页"}]"#,
        &requirements,
    )
    .unwrap();

    assert_eq!(requirements[0].category, RequirementCategory::Technical);
    assert_eq!(findings[0].status, FindingStatus::Matched);
}

#[test]
fn parses_all_requirement_categories_and_finding_statuses() {
    let categories = [
        "disqualification",
        "scoring",
        "evidence",
        "technical",
        "timeline",
        "contract",
    ];
    let statuses = ["matched", "missing", "risk", "manualReview"];
    for category in categories {
        assert!(parse_requirements(&format!(
            r#"[{{"id":"R-1","category":"{category}","title":"要求","evidence":"第 1 页"}}]"#
        ))
        .is_ok());
    }
    let requirements = parse_requirements(
        r#"[{"id":"R-1","category":"evidence","title":"要求","evidence":"第 1 页"}]"#,
    )
    .unwrap();
    for status in statuses {
        assert!(parse_findings(
            &format!(r#"[{{"requirementId":"R-1","status":"{status}","summary":"摘要","evidence":"第 2 页"}}]"#),
            &requirements,
        )
        .is_ok());
    }
}

#[test]
fn rejects_finding_for_unknown_requirement() {
    let requirements = vec![Requirement {
        id: "R-1".into(),
        category: RequirementCategory::Evidence,
        title: "营业执照".into(),
        evidence: "招标文件第 3 章".into(),
    }];
    let error = parse_findings(
        r#"[{"requirementId":"unknown","status":"risk","summary":"未找到","evidence":"投标文件第 2 页"}]"#,
        &requirements,
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::InvalidModelResponse);
}

#[test]
fn rejects_findings_missing_any_required_field() {
    let requirements = vec![Requirement {
        id: "R-1".into(),
        category: RequirementCategory::Evidence,
        title: "营业执照".into(),
        evidence: "招标文件第 3 章".into(),
    }];
    for json in [
        r#"[{"status":"risk","summary":"摘要","evidence":"第 2 页"}]"#,
        r#"[{"requirementId":"R-1","summary":"摘要","evidence":"第 2 页"}]"#,
        r#"[{"requirementId":"R-1","status":"risk","evidence":"第 2 页"}]"#,
        r#"[{"requirementId":"R-1","status":"risk","summary":"摘要"}]"#,
    ] {
        assert_eq!(
            parse_findings(json, &requirements).unwrap_err().code,
            ErrorCode::InvalidModelResponse
        );
    }
}

#[derive(Clone)]
struct Responses(Arc<Mutex<Vec<String>>>);

impl StructuredModelClient for Responses {
    fn complete(&self, _prompt: &str) -> Result<String, app_lib::error::AppError> {
        Ok(self.0.lock().unwrap().remove(0))
    }
}

#[derive(Clone)]
struct RecordingResponses {
    responses: Arc<Mutex<Vec<String>>>,
    prompts: Arc<Mutex<Vec<String>>>,
}

impl RecordingResponses {
    fn new(responses: Vec<String>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses)),
            prompts: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl StructuredModelClient for RecordingResponses {
    fn complete(&self, prompt: &str) -> Result<String, app_lib::error::AppError> {
        self.prompts.lock().unwrap().push(prompt.into());
        Ok(self.responses.lock().unwrap().remove(0))
    }
}

#[derive(Clone)]
struct KeyedRecordingResponses {
    key: String,
    responses: Arc<Mutex<Vec<String>>>,
    prompts: Arc<Mutex<Vec<String>>>,
}

impl StructuredModelClient for KeyedRecordingResponses {
    fn complete(&self, prompt: &str) -> Result<String, app_lib::error::AppError> {
        let _key_is_only_used_by_the_client = &self.key;
        self.prompts.lock().unwrap().push(prompt.into());
        Ok(self.responses.lock().unwrap().remove(0))
    }
}

#[derive(Clone)]
struct AttemptResults(Arc<Mutex<Vec<Result<String, AppError>>>>);

impl StructuredModelClient for AttemptResults {
    fn complete(&self, _prompt: &str) -> Result<String, AppError> {
        self.0.lock().unwrap().remove(0)
    }
}

#[test]
fn retries_once_when_first_response_is_invalid() {
    let client = Responses(Arc::new(Mutex::new(vec![
        "not json".into(),
        r#"[{"id":"R-1","category":"timeline","title":"递交截止时间","evidence":"招标文件第 1 页"}]"#.into(),
    ])));
    let service = ReviewService::new(client);

    let requirements = service.extract_requirements("招标文本").unwrap();

    assert_eq!(requirements.len(), 1);
}

#[test]
fn returns_invalid_model_response_when_both_attempts_are_invalid() {
    let client = Responses(Arc::new(Mutex::new(vec!["bad".into(), "still bad".into()])));
    let service = ReviewService::new(client);

    let error = service.extract_requirements("招标文本").unwrap_err();

    assert_eq!(error.code, ErrorCode::InvalidModelResponse);
}

#[test]
fn retries_after_a_network_error_then_succeeds() {
    let client = AttemptResults(Arc::new(Mutex::new(vec![
        Err(AppError::new(
            ErrorCode::ModelConnectionHttpFailed,
            "network",
        )),
        Ok(
            r#"[{"id":"R-1","category":"evidence","title":"营业执照","evidence":"第 1 页"}]"#
                .into(),
        ),
    ])));

    assert_eq!(
        ReviewService::new(client)
            .extract_requirements("招标文本")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn retries_after_a_timeout_then_succeeds() {
    let client = AttemptResults(Arc::new(Mutex::new(vec![
        Err(AppError::new(ErrorCode::ModelConnectionTimeout, "timeout")),
        Ok(
            r#"[{"id":"R-1","category":"evidence","title":"营业执照","evidence":"第 1 页"}]"#
                .into(),
        ),
    ])));

    assert_eq!(
        ReviewService::new(client)
            .extract_requirements("招标文本")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn returns_the_second_client_error_after_two_failed_attempts() {
    let client = AttemptResults(Arc::new(Mutex::new(vec![
        Err(AppError::new(ErrorCode::ModelConnectionHttpFailed, "first")),
        Err(AppError::new(ErrorCode::ModelConnectionTimeout, "second")),
    ])));

    let error = ReviewService::new(client)
        .extract_requirements("招标文本")
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ModelConnectionTimeout);
    assert_eq!(error.message, "second");
}

#[test]
fn retries_invalid_bid_review_response_and_uses_safe_evidence_prompt() {
    let client = RecordingResponses::new(vec![
        "not json".into(),
        r#"[{"requirementId":"R-1","status":"manualReview","summary":"无法确定","evidence":"投标文件第 2 页"}]"#.into(),
    ]);
    let prompts = client.prompts.clone();
    let service = ReviewService::new(client);
    let requirements = vec![Requirement {
        id: "R-1".into(),
        category: RequirementCategory::Evidence,
        title: "营业执照".into(),
        evidence: "招标文件第 3 章".into(),
    }];

    assert_eq!(
        service.review_bid(&requirements, "投标文本").unwrap().len(),
        1
    );

    let prompts = prompts.lock().unwrap();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[0].contains("证据锚点"));
    assert!(prompts[0].contains("manualReview"));
    assert!(prompts[0].contains("不要作法律结论"));
    assert!(prompts[0].contains("tender-review-skill"));
    assert!(prompts[0].contains("## 商务线·废标"));
    assert!(prompts[0].contains("## 商务线·评分"));
}

#[test]
fn reviews_each_bid_after_requirement_extraction_and_keeps_bid_errors_by_index() {
    let client = RecordingResponses::new(vec![
        r#"[{"id":"R-1","category":"evidence","title":"营业执照","evidence":"招标文件第 1 页"}]"#.into(),
        r#"[{"requirementId":"R-1","status":"matched","summary":"已提供","evidence":"投标 A 第 1 页"}]"#.into(),
        "invalid twice first".into(),
        "invalid twice second".into(),
    ]);
    let prompts = client.prompts.clone();
    let service = ReviewService::new(client);

    let result = service
        .review_tender_and_bids("招标文本", &["投标 A".into(), "投标 B".into()])
        .unwrap();

    assert_eq!(result.requirements.len(), 1);
    assert!(result.bid_findings[0].is_ok());
    assert_eq!(
        result.bid_findings[1].as_ref().unwrap_err().code,
        ErrorCode::InvalidModelResponse
    );
    let prompts = prompts.lock().unwrap();
    assert!(prompts[0].contains("从以下招标文本提取要求"));
    assert!(prompts[0].contains("证据锚点"));
    assert!(prompts[0].contains("不要作法律结论"));
    assert!(prompts[1].contains("根据要求审核投标文本"));
    assert!(prompts[1].contains("tender-review-skill"));
    assert!(prompts[2].contains("投标 B"));
    assert!(prompts[3].contains("投标 B"));
}

#[test]
fn workflow_public_result_excludes_model_key_and_raw_model_data() {
    let raw_requirement_response = r#"[{"id":"R-1","category":"evidence","title":"营业执照","evidence":"招标文件第 1 页","ignored":"raw-requirement-response"}]"#;
    let raw_finding_response = r#"[{"requirementId":"R-1","status":"matched","summary":"已提供","evidence":"投标文件第 1 页","ignored":"raw-finding-response"}]"#;
    let client = KeyedRecordingResponses {
        key: "sk-secret-value".into(),
        responses: Arc::new(Mutex::new(vec![
            raw_requirement_response.into(),
            raw_finding_response.into(),
        ])),
        prompts: Arc::new(Mutex::new(Vec::new())),
    };
    let prompts = client.prompts.clone();
    let service = ReviewService::new(client);

    let result = service
        .review_tender_and_bids("原始招标 prompt", &["原始投标 prompt".into()])
        .unwrap();
    let serialized = serde_json::to_string(&result).unwrap();
    let prompts = prompts.lock().unwrap();

    assert!(!serialized.contains("sk-secret-value"));
    assert!(!serialized.contains(raw_requirement_response));
    assert!(!serialized.contains(raw_finding_response));
    assert!(!serialized.contains(prompts[0].as_str()));
    assert!(!serialized.contains(prompts[1].as_str()));
}
