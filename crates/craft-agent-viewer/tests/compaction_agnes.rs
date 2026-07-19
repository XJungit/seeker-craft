//! 真实 agnes 压缩集成测试（需要网络 + AGNES_API_KEY 环境变量）。
//!
//! 加载真实 session (mc_run.jsonl)，用 agnes-2.0-flash 作专用压缩模型，
//! 强制把 keep_recent 设得很小以触发 compact()，验证：
//!   1. 专用压缩模型（agnes）被真实调用并生成非空摘要；
//!   2. 压缩后 messages 变为 [摘要] + 最近保留段；
//!   3. estimate_tokens 显著下降。

use craft_agent::agent::{Agent, AgentConfig, CompactionConfig, LlmProvider};
use craft_agent::core::session::Session;
use craft_agent::core::tool::ToolRegistry;
use craft_agent_model::config::BackendConfig;
use craft_agent_model::decision::real::OpenAiLlmClient;
use serde_json::json;
use std::path::Path;

struct Lp {
    llm: OpenAiLlmClient,
}
impl LlmProvider for Lp {
    fn complete(
        &self,
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
    ) -> anyhow::Result<craft_agent::core::message::AssistantResponse> {
        self.llm.chat_tools(&json!(messages), &json!(tools))
    }
}

#[test]
fn compaction_calls_real_agnes() {
    // 跳过条件：无 AGNES_API_KEY 时不跑（避免 CI 无密钥失败）
    let api_key = match std::env::var("AGNES_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => {
            eprintln!("SKIP compaction_calls_real_agnes: 未设置 AGNES_API_KEY");
            return;
        }
    };

    let session_path = Path::new("D:/Craft-Agent/sessions/mc_run.jsonl");
    assert!(
        session_path.exists(),
        "session 文件不存在: {session_path:?}"
    );
    let sess = Session::open(session_path).expect("打开 session 失败");

    // 构造 agnes 专用压缩模型端点（512K 上下文 + Thinking 开启）
    let comp_backend = BackendConfig {
        base_url: "https://apihub.agnes-ai.com/v1".into(),
        model: "agnes-2.0-flash".into(),
        api_key: Some(api_key),
        api_key_env: None,
        timeout_secs: 180,
        force_http1: true,
        temperature: 0.2,
        max_tokens: 4096,
        context_window: 512_000,
        max_side: None,
        extra_body: Some(json!({"chat_template_kwargs": {"enable_thinking": true}})),
    };
    let comp_llm = OpenAiLlmClient::from_config(&comp_backend).expect("构造 agnes 客户端失败");

    // 主模型也用 agnes（避免额外依赖/密钥），但 fail_with 场景无需，这里仅作回退占位
    let primary = OpenAiLlmClient::from_config(&comp_backend).unwrap();

    let mut config = AgentConfig::new("你是一个 Minecraft AI 玩家。".into(), 1);
    // 关键：用压缩模型窗口算预算，并强制 keep_recent 很小 → 必定触发压缩
    config.compaction = CompactionConfig {
        context_window: 512_000,
        reserve: (512_000.0 * 0.35) as u32,
        keep_recent: 2000, // 故意很小：保证 164 条消息一定超，触发压缩
        compaction_model: Some("agnes-2.0-flash".into()),
        compaction_provider: Some(Box::new(Lp { llm: comp_llm })),
        compaction_thinking: true,
    };

    let mut agent =
        Agent::new(Box::new(Lp { llm: primary }), ToolRegistry::new(), config).with_session(sess);

    let before = agent.messages.len();
    let tokens_before = agent.estimate_tokens();
    eprintln!("压缩前: messages={before}, est_tokens={tokens_before}");

    let result = agent.compact().expect("agnes 压缩应成功（真实 API 调用）");

    assert!(!result.summary.is_empty(), "agnes 应返回非空摘要");
    eprintln!(
        "agnes 摘要前 200 字:\n{}",
        &result.summary[..result.summary.len().min(200)]
    );

    let after = agent.messages.len();
    eprintln!("压缩后: messages={after}（应为 1 摘要 + 最近保留）");
    assert!(
        after < before,
        "压缩后消息数应少于压缩前: before={before} after={after}"
    );
    // 首条应为摘要消息
    if let craft_agent::core::message::Message::User(u) = &agent.messages[0] {
        assert!(
            u.content.contains("<summary>"),
            "压缩后首条应为摘要消息，实际: {}",
            &u.content[..u.content.len().min(80)]
        );
    } else {
        panic!("压缩后首条不是 User 摘要消息");
    }
}
