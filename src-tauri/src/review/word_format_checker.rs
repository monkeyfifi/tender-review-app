use crate::{
    error::{AppError, ErrorCode},
    review::blind_bid::{BlindBidCheck, BlindBidFinding},
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WordFormatViolation {
    level: String,
    category: String,
    rule: String,
    location: String,
    expected: String,
    actual: String,
    snippet: String,
    suggestion: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalEnvironmentState {
    Ready,
    MissingPython,
    MissingDependency,
    MissingChecker,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalEnvironmentStatus {
    pub state: LocalEnvironmentState,
    pub message: String,
}

pub fn technical_environment_status() -> LocalEnvironmentStatus {
    let checker_root = match word_format_checker_root() {
        Ok(root) => root,
        Err(error) => {
            return LocalEnvironmentStatus {
                state: LocalEnvironmentState::MissingChecker,
                message: error.message,
            }
        }
    };
    let python = match python_command() {
        Ok(python) => python,
        Err(error) => {
            return LocalEnvironmentStatus {
                state: LocalEnvironmentState::MissingPython,
                message: error.message,
            }
        }
    };
    match ensure_python_dependencies(&python, &checker_root) {
        Ok(()) => LocalEnvironmentStatus {
            state: LocalEnvironmentState::Ready,
            message: "环境已就绪，可以执行技术暗标格式检查".into(),
        },
        Err(error) => LocalEnvironmentStatus {
            state: LocalEnvironmentState::MissingDependency,
            message: error.message,
        },
    }
}

pub fn word_format_checker_setup_script() -> Result<PathBuf, AppError> {
    let root = word_format_checker_root()?;
    for script_name in ["setup-word-format-checker.bat", "安装技术暗标检查依赖.bat"] {
        let script = root.join(script_name);
        if script.is_file() {
            return Ok(script);
        }
    }
    Err(checker_error(
        "未找到技术暗标检查依赖安装脚本，请重新安装程序",
    ))
}

pub fn check_with_word_format_checker(path: &Path) -> Result<BlindBidCheck, AppError> {
    if !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("docx"))
    {
        return Ok(BlindBidCheck {
            skipped: true,
            message: "技术暗标格式检查仅支持 DOCX 文件，已跳过".into(),
            findings: Vec::new(),
        });
    }
    let checker_root = word_format_checker_root()?;
    let python = python_command()?;
    ensure_python_dependencies(&python, &checker_root)?;
    let output = python
        .command()
        .arg("-c")
        .arg(WORD_FORMAT_CHECKER_SCRIPT)
        .arg(&checker_root)
        .arg(path)
        .output()
        .map_err(|error| checker_error(format!("无法启动 Python 3：{error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("ModuleNotFoundError") || stderr.contains("No module named") {
            return Err(python_dependency_error(&checker_root, stderr.trim()));
        }
        return Err(checker_error(format!(
            "word-format-checker 执行失败：{}",
            stderr.trim()
        )));
    }
    let violations: Vec<WordFormatViolation> = serde_json::from_slice(&output.stdout)
        .map_err(|error| checker_error(format!("word-format-checker 返回结果无法解析：{error}")))?;
    Ok(BlindBidCheck {
        skipped: false,
        message: "技术暗标格式检查完成，需人工复核".into(),
        findings: violations
            .into_iter()
            .map(|violation| BlindBidFinding {
                biz_level: biz_level(&violation).into(),
                raw_level: violation.level,
                category: violation.category,
                rule: violation.rule,
                location: violation.location,
                expected: violation.expected,
                actual: violation.actual,
                snippet: violation.snippet,
                note: violation.suggestion,
            })
            .collect(),
    })
}

fn biz_level(violation: &WordFormatViolation) -> &'static str {
    const FORMAT_CATEGORIES: &[&str] = &[
        "页面",
        "页眉",
        "页脚",
        "段落",
        "字体",
        "字符样式",
        "文本",
        "结构",
        "表格",
        "图片",
    ];
    if FORMAT_CATEGORIES.contains(&violation.category.as_str()) {
        "必改"
    } else if violation.category == "暗标合规" || violation.rule.contains("身份信息") {
        "警告"
    } else {
        "必改"
    }
}

fn word_format_checker_root() -> Result<PathBuf, AppError> {
    for root in word_format_checker_candidates() {
        if root.join("checker.py").is_file() {
            return Ok(root);
        }
    }
    Err(checker_error(
        "未找到 word-format-checker/checker.py。安装包应已内置该工具；如需手动指定，请设置 WORD_FORMAT_CHECKER_ROOT 指向包含 checker.py 的目录。",
    ))
}

fn word_format_checker_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(configured) = std::env::var("WORD_FORMAT_CHECKER_ROOT") {
        candidates.push(PathBuf::from(configured));
    }
    if let Some(manifest_dir) = option_env!("CARGO_MANIFEST_DIR") {
        candidates.push(
            PathBuf::from(manifest_dir)
                .join("resources")
                .join("word-format-checker"),
        );
    }
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors() {
            candidates.push(ancestor.join("resources").join("word-format-checker"));
            candidates.push(ancestor.join("word-format-checker"));
        }
    }
    if let Some(home) = home_dir() {
        candidates.push(
            home.join(".codex")
                .join("skills")
                .join("word-format-checker")
                .join("assets")
                .join("word-format-checker"),
        );
    }
    dedupe_paths(candidates)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

fn dedupe_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut deduped = Vec::new();
    for path in paths {
        if !deduped.contains(&path) {
            deduped.push(path);
        }
    }
    deduped
}

