//! Minecraft 桥接 mod 适配器（仅 `mod-bridge` 特性编译）。
//!
//! 实现 `craft_agent::core::adapter::GameAdapter`：
//! - [`perceive`]：直接读 mod 结构化状态（物品栏/方块/实体精确数据 + 玩家坐标朝向），
//!   构建 `WorldState` 喂给决策层，**不再依赖 VLM/截图做语义识别**。
//! - [`execute`]：把 `Action` 翻译成 mod 动作命令（`look`/`press`/`mine`/`move`/`look_at`），
//!   由 mod 在进程内驱动游戏，**彻底移除 enigo 的 OS 级键鼠模拟**（不抢你的鼠标键盘、
//!   可后台运行）。`mine` 的成败由 mod 回执的原木数量差判断。
//! - [`capture`]：仍用 xcap 截 MC 窗口（与 enigo 路径解耦，仅用于 viewer 可视化核对）。
//!
//! 这补齐了之前几轮反复出现的"治本缺口"：agent 现在能确认"挖到几块木头 / 离树多远 /
//! 准星是否压在树干上"，而不是靠 VLM 猜。

use crate::bridge::{McBridge, ModAck, ModCommand, ModState, NearbyBlock};
use anyhow::{Context, Result, anyhow};
use craft_agent::core::adapter::GameAdapter;
use craft_agent::core::types::{Action, Direction, ExecResult, Screenshot, Target, WorldState};
use craft_agent_model::vision::VisionClient;
use std::cell::RefCell;
use std::thread;
use std::time::Duration;

/// mine 默认 60 tick（3 秒）→ 木头破坏时间。
const MINE_TICKS: u32 = 60;

/// Minecraft 桥接 mod 适配器。
pub struct MinecraftModAdapter {
    bridge: RefCell<McBridge>,
    /// 缓存最近一次状态（perceive 后供 execute 使用，如 aim_and_mine 精确对准）。
    last: RefCell<Option<ModState>>,
    /// 可选 VLM 客户端：仅在需要视觉补充时调用（如识别 GUI 界面、确认合成配方）。
    /// 日常感知走 mod 结构化数据，不消耗 VLM API 额度。
    vision: Option<Box<dyn VisionClient>>,
}

impl MinecraftModAdapter {
    /// 连接本机已加载 craft-agent-bridge 的 MC（纯 mod 感知，无视觉）。
    pub fn connect(host: &str, port: u16) -> Result<Self> {
        Self::connect_with_vision(host, port, None)
    }

    /// 连接 MC mod，可选 VLM 客户端作为视觉补充。
    /// `vision` 为 None 时仅用 mod 结构化数据（推荐默认）。
    pub fn connect_with_vision(
        host: &str,
        port: u16,
        vision: Option<Box<dyn VisionClient>>,
    ) -> Result<Self> {
        let bridge = McBridge::connect(host, port)?;
        Ok(Self {
            bridge: RefCell::new(bridge),
            last: RefCell::new(None),
            vision,
        })
    }

    /// 拉取最新状态并缓存。连接失败时尝试重连一次。
    pub fn reload(&self) -> Result<ModState> {
        match self.bridge.borrow_mut().query_state() {
            Ok(st) => {
                self.last.replace(Some(st.clone()));
                Ok(st)
            }
            Err(e) => {
                // 尝试重连一次
                if let Err(re) = self.bridge.borrow_mut().reconnect() {
                    return Err(e.context(format!("重连也失败: {re}")));
                }
                let st = self.bridge.borrow_mut().query_state()?;
                self.last.replace(Some(st.clone()));
                Ok(st)
            }
        }
    }

