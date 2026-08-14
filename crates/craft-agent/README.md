# craft-agent

与具体游戏无关的**纯逻辑库**，供 DSH 大脑与 viewer 桥接共用。**不含 Agent 主循环或 LLM 调用**
（in-bot 13 步循环已随阶段3清理移除，2026-08-14 起 DSH 是唯一大脑）。

## 内容

| 模块 | 职责 |
|---|---|
| `types` | `GameTool` / `MinecraftAction` / `ToolEffects` 等核心类型 |
| `tool` | `GameTool` 定义与 `ToolRegistry`（name → tool 注册表） |
| `world_memory` | 空间记忆（WorldMemory）：按区块索引，6 类记忆（资源/结构/容器/实体/危险/传送门/笔记），TTL 30s 去重，命名锚点 |
| `session` | JSONL 会话归档格式（mc_run.jsonl 只写，viewer `/api/session` 展示） |
| `task` | 23 个分层任务（tier 1-6）定义与进度解析 |
| `profile` | 3 层提示词模板数据 |
| `skill` | 技能（跨会话复用经验）数据 |
| `prompt` | `WorldInfo` 知识库结构（序列化保留，无回放驱动，兼容孤儿） |

## Key Traits（供 viewer/DSH 桥接使用）

| Trait | 职责 |
|---|---|
| `GameAdapter` | 游戏适配器：`capture()` / `perceive()` / `execute()`（viewer 桥实现） |
| `GameTool` | 工具定义：name / description / parameters / effects / execute（53 工具在 craft-agent-minecraft 实现） |

> `LlmProvider` / `Agent::run_one_turn` / `build_dynamic_instructions_msg` /
> `regression_system_prompt_byte_stable_across_obs_streak` 已随 in-bot 循环删除；
> 系统提示的字节稳定性现由 **DSH 大脑**负责。

## 历史架构（已移除，2026-08-14）

原 `Agent::run_one_turn()` 13 步主循环（drain_queues → 压缩 → 剔瞬态 → auto_perceive →
modes → SelfPrompter → 动态上下文 → WorldMemory → LLM → 纯文字检测 → 死循环检测 →
execute_batch → 技能抽取）已随阶段3清理整体删除。历史细节见
[`docs/tutorials/agent-loop.md`](../../docs/tutorials/agent-loop.md)（教程保留为历史快照）
与 [ARCHITECTURE.md](../../ARCHITECTURE.md)「历史架构（已移除）」段。
