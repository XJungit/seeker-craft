//! Craft-Agent 全自动测试工具链
//!
//! 提供程序化测试运行、失败分析、自动修复循环的核心基础设施。
//! 设计目标：让 `cargo test` 的输出可被结构化分析，然后自动搜索代码定位根因、
//! 应用已知修复、重新编译验证，形成闭环。
//!
//! # 架构
//!
//! ```text
//! TestRunner          IssueAnalyzer         FixEngine
//!    │                     │                    │
//!    ├─ cargo test        ├─ session.jsonl     ├─ 已知问题表
//!    ├─ 解析输出           ├─ 检测 8 类问题       ├─ 源码定位
//!    └─ 结构化结果         └─ 严重度分级          └─ 自动修复
//!                              │
//!                         AutoFixLoop
//!                         ├─ detect → fix → rebuild → retest
//!                         └─ 迭代直至全部通过
//! ```

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

// ──────────────────────────────────────────────
// 1. TestRunner — 程序化测试运行
// ──────────────────────────────────────────────

/// 单个测试结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    /// 测试全名 (e.g. "regression_system_prompt_byte_stable")
    pub name: String,
    /// 所属 crate (e.g. "craft-agent")
    pub crate_name: String,
    /// 测试文件路径 (相对 workspace)
    pub file_path: Option<String>,
    /// 是否通过
    pub passed: bool,
    /// 失败消息（仅 failed 时有）
    pub failure_message: Option<String>,
    /// 耗时 (ms)
    pub duration_ms: u64,
}

/// 测试运行配置
#[derive(Debug, Clone)]
pub struct TestRunConfig {
    /// workspace 根目录
    pub workspace_root: PathBuf,
    /// 可选：只运行特定 crate 的测试
    pub crate_filter: Option<String>,
    /// 可选：只运行匹配模式的测试
    pub test_filter: Option<String>,
    /// 是否运行 --workspace
    pub workspace: bool,
    /// 超时秒数
    pub timeout_secs: u64,
}

impl Default for TestRunConfig {
    fn default() -> Self {
        Self {
            workspace_root: PathBuf::from("."),
            crate_filter: None,
            test_filter: None,
            workspace: true,
            timeout_secs: 300,
        }
    }
}

/// 测试运行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRunSummary {
    /// 所有测试结果
    pub tests: Vec<TestResult>,
    /// 通过数
    pub passed: usize,
    /// 失败数
    pub failed: usize,
    /// 总耗时 (ms)
    pub total_duration_ms: u64,
    /// 编译是否成功
    pub build_ok: bool,
    /// 编译错误消息（仅 build 失败时有）
    pub build_error: Option<String>,
}

/// 程序化测试运行器
pub struct TestRunner;

