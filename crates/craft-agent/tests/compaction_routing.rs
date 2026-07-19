//! 压缩路由逻辑集成测试（离线、不依赖网络）：
//! 验证 `compact()` 在「专用压缩模型优先、失败回退主模型、再失败才 Err」的路由。
//!
//! 注意：本测试只验证路由与回退逻辑，不验证真实 agnes 调用（那需要网络/API key，
//! 由 viewer crate 的集成测试覆盖）。

use craft_agent::agent::{Agent, AgentConfig, CompactionConfig, LlmProvider};
use craft_agent::core::message::{AssistantResponse, Message, StopReason};
use craft_agent::core::tool::ToolRegistry;
use std::sync::{Arc, Mutex};

/// 记录调用顺序的 mock provider。
struct MockProvider {
    name: &'static str,
    /// None = 成功返回固定摘要；Some(msg) = 每次调用都返回 Err
    fail_with: Option<String>,
    calls: Arc<Mutex<Vec<&'static str>>>,
}

impl LlmProvider for MockProvider {
    fn complete(
        &self,
        _messages: &[serde_json::Value],
        _tools: &[serde_json::Value],
    ) -> anyhow::Result<AssistantResponse> {
        self.calls.lock().unwrap().push(self.name);
        if let Some(err) = &self.fail_with {
            return Err(anyhow::anyhow!("{}", err));
        }
        Ok(AssistantResponse {
            content: Some(format!("summary-by-{}", self.name)),
            reasoning: None,
            tool_calls: vec![],
            usage: Default::default(),
            stop_reason: StopReason::Stop,
        })
    }
}

/// 构造 agent：主模型始终成功，专用模型由参数决定；并塞入足够旧消息以触发实际压缩路径。
fn build_agent(
    primary_fail: Option<String>,
    compaction_provider: Option<Box<dyn LlmProvider>>,
    calls: Arc<Mutex<Vec<&'static str>>>,
) -> Agent {
    let primary = Box::new(MockProvider {
        name: "primary",
        fail_with: primary_fail,
        calls: calls.clone(),
    });
    let mut config = AgentConfig::new("sys".into(), 1);
    config.compaction = CompactionConfig {
        context_window: 1_000_000,
        reserve: 200_000,
        keep_recent: 200,
        compaction_model: None,
        compaction_provider,
        compaction_thinking: false,
    };
    let mut agent = Agent::new(primary, ToolRegistry::new(), config);
    for i in 0..20 {
        agent.messages.push(Message::user(format!(
            "历史消息 #{i}：这是一段用于压缩测试的较长文本内容，用来占满 token 预算并触发实际的摘要生成路径。"
        )));
    }
    agent
}

#[test]
fn compaction_uses_dedicated_provider_first() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let comp = Box::new(MockProvider {
        name: "compaction",
        fail_with: None,
        calls: calls.clone(),
    });
    let mut agent = build_agent(None, Some(comp), calls.clone());
    let result = agent.compact().expect("compact 应成功");
    let log = calls.lock().unwrap().clone();
    assert_eq!(log, vec!["compaction"], "应只调用专用模型，不回退主模型");
    assert!(result.summary.contains("compaction"));
}

#[test]
fn compaction_falls_back_to_primary_on_dedicated_failure() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let comp = Box::new(MockProvider {
        name: "compaction",
        fail_with: Some("agnes 500".into()),
        calls: calls.clone(),
    });
    let mut agent = build_agent(None, Some(comp), calls.clone());
    let result = agent.compact().expect("专用失败应回退主模型成功");
    let log = calls.lock().unwrap().clone();
    // try_summarize 每个 provider 内部重试 3 次：专用 3 次失败 → 回退主模型 1 次成功
    assert!(
        log.first() == Some(&"compaction") && log.contains(&"primary"),
        "专用失败后必须回退主模型: {log:?}"
    );
    assert!(result.summary.contains("primary"));
}

#[test]
fn compaction_errors_when_both_fail() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let comp = Box::new(MockProvider {
        name: "compaction",
        fail_with: Some("agnes 500".into()),
        calls: calls.clone(),
    });
    let mut agent = build_agent(Some("primary 503".into()), Some(comp), calls.clone());
    let err = agent.compact().expect_err("两者都失败应返回 Err");
    let log = calls.lock().unwrap().clone();
    // 专用 3 次 + 主模型 3 次，都应被尝试
    assert!(
        log.iter().filter(|&&c| c == "compaction").count() == 3
            && log.iter().filter(|&&c| c == "primary").count() == 3,
        "专用与主模型各应尝试 3 次: {log:?}"
    );
    assert!(
        err.to_string().contains("均失败"),
        "错误信息应说明两者都失败: {err}"
    );
}
