## 实现 4 个 Mindcraft 学习改进点

基于 Mindcraft 调研确认的 4 个改进点（用户已全选，不计工作量）。每项单提交、门槛全绿后本地提交；probe 实测通过后才推送。

### 1. 工具分组表修复（`to_knowledge_string`）
- **位置**：`crates/craft-agent/src/core/tool.rs:256-312`
- **问题**：groups 数组引用 Mindcraft 旧工具名（`collect`/`move_to`/`digDown`/`rememberHere`…），53 个工具仅 8 个命中分组，45 个全落 `## Other Tools`，LLM 无法从工具名分组快速建立"何时该用哪个工具"的心智模型。
- **修复**：以实际注册工具名（对照 `tools_azalea.rs` 注册表 + `scripts/tools_dump.txt` 交叉核对）重建分组表，采用本项目真实分类：感知 / 移动 / 模式 / 挖掘 / 交互 / 合成 / 采集 / 放置 / 容器 / 背包 / 社交 / 元操作。分组表按注册工具名精确列出，任何未命中工具仍落 `## Other Tools`（兜底不变）。
- **回归测试**：新增 `regression_knowledge_groups_cover_all_registered_tools`——遍历注册工具集，断言每个工具都出现在某个分组标题下（`## Other Tools` 计数为 0 或仅含预期外的工具）。
- **影响**：系统提示字节变化 → 碎一次 DeepSeek 前缀缓存（一次性，C8 knowledge_cache 之后自动稳定）。这是用户明示接受的权衡。

### 2. 物品名单复数容错（`oak_plank→oak_planks`、`wheat_seed→wheat_seeds`）
- **位置**：`azalea/mod.rs:471` `normalize_item_id`、`craft.rs:28` `normalize_item`（两处同构）。
- **修复**：追加容错规则（参考 Mindcraft `commands/index.js:136`）：`ends_with("plank")` → 补 `s`；`ends_with("seed")` → 补 `s`。已带 `minecraft:` 前缀 / 已复数（`planks`/`seeds` 不以 `plank`/`seed` 结尾）不受影响。三处 `ItemKind::from_str(&normalize_item_id(...)).or_else(...)` 调用点（mod.rs:1009/1475/1584）自动受益。
- **回归测试**：`regression_normalize_item_id_plural_fallback`——`oak_plank`→`minecraft:oak_planks`、`wheat_seed`→`minecraft:wheat_seeds`、`minecraft:oak_planks` 原样、`stone`/`stick`/`oak_sapling` 不变。

### 3. LAST_GOALS 目标回顾（对标 Mindcraft `$LAST_GOALS`）
- **位置**：`crates/craft-agent/src/task.rs`（TaskManager）+ `crates/craft-agent/src/agent/mod.rs`（动态上下文注入）+ `agent/prompt.rs`（`build_dynamic_context_msg`）。
- **修复**：
  - TaskManager 增加 `recent_results: VecDeque<RecentResult>`（cap 4，含 task_id、goal、status、原因/时间）；在 `update()` 状态迁移到 Completed/Failed 时记录。
  - `build_dynamic_context_msg` 追加一条用户消息（瞬态、走 TRANSIENT_USER_PREFIXES，前缀如 `【任务回顾】`），格式对齐 Mindcraft：最近成功/失败任务列表（`✓ <goal>（完成）` / `✗ <goal>（失败: <原因>）`）。
  - **不影响系统提示字节**——全部走用户消息瞬态注入，前缀缓存不碎。
- **回归测试**：task.rs 单测（completed/failed 后被记录、cap 4 滚动）；prompt 层测试（注入消息含"任务回顾"且瞬态前缀登记）。

### 4. perceive 当前动作标签（对标 Mindcraft `$ACTION`）
- **位置**：`azalea/handler.rs:3906`（game_state JSON 构建处）+ `adapter_azalea.rs:256`（scene 拼装处）。
- **修复**：
  - handler tick 构建 game_state 时从 `state.action_mgr.peek_pending()` 读当前命令，序列化为紧凑标签（复用 `cmd_signature` 或写一个带实际坐标的短标签，如 `挖掘 (10,64,-20)`），写入 `"current_action"` 字段。
  - adapter scene 追加一行 `当前动作: <标签>`（无 pending 命令时输出 `当前动作: 空闲`，保持场景稳定简短）。
  - **不动系统提示字节**，scene 本就是用户消息。
- **回归测试**：handler 侧无需新增（依赖实机），adapter 侧用现有 perceive 单测结构补一条 scene 含当前动作行的断言（若有现成结构则加，否则 probe 验证）。

### 执行流程
1. 按 1→2→3→4 顺序实现，每个改进点完成后：
   - `cargo test --workspace` + `cargo fmt --all -- --check` + `cargo clippy --workspace --all-targets --features craft-agent-minecraft/azalea-bot -- -D warnings` 全绿
   - `git add -A && git commit`（单提交单关注点，4 个提交）
2. **probe 实测**（推送前必须）：
   - 改进点 2：`scripts/probe/` 新脚本验证 `craft oak_plank`（复数容错生效）或 `equip oak_plank` 类命令不再报未知物品。
   - 改进点 4：probe 脚本 `goto` 后紧跟 `state`，确认 scene 显示 `当前动作`。
   - 改进点 1/3 是 prompt/agent 层，用回归测试锁定 + viewer 实机观测确认分组渲染与任务回顾注入。
3. 回填 `docs/mindcraft-gap.md`（新增 P126 记录段）+ 更新 CHANGELOG。
4. 清理 `.tmp_mc/`（Mindcraft 临时下载目录）；`scripts/dump_tools.py` / `scripts/tools_dump.txt` 保留（分组表维护时可复用）。
5. 只有 probe 实测通过的改进点才 `git push origin main`（按 AGENTS.md 推送纪律，未实测项只本地提交）。