impl TestRunner {
    /// 运行 cargo test 并解析输出
    ///
    /// 支持三种模式：
    /// - `--workspace` (默认)：全量测试
    /// - `-p <crate> --lib`：指定 crate 的 lib 测试
    /// - `-p <crate> --lib <filter>`：指定 crate + 测试名过滤
    pub fn run(config: &TestRunConfig) -> Result<TestRunSummary> {
        let start = std::time::Instant::now();

        // Step 1: cargo build (编译验证)
        let build_ok;
        let build_error;
        let mut cargo = Command::new("cargo");
        cargo.arg("build").current_dir(&config.workspace_root);

        // 加 --no-fail-fast 保证所有 crate 都编译，不因第一个错误停
        cargo.arg("--no-fail-fast");
        if let Some(ref cr) = config.crate_filter {
            cargo.args(&["-p", cr]);
        }

        let build_output = cargo.output()?;
        if build_output.status.success() {
            build_ok = true;
            build_error = None;
        } else {
            build_ok = false;
            build_error = Some(
                String::from_utf8_lossy(&build_output.stderr)
                    .lines()
                    .take(50)
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            // 编译失败时仍尝试跑测试（可能部分测试可运行）
        }

        // Step 2: cargo test
        let mut tests = Vec::new();
        let mut cargo_test = Command::new("cargo");
        cargo_test.arg("test").current_dir(&config.workspace_root);

        if config.workspace {
            cargo_test.arg("--workspace");
        }
        cargo_test.arg("--no-fail-fast");
        cargo_test.arg("--color=never");

        // 格式控制：用 json 输出方便解析
        // 但 cargo test --format json 的输出和人类可读格式不同，我们用人类格式 + 正则解析
        // 这样更健壮，也兼容旧版 cargo

        if let Some(ref cr) = config.crate_filter {
            cargo_test.args(&["-p", cr]);
        }
        if let Some(ref filter) = config.test_filter {
            cargo_test.arg(filter);
        }

        let test_output = cargo_test.output()?;
        let stdout = String::from_utf8_lossy(&test_output.stdout);
        let stderr = String::from_utf8_lossy(&test_output.stderr);

        // 解析测试输出
        // 格式：test test_name ... ok / FAILED
        // 或：test test_name ... FAILED
        // 编译错误可能出现在 stderr 中
        let mut current_crate = config
            .crate_filter
            .clone()
            .unwrap_or_else(|| "unknown".into());

        for line in stdout.lines() {
            // 检测 crate 切换 (e.g., "     Running unittests src/lib.rs (craft-agent)")
            if line.contains("Running") && line.contains('(') {
                if let Some(paren_start) = line.find('(') {
                    if let Some(paren_end) = line[paren_start..].find(')') {
                        current_crate = line[paren_start + 1..paren_start + paren_end].to_string();
                    }
                }
            }

            // 测试结果行: "test test_name ... ok" 或 "test test_name ... FAILED"
            if line.trim_start().starts_with("test ") {
                let content = line.trim();
                if content.contains("... ok") {
                    let name = content[5..content.find("...").unwrap_or(content.len())]
                        .trim()
                        .to_string();
                    tests.push(TestResult {
                        name,
                        crate_name: current_crate.clone(),
                        file_path: None,
                        passed: true,
                        failure_message: None,
                        duration_ms: 0,
                    });
                } else if content.contains("... FAILED") {
                    let name = content[5..content.find("...").unwrap_or(content.len())]
                        .trim()
                        .to_string();
                    // 提取失败消息（从 stderr 中搜索该测试名的失败信息）
                    let failure_msg = extract_failure_for_test(&name, &stderr);
                    tests.push(TestResult {
                        name,
                        crate_name: current_crate.clone(),
                        file_path: None,
                        passed: false,
                        failure_message: failure_msg,
                        duration_ms: 0,
                    });
                }
            }
        }

        // 如果解析不到任何测试但退出了非零，尝试从 stderr 提取错误
        if tests.is_empty() && !test_output.status.success() {
            // 可能是编译错误导致测试无法运行
            let err_lines: Vec<&str> = stderr.lines().collect();
            if !err_lines.is_empty() {
                return Ok(TestRunSummary {
                    tests: vec![],
                    passed: 0,
                    failed: 0,
                    total_duration_ms: start.elapsed().as_millis() as u64,
                    build_ok: false,
                    build_error: Some(
                        err_lines
                            .iter()
                            .take(30)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("\n"),
                    ),
                });
            }
        }

        let passed = tests.iter().filter(|t| t.passed).count();
        let failed = tests.iter().filter(|t| !t.passed).count();

        Ok(TestRunSummary {
            tests,
            passed,
            failed,
            total_duration_ms: start.elapsed().as_millis() as u64,
            build_ok,
            build_error,
        })
    }

    /// 快速检查编译是否通过
    pub fn check_build(config: &TestRunConfig) -> Result<bool> {
        let mut cargo = Command::new("cargo");
        cargo.arg("build").current_dir(&config.workspace_root);

        if let Some(ref cr) = config.crate_filter {
            cargo.args(&["-p", cr]);
        }

        let output = cargo.output()?;
        Ok(output.status.success())
    }
}

/// 从 stderr 中提取特定测试的失败消息
fn extract_failure_for_test(test_name: &str, stderr: &str) -> Option<String> {
    let mut lines = stderr.lines().peekable();
    let mut in_target = false;
    let mut msg_lines = Vec::new();

    // 搜索 "---- test_name ----" 或 "failures:" 段落
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        // 匹配测试失败段落头
        if trimmed.starts_with("----") && trimmed.contains(test_name) {
            in_target = true;
            continue;
        }
        // 匹配 "FAILED" 行
        if trimmed.starts_with("FAILED") && trimmed.contains(test_name) {
            in_target = true;
            continue;
        }
        // 匹配 "thread 'test_name' panicked at"
        if trimmed.contains("panicked at") && trimmed.contains(test_name) {
            msg_lines.push(trimmed.to_string());
            // 收集后续几行（stack trace 的前几行）
            for _ in 0..8 {
                if let Some(next) = lines.next() {
                    let n = next.trim();
                    if n.is_empty() || n.starts_with("----") || n.starts_with("test ") {
                        break;
                    }
                    msg_lines.push(n.to_string());
                }
            }
            break;
        }

        if in_target {
            // 收集失败段落内容直到下一个分隔线
            if trimmed.starts_with("----") || trimmed.is_empty() {
                break;
            }
            msg_lines.push(trimmed.to_string());
        }
    }

    if msg_lines.is_empty() {
        None
    } else {
        Some(msg_lines.join("\n"))
    }
}

// ──────────────────────────────────────────────
// 2. IssueAnalyzer — Session 问题分析
// ──────────────────────────────────────────────

