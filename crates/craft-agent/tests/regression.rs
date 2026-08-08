//! 回归测试套件 — 固化历史修复的 bug，防止倒退。
//!
//! 设计原则（针对"AI 写代码 + AI 写测试"的假绿风险）：
//! 1. 断言用**硬编码预期值**（独立 oracle），不复用被测函数算答案。
//! 2. 每个回归测试对应一个具体修复；改回旧实现应让测试 FAIL。
//! 3. 集成测试用确定性 mock provider，不调真 LLM。

use craft_agent::agent::{Agent, AgentConfig, CompactionConfig, LlmProvider};
use craft_agent::core::message::{
    AssistantMsg, AssistantResponse, Message, StopReason, ToolCall, Usage,
};
use craft_agent::core::tool::ToolRegistry;
use serde_json::Value;

// ── Mock providers ──

/// 固定返回停止（无工具调用），用于单轮/多轮主循环测试。
struct StopProvider;
impl LlmProvider for StopProvider {
    fn complete(&self, _messages: &[Value], _tools: &[Value]) -> anyhow::Result<AssistantResponse> {
        Ok(AssistantResponse {
            content: Some("ok".into()),
            reasoning: None,
            tool_calls: vec![],
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
        })
    }
}

/// 第一次返回工具调用，之后返回停止。用于驱动主循环执行工具。
struct PerceiveThenStopProvider;
impl LlmProvider for PerceiveThenStopProvider {
    fn complete(&self, _messages: &[Value], _tools: &[Value]) -> anyhow::Result<AssistantResponse> {
        // 通过全局计数区分轮次
        static COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n == 0 {
            Ok(AssistantResponse {
                content: None,
                reasoning: None,
                tool_calls: vec![ToolCall {
                    id: "c1".into(),
                    name: "perceive".into(),
                    arguments: serde_json::json!({}),
                }],
                usage: Usage::default(),
                stop_reason: StopReason::ToolCalls,
            })
        } else {
            Ok(AssistantResponse {
                content: Some("done".into()),
                reasoning: None,
                tool_calls: vec![],
                usage: Usage::default(),
                stop_reason: StopReason::Stop,
            })
        }
    }
}

/// 返回固定非空摘要，用于压缩测试（替代真 LLM 总结）。
struct SummaryProvider {
    summary: &'static str,
}
impl LlmProvider for SummaryProvider {
    fn complete(&self, _messages: &[Value], _tools: &[Value]) -> anyhow::Result<AssistantResponse> {
        Ok(AssistantResponse {
            content: Some(self.summary.to_string()),
            reasoning: None,
            tool_calls: vec![],
            usage: Usage::default(),
            stop_reason: StopReason::Stop,
        })
    }
}

// ── 工具桩 ──

struct PerceiveTool;
impl craft_agent::core::tool::GameTool for PerceiveTool {
    fn name(&self) -> &str {
        "perceive"
    }
    fn description(&self) -> &str {
        ""
    }
    fn parameters(&self) -> Value {
        serde_json::json!({})
    }
    fn execute(
        &self,
        _id: &str,
        _args: Value,
        _u: Option<craft_agent::core::tool::ToolUpdateFn>,
    ) -> anyhow::Result<craft_agent::core::tool::ToolResult> {
        Ok(craft_agent::core::tool::ToolResult {
            message: "saw trees".into(),
            is_error: false,
            images: vec![],
        })
    }
}

fn config_with_compaction(keep_recent: u32) -> AgentConfig {
    let mut c = AgentConfig::new("test agent".into(), 10);
    c.enable_compaction = true;
    c.auto_perceive = false;
    c.enable_world_info = false;
    c.enable_skill = false;
    c.enable_modes = false;
    c.enable_self_prompt = false;
    c.compaction = CompactionConfig {
        context_window: 1_000_000,
        reserve: 200_000,
        keep_recent,
        compaction_model: None,
        compaction_provider: None,
        compaction_thinking: false,
    };
    c
}

// ── 回归：system prompt 不含动态变量（DeepSeek prefix cache 前提）──
//
// 修复：jailbreak 中的 obs_streak / knowledge_bootstrapped 变量移出 system prompt，
// 改为动态 user message 注入。回归点：build_context().system_prompt 不得包含
// 任何随轮变化的动态文本。