    /// 视觉补充：截取 MC 画面 + VLM 分析，用于需要"看画面"的场景
    /// （如识别合成台界面、确认背包内容、判断当前 UI 状态）。
    /// 仅当适配器构造时传入了 VLM 客户端才有效，否则返回提示。
    pub fn perceive_visual(&self, prompt: &str) -> Result<String> {
        let vision = self.vision.as_ref().ok_or_else(|| {
            anyhow!("未配置 VLM 视觉客户端；日常感知请用 perceive()（精确 mod 数据）")
        })?;
        let png = self.capture_xcap();
        if png.is_empty() {
            return Err(anyhow!("截图失败：未找到 Minecraft 窗口"));
        }
        vision.chat(&png, prompt).context("VLM 视觉分析失败")
    }

    /// 重连 MC mod（MC 崩溃重启后恢复连接）。重连成功返回 Ok，失败返回可重试的 Err。
    pub fn reconnect(&self) -> Result<()> {
        self.bridge.borrow_mut().reconnect()
    }

    /// 检查 mod 连接是否存活（发送轻量请求验证）。
    pub fn is_connected(&self) -> bool {
        self.bridge.borrow_mut().is_alive()
    }

    /// 右键点击（放置/使用/吃东西/开箱子）。ticks: 持续 tick，默认 5（约 0.25 秒）。
    pub fn right_click(&self, ticks: u32) -> Result<()> {
        self.bridge
            .borrow_mut()
            .send(ModCommand::RightClick { ticks })?;
        Ok(())
    }

    /// 攻击最近实体（按住左键 ticks，默认 30 tick ≈ 1.5 秒）。
    pub fn attack(&self, ticks: u32) -> Result<()> {
        self.bridge
            .borrow_mut()
            .send(ModCommand::Attack { ticks })?;
        Ok(())
    }

    /// 导航到世界坐标（mod 侧每 tick 重新计算朝向 + 前进，无振荡）。
    pub fn move_to(&self, x: f64, y: f64, z: f64) -> Result<()> {
        self.bridge
            .borrow_mut()
            .send(ModCommand::MoveTo { x, y, z })?;
        Ok(())
    }

    /// 精确看向世界坐标（mod 侧直接设置视角，无相对转动误差）。
    pub fn look_at(&self, x: f64, y: f64, z: f64) -> Result<()> {
        self.bridge
            .borrow_mut()
            .send(ModCommand::LookAt { x, y, z })?;
        Ok(())
    }

    /// 合成物品（调 mod 直接操作 Inventory 扣材料加结果，零视觉依赖）。
    pub fn craft(&self, item: &str, count: u32) -> Result<()> {
        self.bridge.borrow_mut().send(ModCommand::Craft {
            item: item.to_string(),
            count,
        })?;
        Ok(())
    }

    /// xcap 截 MC 窗口（仅用于 viewer 可视化；失败不影响主流程，返回空截图）。
    fn capture_xcap(&self) -> Screenshot {
        let windows = match xcap::Window::all() {
            Ok(w) => w,
            Err(_) => return Vec::new(),
        };
        for w in windows {
            if let (Ok(title), Ok(img)) = (w.title(), w.capture_image())
                && title.to_lowercase().contains("minecraft")
            {
                let mut png = Vec::new();
                if image::DynamicImage::ImageRgba8(img)
                    .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                    .is_ok()
                {
                    return png;
                }
            }
        }
        Vec::new()
    }
}