/// 单个工具的失败统计（P12 新增，用于 per-tool 错误粒度分析）。
#[derive(Debug, Clone, Default)]
struct ToolErrorStats {
    calls: usize,
    errors: usize,
    /// 去重后的错误文本样本（最多 3 条，每条截断 400 字符）。
    /// 让诊断报告直接显示「为什么失败」，免去手动翻 jsonl。
    error_samples: Vec<String>,
}

/// 分析出的问题
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzedIssue {
    pub category: String,
    pub severity: String, // "CRITICAL" | "HIGH" | "MEDIUM" | "LOW"
    pub step: Option<usize>,
    pub detail: String,
    /// 建议修复的文件路径（相对于 workspace）
    pub suggested_fix_file: Option<String>,
    /// 建议修复的简要描述
    pub suggested_fix: Option<String>,
}

/// IssueAnalyzer 分析结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisReport {
    pub issues: Vec<AnalyzedIssue>,
    pub total_steps: usize,
    pub total_calls: usize,
    pub total_errors: usize,
    pub error_rate: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Session 问题分析器
///
/// 加载 session jsonl，检测 8 类问题模式，并输出修复建议。
/// 与 scan_run.ps1 功能重叠，但提供 Rust 级 API 供程序化使用。
pub struct IssueAnalyzer;

