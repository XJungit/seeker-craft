# craft-agent-model

VLM/LLM 客户端与配置模型。提供：

- `OpenAiLlmClient` — OpenAI 兼容的 chat/tools API 客户端（DeepSeek / Agnes / 任何兼容端）
- `BackendConfig` 配置模型 — 从 `config/agent.toml` 加载多后端
- `WorldState` / `Action` — 决策树模型的感知-决策类型

## 主要类型

| 类型 | 说明 |
|---|---|
| `LlmClient` | 纯文本 / 带工具 chat |
| `DecisionClient` | 感知→决策的端到端客户端 |
| `BackendConfig` | API endpoint / model / key / timeout 配置 |

## 配置

```toml
[llm.backends.longcat]
model = "deepseek-v4-flash"
api_base = "https://api.deepseek.com"
api_key_env = "DEEPSEEK_API_KEY"
context_window = 500000
timeout_secs = 60
max_tokens = 4096
```

## 自定义 Provider

实现 `LlmProvider` trait 即可接入任意后端：

```rust
impl LlmProvider for MyProvider {
    fn complete(&self, msgs: &[Value], tools: &[Value]) -> Result<AssistantResponse> {
        // 调用你自己的 API
    }
}
```
