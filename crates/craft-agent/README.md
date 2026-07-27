# craft-agent

通用游戏 Agent 运行时核心。提供与具体游戏无关的抽象层：Agent 主循环、工具系统、会话管理、上下文压缩、提示词组装。

## Key Traits

| Trait | 职责 |
|---|---|
| `GameAdapter` | 游戏适配器：`capture()` / `perceive()` / `execute()` |
| `GameTool` | 工具定义：name / description / parameters / effects / execute |
| `LlmProvider` | LLM 调用抽象：`complete(msgs, tools) → AssistantResponse` |

## Agent 主循环（13 步）

`Agent::run_one_turn()` 每轮：

1. `drain_queues()` — steering/follow_up 队列
2. 压缩检查 — 超预算 → `compact()`
3. 易变注入清理 — 移除上一轮 3 类 user message
4. auto_perceive — 注入结构化状态快照
5. modes 反应 — `check_modes()` → `[MODE: ...]` 提示
6. SelfPrompter — 重新注入 `[当前目标]`
7. 动态上下文 — WorldInfo + Skill + Few-shot + obs 警告
8. WorldMemory 邻近记忆 — 半径 64 格渲染
9. LLM complete — 带 RetryConfig 退避重试
10. 纯文字回复检测（P56）— 注入续跑 nudge
11. 死循环检测 — 4+ 重复签名 → nudge
12. 并行执行工具 — 按副作用分组，批内并行
13. 技能抽取 — 非 obs 工具调用提取经验

详见 [`docs/tutorials/agent-loop.md`](../../docs/tutorials/agent-loop.md)。

## P56-P58: Plain-Text Reply 治理

- **P56**: `is_premature_completion` 检测 9+ 关键词（✅, 任务完成, 已验证, 最终确认等），
  注入续跑 nudge 强制 LLM 继续产生 tool_calls。
- **P58**: 当 LLM 调 `set_goal(goal="")` 清空目标 + 文字宣告完成时，
  拒绝 `stop_goal()` 并注入强制 perceive 验证 nudge。
  `fake_completion_count` 计数器追踪此类绕过尝试。

## 上下文压缩

参考 pi_agent_rust 的三层精度估算 + append-only 消息队列 + LLM 驱动的摘要压缩。

## System Prompt 字节稳定性

DeepSeek prefix cache 要求 system prompt 每次调用字节完全一致。
- 严禁把动态变量塞进 system prompt
- 动态内容通过 `build_dynamic_instructions_msg()` 返回 user message
- 回归测试：`regression_system_prompt_byte_stable_across_obs_streak`

## 使用

```rust
let mut agent = Agent::new(provider, tools, config);
loop {
    let (log, done) = agent.run_one_turn()?;
    if done { break; }
}
```

详见 [`docs/tutorials/agent-loop.md`](../../docs/tutorials/agent-loop.md)。
