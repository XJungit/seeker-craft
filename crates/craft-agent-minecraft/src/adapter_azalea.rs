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
use anyhow::{anyhow, Context, Result};
use craft_agent::core::adapter::GameAdapter;
use craft_agent::core::memory::WorldMemory;
use craft_agent::core::types::{Action, ExecResult, MinecraftAction, Screenshot, WorldState};
use std::sync::{Arc, Mutex};

/// Minecraft azalea 适配器。
pub struct MinecraftAzaleaAdapter {
    bot: Arc<AzaleaBot>,
    /// 缓存最近一次结构化状态（perceive 后供 execute / harness 使用）。
    last: Mutex<Option<WorldState>>,
    /// 卡住检测：上次 Y 与连续未变化的次数（挖到基岩/空气时坐标不动）。
    last_y: Mutex<Option<azalea::Vec3>>,
    stuck_count: Mutex<u32>,
    /// 共享世界记忆库（由 Agent 传入，perceive/action 后回填）。可为空（不记录）。
    memory: Option<WorldMemory>,
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
}

impl MinecraftAzaleaAdapter {
    /// 连接本机已开放的 vanilla 26.2 局域网服（如 localhost:4444）。
    /// 返回 `ArcAzaleaAdapter`（共享 Arc），后台任务消费 bot 事件流更新缓存。
    pub async fn connect(
        address: &str,
        username: &str,
        memory: Option<WorldMemory>,
    ) -> Result<ArcAzaleaAdapter> {
        let bot = AzaleaBot::connect(address, username, memory.clone()).await
            .context("azalea bot 连接失败（确认服为纯 vanilla 26.2 且未开客户端 mod 校验）")?;
        let bot = Arc::new(bot);

        let adapter = Arc::new(Mutex::new(Self {
            bot: bot.clone(),
            last: Mutex::new(None),
            last_y: Mutex::new(None),
            stuck_count: Mutex::new(0),
            memory,
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
                            // 卡住检测：仅当 X/Y/Z 三轴都几乎没动才算"卡住"。
                            // 旧逻辑只看 Y，导致 bot 在平地行走（Y 恒定）也被误判卡住。
                            let mut last_pos = g.last_y.lock().unwrap();
                            let mut stuck = g.stuck_count.lock().unwrap();
                            let moved = match *last_pos {
                                Some(p) => {
                                    (position.x - p.x).abs() > 0.5
                                        || (position.y - p.y).abs() > 0.5
                                        || (position.z - p.z).abs() > 0.5
                                }
                                None => true,
                            };
                            if moved {
                                *stuck = 0;
                            } else {
                                *stuck += 1;
                            }
                            *last_pos = Some(position);
                            // 只回报客观事实（卡住计数），不给指令性结论——
                            // "卡住怎么办"由 system 行为准则统一处理，避免感知层越界决策。
                            let stuck_hint = if *stuck >= 2 {
                                format!(" 卡住计数={}（坐标连续{}轮几乎未移动）", *stuck, *stuck)
                            } else {
                                String::new()
                            };
                            drop(stuck);
                            drop(last_pos);
                            let scene = format!(
                                "坐标=({:.1},{:.1},{:.1}) 朝向=({:.0}°,{:.0}°) 生命={:.1}/20 食物={}/20 主手={} 群系={} 脚下={} 前方={} 附近3x3=[{}] 附近10x10=[{}] 实体=[{}] 背包=[{}] 玩家={}{}",
                                position.x, position.y, position.z, yaw, pitch,
                                health, food, held_item, biome,
                                block_under, block_ahead, nearby, nearby_blocks, nearby_entities, inventory,
                                player_count, stuck_hint
                            );
                            *g.last.lock().unwrap() = Some(WorldState {
                                scene_desc: scene.clone(),
                                marked_elements: vec![],
                                detected_targets: vec![],
                                self_hint: scene,
                                screenshot: Arc::new(Vec::new()),
                                health: Some(health),
                                hunger: Some(food),
                                experience_level: game_state["experience_level"].as_u64().map(|v| v as u32),
                                experience_progress: game_state["experience_progress"].as_f64().map(|v| v as f32),
                                position: Some(vec![position.x, position.y, position.z]),
                                yaw: Some(yaw),
                                pitch: Some(pitch),
                                biome: Some(biome),
                                gamemode: Some("survival".to_string()),
                                inventory: game_state["inventory"].as_array().cloned(),
                                held_item: Some(held_item),
                                selected_slot: game_state["selected_slot"].as_u64().map(|v| v as usize),
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
                return Ok(st);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if let Some(st) = self.last.lock().unwrap().clone() {
            Ok(st)
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

    /// 将 MinecraftAction 转换为 BotCommand（同步等待结果）。
    fn exec_mc_sync(&self, mc: MinecraftAction, timeout_ms: u64) -> Result<ExecResult> {
        let cmd = mc_to_cmd(mc);
        match self.bot.push_cmd_and_wait(cmd, timeout_ms) {
            Ok(msg) => Ok(ExecResult { ok: true, detail: msg }),
            Err(e) => Ok(ExecResult { ok: false, detail: format!("{e}") }),
        }
    }

    fn execute_mc(&mut self, mc: MinecraftAction) -> Result<ExecResult> {
        // 所有动作同步等待结果，让工具返回真实反馈而非"已下发"
        self.exec_mc_sync(mc, 120_000)
    }
}

/// 将 MinecraftAction 转换为 BotCommand（供 push_cmd_and_wait 使用）。
fn mc_to_cmd(mc: MinecraftAction) -> BotCommand {
    match mc {
        MinecraftAction::Goto { x, y, z } => BotCommand::Goto { x, y, z },
        MinecraftAction::MineBlock { x, y, z } => BotCommand::Mine { x, y, z },
        MinecraftAction::MineBelow => BotCommand::MineBelow,
        MinecraftAction::InteractBlock { x, y, z } => BotCommand::BlockInteract { x, y, z },
        MinecraftAction::Chat { content } => BotCommand::Chat { content },
        MinecraftAction::Attack { target } => BotCommand::Attack { target },
        MinecraftAction::Craft { item, count } => BotCommand::Craft2x2 { item, count },
        MinecraftAction::Craft3x3 { item, count } => BotCommand::Craft3x3 { item, count },
        MinecraftAction::Smelt { output, fuel, count } => BotCommand::Smelt { output, fuel, count },
        MinecraftAction::Gather { item, count } => BotCommand::Gather { item, count },
        MinecraftAction::Place { item, x, y, z } => BotCommand::Place { item, x, y, z },
        MinecraftAction::OpenContainer { x, y, z } => BotCommand::OpenContainer { x, y, z },
        MinecraftAction::AutoCraft { item, count } => BotCommand::AutoCraft { item, count },
        MinecraftAction::Enchant { item, level } => BotCommand::Enchant { item, level },
        MinecraftAction::Trade { offer } => BotCommand::Trade { offer },
        MinecraftAction::InteractEntity { kind } => BotCommand::InteractEntity { kind },
    }
}

