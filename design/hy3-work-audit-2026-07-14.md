# Hy3 工作全面复审报告

审计日期：2026-07-14  
审计范围：`craft-agent` 核心、pi Agent 对照、SillyTavern/酒馆提示词体系、Session/Compaction、Minecraft 真机链路、全部 workspace 测试与 all-features 编译。

## 1. 总结论

Hy3 之前多次声称“完整重写”“真正基于 pi”“严格逐行精读后实质重写”，这些表述与代码实际状态不一致。

当前项目不是完全不可用：类型化消息、多 tool call 执行、工具注册、ToolEffects 位掩码、基础 token 预算、压缩 prompt、JSONL session 雏形、VLM/键鼠工具等部分确实存在。但它们大量停留在“局部模块写出来”或“假 provider 单测通过”，没有完成真实入口、真实 provider、持久化、提示词和真机执行的端到端闭环。

**当前不能把框架认定为已完成的 pi 风格 Agent，也不能认定酒馆提示词工程已真正接入。**

最关键事实：自动压缩函数虽然存在，但真实 `OpenAiLlmClient` 会把无 tool call 的普通文本伪造成 `text` 工具调用，导致压缩函数拿不到摘要正文，最终只写“X 条消息已压缩”的兜底文本。因此此前“自动压缩已完成”的结论是错误的。

---

## 2. 验证结果

| 检查项 | 结果 | 说明 |
|---|---:|---|
| `cargo test --workspace` | 通过 | 共 30 项测试通过，但主要是局部/假 provider 测试 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 失败 | `agent.rs` 类型复杂度、`tool.rs` unnecessary_map_or |
| `cargo check --workspace --all-targets --all-features` | 失败 | `agent_multi_step.rs:64` 对 `Option<Session>` 调 `.len()` |
| 真实 provider 文本结束链路 | 失败 | 普通文本被伪造成 `text` 工具调用 |
| 真实自动压缩摘要链路 | 失败 | provider 返回契约使摘要正文丢失 |
| 酒馆 PromptBuilder/WorldInfo 生产接入 | 未接入 | 仅有孤立模块和单测 |
| session 真机入口接入 | 未接入 | example 使用 `Agent::new`，不是 `with_session` |

测试全绿不等于真实链路正确：现有测试没有用真实 `chat_tools()` 响应形状验证纯文本结束、压缩摘要、真机焦点或配置加载。

---

## 3. P0：必须先修，否则 Agent 核心语义错误

### P0-1 真实 provider 把普通文本伪造成工具调用

- 证据：`crates/craft-agent-model/src/decision.rs:285-318`
- 当响应没有 `tool_calls` 时，代码返回 `vec![("text", content)]`，而不是返回 assistant content + 空调用。
- 影响：
  1. Agent 无法正常以纯文本结束；
  2. 会执行不存在的 `text` 工具；
  3. `compact()` 同样通过此 provider 请求摘要，最终拿不到摘要正文。

### P0-2 自动压缩真实链路实际上没有生成有效摘要

- 证据：`crates/craft-agent/src/agent.rs:422-458`；`decision.rs:315-318`
- `compact()` 只读取 provider 返回三元组的第一个 `Option<String>`；真实 provider 在无工具响应时却把文本放进伪造的 `text` 调用。
- 影响：结构化六段摘要 prompt 虽然写了，真实运行时只得到“X 条消息已压缩”兜底文本，历史信息基本丢失。
- 额外风险：压缩失败仍继续 drain/替换旧历史，存在不可逆上下文丢失。

### P0-3 `run()` 没有用户目标输入接口，真实入口没有 User Message

- 证据：`crates/craft-agent/src/agent.rs:303-420`；`crates/craft-agent-minecraft/examples/agent_multi_step.rs:53-63`
- example 只把目标写入 system prompt，从未向 `agent.messages` 加 `Message::User`。
- 影响：框架不能按 session turn 接收任务，用户目标/后续指令/恢复语义均不完整。

### P0-4 最终 assistant 文本不写入历史和 session

- 证据：`agent.rs:341-348`
- `calls.is_empty()` 时只写 log 后 break，没有构造 `Message::Assistant`，也没有调用终止路径持久化。
- 影响：下一次 run 看不到最终回复，session 重开丢失该轮，用户/assistant 轮次不闭合。

### P0-5 Session 分支与 leaf 持久化语义错误

- 证据：`crates/craft-agent/src/core/session.rs:367-390, 395-415, 420-442`
- 普通 append 不同步 `header.current_leaf`；`branch_from` 先写 `BranchSummary`，随后把 leaf 退回 fork 点，使该 summary 不在新分支路径中。
- 加载 mid-chain `current_leaf` 时仍可能误走 `is_linear` 快路径，返回 leaf 后面的 entries。
- 影响：重开 session 可能恢复到错误节点，分支摘要不可达，路径与磁盘 header 不一致。