impl IssueAnalyzer {
    /// 分析 session 文件
    pub fn analyze(session_path: &Path) -> Result<AnalysisReport> {
        let content = std::fs::read_to_string(session_path)?;
        let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();

        if lines.is_empty() {
            return Ok(AnalysisReport {
                issues: vec![],
                total_steps: 0,
                total_calls: 0,
                total_errors: 0,
                error_rate: 0.0,
                input_tokens: 0,
                output_tokens: 0,
            });
        }

        let mut issues = Vec::new();
        let mut total_calls = 0usize;
        let mut total_errors = 0usize;
        let mut total_input_tokens = 0u64;
        let mut total_output_tokens = 0u64;
        let mut step_count = 0usize;
        let mut call_signatures: Vec<(usize, String)> = Vec::new();
        let mut position_seq: Vec<(usize, i32, i32, i32)> = Vec::new();
        let mut perceive_contents: Vec<(usize, String)> = Vec::new();

        // 解析 session
        for line in &lines {
            let val: serde_json::Value = serde_json::from_str(line)?;
            let msg_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if msg_type != "message" {
                continue;
            }
            let msg = match val.get("message") {
                Some(m) => m,
                None => continue,
            };
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");

            match role {
                "assistant" => {
                    step_count += 1;
                    // 记录 usage
                    if let Some(usage) = msg.get("usage") {
                        total_input_tokens += usage
                            .get("input_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        total_output_tokens += usage
                            .get("output_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                    }
                    // 记录 tool_calls 签名
                    if let Some(calls) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                        for tc in calls {
                            let name = tc.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                            let args = tc
                                .get("arguments")
                                .map(|a| a.to_string())
                                .unwrap_or_default();
                            call_signatures.push((step_count, format!("{}|{}", name, args)));
                            total_calls += 1;
                        }
                    }
                    // 纯文字回复检测
                    let has_calls = msg
                        .get("tool_calls")
                        .and_then(|v| v.as_array())
                        .map(|a| !a.is_empty())
                        .unwrap_or(false);
                    if !has_calls {
                        if let Some(content) = msg.get("content").and_then(|v| v.as_str()) {
                            let lower = content.to_lowercase();
                            let is_pseudo = content.contains("【工具")
                                || content.contains("[工具")
                                || content.contains("→ 命令完成")
                                || lower.contains("goto(")
                                || lower.contains("mine(")
                                || lower.contains("gather(")
                                || lower.contains("craft(");
                            if is_pseudo {
                                issues.push(AnalyzedIssue {
                                    category: "伪调用文本".into(),
                                    severity: "CRITICAL".into(),
                                    step: Some(step_count),
                                    detail: format!("step {}: 文字伪调用: {:.100}", step_count, content),
                                    suggested_fix_file: Some("crates/craft-agent-model/src/decision.rs".into()),
                                    suggested_fix: Some("检查 fold_tool_history 是否被重新引入，或在 prompt 中加强 '必须用 function calling 输出工具调用' 的指令".into()),
                                });
                            }
                        }
                    }
                }
                "tool" | "toolresult" => {
                    let is_err = msg
                        .get("is_error")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if is_err {
                        total_errors += 1;
                    }
                    let tool_name = msg.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
                    let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");

                    // 从 perceive 结果提取坐标
                    if tool_name == "perceive" {
                        perceive_contents.push((step_count, content.to_string()));
                        // 提取位置
                        if let Some(pos) = extract_position(content) {
                            position_seq.push((step_count, pos.0, pos.1, pos.2));
                        }
                    }
                }
                _ => {}
            }
        }

        // 检测：伪工具名（不在已知 37 个工具集里）
        let known_tools: &[&str] = &[
            "perceive",
            "memory",
            "go",
            "goto",
            "mine",
            "mine_below",
            "mine_above",
            "interact_block",
            "attack",
            "defend",
            "craft",
            "craft_3x3",
            "smelt",
            "auto_craft",
            "enchant",
            "gather",
            "place",
            "open",
            "pickup",
            "chest_view",
            "chest_withdraw",
            "chest_deposit",
            "equip",
            "discard",
            "consume",
            "interact_entity",
            "trade",
            "chat",
            "set_goal",
            "pause_goal",
            "resume_goal",
            "build",
            "build_blueprint",
            "list_blueprints",
            "run_plan",
            "run_script",
            "new_action",
            "list_actions",
            "search_wiki",
        ];
        for (step, sig) in &call_signatures {
            if let Some(name) = sig.split('|').next() {
                if !known_tools.contains(&name) {
                    issues.push(AnalyzedIssue {
                        category: "伪工具名".into(),
                        severity: "CRITICAL".into(),
                        step: Some(*step),
                        detail: format!("step {}: 工具名 '{}' 不在已知 37 个工具集里", step, name),
                        suggested_fix_file: Some("crates/craft-agent-minecraft/src/tools_azalea.rs".into()),
                        suggested_fix: Some(format!("检查 LLM 是否在编造工具名 '{}'，需在 prompt 中强调只能使用已注册的工具", name)),
                    });
                }
            }
        }

        // 检测：重复调用（死循环）
        let mut consecutive = 1usize;
        for i in 1..call_signatures.len() {
            if call_signatures[i].1 == call_signatures[i - 1].1 {
                consecutive += 1;
                if consecutive == 4 {
                    issues.push(AnalyzedIssue {
                        category: "重复调用(死循环)".into(),
                        severity: "HIGH".into(),
                        step: Some(call_signatures[i].0),
                        detail: format!(
                            "step {}: 连续 4+ 次相同调用: {:.80}",
                            call_signatures[i].0,
                            call_signatures[i].1
                        ),
                        suggested_fix_file: Some("crates/craft-agent/src/agent/mod.rs".into()),
                        suggested_fix: Some("检查死循环检测逻辑（recent_calls），确认 nudge 注入在 tool result 之后".into()),
                    });
                }
            } else {
                consecutive = 1;
            }
        }

        // 检测：坐标卡死
        if position_seq.len() >= 5 {
            let mut streak = 1usize;
            for i in 1..position_seq.len() {
                if position_seq[i].1 == position_seq[i - 1].1
                    && position_seq[i].2 == position_seq[i - 1].2
                    && position_seq[i].3 == position_seq[i - 1].3
                {
                    streak += 1;
                    if streak == 5 {
                        let p = &position_seq[i];
                        issues.push(AnalyzedIssue {
                            category: "坐标卡死".into(),
                            severity: "HIGH".into(),
                            step: Some(p.0),
                            detail: format!(
                                "step {}: 连续 5+ 轮 position 不变: ({}, {}, {})",
                                p.0, p.1, p.2, p.3
                            ),
                            suggested_fix_file: Some("crates/craft-agent-minecraft/src/adapter_azalea.rs".into()),
                            suggested_fix: Some("检查 stuck_since 时间制卡住检测，或 goto 超时设置（当前 3s/60 ticks）".into()),
                        });
                    }
                } else {
                    streak = 1;
                }
            }
        }

        // 检测：工具失败率高
        let error_rate = if total_calls > 0 {
            total_errors as f64 / total_calls as f64
        } else {
            0.0
        };
        if error_rate > 0.3 && total_errors >= 3 {
            issues.push(AnalyzedIssue {
                category: "工具失败率高".into(),
                severity: "HIGH".into(),
                step: None,
                detail: format!(
                    "整体失败率: {}/{} ({:.1}%)",
                    total_errors,
                    total_calls,
                    error_rate * 100.0
                ),
                suggested_fix_file: Some("crates/craft-agent-minecraft/src/azalea/".into()),
                suggested_fix: Some(
                    "检查各工具的具体实现，看是超时问题、寻路问题还是服务端同步问题".into(),
                ),
            });
        }

        // P12 增强：按工具粒度统计失败率 + 收集真实错误文本样本。
        // 原 harness 只报「整体失败率 X%」，调试时仍需手动翻 jsonl 找具体错误。
        // 改为：对每个工具单独统计 calls/errors，失败率 >=30% 或 errors>=3 时单独发 issue，
        // 并附带最多 3 条去重后的真实错误文本（每条截断 400 字符）。
        // 与 scan_run.ps1 的 byTool 分析对齐，但提供 Rust 级 API 供程序化使用。
        let mut by_tool: HashMap<String, ToolErrorStats> = HashMap::new();
        // 重新遍历 tool result 收集 per-tool 错误（前面循环只累加了总数）
        for line in &lines {
            let val: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if val.get("type").and_then(|v| v.as_str()) != Some("message") {
                continue;
            }
            let msg = match val.get("message") {
                Some(m) => m,
                None => continue,
            };
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
            if role != "tool" && role != "toolresult" {
                continue;
            }
            let tool_name = msg
                .get("tool_name")
                .and_then(|v| v.as_str())
                .unwrap_or("(unknown)")
                .to_string();
            let is_err = msg
                .get("is_error")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let entry = by_tool.entry(tool_name.clone()).or_default();
            if is_err {
                entry.errors += 1;
                // 收集错误文本样本（去重，最多 3 条，每条截断 400 字符）
                let raw = msg
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                let truncated = if raw.len() > 400 {
                    format!("{} ...", &raw[..400])
                } else {
                    raw
                };
                if !truncated.is_empty() && !entry.error_samples.contains(&truncated) {
                    if entry.error_samples.len() < 3 {
                        entry.error_samples.push(truncated);
                    }
                }
            }
        }
        // 同时统计每个工具的总调用数（从 call_signatures）
        for (_, sig) in &call_signatures {
            let name = sig.split('|').next().unwrap_or("(unknown)").to_string();
            let entry = by_tool.entry(name).or_default();
            entry.calls += 1;
        }
        // 对失败率高的工具单独发 issue
        for (tool_name, stats) in &by_tool {
            if stats.calls == 0 {
                continue;
            }
            let rate = stats.errors as f64 / stats.calls as f64;
            if rate >= 0.3 || stats.errors >= 3 {
                let sev = if rate >= 0.5 { "HIGH" } else { "MEDIUM" };
                let mut detail = format!(
                    "{} : {}/{} failed ({:.0}%)",
                    tool_name,
                    stats.errors,
                    stats.calls,
                    rate * 100.0
                );
                for (i, sample) in stats.error_samples.iter().enumerate() {
                    detail.push_str(&format!("\n        why[{}]: {}", i + 1, sample));
                }
                if stats.error_samples.len() < stats.errors.min(3) {
                    detail.push_str(&format!(
                        "\n        ... and {} more error(s) not shown",
                        stats.errors - stats.error_samples.len()
                    ));
                }
                issues.push(AnalyzedIssue {
                    category: "工具失败率高".into(),
                    severity: sev.into(),
                    step: None,
                    detail,
                    suggested_fix_file: Some("crates/craft-agent-minecraft/src/azalea/".into()),
                    suggested_fix: Some(format!(
                        "查看 {} 工具实现：检查超时/寻路/服务端同步问题，或加自愈重试逻辑",
                        tool_name
                    )),
                });
            }
        }

        Ok(AnalysisReport {
            issues,
            total_steps: step_count,
            total_calls,
            total_errors,
            error_rate,
            input_tokens: total_input_tokens,
            output_tokens: total_output_tokens,
        })
    }
}

/// 从 perceive 文本中提取坐标
/// 匹配 "位置: (x, y, z)" 或 "坐标: (x, y, z)" 或 "position: (x, y, z)"
fn extract_position(content: &str) -> Option<(i32, i32, i32)> {
    let markers = ["位置:", "坐标:", "position:"];
    for marker in &markers {
        if let Some(pos) = content.find(*marker) {
            let after = &content[pos + marker.len()..];
            // 找第一个 '(' 和接下来的 ')'
            if let Some(paren_start) = after.find('(') {
                let paren_content = &after[paren_start + 1..];
                if let Some(paren_end) = paren_content.find(')') {
                    let coords = &paren_content[..paren_end];
                    let parts: Vec<&str> = coords.split(',').collect();
                    if parts.len() >= 3 {
                        let x = parts[0].trim().parse::<i32>().ok()?;
                        let y = parts[1].trim().parse::<i32>().ok()?;
                        let z = parts[2].trim().parse::<i32>().ok()?;
                        return Some((x, y, z));
                    }
                }
            }
        }
    }
    None
}

// ──────────────────────────────────────────────
// 3. FixEngine — 已知问题表 + 自动修复
// ──────────────────────────────────────────────

/// 已知问题条目（来自 AGENTS.md 和项目经验）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownIssue {
    /// 问题现象（匹配用）
    pub symptom: Vec<String>,
    /// 根因描述
    pub root_cause: String,
    /// 需要修改的文件路径数组
    pub fix_files: Vec<String>,
    /// 修复描述
    pub fix_description: String,
    /// 需要运行的回归测试
    pub regression_tests: Vec<String>,
}

/// 自动修复引擎
#[allow(dead_code)]
pub struct FixEngine {
    known_issues: Vec<KnownIssue>,
    workspace_root: PathBuf,
}

impl FixEngine {
    /// 加载已知问题表
    pub fn load(workspace_root: &Path) -> Self {
        Self {
            known_issues: build_known_issues(),
            workspace_root: workspace_root.to_path_buf(),
        }
    }

