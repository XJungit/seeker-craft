//! Minecraft azalea 客户端协议层适配器（仅 `azalea-bot` 特性编译）。
//!
//! 实现 `craft_agent::core::adapter::GameAdapter`：
//! - [`perceive`]：直接读 azalea 结构化状态（坐标/维度/背包/附近玩家/传送门/击杀统计），
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
use std::sync::{Arc, Mutex, atomic::AtomicBool};

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
    /// 任务完成停止标志：TaskCompleteTool 验证通过后置 true。
    pub should_stop: Arc<AtomicBool>,
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
    /// 轻量实时位置（读 `bot.last_position` 每 tick 缓存，不做感知扫描）。
    /// 返回 `Some((x, y, z))`；未连上/无位置时 `None`。
    pub fn current_position(&self) -> Option<(f64, f64, f64)> {
        let guard = self.0.lock().unwrap();
        guard
            .bot
            .last_position
            .lock()
            .unwrap()
            .map(|p| (p.x, p.y, p.z))
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
            should_stop: Arc::new(AtomicBool::new(false)),
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
                            hotbar,
                            armor,
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
                            overhead_solid,
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
                            let dimension = game_state["dimension"]
                                .as_str()
                                .unwrap_or("unknown")
                                .to_string();
                            let portal_active =
                                game_state["portal_active"].as_bool().unwrap_or(false);
                            let kill_counts = game_state["kill_counts"]
                                .as_object()
                                .map(|counts| {
                                    let mut entries: Vec<_> = counts
                                        .iter()
                                        .filter_map(|(kind, count)| {
                                            count.as_u64().map(|value| format!("{kind}:{value}"))
                                        })
                                        .collect();
                                    entries.sort();
                                    entries.join(", ")
                                })
                                .unwrap_or_default();
                            let overhead_hint = if overhead_solid > 0 {
                                let advice = if overhead_solid >= 15 {
                                    "（深埋！调用 mine_above 可逐格向上挖出地表）"
                                } else {
                                    "（上方有实心方块，需 mine_above 挖出）"
                                };
                                format!("头顶: {} 格实心{}", overhead_solid, advice)
                            } else {
                                "头顶: 空气（已在地下洞穴或地表）".to_string()
                            };
                            // P124：有矿石但无镐 → 注入合成建议（否则为空串，不占 token）
                            let pick_hint =
                                pickaxe_warning(&nearby_blocks, &game_state, &held_item);
                            // P129：饱食过低 → 注入进食方案（确定性兜底，LLM 常无视 goal 指令）
                            let hunger_hint = hunger_warning(food, &game_state);
                            // P135：工具耐久 ≤20% → 注入换工具警示（预判断镐，避免中途
                            // 工具销毁困在地下；依赖 handler 注入的 dmg/max 槽位字段）
                            let durable_hint = tool_durability_warning(&game_state);
                            // 末尾提示行：头顶信息 + 可选警示（合并多行）
                            let mut tail_lines = vec![overhead_hint];
                            for h in [pick_hint, hunger_hint, durable_hint] {
                                if !h.is_empty() {
                                    tail_lines.push(h);
                                }
                            }
                            let scene_tail = tail_lines.join("\n");
                            // P135：主手耐久后缀（如 "stone_pickaxe (87/131)"，非工具不加）。
                            // 主手槽 = hotbar 起始槽(36) + selected_hotbar_slot；handler 已把
                            // dmg/max 注入 inventory 槽位 JSON，这里只做展示层拼接。
                            let held_disp = {
                                let sel =
                                    game_state["selected_slot"].as_u64().unwrap_or(0) as usize;
                                let hand_slot_pos = 36 + sel;
                                let mut suffix = String::new();
                                if let Some(arr) = game_state["inventory"].as_array() {
                                    for s in arr {
                                        if s["slot"].as_u64().unwrap_or(0) as usize == hand_slot_pos
                                            && s["max"].as_i64().unwrap_or(0) > 0
                                        {
                                            let dmg = s["dmg"].as_i64().unwrap_or(0);
                                            let max = s["max"].as_i64().unwrap_or(0);
                                            suffix = format!(" (耐久 {}/{})", max - dmg, max);
                                            break;
                                        }
                                    }
                                }
                                format!("{held_item}{suffix}")
                            };
                            // P126d：当前执行动作（game_state 由 handler tick 注入，
                            // 对标 Mindcraft $ACTION）；无则显示「空闲」保持场景信息完整。
                            let current_action = game_state["current_action"]
                                .as_str()
                                .unwrap_or_default()
                                .to_string();
                            let current_action_line = if current_action.is_empty() {
                                "当前动作: 空闲".to_string()
                            } else {
                                format!("当前动作: {}", current_action)
                            };
                            let scene = format!(
                                "位置: ({:.0}, {:.0}, {:.0})\n\
                                  生命: {:.0}/20  饱食: {}/20  主手: {}\n\
                                  {}\n\
                                  维度: {}\n\
                                  群系: {}  脚下: {}  前方: {}\n\
                                  传送门: {}\n\
                                  附近: [{}]\n\
                                  资源: {}{}\n\
                                  实体: [{}]\n\
                                  击杀统计: [{}]\n\
                                  装备: [{}]\n\
                                  背包: [{}]\n\
                                  hotbar: [{}]\n\
                                  玩家: {}{}\n\
                                  {}",
                                position.x,
                                position.y,
                                position.z,
                                health,
                                food,
                                held_disp,
                                current_action_line,
                                dimension,
                                biome,
                                block_under,
                                block_ahead,
                                if portal_active {
                                    "已激活"
                                } else {
                                    "未检测到"
                                },
                                nearby,
                                resource_summary,
                                blocks_line,
                                nearby_entities,
                                kill_counts,
                                armor,
                                inventory,
                                hotbar,
                                player_count,
                                stuck_hint,
                                scene_tail
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
                                dimension: Some(dimension),
                                portal_active: Some(portal_active),
                                kill_counts: game_state["kill_counts"].as_object().map(|counts| {
                                    counts
                                        .iter()
                                        .filter_map(|(kind, count)| {
                                            count.as_u64().map(|value| (kind.clone(), value as u32))
                                        })
                                        .collect()
                                }),
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
                dimension: None,
                portal_active: None,
                kill_counts: None,
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
        if let Some(real_pos) = *self.bot.last_position.lock().unwrap() {
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
        || msg.contains("Could not ")
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

#[cfg(test)]
mod failure_detail_tests {
    use super::is_failure_detail;

    #[test]
    fn missing_attack_target_is_a_failure() {
        assert!(is_failure_detail(
            "Action output:\nCould not find a valid cow within 4.5 blocks."
        ));
    }
}

#[cfg(test)]
mod p68_follow_give_tests {
    use super::mc_to_cmd;
    use crate::azalea::BotCommand;
    use craft_agent::core::types::MinecraftAction;

    #[test]
    fn action_follow_maps_to_botcmd() {
        assert!(matches!(
            mc_to_cmd(MinecraftAction::Follow {
                target: Some("steve".into())
            }),
            BotCommand::Follow { target: Some(_) }
        ));
        assert!(matches!(
            mc_to_cmd(MinecraftAction::Follow { target: None }),
            BotCommand::Follow { target: None }
        ));
    }

    #[test]
    fn action_stopfollow_maps_to_botcmd() {
        assert!(matches!(
            mc_to_cmd(MinecraftAction::StopFollow),
            BotCommand::StopFollow
        ));
    }

    #[test]
    fn action_give_maps_to_botcmd() {
        match mc_to_cmd(MinecraftAction::Give {
            item: "cooked_beef".into(),
            count: 3,
            target: None,
        }) {
            BotCommand::Give {
                item,
                count,
                target,
            } => {
                assert_eq!(item, "cooked_beef");
                assert_eq!(count, 3);
                assert!(target.is_none());
            }
            _ => panic!("Give 未映射到 BotCommand::Give"),
        }
    }
}

/// 将 MinecraftAction 转换为 BotCommand（供 push_cmd_and_wait 使用）。
fn mc_to_cmd(mc: MinecraftAction) -> BotCommand {
    match mc {
        MinecraftAction::Goto { x, y, z } => BotCommand::Goto { x, y, z },
        MinecraftAction::MineBlock { x, y, z } => BotCommand::Mine { x, y, z },
        MinecraftAction::MineBelow => BotCommand::MineBelow,
        MinecraftAction::MineAbove => BotCommand::MineAbove,
        MinecraftAction::InteractBlock { x, y, z } => BotCommand::BlockInteract { x, y, z },
        MinecraftAction::TillAndSow { x, y, z, seed } => BotCommand::TillAndSow { x, y, z, seed },
        MinecraftAction::Sleep => BotCommand::Sleep,
        MinecraftAction::Harvest => BotCommand::Harvest,
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
        MinecraftAction::MakeObsidian { count } => BotCommand::MakeObsidian { count },
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
        MinecraftAction::Follow { target } => BotCommand::Follow { target },
        MinecraftAction::GotoPlayer { target } => BotCommand::GotoPlayer { name: target },
        MinecraftAction::SearchBlock { item, radius } => BotCommand::SearchBlock { item, radius },
        MinecraftAction::MoveAway { target, distance } => BotCommand::MoveAway { target, distance },
        MinecraftAction::StopFollow => BotCommand::StopFollow,
        MinecraftAction::SetMode { mode, enabled } => BotCommand::SetMode { mode, enabled },
        MinecraftAction::UseItem { item, yaw, pitch } => BotCommand::UseItem { item, yaw, pitch },
        MinecraftAction::Shoot { target } => BotCommand::Shoot { target },
        MinecraftAction::Give {
            item,
            count,
            target,
        } => BotCommand::Give {
            item,
            count,
            target,
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

/// P124：视野内是否出现矿石（"name:count" 摘要中任何 *_ore 且数量 > 0）。
fn ore_in_view(nearby_blocks: &str) -> bool {
    nearby_blocks.split(',').any(|tok| {
        let Some((name, cnt)) = tok.trim().split_once(':') else {
            return false;
        };
        name.trim().ends_with("ore") && cnt.trim().parse::<u32>().unwrap_or(0) > 0
    })
}

/// P124：背包（game_state.inventory）或主手是否持有任何镐。
/// item id 为 "minecraft:iron_pickaxe" 等 snake_case 全名。
fn inventory_has_pickaxe(game_state: &serde_json::Value, held_item: &str) -> bool {
    if let Some(arr) = game_state["inventory"].as_array() {
        for s in arr {
            let id = s["id"].as_str().unwrap_or("");
            if id.ends_with("_pickaxe") && s["count"].as_u64().unwrap_or(0) > 0 {
                return true;
            }
        }
    }
    held_item.ends_with("_pickaxe")
}

/// P124：背包无镐警示文本（有矿石但无任何镐时注入 perceive，否则空串）。
///
/// 背景（2026-08-07 实机观测）：bot 埋在地下铁矿石壁中，背包 0 把镐，
/// LLM 因感知不到"缺工具"信号，反复对矿石坐标 goto/mine 空转——工具层
/// gather 已有防死循环，但 LLM 决策层看不到原因而回退循环。
/// 该警示让 LLM 在下一步就能看到缺失并规划合成镐。
fn pickaxe_warning(nearby_blocks: &str, game_state: &serde_json::Value, held_item: &str) -> String {
    if !ore_in_view(nearby_blocks) || inventory_has_pickaxe(game_state, held_item) {
        return String::new();
    }
    "警示：视野内发现矿石，但背包无任何镐——矿石/石头类方块徒手挖不掉（不掉落物品）。\n\
         建议先合成木镐：craft('oak_planks') → craft('stick') → craft('wooden_pickaxe') → equip('wooden_pickaxe')。\n\
         若已有 3 块 cobblestone + 2 根 stick，可上工作台 craft_3x3('stone_pickaxe') 后再来挖矿。"
        .to_string()
}

/// P129：饱食度过低 → 注入进食警示（否则为空串，不占 token）。
/// 确定性兜底：LLM 会无视 goal 里的"先吃食物"指令（实测 red_mushroom+bowl
/// 在手仍跑 63m 外找 brown_mushroom），与 P124 无镐警示同理——perceive 里
/// 直接列出背包现成的可吃方案，不用依赖 LLM 自发的规划能力。
fn hunger_warning(food: u32, game_state: &serde_json::Value) -> String {
    if food > 6 {
        return String::new();
    }
    // 扫背包：第一个可食用物品 + 是否蘑菇&碗（可合成蘑菇煲）
    let mut edible: Option<String> = None;
    let mut has_mushroom = false;
    let mut has_bowl = false;
    if let Some(arr) = game_state["inventory"].as_array() {
        for s in arr {
            let id = s["id"].as_str().unwrap_or("");
            let name = id.strip_prefix("minecraft:").unwrap_or(id);
            if s["count"].as_u64().unwrap_or(0) == 0 {
                continue;
            }
            if is_edible(name) && edible.is_none() {
                edible = Some(name.to_string());
            }
            if name.ends_with("mushroom") {
                has_mushroom = true;
            }
            if name == "bowl" {
                has_bowl = true;
            }
        }
    }
    let severity = if food <= 3 {
        "濒临饿死"
    } else {
        "饱食度偏低"
    };
    let plan = if let Some(f) = edible {
        format!("立即 consume('{f}') 进食！")
    } else if has_mushroom && has_bowl {
        "背包有蘑菇+碗——立即 craft('mushroom_stew') 合成蘑菇煲（1 蘑菇 + 1 碗，2x2），再 consume('mushroom_stew') 吃！"
            .to_string()
    } else {
        "背包没有可吃食物——就近找食物：搜村庄/甜浆果丛，或猎杀动物（生肉 smelt 后吃）。".to_string()
    };
    format!("警示：{severity}（饱食 {food}/20，长期不吃会掉血）。{plan}")
}

/// P135：工具耐久警示（剩余 ≤20% 时注入，否则空串）。
///
/// 背景（2026-08-08 实机观测）：stone/iron 镐三次"神秘消失"——MC 工具耐久
/// 耗尽即自动销毁（无损坏状态残留），而 perceive 不显示耐久，LLM 无法预知
/// 而规划换镐，断镐后才发现"背包无镐"（P124 警示）→ 重铸 → 再断，反复空转。
/// 该警示让 LLM 在耐久不足前就看到"即将损坏"信号，提前 craft/equip 替换。
fn tool_durability_warning(game_state: &serde_json::Value) -> String {
    // 收集耐久 ≤20% 的工具（含主手与背包），按剩余百分比升序。
    let mut low: Vec<(String, i32, i32)> = Vec::new();
    if let Some(arr) = game_state["inventory"].as_array() {
        for s in arr {
            let max = s["max"].as_i64().unwrap_or(0);
            if max <= 0 {
                continue;
            }
            let dmg = s["dmg"].as_i64().unwrap_or(0);
            let left = max - dmg;
            if left * 5 <= max {
                let id = s["id"].as_str().unwrap_or("");
                let name = id.strip_prefix("minecraft:").unwrap_or(id);
                low.push((name.to_string(), left as i32, max as i32));
            }
        }
    }
    if low.is_empty() {
        return String::new();
    }
    low.sort_by_key(|(_, left, _)| *left);
    let desc = low
        .iter()
        .map(|(n, l, m)| format!("{n}({l}/{m})"))
        .collect::<Vec<_>>()
        .join("、");
    format!(
        "警示：工具耐久告急（≤20%）——{desc}。MC 工具耐久归零会直接销毁（不会自动损坏）！\n\
         建议：先 craft 一把备用镐/工具或 equip 已有的，再继续当前动作，避免中途断镐困在地下。"
    )
}

/// Java 版可食用物品（含生食与熟食）。蘑菇/碗本身不可食用但可合成蘑菇煲。
fn is_edible(name: &str) -> bool {
    matches!(
        name,
        "bread"
            | "apple"
            | "golden_apple"
            | "beef"
            | "cooked_beef"
            | "porkchop"
            | "cooked_porkchop"
            | "chicken"
            | "cooked_chicken"
            | "mutton"
            | "cooked_mutton"
            | "rabbit"
            | "cooked_rabbit"
            | "cod"
            | "cooked_cod"
            | "salmon"
            | "cooked_salmon"
            | "mushroom_stew"
            | "suspicious_stew"
            | "beetroot_soup"
            | "carrot"
            | "potato"
            | "baked_potato"
            | "poisonous_potato"
            | "beetroot"
            | "melon_slice"
            | "cookie"
            | "pumpkin_pie"
            | "sweet_berries"
            | "glow_berries"
            | "dried_kelp"
            | "honey_bottle"
            | "cake"
            | "golden_carrot"
    )
}

#[cfg(test)]
mod p124_pickaxe_warning_tests {
    use super::{
        hunger_warning, inventory_has_pickaxe, is_edible, ore_in_view, pickaxe_warning,
        tool_durability_warning,
    };
    use serde_json::json;

    fn inv(items: &[(&str, u64)]) -> serde_json::Value {
        json!({"inventory": items.iter().map(|(id, c)| json!({"id": id, "count": c})).collect::<Vec<_>>()})
    }

    #[test]
    fn ore_in_view_detects_ore_counts() {
        assert!(ore_in_view("stone:42, iron_ore:3, deepslate:10"));
        assert!(ore_in_view("deepslate_iron_ore:1"));
        // 数量为 0 的矿石不算"看得到"
        assert!(!ore_in_view("iron_ore:0, stone:5"));
        assert!(!ore_in_view("stone:42, dirt:3"));
        assert!(!ore_in_view(""));
    }

    #[test]
    fn inventory_has_pickaxe_checks_ids() {
        assert!(inventory_has_pickaxe(
            &inv(&[("minecraft:stone_pickaxe", 1)]),
            "dirt"
        ));
        assert!(inventory_has_pickaxe(
            &inv(&[("minecraft:iron_pickaxe", 1), ("minecraft:cobblestone", 32)]),
            "air"
        ));
        // 背包没有但主手是镐 → 也算有
        assert!(inventory_has_pickaxe(&inv(&[]), "minecraft:iron_pickaxe"));
        // 都没有 → 无镐
        assert!(!inventory_has_pickaxe(
            &inv(&[("minecraft:oak_log", 8), ("minecraft:stick", 4)]),
            "dirt"
        ));
        // 0 个镐不算持有
        assert!(!inventory_has_pickaxe(
            &inv(&[("minecraft:iron_pickaxe", 0)]),
            "dirt"
        ));
    }

    #[test]
    fn warning_injected_only_when_ore_and_no_pickaxe() {
        let with_ore = "stone:88, iron_ore:4";
        let no_ore = "stone:88, dirt:4";

        // 有矿石 + 无镐 → 非空警示，且提示合成木镐
        let w = pickaxe_warning(with_ore, &inv(&[("minecraft:oak_log", 8)]), "dirt");
        assert!(w.contains("背包无任何镐"));
        assert!(w.contains("wooden_pickaxe"));

        // 有矿石但背包已有镐 → 空
        assert!(
            pickaxe_warning(with_ore, &inv(&[("minecraft:stone_pickaxe", 1)]), "dirt").is_empty()
        );

        // 有矿石但主手是镐 → 空
        assert!(pickaxe_warning(with_ore, &inv(&[]), "minecraft:stone_pickaxe").is_empty());

        // 无矿石 → 空（不论有没有镐，不打扰 LLM）
        assert!(pickaxe_warning(no_ore, &inv(&[]), "dirt").is_empty());
    }

    #[test]
    fn hunger_warning_suggests_stew_when_mushroom_and_bowl() {
        // 饱食正常 → 空
        assert!(hunger_warning(12, &inv(&[])).is_empty());
        // 低饱食 + 有现成食物 → 直接吃
        let w = hunger_warning(4, &inv(&[("minecraft:bread", 3)]));
        assert!(w.contains("consume('bread')"));
        // 低饱食 + 蘑菇&碗（但蘑菇不可直接吃）→ 提示合成蘑菇煲
        let w = hunger_warning(
            4,
            &inv(&[("minecraft:red_mushroom", 13), ("minecraft:bowl", 16)]),
        );
        assert!(w.contains("mushroom_stew"));
        assert!(w.contains("craft('mushroom_stew')"));
        // 低饱食 + 无可吃 → 提示找食物
        let w = hunger_warning(
            4,
            &inv(&[("minecraft:cobblestone", 64), ("minecraft:iron_pickaxe", 1)]),
        );
        assert!(w.contains("找食物"));
        // 濒临饿死措辞
        assert!(hunger_warning(2, &inv(&[("minecraft:apple", 1)])).contains("濒临饿死"));
    }

    #[test]
    fn is_edible_classifies_common_foods() {
        assert!(is_edible("bread"));
        assert!(is_edible("cooked_beef"));
        assert!(is_edible("mushroom_stew"));
        assert!(is_edible("apple"));
        assert!(is_edible("sweet_berries"));
        // 蘑菇/碗/工具/矿物都不可直接吃
        assert!(!is_edible("red_mushroom"));
        assert!(!is_edible("bowl"));
        assert!(!is_edible("cobblestone"));
        assert!(!is_edible("iron_ingot"));
    }

    fn inv_with_durability(items: &[(&str, i64, i64)]) -> serde_json::Value {
        // (id, dmg, max)——max=0 表示非工具
        json!({"inventory": items.iter().map(|(id, d, m)| json!({"id": id, "dmg": d, "max": m})).collect::<Vec<_>>()})
    }

    #[test]
    fn tool_durability_warning_only_for_low_durability_tools() {
        // 所有工具满耐久 → 空
        let fresh = inv_with_durability(&[
            ("minecraft:stone_pickaxe", 0, 131),
            ("minecraft:iron_pickaxe", 10, 250),
        ]);
        assert!(tool_durability_warning(&fresh).is_empty());

        // 一把耐久 24/131（≈18% ≤20%）→ 警示，且点名该工具
        let low = inv_with_durability(&[
            ("minecraft:stone_pickaxe", 107, 131),
            ("minecraft:iron_pickaxe", 10, 250),
        ]);
        let w = tool_durability_warning(&low);
        assert!(w.contains("stone_pickaxe"));
        assert!(w.contains("24/131"));
        assert!(w.contains("耐久归零"));

        // 多个低耐久 → 都列出；非工具（max=0）忽略
        let multi = inv_with_durability(&[
            ("minecraft:stone_pickaxe", 107, 131),
            ("minecraft:iron_pickaxe", 225, 250),
            ("minecraft:cobblestone", 0, 0),
        ]);
        let w = tool_durability_warning(&multi);
        assert!(w.contains("stone_pickaxe"));
        assert!(w.contains("iron_pickaxe"));
        assert!(!w.contains("cobblestone"));

        // 恰好 20% 也触发（边界）
        let edge = inv_with_durability(&[("minecraft:iron_pickaxe", 200, 250)]);
        assert!(!tool_durability_warning(&edge).is_empty());

        // 空背包 → 空
        assert!(tool_durability_warning(&json!({"inventory": []})).is_empty());
    }
}