#[derive(Debug, Clone)]
struct PythonCommand {
    program: String,
    prefix_args: Vec<String>,
}

impl PythonCommand {
    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.prefix_args);
        command
    }
}

fn python_command() -> Result<PythonCommand, AppError> {
    for candidate in python_candidates() {
        match candidate
            .command()
            .arg("-c")
            .arg("import sys; raise SystemExit(0 if sys.version_info >= (3, 8) else 3)")
            .output()
        {
            Ok(output) if output.status.success() => return Ok(candidate),
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(checker_error(format!(
                    "检测 Python 3 时失败：{error}。请安装 Python 3.8 或更高版本。"
                )))
            }
        }
    }
    Err(checker_error(
        "未检测到 Python 3。请安装 Python 3.8 或更高版本，并勾选 Add python.exe to PATH；安装后重新启动本程序。",
    ))
}

fn python_candidates() -> Vec<PythonCommand> {
    let mut candidates = Vec::new();
    if let Ok(configured) = std::env::var("PYTHON") {
        candidates.push(PythonCommand {
            program: configured,
            prefix_args: Vec::new(),
        });
    }
    if cfg!(windows) {
        candidates.extend([
            PythonCommand {
                program: "python".into(),
                prefix_args: Vec::new(),
            },
            PythonCommand {
                program: "py".into(),
                prefix_args: vec!["-3".into()],
            },
        ]);
    } else {
        candidates.extend([
            PythonCommand {
                program: "python3".into(),
                prefix_args: Vec::new(),
            },
            PythonCommand {
                program: "python".into(),
                prefix_args: Vec::new(),
            },
        ]);
    }
    candidates
}

fn ensure_python_dependencies(python: &PythonCommand, checker_root: &Path) -> Result<(), AppError> {
    let output = python
        .command()
        .arg("-c")
        .arg("import docx")
        .output()
        .map_err(|error| checker_error(format!("检测 Python 依赖时失败：{error}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(python_dependency_error(
            checker_root,
            String::from_utf8_lossy(&output.stderr).trim(),
        ))
    }
}

fn python_dependency_error(checker_root: &Path, details: &str) -> AppError {
    checker_error(format!(
        "已检测到 Python 3，但缺少 word-format-checker 依赖。请在命令行执行：python -m pip install -r \"{}\"；Windows 也可执行：py -3 -m pip install -r \"{}\"。详情：{}",
        checker_root.join("requirements.txt").display(),
        checker_root.join("requirements.txt").display(),
        details
    ))
}

fn checker_error(message: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::ReportGenerationFailed, message)
}

const WORD_FORMAT_CHECKER_SCRIPT: &str = r#"
import json
import pathlib
import sys

checker_root = pathlib.Path(sys.argv[1])
docx_path = pathlib.Path(sys.argv[2])
sys.path.insert(0, str(checker_root))
from checker import check_docx

violations = check_docx(docx_path.read_bytes())
print(json.dumps([item.to_dict() for item in violations], ensure_ascii=True))
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_technical_format_findings_for_docx() {
        let temp = tempfile::tempdir().unwrap();
        let path = write_docx(temp.path().join("blind.docx"), "供应商有限公司");

        let check = check_with_word_format_checker(&path).unwrap();

        assert!(check.message.contains("技术暗标格式检查完成"));
        assert!(!check.message.contains("word-format-checker"));
        assert!(check
            .findings
            .iter()
            .any(|finding| finding.category == "暗标合规" && finding.rule == "疑似身份信息"));
    }

    #[test]
    fn finds_bundled_word_format_checker_resource() {
        let root = word_format_checker_root().unwrap();

        assert!(root.join("checker.py").is_file());
        assert!(root.join("requirements.txt").is_file());
    }

    #[test]
    fn bundled_setup_scripts_are_cmd_safe_ascii() {
        let root = word_format_checker_root().unwrap();
        for script_name in ["setup-word-format-checker.bat", "安装技术暗标检查依赖.bat"] {
            let script = root.join(script_name);
            let bytes = std::fs::read(&script).unwrap();

            assert!(
                bytes.iter().all(u8::is_ascii),
                "{} must stay ASCII-only so Windows cmd can parse it before code-page setup",
                script.display()
            );
        }
    }

    #[test]
    fn reports_local_environment_status_without_running_a_document_check() {
        let status = technical_environment_status();

        assert!(!status.message.trim().is_empty());
        assert!(matches!(
            status.state,
            LocalEnvironmentState::Ready
                | LocalEnvironmentState::MissingPython
                | LocalEnvironmentState::MissingDependency
                | LocalEnvironmentState::MissingChecker
        ));
    }

    fn write_docx(path: std::path::PathBuf, text: &str) -> std::path::PathBuf {
        let python = python_command().unwrap();
        let status = python
            .command()
            .arg("-c")
            .arg(
                r#"
import sys
from docx import Document

path, text = sys.argv[1], sys.argv[2]
doc = Document()
doc.add_paragraph(text)
doc.save(path)
"#,
            )
            .arg(&path)
            .arg(text)
            .status()
            .unwrap();
        assert!(status.success());
        path
    }
}