    /// 根据 IssueAnalyzer 的分析结果匹配已知问题
    pub fn match_issues<'a>(
        &'a self,
        report: &'a AnalysisReport,
    ) -> Vec<(&'a KnownIssue, &'a AnalyzedIssue)> {
        let mut matches = Vec::new();
        for issue in &report.issues {
            for known in &self.known_issues {
                for symptom in &known.symptom {
                    if issue.detail.contains(symptom) || issue.category.contains(symptom) {
                        matches.push((known, issue));
                        break;
                    }
                }
            }
        }
        matches
    }

    /// 生成修复建议报告
    pub fn generate_fix_plan(&self, report: &AnalysisReport) -> String {
        let matched = self.match_issues(report);
        let mut plan = String::new();

        plan.push_str("═══ 自动修复计划 ═══\n\n");

        if matched.is_empty() && report.issues.is_empty() {
            plan.push_str("✅ 未检测到问题，无需修复\n");
            return plan;
        }

        if matched.is_empty() && !report.issues.is_empty() {
            plan.push_str("⚠️ 检测到以下问题，但不在已知问题表中：\n");
            for issue in &report.issues {
                plan.push_str(&format!(
                    "  [{}] {}: {}\n",
                    issue.severity, issue.category, issue.detail
                ));
            }
            plan.push_str("\n需要人工分析根因后补充已知问题表。\n");
            return plan;
        }

        for (known, issue) in &matched {
            plan.push_str(&format!("📌 [{}] {}\n", issue.severity, issue.category));
            plan.push_str(&format!("   现象: {}\n", issue.detail));
            plan.push_str(&format!("   根因: {}\n", known.root_cause));
            plan.push_str(&format!("   修复: {}\n", known.fix_description));
            plan.push_str(&format!("   文件: {}\n", known.fix_files.join(", ")));
            plan.push_str(&format!(
                "   回归测试: {}\n\n",
                known.regression_tests.join(", ")
            ));
        }

        plan
    }
}