/// 把结构化状态拼成给 LLM 读的场景描述——清晰结构化，参考 Mindcraft !stats / !inventory / !nearbyBlocks 格式。
fn build_scene_desc(st: &ModState) -> String {
    let mut s = String::new();
    let px = st.position[0];
    let py = st.position[1];
    let pz = st.position[2];

    // ── STATS ──
    s.push_str("STATS\n");
    s.push_str(&format!(
        "  Position: x={:.1}, y={:.1}, z={:.1}\n",
        px, py, pz
    ));
    s.push_str(&format!("  Yaw: {:.1}°  Pitch: {:.1}°\n", st.yaw, st.pitch));
    s.push_str(&format!(
        "  Health: {:.0}/20  Hunger: {:.0}/20\n",
        st.health, st.hunger
    ));
    s.push_str(&format!(
        "  Gamemode: {}  Dimension: {}  Biome: {}\n",
        st.gamemode, st.dimension, st.biome
    ));
    s.push_str(&format!(
        "  Light: sky={}/15 block={}/15  Weather: rain={} thunder={}\n",
        st.sky_light, st.block_light, st.raining, st.thundering
    )    );
    if !st.effects.is_empty() {
        let fx: Vec<String> = st
            .effects
            .iter()
            .map(|e| {
                format!(
                    "{} lv{} ({}s)",
                    e.id.replace("minecraft:", ""),
                    e.amplifier + 1,
                    (e.duration as f32 / 20.0).round()
                )
            })
            .collect();
        s.push_str(&format!("  Effects: {}\n", fx.join(", ")));
    }

    // ── INVENTORY ──
    let hotbar: Vec<String> = st
        .inventory
        .iter()
        .filter(|i| i.count > 0 && i.slot < 9)
        .map(|i| format!("[{s}] {item}x{c}", s = i.slot + 1, item = i.id.replace("minecraft:", ""), c = i.count))
        .collect();
    let main_inv: Vec<String> = st
        .inventory
        .iter()
        .filter(|i| i.count > 0 && i.slot >= 9)
        .map(|i| format!("[{s}] {item}x{c}", s = i.slot, item = i.id.replace("minecraft:", ""), c = i.count))
        .collect();
    s.push_str(&format!(
        "HOTBAR (slot 1-9):  {}\n",
        if hotbar.is_empty() { "(empty)" } else { &hotbar.join(", ") }
    ));
    s.push_str(&format!(
        "INVENTORY (slots 9-):  {}\n",
        if main_inv.is_empty() { "(empty)" } else { &main_inv.join(", ") }
    ));

    // Find which hotbar slot is currently held
    let held_info = if let Some(held_slot) = st
        .inventory
        .iter()
        .find(|i| i.slot < 9 && i.id == st.held_item && i.count > 0)
        .map(|i| i.slot + 1)
    {
        format!(
            "{} (hotbar slot {})",
            st.held_item.replace("minecraft:", ""),
            held_slot
        )
    } else {
        st.held_item.replace("minecraft:", "")
    };
    s.push_str(&format!("  Held: {}\n", held_info));

    // ── TARGETED ──
    if let Some(b) = &st.targeted_block {
        s.push_str(&format!(
            "TARGETED: {} (distance {:.1}m)\n",
            b.id.replace("minecraft:", ""),
            b.dist
        ));
    } else {
        s.push_str("TARGETED: none (pointing at sky or far away)\n");
    }

    // ── NEARBY BLOCKS (limited to 30, priority: logs/ores > stone/planks > other) ──
    let mut blocks: Vec<&crate::bridge::NearbyBlock> = st.nearby_blocks.iter().collect();
    // Sort: resources first, then by distance
    blocks.sort_by(|a, b| {
        let prio = |id: &str| -> u8 {
            if id.contains("_log") || id.contains("_ore") {
                0
            } else if id.contains("planks") || id.contains("crafting") {
                1
            } else if id.contains("stone") || id.contains("cobble") {
                2
            } else {
                3
            }
        };
        prio(&a.id).cmp(&prio(&b.id)).then(
            a.dist
                .partial_cmp(&b.dist)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
    let display = blocks.iter().take(30).collect::<Vec<_>>();
    let total = blocks.len();
    let block_lines: Vec<String> = display
        .iter()
        .map(|b| {
            format!(
                "  {} at ({:.1},{:.1},{:.1}) dist={:.1}m",
                b.id.replace("minecraft:", ""),
                b.x,
                b.y,
                b.z,
                b.dist
            )
        })
        .collect();
    if !block_lines.is_empty() {
        s.push_str(&format!(
            "NEARBY BLOCKS ({} total, showing top {}):\n{}\n",
            total,
            block_lines.len(),
            block_lines.join("\n")
        ));
    } else {
        s.push_str("NEARBY BLOCKS: none\n");
    }

    // ── NEARBY ENTITIES ──
    if !st.entities.is_empty() {
        let ents: Vec<String> = st
            .entities
            .iter()
            .map(|e| {
                format!(
                    "  {} at ({:.1},{:.1},{:.1}) dist={:.1}m hp={:.0}",
                    e.r#type.replace("minecraft:", ""),
                    e.x,
                    e.y,
                    e.z,
                    e.dist,
                    e.health
                )
            })
            .collect();
        s.push_str(&format!("NEARBY ENTITIES:\n{}\n", ents.join("\n")));
    }

    s
}

/// 从状态构建 3D 检测目标（供决策层 click/aim 查表）：以附近原木为主。
fn build_targets(st: &ModState) -> Vec<Target> {
    let mut out = Vec::new();
    let (px, py, pz) = (st.position[0], st.position[1], st.position[2]);
    // 把世界坐标投影到屏幕偏移（近似：以玩家 yaw/pitch 为基准的球面投影）。
    // 仅用于让 LLM 知道"目标在哪个方向"，精确对准由 mod 的 look_at 完成。
    for b in &st.nearby_blocks {
        if !b.id.contains("log") && !b.id.contains("planks") && !b.id.contains("crafting") {
            continue;
        }
        // 方向向量
        let dx = b.x - px;
        let dy = b.y + 0.5 - (py + 1.62); // 方块中心 vs 眼睛高度
        let dz = b.z - pz;
        // 水平角（相对玩家朝南基准），转成"右为正"的屏幕 dx 近似
        let horiz = (dx * dx + dz * dz).sqrt();
        let off_x = (dx / horiz.max(0.001)) * 200.0; // 粗略：朝 +x 偏右
        let off_y = -(dy / horiz.max(0.001)) * 200.0; // 方块更高→偏上
        let label = b.id.replace("minecraft:", "");
        out.push(Target {
            label,
            bbox: [off_x as i32 - 20, off_y as i32 - 20, 40, 40],
            offset_from_crosshair: (off_x as i32, off_y as i32),
        });
    }
    out
}

/// 在附近方块里找最匹配 target 描述的（含关键字，取最近）。
fn nearest_block<'a>(st: &'a ModState, target: &str) -> Option<&'a NearbyBlock> {
    let key = target.to_lowercase();
    let kw = key
        .split_whitespace()
        .next()
        .unwrap_or(&key)
        .trim_matches('*')
        .to_string();
    st.nearby_blocks
        .iter()
        .filter(|b| b.id.to_lowercase().contains(&kw) || kw.is_empty())
        .min_by(|a, b| {
            a.dist
                .partial_cmp(&b.dist)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

impl GameAdapter for MinecraftModAdapter {
    fn capture(&self) -> Result<Screenshot> {
        Ok(self.capture_xcap())
    }

    fn perceive(&self) -> Result<WorldState> {
        let st = self.reload()?;
        // mod 模式不需要截图——精确结构化数据已经足够，截图留给 visual_perceive
        let scene_desc = build_scene_desc(&st);
        let targets = build_targets(&st);
        Ok(WorldState {
            scene_desc,
            marked_elements: vec![],
            detected_targets: targets,
            self_hint: format!(
                "health={:.0} hunger={:.0} yaw={:.1} pitch={:.1} 原木/木板合计={} 下雨={} 光照{}/{} 效果数={}",
                st.health,
                st.hunger,
                st.yaw,
                st.pitch,
                st.inventory
                    .iter()
                    .filter(|i| i.id.contains("log") || i.id.contains("planks"))
                    .map(|i| i.count)
                    .sum::<u32>(),
                st.raining,
                st.sky_light,
                st.block_light,
                st.effects.len()
            ),
            screenshot: Vec::new(),
        })
    }

    fn perceive_with_prompt(&self, prompt: &str) -> Result<String> {
        let st = self.reload()?;
        // 把结构化状态原样给 LLM，附上它的问题，让它基于精确数据推理。
        let json = serde_json::to_string_pretty(&st).context("序列化状态失败")?;
        Ok(format!(
            "以下是游戏的结构化状态（精确数据，无需看图猜测）：\n{json}\n\n请基于以上状态回答：{prompt}"
        ))
    }

    fn execute(&mut self, action: Action) -> Result<ExecResult> {
        let ack_to_result = |ack: ModAck, detail: String| -> ExecResult {
            ExecResult {
                ok: ack.status == "ok",
                detail: if ack.detail.is_empty() {
                    detail
                } else {
                    format!("{detail} | {}", ack.detail)
                },
            }
        };
        match action {
            Action::Look { dx, dy } => {
                let ack = self.bridge.borrow_mut().send(ModCommand::Look { dx, dy })?;
                Ok(ack_to_result(ack, format!("look dx={dx} dy={dy}")))
            }
            Action::Move { dir, ticks } => {
                let d = match dir {
                    Direction::Forward => "forward",
                    Direction::Back => "back",
                    Direction::Left => "left",
                    Direction::Right => "right",
                    Direction::Up => "up",
                    Direction::Down => "down",
                };
                let ack = self.bridge.borrow_mut().send(ModCommand::Move {
                    dir: d.to_string(),
                    ticks,
                })?;
                Ok(ack_to_result(ack, format!("move {d} x{ticks}")))
            }
            Action::Press { keys, ticks } => {
                let detail = format!("press {keys} x{ticks}");
                let ack = self.bridge.borrow_mut().send(ModCommand::Press {
                    keys: keys.clone(),
                    ticks,
                })?;
                Ok(ack_to_result(ack, detail))
            }
            Action::Mine { ticks } => {
                let ack = self.bridge.borrow_mut().send(ModCommand::Mine { ticks })?;
                let detail = match (ack.logs_before, ack.logs_after) {
                    (Some(b), Some(a)) if a > b => {
                        format!("mine 成功，原木 +{}（{b}→{a}）", a - b)
                    }
                    (Some(b), Some(a)) => {
                        format!("mine 完成但未增加原木（{b}→{a}，可能未对准方块或方块不可破坏）")
                    }
                    _ => "mine 完成（mod 未返回原木计数）".into(),
                };
                Ok(ack_to_result(ack, detail))
            }
            Action::AimAndMine { target } => {
                // 用最近匹配方块精确对准再挖（mod 进程内 look_at，不靠 VLM 猜）。
                let st = self.reload()?;
                match nearest_block(&st, &target) {
                    Some(b) => {
                        self.bridge.borrow_mut().send(ModCommand::LookAt {
                            x: b.x,
                            y: b.y + 0.5,
                            z: b.z,
                        })?;
                        thread::sleep(Duration::from_millis(180));
                        let ack = self
                            .bridge
                            .borrow_mut()
                            .send(ModCommand::Mine { ticks: MINE_TICKS })?;
                        let detail = match (ack.logs_before, ack.logs_after) {
                            (Some(bb), Some(aa)) if aa > bb => {
                                format!("aim_and_mine 成功，{} 原木 +{}", target, aa - bb)
                            }
                            _ => format!(
                                "aim_and_mine 已对准 {} 并挖 3s（未确认增加，可能还需走近）",
                                target
                            ),
                        };
                        Ok(ack_to_result(ack, detail))
                    }
                    None => {
                        // 没有匹配方块 → 盲挖（保留行为，避免完全卡死）
                        let ack = self
                            .bridge
                            .borrow_mut()
                            .send(ModCommand::Mine { ticks: MINE_TICKS })?;
                        Ok(ack_to_result(
                            ack,
                            format!("aim_and_mine 未找到匹配 '{target}'，盲挖 3s"),
                        ))
                    }
                }
            }
            Action::Click { element_id } => Err(anyhow!(
                "mod 模式无 2D 点击（element_id={element_id}）；改用 look/mine 操作 3D 世界"
            )),
        }
    }
}
