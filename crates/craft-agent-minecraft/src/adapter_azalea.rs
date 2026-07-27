//! Minecraft azalea 客户端协议层适配器（仅 `azalea-bot` 特性编译）。
//!
//! 实现 `craft_agent::core::adapter::GameAdapter`：
//! - [`perceive`]：直接读 azalea 结构化状态（坐标/背包/附近玩家），
//!   构建 `WorldState` 喂给决策层，**不依赖 VLM/截图**。
//! - [`execute`]：把 `Action::Minecraft(...)` 翻译成 `AzaleaBot` 命令。
//! - [`capture`]：azalea 路线无窗口截图，返回空占位（viewer 用 debug 渲染）。
//!
//! 这补齐了 Fabric mod 路线的"治本缺口"：原生结构化感知 + 原生动作执行，
//! 无需维护 Java 补丁或 OS 级键鼠模拟。

use crate::azalea::{AzaleaBot, BotCommand, BotEvent};
use anyhow::{Context, Result, anyhow};
use craft_agent::core::adapter::GameAdapter;
use craft_agent::core::memory::WorldMemory;
use craft_agent::core::types::{Action, ExecResult, MinecraftAction, Screenshot, WorldState};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Minecraft azalea 适配器。
#[allow(dead_code)]
pub struct MinecraftAzaleaAdapter {
    bot: Arc<AzaleaBot>,
    /// 缓存最近一次结构化状态（perceive 后供 execute / harness 使用）。
    last: Mutex<Option<WorldState>>,
    /// 卡住检测：上次位置 + 首次卡住的时间戳（秒）。
    /// 旧实现用 stuck_count 每 State 事件 +1，1 秒 20 tick 就累计成"50 轮"误导 LLM。
    /// 改为时间制：记录首次未移动的秒数，显示"卡住 N 秒"而非"轮"。
    last_y: Mutex<Option<azalea::Vec3>>,
    stuck_since: Mutex<Option<u64>>,
    /// 共享世界记忆库（由 Agent 传入，perceive/action 后回填）。可为空（不记录）。
    memory: Option<WorldMemory>,
    /// 玩家聊天消息队列（agent loop 每步前消费）
    pub chat_queue: Arc<Mutex<VecDeque<String>>>,
}

#[derive(Clone)]
pub struct ArcAzaleaAdapter(pub Arc<Mutex<MinecraftAzaleaAdapter>>);

impl ArcAzaleaAdapter {
    /// 连接并构造共享 Arc 适配器（转发至 `MinecraftAzaleaAdapter::connect`）。
    pub async fn connect(address: &str, username: &str) -> Result<ArcAzaleaAdapter> {
        MinecraftAzaleaAdapter::connect(address, username, None).await
    }
    /// 带世界记忆库的连接（记忆由 Agent 共享，适配器回填）。
    pub async fn connect_with_memory(
        address: &str,
        username: &str,
        memory: WorldMemory,
    ) -> Result<ArcAzaleaAdapter> {
        MinecraftAzaleaAdapter::connect(address, username, Some(memory)).await
    }
}

impl GameAdapter for ArcAzaleaAdapter {
    fn capture(&self) -> Result<Screenshot> {
        // azalea 路线无窗口截图；返回空占位（viewer 用结构化状态渲染）。
        Ok(Arc::new(Vec::new()))
    }

    fn perceive(&self) -> Result<WorldState> {
        self.0.lock().unwrap().perceive()
    }

    fn perceive_with_prompt(&self, _prompt: &str) -> Result<String> {
        // azalea 路线用结构化数据，无 VLM 视觉补充需求。
        Err(anyhow!(
            "azalea 适配器无 VLM 视觉补充；日常感知请用 perceive()（精确结构化数据）"
        ))
    }

    fn execute(&mut self, action: Action) -> Result<ExecResult> {
        self.0.lock().unwrap().execute(action)
    }
}

impl ArcAzaleaAdapter {
    /// `&self` 版执行（供工具上下文持有 Arc 时调用，无需 &mut）。
    pub fn execute_shared(&self, action: Action) -> Result<ExecResult> {
        self.0.lock().unwrap().execute(action)
    }
    /// `&self` 版感知。
    pub fn perceive_shared(&self) -> Result<WorldState> {
        self.0.lock().unwrap().perceive()
    }
    /// 消费玩家聊天消息队列
    pub fn drain_chat(&self) -> Vec<String> {
        let guard = self.0.lock().unwrap();
        let mut q = guard.chat_queue.lock().unwrap();
        q.drain(..).collect()
    }
}

