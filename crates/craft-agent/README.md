# craft-agent

通用游戏 Agent 运行时核心。提供与具体游戏无关的抽象层：Agent 主循环、工具系统、会话管理、上下文压缩、提示词组装。

## Key Traits

| Trait | 职责 |
|---|---|
| `GameAdapter` | 游戏适配器：`capture()` / `perceive()` / `execute()` |
| `GameTool` | 工具定义：name / description / parameters / effects / execute |
| `LlmProvider` | LLM 调用抽象：`complete(msgs, tools) → AssistantResponse` |

## Agent 主循环

`Agent::run_one_turn()` 每轮：
1. 自动感知（auto_perceive）→ 注入游戏状态
2. 模式检查（modes）→ 紧急行为响应
3. 自提示注入 → 保持长期目标
4. 动态上下文（WorldInfo + Skills）注入
5. 组装 context → LLM call → 解析 tool_calls → 执行工具 → 记录 session

## 上下文压缩

参考 pi_agent_rust 的三层精度估算 + append-only 消息队列 + LLM 驱动的摘要压缩。

## 使用

```rust
let mut agent = Agent::new(provider, tools, config);
loop {
    let (log, done) = agent.run_one_turn()?;
    if done { break; }
}
```

详见 [`docs/tutorials/agent-loop.md`](../../docs/tutorials/agent-loop.md)。
