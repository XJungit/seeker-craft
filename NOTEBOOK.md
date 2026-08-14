# NOTEBOOK — 持续学习记录（时间顺序）

> 按 AGENTS.md「自进化持续学习」纪律维护：每轮把教训沉淀到这里（时间顺序），
> 稳定规则升级到 SKILL.md/AGENTS.md。先读本文件再迭代。

---

## 2026-08-15 — DSH 桥接：动态上下文与 DeepSeek 前缀缓存的正确做法

### 背景
DSH 成为唯一大脑后，需要把 bot 感知状态注入模型，同时保住 DeepSeek 前缀缓存命中率。
复刻 in-bot 时代的「静态→系统提示、动态→用户消息」设计。

### 关键认知（已源码+文档双重验证）

1. **turn vs step**：turn=一轮对话；step=turn 内的一次模型请求（每调一次工具多一个 step）。
   一个 turn 可数十上百 step，所以「每步注入大状态」会爆上下文——必须控制。

2. **systemPrompt.variable（系统提示内插值）vs systemPrompt.context（动态上下文）**：
   - `variable` 渲染进 **system 提示段** → 字节变化会碎前缀缓存。
   - `context` 由 agent-loop 每个 step 渲染成 **user 角色快照**追加到会话末尾
     （"Current runtime context. This snapshot supersedes earlier..."），**不进 system**
     → 不碎前缀缓存。这是 DSH 官方为动态信息准备的通道。

3. **context 的「去重 + supersedes」**：`RuntimeContextProjection.project()` 里
   `if (retained.text === snapshot) return` —— 内容没变就不追加；内容变才追加新快照，
   且 supersedes 语义让模型只认最新。配合 30s 缓存，不会每 step 都注入。

4. **DSH 的 surface replace 机制存在且能「删旧节点」**：`session.append(..., {surfaceOp:
   {op:'replace', start, end}, sourceEventSeqs:[...]})` 会 splice 删除被遮蔽节点——
   compaction 和 toolResultPruner 都用它折叠历史/剪大结果。

5. **⚠️ 手动 session.append 自制 user message 是危险 hack**：agent-loop 的
   `deriveMessages()` 会把 surface 里所有 user/message 当真实对话**原样发给 LLM**。
   手工构造的消息若不符合 DSH LLM 消息契约（id/source/content 规范结构），
   provider 直接 400（实测 `invalid_parameter_value`）。
   **生产注入必须走 systemPrompt.context（agent-loop 用 createUserMessage 构造合法消息），
   绝不自己 append。**

### 结论
- **动态感知** → `systemPrompt.context`（append+去重+supersedes）✅ 已实现（提交 84380e7）
- **静态内容**（53 工具 tool_list / viewer_url / 模式 / 护栏）→ 留在 system 提示段 ✅
- **token 控制** → toolResultPruner（剪大工具结果，默认 8192 字符阈值）
  + compaction（超阈值自动折叠历史）——craft-bot 预设已配好 ✅
- **放弃**路线 A（每步 surface replace 替换感知）：replace 机制可行但需完全对齐
  message 契约、复刻 agent-loop 内部逻辑，太脆；DSH 官方用 append+去重+compaction
  已达到「历史不无限累积」的目标。

### 待办
- [ ] craft-bot 预设端到端验证（重启 DSH 后开 craft-bot 会话，确认工具出现、面板显示、
      {{viewer_url}}/{{tool_list}} 不报错、bot 状态作为 user 快照注入且 30s 去重）