/// 构建已知问题表（与 AGENTS.md 4.2 节对齐）
fn build_known_issues() -> Vec<KnownIssue> {
    vec![
        KnownIssue {
            symptom: vec![
                "伪调用".to_string(),
                "伪工具".to_string(),
                "文本伪调用".to_string(),
                "【工具执行】".to_string(),
                "【工具调用】".to_string(),
            ],
            root_cause: "fold_tool_history 将工具调用历史折叠为文本格式，导致 LLM 模仿输出伪调用"
                .into(),
            fix_files: vec!["crates/craft-agent-model/src/decision.rs".into()],
            fix_description:
                "删除 fold_tool_history 及其调用，直接透传原始 tool_calls 和 role:tool 消息给 LLM"
                    .into(),
            regression_tests: vec!["regression_system_prompt_byte_stable".into()],
        },
        KnownIssue {
            symptom: vec![
                "坐标卡死".to_string(),
                "position 不变".to_string(),
                "卡住".to_string(),
            ],
            root_cause: "goto 超时太长或距离太远，导致 bot 卡住不动".into(),
            fix_files: vec!["crates/craft-agent-minecraft/src/azalea/mod.rs".into()],
            fix_description: "最大 32m 距离限制，超时 3s（60 ticks），stuck 检测改为时间制".into(),
            regression_tests: vec![],
        },
        KnownIssue {
            symptom: vec![
                "mine_below".to_string(),
                "挖到基岩".to_string(),
                "Y≤".to_string(),
            ],
            root_cause: "mine_below 无 Y 检测，bot 会挖穿基岩继续下挖".into(),
            fix_files: vec!["crates/craft-agent-minecraft/src/adapter_azalea.rs".into()],
            fix_description: "Y≤-61 自动停止并显示警告".into(),
            regression_tests: vec![],
        },
        KnownIssue {
            symptom: vec![
                "合成失败".to_string(),
                "craft".to_string(),
                "合成".to_string(),
            ],
            root_cause: "move_stack 放整堆导致配方形状错误，或 clear_grid 不彻底".into(),
            fix_files: vec!["crates/craft-agent-minecraft/src/azalea/craft.rs".into()],
            fix_description: "用 place_one 每次 1 个，clear_grid 用 left_click 代替 shift_click"
                .into(),
            regression_tests: vec![],
        },
        KnownIssue {
            symptom: vec![
                "熔炼超时".to_string(),
                "smelt".to_string(),
                "smelt 超时".to_string(),
            ],
            root_cause: "do_smelt 只等 1.2s 但熔炼需要 10-20s".into(),
            fix_files: vec!["crates/craft-agent-minecraft/src/azalea/craft.rs".into()],
            fix_description: "等待 20s，并增加超时时间设置".into(),
            regression_tests: vec![],
        },
        KnownIssue {
            symptom: vec!["chat 不显示".to_string(), "聊天不显示".to_string()],
            root_cause: "Event::Chat 用 Debug 格式化，内容不完整".into(),
            fix_files: vec!["crates/craft-agent-minecraft/src/azalea/mod.rs".into()],
            fix_description: "用 packet.content() 替代 Debug 格式化".into(),
            regression_tests: vec![],
        },
        KnownIssue {
            symptom: vec![
                "选中的槽位不对".to_string(),
                "selected_slot 硬编码".to_string(),
            ],
            root_cause: "selected_slot 硬编码 0，不读取 bot 实际手持槽".into(),
            fix_files: vec!["crates/craft-agent-minecraft/src/azalea/mod.rs".into()],
            fix_description: "读 bot.selected_hotbar_slot() 获取实际槽位".into(),
            regression_tests: vec![],
        },
        KnownIssue {
            symptom: vec![
                "吃东西不生效".to_string(),
                "consume".to_string(),
                "吃东西".to_string(),
            ],
            root_cause: "consume 只调一次 start_use_item，MC 需要长按".into(),
            fix_files: vec!["crates/craft-agent-minecraft/src/azalea/mod.rs".into()],
            fix_description: "每 50ms 循环 start_use_item() 持续 2.5s".into(),
            regression_tests: vec![],
        },
        KnownIssue {
            symptom: vec!["self_defense 自毁".to_string(), "自毁".to_string()],
            root_cause: "self_defense 无距离检查，攻击远处的敌对生物".into(),
            fix_files: vec!["crates/craft-agent-minecraft/src/azalea/mod.rs".into()],
            fix_description: "距离≤4 格 + !is_busy() 才触发".into(),
            regression_tests: vec![],
        },
        KnownIssue {
            symptom: vec![
                "开放失败".to_string(),
                "is_error 标志误报".to_string(),
                "is_error".to_string(),
            ],
            root_cause: "is_error 标志始终为 false，即使操作失败".into(),
            fix_files: vec!["crates/craft-agent-minecraft/src/adapter_azalea.rs".into()],
            fix_description: "实现 is_failure_detail 函数，通过分析返回消息内容判断操作是否成功"
                .into(),
            regression_tests: vec![],
        },
        KnownIssue {
            symptom: vec!["perceive 编号".to_string(), "perceive 格式".to_string()],
            root_cause: "perceive 输出格式不匹配，导致 LLM 无法理解".into(),
            fix_files: vec!["crates/craft-agent-minecraft/src/adapter_azalea.rs".into()],
            fix_description:
                "确保 perceive 输出包含位置、生命、饱食、群系、背包、资源摘要等关键信息".into(),
            regression_tests: vec![],
        },
    ]
}

