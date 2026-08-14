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

### 生态调研（2026-08-15 补充：同类插件实证）

用户在问「有没有人把这种上下文管理做好且没出 400」后，GitHub 检索 DSH 插件生态，
找到两个**同类型（动态状态/记忆注入模型上下文）且已开源**的插件，二者都印证了上述结论：

1. **`Towzai/dsh-memory`**（跨会话记忆 + 自动注入，最像我们的场景）
   - 静态注入：`ctx.systemPrompt.context({ name:'memory:recall', order:-50, text: () => buildMemorySectionSync(storage) })` —— 与 bot_state 完全同一 API。
   - 动态注入：`ctx.on('agent/pre-step', ...)` 检索记忆后**追加到消息列表尾部**
     （`createUserMessage` + `source:{kind:'plugin', form:'recall'}`），
     注释明确：**"Appending at the tail (never mid-list) keeps the prompt prefix stable,
     so provider prefix-cache hits are preserved"** —— 主动遵守"只尾部追加、不中断缀、不删历史"。
   - 去重：`DynamicInjector` 按 session 记录 `lastSnapshots`，`if (snapshot === last) return decision`
     —— 与 `RuntimeContextProjection` 同思路。
   - **从未尝试 replace 旧消息**（无 400 风险点）。

2. **`quan2005/dsh-plugin-jinji`**（谨迹记忆，启动注入路线）
   - 只在 `agent/session-start` 异步预计算一次摘要，`ctx.systemPrompt.context({ name:'jinji:memory-summary', order:130, text: ... })`
     同步返回缓存（按 agent 缓存，不随每步变化 → 根本不需要折叠/删除）。
   - 明确遵守"context provider 同步、fs 异步 → 预取缓存"契约 —— 与我们的 botStateCache+setInterval 同模式。

3. **`Leo-Ayh-Oday/dsh-orcana`**（运行时治理，Evidence Freshness）
   - 有"证据新鲜度"概念，但实现是**注入"证据是否过期"的元信息 + steer 提醒**（告诉模型哪个旧、别信），
     不是物理删旧状态。

**结论（第三方实证）**：DSH 生态里做同类注入的插件**全部走 `systemPrompt.context` + 尾部追加 + 去重 +
compaction 兜底，没有一家实现"快照删除、下一轮重新注入"**。用户预想的"删旧快照"方案：
① 我们实测手动 append/replace → 400 `invalid_parameter_value`（见上第 5 条）；
② 生态无成功先例；
③ DSH 官方把快照留在 append-only 日志里（回放/审计），模型窗口裁剪交给 compaction。
→ 维持当前方案（`systemPrompt.context`）正确，用户方案不采用。

参考实现：`Towzai/dsh-memory`（尤其 `src/dynamic.ts` 的 pre-step 动态注入写法）可作为
dsh-bridge 后续迭代的同类参考。

### 待办
- [ ] craft-bot 预设端到端验证（重启 DSH 后开 craft-bot 会话，确认工具出现、面板显示、
      {{viewer_url}}/{{tool_list}} 不报错、bot 状态作为 user 快照注入且 30s 去重）