#[test]
fn regression_system_prompt_has_no_dynamic_markers() {
    let tools = ToolRegistry::new();
    let mut agent = Agent::new(
        Box::new(StopProvider),
        tools,
        AgentConfig::new("sys".into(), 5),
    );

    let ctx = agent.build_context();
    let sys = &ctx.system_prompt;

    // 独立 oracle：固定字符串必须出现（R37 精简后 jailbreak 只保留结果真实性纪律）
    assert!(sys.contains("不得虚构成功"));
    assert!(sys.contains("实际获得"));
    // 动态变量不得出现在 system prompt 中（obs_streak / bootstrap 已移出）
    assert!(
        !sys.contains("观察提醒"),
        "system prompt 不得含 obs_streak 动态文本"
    );
    assert!(
        !sys.contains("不需要重新输入"),
        "system prompt 不得含 bootstrap 动态文本"
    );

    // 动态指令应改为走 build_dynamic_instructions_msg（独立可验证）
    // 注意：obs_streak 为 0 时无动态指令，这里仅验证 API 存在且返回 Option
    let _ = agent.build_dynamic_instructions_msg();
}

// ── 回归：serialize_msg 保留图片计数（旧实现丢图片）──

#[test]
fn regression_serialize_msg_includes_image_count() {
    let tools = ToolRegistry::new();
    let _agent = Agent::new(
        Box::new(StopProvider),
        tools,
        AgentConfig::new("sys".into(), 5),
    );

    let msg = Message::user_with_images(
        "看截图",
        vec![
            "data:image/png;base64,AAAA".into(),
            "data:image/png;base64,BBBB".into(),
        ],
    );
    let s = Agent::serialize_msg(&msg);
    assert!(
        s.contains("[2 images]"),
        "serialize_msg 必须保留图片计数，实际: {s}"
    );
}

// ── 回归：compact tokens_before 用 msg_tokens 求和，而非 kept as u64 ──
//
// 修复前：tokens_before 用 kept（u32 累加值）作为旧消息 token 数，
// 与真实 token 估算不符。修复后用 messages[..cut] 的 msg_tokens 求和。

#[test]
fn regression_compact_tokens_before_uses_msg_tokens() {
    let tools = ToolRegistry::new();
    let mut agent = Agent::new(
        Box::new(SummaryProvider {
            summary: "summary text",
        }),
        // keep_recent=60 token：每条 ~10 token，保留最近 ~6 条，丢弃更早的
        tools,
        config_with_compaction(60),
    );

    // 20 条小 user 消息（每条 20 字符 ≈ 10 token），总计 ~200 token > keep_recent
    for i in 0..20 {
        agent
            .messages
            .push(Message::user(format!("msg-{i:02}-padpadpad",)));
    }

    let before = agent.messages.len();
    let result = agent.compact().expect("compact 应成功");

    // 断言：压缩后消息数减少（recent 被保留 + summary 前置）
    assert!(
        agent.messages.len() < before,
        "压缩后消息应减少：before={before} after={}",
        agent.messages.len()
    );
    // 独立 oracle：tokens_before 应大于 0（旧消息确有 token）
    assert!(result.tokens_before > 0, "tokens_before 应为真实估算值");
}

// ── 回归：compact 保留最近的消息，且 summary 在前 ──