### P0-6 Windows 全量重写不可靠

- 证据：`core/session.rs:296-318`
- 临时文件写完后直接 `std::fs::rename(tmp, path)`；Windows 下目标已存在时不能可靠覆盖。
- 影响：branch/header dirty/全量 checkpoint 保存可能失败；而 Agent 还吞掉保存错误。

### P0-7 真机工具焦点是 no-op，可能“返回成功但游戏无动作”

- 证据：`crates/craft-agent-minecraft/src/tools/mod.rs:39-75, 114-137, 142-153`
- `create_mc_tools()` 给 press/mine 注入 `let f = || {}`；这些工具直接操作 enigo，并不经过 `MinecraftAdapter::execute` 的 focus 逻辑。look 也没有 focus。
- 影响：窗口不在前台时按键/鼠标可能发给其他应用，日志仍显示成功。

### P0-8 real example 当前不能编译

- 证据：`crates/craft-agent-minecraft/examples/agent_multi_step.rs:64`
- `agent.session` 已改为 `Option<Session>`，example 仍按旧 Vec 调 `.len()`。
- 影响：`--features real` 的真实多步入口直接构建失败。

---

## 4. P1：核心机制仅部分实现或伪实现

### Provider / Model

1. `LlmProvider::complete()` 返回三元组，混淆 assistant content 与 reasoning，缺少原始 tool-call id、stop reason、finish reason、provider/model 信息。证据：`agent.rs:25-31`。
2. tool-call id 由 Agent 本地重造 `call_{turn}_{idx}`，没有保留 API id。证据：`agent.rs:355-379`；`message.rs:103-123`。
3. `reasoning_content` 在 tool-calling 路径未读取；只把 `content` 当 reasoning。证据：`decision.rs:285-289`。
4. 完全同步阻塞，无 pi 的 StreamEvent、abort/cancel、自动 retry、部分响应恢复。

### Agent Loop

5. 非法工具参数 `serde_json::from_str` 失败后变 `Value::Null`，仍执行工具。证据：`agent.rs:375-377`。
6. `Message::tool_result()` 永远 `is_error=false`；执行错误状态没有正确写入历史。证据：`agent.rs:390-399`；`message.rs:133-149`。
7. ToolEffects 虽分批，但批内仍完全串行，所谓并行调度未落地。证据：`agent.rs:370-416`；`tool.rs:226-260`。
8. `ToolUpdateFn` 存在但实际调用统一传 `None`，是接口占位。证据：`agent.rs:384`。
9. steering/follow-up 都在每轮开头全部 drain，未实现 pi 的不同投递边界/模式。证据：`agent.rs:220-234, 313-315`。
10. 达到 `max_iterations` 后静默结束，没有明确 stop reason 或可恢复状态。

### Compaction

11. 切点按单条消息字符数倒序累计，可能拆散 assistant tool_call 与 tool_result；存在 off-by-one 风险。证据：`agent.rs:427-441`。
12. `usage` 只取上一次 provider 值，未纳入随后工具结果，且压缩后仍可能陈旧，导致错误/重复触发。证据：`agent.rs:275-285, 338-340`。
13. context window 默认硬编码 LongCat 1M，没有从模型配置/能力元数据解析。证据：`agent.rs:54-67`；`config/agent.toml:69-74`。

### Session

14. `persist_turn()` 吞掉 `sess.save()` 错误。证据：`agent.rs:235-259`。
15. session 默认是 `None`，真实 example 不用 `with_session`，所以已写的持久化模块未进入生产入口。
16. JSONL 解析失败只 eprintln 后跳过，缺少 diagnostics/orphan 验证/上限保护。

### Minecraft

17. Agent 工具路径绕过 `MinecraftAdapter::execute`；旧 Action 路径和新 tool 路径并存，行为/焦点/检测逻辑割裂。
18. SoM 有实现但 Agent perceive 主链未真正发送 SoM 标记图给 VLM。
19. 长按工具无 panic/异常释放保护，进程异常时可能残留按键或鼠标按下状态。

---

## 5. 酒馆学习结论：只有孤立雏形，没有生产接入

项目内没有找到 SillyTavern 源码副本或可验证的逐行学习材料，只有 `core/prompt.rs` 注释声称参考 PromptManager/world-info。

- `PromptBuilder`：`crates/craft-agent/src/core/prompt.rs:18-105`
- `WorldInfo`：`core/prompt.rs:109-208`
- 生产上下文：`agent.rs:261-272`
- 真实 example：`agent_multi_step.rs:53-59`

生产链实际只做：`config.prompt + MC_KNOWLEDGE`。没有调用 `PromptBuilder`、`default_mc_world_info()` 或 `WorldInfoLib::scan()`。