// ──────────────────────────────────────────────
// 4. AutoFixLoop — 自动修复主循环
// ──────────────────────────────────────────────

/// 自动修复循环结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoFixResult {
    /// 经过的迭代次数
    pub iterations: u32,
    /// 最终测试结果
    pub final_test_summary: TestRunSummary,
    /// 修复历史
    pub fix_history: Vec<String>,
    /// 是否全部通过
    pub all_passed: bool,
}

/// 自动修复循环
///
/// 流程：
/// 1. 编译验证
/// 2. 运行测试
/// 3. 分析失败
/// 4. 匹配已知问题
/// 5. 输出修复计划
/// 6. (用户/自动) 应用修复
/// 7. 回到 1
#[allow(dead_code)]
pub struct AutoFixLoop {
    workspace_root: PathBuf,
    max_iterations: u32,
    fix_engine: FixEngine,
    fix_history: Vec<String>,
}

impl AutoFixLoop {
    pub fn new(workspace_root: &Path, max_iterations: u32) -> Self {
        Self {
            workspace_root: workspace_root.to_path_buf(),
            max_iterations,
            fix_engine: FixEngine::load(workspace_root),
            fix_history: Vec::new(),
        }
    }

    /// 运行一轮诊断：编译 + 测试 + 分析
    pub fn diagnose(&self) -> Result<(TestRunSummary, AnalysisReport)> {
        // 1. 编译 + 测试
        let config = TestRunConfig {
            workspace_root: self.workspace_root.clone(),
            workspace: true,
            ..Default::default()
        };
        let test_summary = TestRunner::run(&config)?;

        // 2. 分析 session（如果有）
        let session_path = self.workspace_root.join("sessions/mc_run.jsonl");
        let analysis = if session_path.exists() {
            IssueAnalyzer::analyze(&session_path)?
        } else {
            AnalysisReport {
                issues: vec![],
                total_steps: 0,
                total_calls: 0,
                total_errors: 0,
                error_rate: 0.0,
                input_tokens: 0,
                output_tokens: 0,
            }
        };

        Ok((test_summary, analysis))
    }