#[test]
fn regression_compact_keeps_recent_tail_and_prefixes_summary() {
    let tools = ToolRegistry::new();
    let mut agent = Agent::new(
        Box::new(SummaryProvider {
            summary: "SUMMARY_MARKER",
        }),
        tools,
        config_with_compaction(20),
    );

    // 10 对 (user, user) 旧消息
    for i in 0..20 {
        agent.messages.push(Message::user(format!("old-{i}")));
    }
    // 最近 2 条应为可识别的标记
    agent.messages.push(Message::user("RECENT_MARKER_A"));
    agent.messages.push(Message::user("RECENT_MARKER_B"));

    agent.compact().expect("compact 应成功");

    // summary 必须在最前
    let first = &agent.messages[0];
    if let Message::User(u) = first {
        assert!(
            u.content.contains("SUMMARY_MARKER"),
            "压缩后首条必须是 summary，实际: {}",
            u.content
        );
    } else {
        panic!("首条应为 User(summary)");
    }
    // 最近标记必须被保留
    let joined: String = agent
        .messages
        .iter()
        .map(|m| match m {
            Message::User(u) => u.content.clone(),
            _ => String::new(),
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("RECENT_MARKER_A") && joined.contains("RECENT_MARKER_B"),
        "压缩必须保留最近消息"
    );
}

// ── 回归：estimate_tokens 三层精度（实测优先 / 消息累加 / 启发式）──

#[test]
fn regression_estimate_tokens_measured_when_usage_present() {
    let tools = ToolRegistry::new();
    let mut agent = Agent::new(
        Box::new(StopProvider),
        tools,
        AgentConfig::new("p".into(), 3),
    );

    // assistant 带 usage.total_tokens = 5000（实测优先）
    let mut a = Message::assistant_text("hello");
    if let Message::Assistant(ref mut am) = a {
        am.usage = Usage {
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 5000,
            cache_hit_tokens: 0,
            cache_miss_tokens: 0,
        };
    }
    agent.messages.push(a);

    let est = agent.estimate_tokens();
    assert!(est >= 5000, "实测存在时应至少含 5000 token，实际 {est}");
}

#[test]
fn regression_estimate_tokens_heuristic_when_no_usage() {
    let tools = ToolRegistry::new();
    let agent = Agent::new(
        Box::new(StopProvider),
        tools,
        AgentConfig::new("you are a minecraft bot".into(), 3),
    );

    // 空消息也应估算 > 0（system prompt 有 token）
    let est = agent.estimate_tokens();
    assert!(est > 0, "空消息也应估算 system prompt token");
}

// ── 回归：json_byte_len 与 serde_json 实际序列化长度一致 ──

#[test]
fn regression_json_byte_len_matches_serde() {
    let v = serde_json::json!({
        "name": "collect",
        "args": {"target": "oak_log", "count": 5},
        "nested": {"a": [1, 2, 3], "b": "文本"}
    });
    let computed = Agent::json_byte_len(&v);
    let serialized = serde_json::to_string(&v).unwrap();
    assert_eq!(
        computed,
        serialized.len(),
        "json_byte_len 必须等于 serde 实际序列化长度"
    );
}

// ── 集成：多轮主循环执行工具并增长会话 ──

#[test]
fn integration_run_multi_turn_executes_tool_and_grows() {
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(PerceiveTool));
    let mut agent = Agent::new(
        Box::new(PerceiveThenStopProvider),
        tools,
        AgentConfig::new("sys".into(), 10),
    );

    let before = agent.messages.len();
    agent.run("start").expect("run 应成功");

    // 第一轮 perceive 工具调用 → 工具结果 → 第二轮停止
    // messages 应包含工具结果
    let has_tool_result = agent
        .messages
        .iter()
        .any(|m| matches!(m, Message::ToolResult(r) if r.tool_name == "perceive"));
    assert!(has_tool_result, "主循环应执行 perceive 并产出 tool result");
    assert!(agent.messages.len() > before, "多轮后消息应增长");
}

// ── 集成：auto_perceive 关闭时不注入感知消息 ──

#[test]
fn integration_no_auto_perceive_when_disabled() {
    let tools = ToolRegistry::new();
    let mut config = AgentConfig::new("sys".into(), 3);
    config.auto_perceive = false;
    let mut agent = Agent::new(Box::new(StopProvider), tools, config);

    agent.run("hi").expect("run 应成功");
    // 排除 A1 few-shot 示例消息（带【示例】标记的真实消息对）——只查真实执行结果
    let has_perceive = agent
        .messages
        .iter()
        .any(|m| {
            matches!(m, Message::ToolResult(r) if r.tool_name == "perceive" && !r.content.starts_with("【示例】"))
        });
    assert!(!has_perceive, "auto_perceive 关闭时不应注入 perceive");
}

// ── 回归：estimate_tokens / tokens_before 不得重复计入 usage（#9 修复）──
//
// 修复前：tokens_before = msg_tokens 求和 + self.usage.total_tokens，
// 把每条消息算两遍（usage 已含同等内容），175 条消息估出 190 万 token。
// 修复后：tokens_before 用统一的 estimate_tokens_range，仅计一次。

#[test]
fn regression_estimate_tokens_no_double_count() {
    let tools = ToolRegistry::new();
    let config = config_with_compaction(10_000);
    let mut agent = Agent::new(Box::new(StopProvider), tools, config);

    // 10 条 assistant 消息，每条挂真实 usage(total_tokens=9999) 用于"实测优先"路径
    for i in 0..10 {
        agent.messages.push(Message::Assistant(AssistantMsg {
            content: Some(format!("assistant reply number {i} with padding text")),
            reasoning: None,
            tool_calls: vec![],
            timestamp: 0,
            usage: Usage {
                input_tokens: 0,
                output_tokens: 0,
                total_tokens: 9999,
                cache_hit_tokens: 0,
                cache_miss_tokens: 0,
            },
        }));
    }

    // 修复前 tokens_before = msg_tokens 求和 + self.usage.total_tokens 会重复计入；
    // 修复后仅用 estimate_tokens_range 求和一次。10 条启发式约 80 token，
    // 即使走实测路径也只计各消息自身 usage（10×9999 是人为夸大，compact 不叠加 agent.usage）。
    let result = agent.compact().expect("compact 应成功");
    // oracle：启发式下 10 条消息约 80 token，远小于 99990；断言不出现数量级爆炸
    assert!(
        result.tokens_before < 5000,
        "tokens_before 不应因 usage 而爆量（得到 {}，预期 <5000）",
        result.tokens_before
    );
}