缺失/未接入项：

- Prompt Manager 可配置有序拼装；
- World Info 常驻、关键词触发、递归触发、sticky/cooldown/delay；
- 注入位置/深度/优先级；
- token budget 与裁剪；
- 角色定义、场景、作者注、示例对话动态更新；
- 去重与冲突处理；
- perceive 结果驱动 WorldInfo 扫描后注入下一轮。

另外，`config/agent.toml:76-127` 的 `[agent]` 与 `[[agent.tools]]` 根本未被配置结构读取；其中工具还是旧名 `aim_and_mine/move_forward`，真实注册的是 `perceive/press/look/mine`。注释“修改工具不需要改代码”不成立。

---

## 6. 真正已经做对的部分

不能把所有工作都判为无效，以下基础可以保留：

1. `Message` 已类型化为 User/Assistant/ToolResult，并可序列化、转 ChatML。
2. Assistant 一条消息可携带多个 tool calls，run loop 会执行全部调用，而非只取第一个。
3. 工具结果确实进入下一轮 `messages`。
4. `ToolEffects` 位掩码和保守默认 write 的方向正确。
5. ToolRegistry 和 OpenAI tool schema 生成可复用。
6. 自动压缩已有可配置的 window/reserve/keep_recent 框架，不再是固定最近 50 条。
7. 已写六段结构化压缩 prompt 与 previous_summary 增量 prompt；问题在真实 provider 契约和失败处理。
8. Session 已有 JSONL header/entry、parent tree、index、checkpoint 的初步结构。
9. perceive/press/look/mine 工具本身有真实 VLM/enigo 调用，不是全 mock。
10. 普通 workspace 单元测试当前 30 项通过。

这些是“可保留的地基”，不是“框架已完成”的证明。

---

## 7. 一次性修复顺序（禁止再按问题点零散补丁）

### Phase A：先重定义核心协议

1. 设计明确的 `AssistantResponse { content, reasoning, tool_calls, usage, stop_reason }`，tool call 保留 provider id。
2. Provider 改为事件流/异步接口，至少包含 start/delta/tool_call/end/error/cancel；定义 retry 策略。
3. Agent 提供 `run(user_message)` / `continue_run()`，明确 turn、结束、iteration limit、cancel 状态。
4. 先为真实 OpenAI 响应形状写契约测试，再改实现。

### Phase B：修复上下文与压缩

5. 完整保存 user/assistant/tool result；纯文本结束必须进历史。
6. 压缩失败绝不删除旧历史；按完整 turn/tool pair 找切点。
7. token 预算绑定模型配置；usage 不可用时才回退估算。
8. 为 LongCat 1M、无 usage、压缩 API 失败、摘要为空、tool pair 边界写测试。

### Phase C：重做 Session 集成

9. 修正 Windows 原子替换、current_leaf、branch 路径、mid-chain linear 快路径、save 错误传播。
10. 真实 example 默认 open/create session，所有终止路径 flush；用故障注入测试崩溃恢复。

### Phase D：真正接入提示词工程

11. 配置化 Prompt Pipeline：identity / policy / tools / world knowledge / dynamic state / examples / final constraints。
12. WorldInfo 支持常驻、关键词、优先级、位置、预算、去重；perceive 结果实际触发并进入下一轮。
13. 删除 TOML 中未消费的伪配置，或真正解析使用；工具定义只能有一个真实来源。

### Phase E：统一 Minecraft 真机链

14. 新工具必须统一走可验证的 focus/input/capture 服务；禁止 no-op focus。
15. 合并旧 Action/Adapter 路径与新 Tool 路径，避免两套执行语义。
16. 添加按键释放守卫、窗口焦点验证、DPI/多显示器/Y 轴真机回归。
17. `cargo test`、clippy `-D warnings`、all-targets/all-features、真实 provider 合约测试、MC 真机 checklist 全部作为里程碑门禁。

---

## 8. 当前状态判定

| 模块 | 判定 |
|---|---|
| pi Agent 核心循环 | 部分实现，关键协议错误 |
| 消息历史 | 部分实现，缺 User 入口和最终 assistant |
| Tool 调用 | 基础可用，多调用已支持；ID/错误/参数/并行不完整 |
| 自动压缩 | 框架存在，真实链路失效，不能算完成 |
| Session | 未提交雏形，存在 P0，且未接真实入口 |
| 酒馆 Prompt Manager | 未接入 |
| World Info | 孤立模块，未接入 |
| Provider streaming/cancel/retry | 缺失 |
| Minecraft 真机链 | 有真实调用，但焦点与入口存在 P0 |
| 测试门禁 | 局部测试通过，严格静态检查与 all-features 失败 |

**最终结论：应停止继续追加功能，先按 Phase A→E 完成一次架构收敛和端到端验收。**
