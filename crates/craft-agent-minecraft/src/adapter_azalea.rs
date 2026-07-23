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

use crate::azalea::{AzaleaBot, BotEvent};
use anyhow::{anyhow, Context, Result};
use craft_agent::core::adapter::GameAdapter;
use craft_agent::core::types::{Action, ExecResult, MinecraftAction, Screenshot, WorldState};
use std::sync::{Arc, Mutex};

/// Minecraft azalea 适配器。
pub struct MinecraftAzaleaAdapter {
    bot: Arc<AzaleaBot>,
    /// 缓存最近一次结构化状态（perceive 后供 execute / harness 使用）。
    last: Mutex<Option<WorldState>>,
    /// 卡住检测：上次 Y 与连续未变化的次数（挖到基岩/空气时坐标不动）。
    last_y: Mutex<Option<f64>>,
    stuck_count: Mutex<u32>,
}

#[derive(Clone)]
pub struct ArcAzaleaAdapter(pub Arc<Mutex<MinecraftAzaleaAdapter>>);

impl ArcAzaleaAdapter {
    /// 连接并构造共享 Arc 适配器（转发至 `MinecraftAzaleaAdapter::connect`）。
    pub async fn connect(address: &str, username: &str) -> Result<ArcAzaleaAdapter> {
        MinecraftAzaleaAdapter::connect(address, username).await
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
    pub async fn connect(address: &str, username: &str) -> Result<ArcAzaleaAdapter> {
        let bot = AzaleaBot::connect(address, username).await
            .context("azalea bot 连接失败（确认服为纯 vanilla 26.2 且未开客户端 mod 校验）")?;
        let bot = Arc::new(bot);

        let adapter = Arc::new(Mutex::new(Self {
            bot: bot.clone(),
            last: Mutex::new(None),
            last_y: Mutex::new(None),
            stuck_count: Mutex::new(0),
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
                        } = ev
                        {
                            // 卡住检测：Y 不变则累加，变化则清零。
                            let mut last_y = g.last_y.lock().unwrap();
                            let mut stuck = g.stuck_count.lock().unwrap();
                            if let Some(py) = *last_y {
                                if (position.y - py).abs() < 0.01 {
                                    *stuck += 1;
                                } else {
                                    *stuck = 0;
                                }
                            }
                            *last_y = Some(position.y);
                            let stuck_hint = if *stuck >= 2 {
                                format!(" [卡住{}轮：Y 坐标不变，可能挖到基岩/空气或脚下方块无法破坏，建议停止下探并向玩家 chat 汇报]", *stuck)
                            } else {
                                String::new()
                            };
                            drop(stuck);
                            drop(last_y);
                            let scene = format!(
                                "坐标=({:.1},{:.1},{:.1}) 背包前5={:?} 附近玩家={}{}",
                                position.x, position.y, position.z, inventory, player_count, stuck_hint
                            );
                            *g.last.lock().unwrap() = Some(WorldState {
                                scene_desc: scene.clone(),
                                marked_elements: vec![],
                                detected_targets: vec![],
                                self_hint: scene,
                                screenshot: Arc::new(Vec::new()),
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

    fn execute_mc(&mut self, mc: MinecraftAction) -> Result<ExecResult> {
        match mc {
            MinecraftAction::Goto { x, y, z } => {
                self.bot.goto(x, y, z);
                Ok(ExecResult {
                    ok: true,
                    detail: format!("goto ({x},{y},{z}) 已下发"),
                })
            }
            MinecraftAction::MineBlock { x, y, z } => {
                self.bot.mine(x, y, z);
                Ok(ExecResult {
                    ok: true,
                    detail: format!("mine ({x},{y},{z}) 已下发"),
                })
            }
            MinecraftAction::MineBelow => {
                self.bot.mine_below();
                Ok(ExecResult {
                    ok: true,
                    detail: "mine_below 已下发".to_string(),
                })
            }
            MinecraftAction::InteractBlock { x, y, z } => {
                self.bot.block_interact(x, y, z);
                Ok(ExecResult {
                    ok: true,
                    detail: format!("interact ({x},{y},{z}) 已下发"),
                })
            }
            MinecraftAction::Chat { content } => {
                self.bot.chat(&content);
                Ok(ExecResult {
                    ok: true,
                    detail: format!("chat: {content}"),
                })
            }
            MinecraftAction::Attack { target } => {
                let detail = format!("attack 已下发（目标={target}，攻击最近非玩家实体）");
                self.bot.attack(target);
                Ok(ExecResult {
                    ok: true,
                    detail,
                })
            }
            MinecraftAction::Craft { item, count } => {
                // 走 2×2 背包合成（azalea 公开 API 实现，无需工作台）。
                // 命令经队列异步在 handler 内执行；结果通过事件流回传（见 mod.rs）。
                self.bot.craft_2x2(item.clone(), count);
                Ok(ExecResult {
                    ok: true,
                    detail: format!("craft 已下发（2×2 背包合成：{item} x{count}）"),
                })
            }
            MinecraftAction::Craft3x3 { item, count } => {
                self.bot.craft_3x3(item.clone(), count);
                Ok(ExecResult {
                    ok: true,
                    detail: format!("craft3x3 已下发（需已打开工作台：{item} x{count}）"),
                })
            }
            MinecraftAction::Smelt { output, fuel, count } => {
                self.bot.smelt(output.clone(), fuel.clone(), count);
                Ok(ExecResult {
                    ok: true,
                    detail: format!("smelt 已下发（需已打开熔炉：{output} + {fuel} x{count}）"),
                })
            }
            MinecraftAction::Gather { item, count } => {
                self.bot.gather(item.clone(), count);
                Ok(ExecResult {
                    ok: true,
                    detail: format!("gather 已下发（采集最近 {item} x{count}）"),
                })
            }
            MinecraftAction::Place { item, x, y, z } => {
                self.bot.place(item.clone(), x, y, z);
                Ok(ExecResult {
                    ok: true,
                    detail: format!("place 已下发（{item} @ ({x},{y},{z})）"),
                })
            }
            MinecraftAction::OpenContainer { x, y, z } => {
                self.bot.open_container(x, y, z);
                Ok(ExecResult {
                    ok: true,
                    detail: format!("open 已下发（容器 @ ({x},{y},{z})）"),
                })
            }
        }
    }
}