impl MinecraftAzaleaAdapter {
    /// 连接本机已开放的 vanilla 26.2 局域网服（如 localhost:4444）。
    /// 返回 `ArcAzaleaAdapter`（共享 Arc），后台任务消费 bot 事件流更新缓存。
    pub async fn connect(
        address: &str,
        username: &str,
        memory: Option<WorldMemory>,
    ) -> Result<ArcAzaleaAdapter> {
        let bot = AzaleaBot::connect(address, username, memory.clone())
            .await
            .context("azalea bot 连接失败（确认服为纯 vanilla 26.2 且未开客户端 mod 校验）")?;
        let bot = Arc::new(bot);

        let chat_queue: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
        let adapter = Arc::new(Mutex::new(Self {
            bot: bot.clone(),
            last: Mutex::new(None),
            last_y: Mutex::new(None),
            stuck_since: Mutex::new(None),
            memory,
            chat_queue: chat_queue.clone(),
        }));

        // 后台消费事件流，更新共享 Arc 内的 `last` 缓存。
        // 用独立 OS 线程 + 独立 current_thread runtime 跑，与 demo 主
        // runtime 完全隔离，避免主 runtime 退出时 drop 嵌套 panic。
        let adapter_weak = Arc::downgrade(&adapter);
        let bot_for_task = bot.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("azalea adapter event runtime");
            rt.block_on(async move {
                while let Some(ev) = bot_for_task.next_event().await {
                    if let Some(a) = adapter_weak.upgrade() {
                        // 处理聊天消息：转发到 chat_queue
                        if let BotEvent::Chat { content } = ev {
                            let chat_queue = {
                                let g = a.lock().unwrap();
                                g.chat_queue.clone()
                            };
                            chat_queue.lock().unwrap().push_back(content);
                            continue;
                        }
                        let g = a.lock().unwrap();
                        if let BotEvent::State {
                            position,
                            inventory,
                            player_count,
                            yaw,
                            pitch,
                            block_under,
                            block_ahead,
                            health,
                            food,
                            saturation: _,
                            held_item,
                            biome,
                            nearby,
                            nearby_blocks,
                            nearby_entities,
                            game_state,
                        } = ev
                        {
                            // 卡住检测：按时间（秒）计，不是按 State 事件计数。
                            // 旧实现用 stuck_count 每 State +1，但 State 每 20 tick（1 秒）发一次，
                            // 50 秒就累计成"50 轮"误导 LLM 以为已经卡了 50 轮对话。
                            // 改为记录首次卡住的秒数，显示"卡住 N 秒"。
                            let mut last_pos = g.last_y.lock().unwrap();
                            let mut stuck_since = g.stuck_since.lock().unwrap();
                            let now_secs = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0);
                            let moved = match *last_pos {
                                Some(p) => {
                                    (position.x - p.x).abs() > 0.5
                                        || (position.y - p.y).abs() > 0.5
                                        || (position.z - p.z).abs() > 0.5
                                }
                                None => true,
                            };
                            if moved {
                                *stuck_since = None;
                            } else if stuck_since.is_none() {
                                *stuck_since = Some(now_secs);
                            }
                            *last_pos = Some(position);
                            // 注：原"卡住 N 秒"提示词已移除——挖掘/合成/采集时 position 不变
                            // 但 bot 实际在工作，时间制误报严重，反复误导 LLM 触发 goto 脱困
                            // 死循环。卡住检测由 agent 层的死循环检测（recent_calls 4+ 重复）
                            // 兜底，perceive 不再注入此 hint。
                            drop(stuck_since);
                            drop(last_pos);
                            let stuck_hint = String::new();
                            // 资源分类摘要：把 10x10 方块列表归纳为 木材/石头/矿石 三类总量。
                            // 避免 WI 模板把整个 10x10 行作为 label（"Wood source: 10x10: [stone:571, ...]"）。
                            // LLM 看摘要即可决策；需要精确坐标时用 memory 工具查询。
                            let resource_summary = summarize_resources(&nearby_blocks);
                            // P0 改进3: 语义压缩 — 只保留有价值的方块，过滤空气/石头/泥土
                            let compressed_blocks = compress_block_list(&nearby_blocks);
                            let blocks_line = if compressed_blocks.is_empty() {
                                String::new()
                            } else {
                                format!("\n特殊方块: [{}]", compressed_blocks)
                            };
                            let scene = format!(
                                "位置: ({:.0}, {:.0}, {:.0})\n\
                                 生命: {:.0}/20  饱食: {}/20  主手: {}\n\
                                 群系: {}  脚下: {}  前方: {}\n\
                                 附近: [{}]\n\
                                 资源: {}{}\n\
                                 实体: [{}]\n\
                                 背包: [{}]\n\
                                 玩家: {}{}",
                                position.x,
                                position.y,
                                position.z,
                                health,
                                food,
                                held_item,
                                biome,
                                block_under,
                                block_ahead,
                                nearby,
                                resource_summary,
                                blocks_line,
                                nearby_entities,
                                inventory,
                                player_count,
                                stuck_hint
                            );
                            *g.last.lock().unwrap() = Some(WorldState {
                                scene_desc: scene.clone(),
                                marked_elements: vec![],
                                detected_targets: vec![],
                                self_hint: scene,
                                screenshot: Arc::new(Vec::new()),
                                health: Some(health),
                                hunger: Some(food),
                                experience_level: game_state["experience_level"]
                                    .as_u64()
                                    .map(|v| v as u32),
                                experience_progress: game_state["experience_progress"]
                                    .as_f64()
                                    .map(|v| v as f32),
                                position: Some(vec![position.x, position.y, position.z]),
                                yaw: Some(yaw),
                                pitch: Some(pitch),
                                biome: Some(biome),
                                gamemode: Some("survival".to_string()),
                                inventory: game_state["inventory"].as_array().cloned(),
                                held_item: Some(held_item),
                                selected_slot: game_state["selected_slot"]
                                    .as_u64()
                                    .map(|v| v as usize),
                            });
                        }
                    } else {
                        break;
                    }
                }
            });
        });

        Ok(ArcAzaleaAdapter(adapter))
    }

    fn perceive(&self) -> Result<WorldState> {
        // 首帧 State 可能尚未到达（connect 返回早于首次 Tick 快照）。
        // 轮询等待最多 ~3s，避免返回占位串导致 LLM 首回合拿到无意义 context。
        for _ in 0..30 {
            if let Some(st) = self.last.lock().unwrap().clone() {
                return Ok(self.refresh_position(st));
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if let Some(st) = self.last.lock().unwrap().clone() {
            Ok(self.refresh_position(st))
        } else {
            Ok(WorldState {
                scene_desc: "等待首次状态快照...".to_string(),
                marked_elements: vec![],
                detected_targets: vec![],
                self_hint: "等待首次状态快照...".to_string(),
                screenshot: Arc::new(Vec::new()),
                health: None,
                hunger: None,
                experience_level: None,
                experience_progress: None,
                position: None,
                yaw: None,
                pitch: None,
                biome: None,
                gamemode: None,
                inventory: None,
                held_item: None,
                selected_slot: None,
            })
        }
    }

    fn execute(&mut self, action: Action) -> Result<ExecResult> {
        match action {
            Action::Minecraft(mc) => self.execute_mc(mc),
            other => Err(anyhow!(
                "azalea 适配器仅接受 Action::Minecraft，收到: {:?}",
                other
            )),
        }
    }

    /// P8 修复（2026-07-26）：用 bot 实时位置覆盖 WorldState 缓存中的 position 字段。
    ///
    /// 背景：BotEvent::State 每 20 tick（1s）推送一次，perceive 调用时拿到的 WorldState
    /// 可能是 1 秒前的快照——bot 实际已移动到新位置，但 perceive 报告旧位置，
    /// 导致 perception_drift（scan_run 检测到 8.4m 偏差）。
    ///
    /// 修复：perceive 返回前，从 `bot.last_position`（每 tick 更新的实时位置）读取最新坐标,
    /// 覆盖 WorldState.position 和 scene_desc 中的"位置: (x, y, z)"行。
    /// 同时刷新 scene_desc 中的位置行，确保 LLM 看到的文本与 position 字段一致。
    fn refresh_position(&self, mut st: WorldState) -> WorldState {
        if let Some(real_pos) = self.bot.last_position.lock().unwrap().clone() {
            // 覆盖结构化 position 字段
            st.position = Some(vec![real_pos.x, real_pos.y, real_pos.z]);
            // 覆盖 scene_desc 中的"位置: (x, y, z)"行
            let new_pos_line = format!(
                "位置: ({:.0}, {:.0}, {:.0})",
                real_pos.x, real_pos.y, real_pos.z
            );
            if let Some(idx) = st.scene_desc.find("位置: (") {
                // 找到"位置: ("后到第一个换行符的范围,替换为新位置行
                let line_end = st.scene_desc[idx..]
                    .find('\n')
                    .map(|e| idx + e)
                    .unwrap_or(st.scene_desc.len());
                st.scene_desc.replace_range(idx..line_end, &new_pos_line);
            }
        }
        st
    }

    /// 将 MinecraftAction 转换为 BotCommand（同步等待结果）。
    pub fn exec_mc_sync(&self, mc: MinecraftAction, timeout_ms: u64) -> Result<ExecResult> {
        let cmd = mc_to_cmd(mc);
        match self.bot.push_cmd_and_wait(cmd, timeout_ms) {
            Ok(msg) => {
                // P5 关键修复：handler 通过 String 通道回传结果，成功/失败都走 Ok(msg)。
                // 原代码无条件 ok: true，导致 "Failed to..." 消息也被标记为成功，
                // 工具层 is_error=false，scan_run.ps1 漏报失败率。
                // 这里通过消息内容检测失败（所有失败消息都包含这些关键词）。
                let ok = !is_failure_detail(&msg);
                Ok(ExecResult { ok, detail: msg })
            }
            Err(e) => Ok(ExecResult {
                ok: false,
                detail: format!("{e}"),
            }),
        }
    }

    fn execute_mc(&mut self, mc: MinecraftAction) -> Result<ExecResult> {
        // 所有动作同步等待结果，让工具返回真实反馈而非"已下发"
        self.exec_mc_sync(mc, 120_000)
    }
}

/// 检测 handler 回传的消息是否表示失败。
///
/// handler 通过 String 通道回传结果（成功/失败都是 String），无法直接区分。
/// 所有失败消息都包含以下关键词之一（与 mod.rs 里 tx.send 的格式对齐）：
/// - 英文："Failed to " / " failed: " / "Pickup failed"
/// - 中文："失败" / "未持有" / "未知物品" / "无空间" / "不支持的槽位" / "获取背包失败"
///
/// 成功消息（"Successfully" / "Placed" / "Opened" / "已装备" / "已开始" 等）不含这些词。
fn is_failure_detail(msg: &str) -> bool {
    msg.contains("Failed to ")
        || msg.contains(" failed: ")
        || msg.contains("Pickup failed")
        || msg.contains("失败")
        || msg.contains("未持有")
        || msg.contains("未知物品")
        || msg.contains("无空间")
        || msg.contains("不支持的槽位")
        || msg.contains("获取背包失败")
        || msg.contains("命令执行超时")
        || msg.contains("超时")
        || msg.contains("cannot")
        || msg.contains("can't")
        || msg.contains("不能")
        || msg.contains("无法")
        || msg.contains("不足")
        || msg.contains("没有")
        || msg.contains("not found")
        || msg.contains("not have")
        || msg.contains("insufficient")
}

/// 将 MinecraftAction 转换为 BotCommand（供 push_cmd_and_wait 使用）。
fn mc_to_cmd(mc: MinecraftAction) -> BotCommand {
    match mc {
        MinecraftAction::Goto { x, y, z } => BotCommand::Goto { x, y, z },
        MinecraftAction::MineBlock { x, y, z } => BotCommand::Mine { x, y, z },
        MinecraftAction::MineBelow => BotCommand::MineBelow,
        MinecraftAction::MineAbove => BotCommand::MineAbove,
        MinecraftAction::InteractBlock { x, y, z } => BotCommand::BlockInteract { x, y, z },
        MinecraftAction::Chat { content } => BotCommand::Chat { content },
        MinecraftAction::Attack { target } => BotCommand::Attack { target },
        MinecraftAction::Craft { item, count } => BotCommand::Craft2x2 { item, count },
        MinecraftAction::Craft3x3 {
            item,
            count,
            table_pos,
        } => BotCommand::Craft3x3 {
            item,
            count,
            table_pos,
        },
        MinecraftAction::Smelt {
            output,
            fuel,
            count,
            table_pos,
        } => BotCommand::Smelt {
            output,
            fuel,
            count,
            table_pos,
        },
        MinecraftAction::Gather { item, count } => BotCommand::Gather { item, count },
        MinecraftAction::Place { item, x, y, z } => BotCommand::Place { item, x, y, z },
        MinecraftAction::OpenContainer { x, y, z } => BotCommand::OpenContainer { x, y, z },
        MinecraftAction::AutoCraft { item, count } => BotCommand::AutoCraft { item, count },
        MinecraftAction::Enchant { item, level } => BotCommand::Enchant { item, level },
        MinecraftAction::Trade { offer } => BotCommand::Trade { offer },
        MinecraftAction::InteractEntity { kind } => BotCommand::InteractEntity { kind },
        MinecraftAction::Pickup => BotCommand::Pickup,
        MinecraftAction::Defend => BotCommand::Defend,
        MinecraftAction::Equip { item, slot } => BotCommand::Equip { item, slot },
        MinecraftAction::Discard { item, count } => BotCommand::Discard { item, count },
        MinecraftAction::Consume { item } => BotCommand::Consume { item },
        MinecraftAction::ChestView { x, y, z } => BotCommand::ChestView { x, y, z },
        MinecraftAction::ChestWithdraw {
            x,
            y,
            z,
            item,
            count,
        } => BotCommand::ChestWithdraw {
            x,
            y,
            z,
            item,
            count,
        },
        MinecraftAction::ChestDeposit {
            x,
            y,
            z,
            item,
            count,
        } => BotCommand::ChestDeposit {
            x,
            y,
            z,
            item,
            count,
        },
    }
}

/// 把 10x10 方块列表（"stone:571, dirt:206, darkoaklog:8, coalore:16, ..."）
/// 归纳为三类资源摘要："木材:13 石头:874 矿石:24"。
///
/// 作用：
/// - LLM 看摘要就能决策（要不要砍树/挖矿），无需翻一长串方块名
/// - 避免 WI 模板把整个 10x10 行作为 label 重复堆砌
///
/// 分类规则：
/// - 木材：原木/木板/树叶/树苗/枯叶类（log/stem/planks/leaves/sapling/leaflitter/wood）
/// - 石头：石头/泥土/沙/沙砾/基岩/草方块等基础地形（含下半部分空白）
/// - 矿石：所有 _ore 结尾 + ancient_debris
fn summarize_resources(nearby_blocks: &str) -> String {
    let mut wood = 0u32;
    let mut stone = 0u32;
    let mut ore = 0u32;
    for tok in nearby_blocks.split(',') {
        let tok = tok.trim();
        let Some((name, cnt)) = tok.split_once(':') else {
            continue;
        };
        let name = name.trim().to_lowercase();
        let cnt: u32 = cnt.trim().parse().unwrap_or(0);
        if name.ends_with("log")
            || name.ends_with("stem")
            || name.ends_with("planks")
            || name.ends_with("leaves")
            || name.ends_with("sapling")
            || name.ends_with("wood")
            || name == "leaflitter"
            || name.ends_with("leaf")
        {
            wood = wood.saturating_add(cnt);
        } else if name.ends_with("ore") || name == "ancientdebris" || name == "ancient_debris" {
            ore = ore.saturating_add(cnt);
        } else {
            // 其余归石头/泥土类基础地形
            stone = stone.saturating_add(cnt);
        }
    }
    format!("木材:{wood} 石头:{stone} 矿石:{ore}")
}

/// P0 改进3: 压缩 10x10 方块列表 — 只保留有价值的方块（过滤掉空气/石头/泥土等常见地形）
/// 这样 perceive 输出从 ~200 token 降到 ~30 token，同时保留 LLM 决策所需的关键信息
fn compress_block_list(nearby_blocks: &str) -> String {
    /// 无信息量的基础地形方块（大量出现，不值得在 perceive 里列出）
    const COMMON_BLOCKS: &[&str] = &[
        "air",
        "stone",
        "dirt",
        "grass_block",
        "grass",
        "sand",
        "gravel",
        "bedrock",
        "water",
        "lava",
        "clay",
        "snow",
        "ice",
        "packed_ice",
        "cobblestone",
        "mossy_cobblestone",
        "deepslate",
        "tuff",
        "andesite",
        "diorite",
        "granite",
        "calcite",
        "netherrack",
        "end_stone",
        "terracotta",
    ];
    let mut interesting: Vec<&str> = Vec::new();
    for tok in nearby_blocks.split(',') {
        let tok = tok.trim();
        let Some((name, cnt_str)) = tok.split_once(':') else {
            continue;
        };
        let name = name.trim().to_lowercase();
        let cnt: u32 = cnt_str.trim().parse().unwrap_or(0);
        // 过滤：常见方块 或 数量为 0
        if COMMON_BLOCKS.contains(&name.as_str()) || cnt == 0 {
            continue;
        }
        interesting.push(tok);
    }
    if interesting.is_empty() {
        String::new()
    } else {
        interesting.join(", ")
    }
}