    /// 生成修复计划
    pub fn plan_fixes(&self, test_summary: &TestRunSummary, analysis: &AnalysisReport) -> String {
        let mut plan = String::new();
        plan.push_str("═══ 诊断报告 ═══\n\n");

        // 编译状态
        plan.push_str(&format!(
            "编译: {}\n",
            if test_summary.build_ok {
                "✅ 通过"
            } else {
                "❌ 失败"
            }
        ));
        if let Some(ref err) = test_summary.build_error {
            plan.push_str(&format!("编译错误 (前50行):\n{}\n", err));
        }

        // 测试状态
        plan.push_str(&format!(
            "测试: {}/{} 通过, {} 失败 ({}ms)\n\n",
            test_summary.passed,
            test_summary.passed + test_summary.failed,
            test_summary.failed,
            test_summary.total_duration_ms
        ));

        if !test_summary.failed_tests().is_empty() {
            plan.push_str("失败的测试:\n");
            for t in test_summary.failed_tests() {
                plan.push_str(&format!("  ❌ {} ({})\n", t.name, t.crate_name));
                if let Some(ref msg) = t.failure_message {
                    let head: String = msg.chars().take(200).collect();
                    plan.push_str(&format!("     原因: {}\n", head));
                }
            }
            plan.push('\n');
        }

        // Session 分析
        if analysis.total_steps > 0 {
            plan.push_str(&format!(
                "Session: {} 步, {} 工具调用, {} 错误 ({:.1}%)\n",
                analysis.total_steps,
                analysis.total_calls,
                analysis.total_errors,
                analysis.error_rate * 100.0
            ));
        }

        // 修复计划
        plan.push_str(&self.fix_engine.generate_fix_plan(analysis));

        plan
    }

    /// 运行自动修复循环（当前为诊断模式，输出修复计划）
    pub fn run_diagnose(&mut self) -> Result<AutoFixResult> {
        let (test_summary, analysis) = self.diagnose()?;
        let plan = self.plan_fixes(&test_summary, &analysis);

        println!("{}", plan);

        // 记录修复历史
        self.fix_history.push(plan);

        let all_passed = test_summary.failed == 0;
        let final_test_summary = test_summary;
        Ok(AutoFixResult {
            iterations: 0,
            final_test_summary,
            fix_history: self.fix_history.clone(),
            all_passed,
        })
    }
}

// ──────────────────────────────────────────────
// 5. TestRunSummary 扩展方法
// ──────────────────────────────────────────────

impl TestRunSummary {
    /// 获取失败的测试列表
    pub fn failed_tests(&self) -> Vec<&TestResult> {
        self.tests.iter().filter(|t| !t.passed).collect()
    }

    /// 获取通过的测试列表
    pub fn passed_tests(&self) -> Vec<&TestResult> {
        self.tests.iter().filter(|t| t.passed).collect()
    }

    /// 按 crate 分组
    pub fn by_crate(&self) -> HashMap<String, Vec<&TestResult>> {
        let mut map: HashMap<String, Vec<&TestResult>> = HashMap::new();
        for t in &self.tests {
            map.entry(t.crate_name.clone()).or_default().push(t);
        }
        map
    }
}

// ──────────────────────────────────────────────
// 6. 调试辅助
// ──────────────────────────────────────────────

/// 生成简短的测试状态摘要行
pub fn format_test_summary(summary: &TestRunSummary) -> String {
    let total = summary.passed + summary.failed;
    if summary.failed == 0 {
        format!(
            "✅ 全部 {} 个测试通过 ({}ms)",
            total, summary.total_duration_ms
        )
    } else {
        format!(
            "❌ {}/{} 通过, {} 失败 ({}ms)",
            summary.passed, total, summary.failed, summary.total_duration_ms
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_position() {
        let cases = vec![
            ("位置: (10, 64, -20)", Some((10, 64, -20))),
            ("坐标: (0, 64, 0) 脚下: grass", Some((0, 64, 0))),
            ("position: (-5, 64, 15)", Some((-5, 64, 15))),
            ("没有坐标信息", None),
            ("位置: (abc, 64, 0)", None),
        ];
        for (input, expected) in cases {
            assert_eq!(extract_position(input), expected, "input: {input}");
        }
    }

    #[test]
    fn test_known_issues_not_empty() {
        let issues = build_known_issues();
        assert!(!issues.is_empty(), "已知问题表不应为空");
        assert!(
            issues.iter().all(|i| !i.symptom.is_empty()),
            "每个问题至少有一个症状"
        );
    }

    #[test]
    fn test_test_runner_config_default() {
        let cfg = TestRunConfig::default();
        assert!(cfg.workspace);
        assert_eq!(cfg.timeout_secs, 300);
    }
}
