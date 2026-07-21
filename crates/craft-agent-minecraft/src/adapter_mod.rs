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
use crate::survival_decisions::UnstuckDetector;
use anyhow::{Context, Result, anyhow};
use craft_agent::core::adapter::GameAdapter;
use craft_agent::core::types::{Action, ExecResult, Screenshot, Target, WorldState};
use craft_agent_model::vision::VisionClient;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use std::collections::HashMap;

type PlacesMap = HashMap<String, (f64, f64, f64)>;

/// Minecraft 桥接 mod 适配器。
pub struct MinecraftModAdapter {
    bridge: Mutex<McBridge>,
    /// 缓存最近一次状态（perceive 后供 execute 使用，如 aim_and_mine 精确对准）。
    last: Mutex<Option<ModState>>,
    /// 可选 VLM 客户端：仅在需要视觉补充时调用（如识别 GUI 界面、确认合成配方）。
    /// 日常感知走 mod 结构化数据，不消耗 VLM API 额度。
    vision: Option<Arc<dyn VisionClient>>,
    /// 位置记忆（rememberHere / goToRememberedPlace）
    saved_places: Mutex<HashMap<String, (f64, f64, f64)>>,
    /// 行为模式开关（setMode）
    modes: Mutex<HashMap<String, bool>>,
    /// 卡住检测器：追踪位置变化，感知时发出警告
    unstuck: Mutex<UnstuckDetector>,
}

#[derive(Clone)]
pub struct ArcGameAdapter(pub Arc<Mutex<MinecraftModAdapter>>);

impl ArcGameAdapter {
    fn lock_adapter(&self) -> Result<std::sync::MutexGuard<'_, MinecraftModAdapter>> {
        self.0
            .lock()
            .map_err(|e| anyhow!("adapter mutex poisoned: {e}"))
    }
}

impl GameAdapter for ArcGameAdapter {
    fn capture(&self) -> Result<Screenshot> {
        self.lock_adapter()?.capture()
    }
    fn perceive(&self) -> Result<WorldState> {
        self.lock_adapter()?.perceive()
    }
    fn perceive_with_prompt(&self, prompt: &str) -> Result<String> {
        self.lock_adapter()?.perceive_with_prompt(prompt)
    }
    fn execute(&mut self, action: Action) -> Result<ExecResult> {
        self.lock_adapter()?.execute(action)
    }
}

impl MinecraftModAdapter {
    /// 安全获取桥接器锁，返回 Result 而非 panic。
    fn lock_bridge(&self) -> Result<std::sync::MutexGuard<'_, McBridge>> {
        self.bridge
            .lock()
            .map_err(|e| anyhow!("bridge mutex poisoned: {e}"))
    }

    /// 读取最近一次缓存的状态（不发起网络请求）。
    pub fn get_last_state(&self) -> Option<ModState> {
        self.last.lock().ok().and_then(|g| g.clone())
    }

    fn lock_places(&self) -> Result<std::sync::MutexGuard<'_, PlacesMap>> {
        self.saved_places
            .lock()
            .map_err(|e| anyhow!("places mutex poisoned: {e}"))
    }

    fn lock_modes(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, bool>>> {
        self.modes
            .lock()
            .map_err(|e| anyhow!("modes mutex poisoned: {e}"))
    }

    /// 连接本机已加载 craft-agent-bridge 的 MC（纯 mod 感知，无视觉）。
    pub fn connect(host: &str, port: u16) -> Result<Self> {
        Self::connect_with_vision(host, port, None)
    }

    /// 连接 MC mod，可选 VLM 客户端作为视觉补充。
    /// `vision` 为 None 时仅用 mod 结构化数据（推荐默认）。
    pub fn connect_with_vision(
        host: &str,
        port: u16,
        vision: Option<Arc<dyn VisionClient>>,
    ) -> Result<Self> {
        let bridge = McBridge::connect(host, port)?;
        Ok(Self {
            bridge: Mutex::new(bridge),
            last: Mutex::new(None),
            vision,
            saved_places: Mutex::new(HashMap::new()),
            modes: Mutex::new(HashMap::new()),
            unstuck: Mutex::new(UnstuckDetector::new()),
        })
    }

    /// 发送调试命令（smoke 测试造环境用：debug_spawn/give/damage/heal/clear）。
    pub fn send_debug(&self, cmd: ModCommand) -> Result<ModAck> {
        self.lock_bridge()?.send(cmd)
    }

    /// 拉取最新状态并缓存。连接失败时尝试重连一次。
    pub fn reload(&self) -> Result<ModState> {
        match self.lock_bridge()?.query_state() {
            Ok(st) => {
                if let Ok(mut last) = self.last.lock() {
                    *last = Some(st.clone());
                }
                Ok(st)
            }
            Err(e) => {
                if let Err(re) = self.lock_bridge()?.reconnect() {
                    return Err(e.context(format!("重连也失败: {re}")));
                }
                let st = self.lock_bridge()?.query_state()?;
                if let Ok(mut last) = self.last.lock() {
                    *last = Some(st.clone());
                }
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
        self.lock_bridge()?.reconnect()
    }

    /// 手动重置卡住检测器（在成功移动后调用）。
    pub fn reset_stuck_detector(&self) {
        if let Ok(mut d) = self.unstuck.lock() {
            d.reset();
        }
    }

    /// 检查 mod 连接是否存活（发送轻量请求验证）。
    pub fn is_connected(&self) -> bool {
        self.lock_bridge().is_ok_and(|mut b| b.is_alive())
    }

    /// 使用主手物品（吃东西/用桶/扔珍珠）。ticks: 长按时长（32≈1.6s 吃食物）。
    pub fn use_item(&self, ticks: u32) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::UseItem { ticks })
    }

    /// 攻击最近敌对实体（mod 侧自动装备武器+朝向，单次攻击）。
    /// ticks 仅用于 Rust 侧等待，mod 侧单次攻击。
    pub fn attack(&self, ticks: u32) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::Attack { ticks })
    }

    /// 导航到世界坐标（mod 侧主线程 tick 回调执行移动，轮询等待完成）。
    /// 返回 ModAck（含 reached/final_dist/stuck，调用方应检查是否真正到达）。
    pub fn move_to(&self, x: f64, y: f64, z: f64) -> Result<ModAck> {
        if let Ok(mut d) = self.unstuck.lock() {
            d.reset();
        }
        self.lock_bridge()?.send(ModCommand::MoveTo { x, y, z })
    }

    /// 新寻路系统：异步 A*，8 种 movement，auto-replan，auto-dig。
    /// 不阻塞——立即返回，由 mod 侧每 tick 执行。
    pub fn nav_to(&self, x: f64, y: f64, z: f64) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::NavTo { x, y, z })
    }

    /// 停止新寻路系统。
    pub fn nav_stop(&self) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::NavStop)
    }

    /// 查询新寻路系统状态。
    pub fn nav_status(&self) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::NavStatus)
    }

    /// 精确放置方块到指定坐标（mod 侧 useItemOn，不依赖准星朝向）。
    pub fn place_at(&self, x: i32, y: i32, z: i32, item: &str) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::PlaceAt {
            x,
            y,
            z,
            item: item.into(),
        })
    }

    /// 精确破坏指定坐标方块（mod 侧 destroyBlock，含掉落）。
    pub fn dig_at(&self, x: i32, y: i32, z: i32) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::DigAt { x, y, z })
    }

    /// 切换快捷栏选中格（mod 侧反射设置 Inventory.selected）。
    pub fn select_slot(&self, slot: u32) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::SelectSlot { slot })
    }

    /// 从主背包移动物品到快捷栏（mod 侧直接交换槽位）。
    pub fn move_to_hotbar(&self, item: &str) -> Result<ModAck> {
        self.lock_bridge()?
            .send(ModCommand::MoveToHotbar { item: item.into() })
    }

    /// 精确槽位移动物品（slot 0-8=hotbar, 9-35=main inventory）。
    /// count=None 整组移动；count=Some(n) 拆分 n 个。
    pub fn move_slot(&self, from_slot: u32, to_slot: u32, count: Option<u32>) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::MoveSlot {
            from_slot,
            to_slot,
            count,
        })
    }

    /// 查询单个方块（Rust A* 寻路用）。
    pub fn get_block(&self, x: i32, y: i32, z: i32) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::GetBlock { x, y, z })
    }

    /// 查询区域内所有非空方块（Rust A* 寻路用）。
    pub fn get_blocks(
        &self,
        x1: i32,
        y1: i32,
        z1: i32,
        x2: i32,
        y2: i32,
        z2: i32,
    ) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::GetBlocks {
            x1,
            y1,
            z1,
            x2,
            y2,
            z2,
        })
    }

    /// 读取当前打开的容器/GUI内容（参考 Numen inspect_gui）。
    pub fn inspect_gui(&self) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::InspectGui)
    }

    /// 关闭当前打开的容器/GUI。
    pub fn close_gui(&self) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::CloseGui)
    }

    /// 在打开的容器中进行物品转移（Shift+路由或精确槽位移）。
    /// moves: JSON 数组，每项 {from: int, to?: int|null, count?: int}
    pub fn transfer(&self, moves: serde_json::Value) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::Transfer { moves })
    }

    /// 装备物品到指定槽位（支持盔甲/offhand/mainhand）。
    /// slot: "mainhand" | "offhand" | "head" | "chest" | "legs" | "feet" | "auto"
    pub fn equip_item(&self, item: &str, slot: Option<&str>) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::EquipItem {
            item: item.into(),
            slot: slot.map(|s| s.into()),
        })
    }

    /// 吃指定物品（自动切到快捷栏+useItem）。
    pub fn eat_item(&self, item: &str, ticks: Option<u32>) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::EatItem {
            item: item.into(),
            ticks,
        })
    }

    /// 丢弃物品为地面实体（真正生成 ItemEntity，带拾取冷却）。
    pub fn drop_items(&self, item: &str, num: u32) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::DropItems {
            item: item.into(),
            num,
        })
    }

    /// 等待指定秒数。
    pub fn wait(&self, seconds: u32) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::Wait { seconds })
    }

    /// 原地垫方块脱困：在脚下放方块并跳起。
    pub fn pillar_up(&self, count: Option<u32>, item: Option<&str>) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::PillarUp {
            count,
            item: item.map(|s| s.into()),
        })
    }

    /// 列出在线玩家（支持 goToPlayer/attackPlayer 基础）。
    pub fn list_players(&self) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::ListPlayers)
    }

    /// 按名字导航到指定玩家。
    pub fn go_to_player(&self, player_name: &str, closeness: Option<f64>) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::GoToPlayer {
            player_name: player_name.into(),
            closeness,
        })
    }

    /// 攻击指定玩家。
    pub fn attack_player(&self, player_name: &str, ticks: u32) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::AttackPlayer {
            player_name: player_name.into(),
            ticks,
        })
    }

    /// 给指定玩家物品。
    pub fn give_player(&self, player_name: &str, item: &str, num: u32) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::GivePlayer {
            player_name: player_name.into(),
            item: item.into(),
            num,
        })
    }

    /// 自动拾取附近掉落物（参考 Numen collect_items）。
    pub fn collect_items(
        &self,
        item_ids: Vec<String>,
        radius: f64,
        max_count: u32,
    ) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::CollectItems {
            item_ids,
            radius,
            max_count,
        })
    }

    /// 停止所有当前动作（参考 mindcraft !stop）。
    pub fn stop(&self) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::Stop)
    }

    /// 设置持续目标（参考 mindcraft !goal）。
    pub fn set_goal(&self, goal: &str) -> Result<ModAck> {
        self.lock_bridge()?
            .send(ModCommand::SetGoal { goal: goal.into() })
    }

    /// 获取当前持续目标。
    pub fn get_goal(&self) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::GetGoal)
    }

    // ══════════════════════════════════════════════════════════════
    // 第三批方法（参考 mindcraft 41 actions + 14 queries）
    // ══════════════════════════════════════════════════════════════

    /// 持续跟随指定玩家（resume=true 模式）。
    pub fn follow_player(&self, player_name: &str, follow_dist: Option<f64>) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::FollowPlayer {
            player_name: player_name.into(),
            follow_dist,
        })
    }

    /// 搜索 minecraft.wiki。
    pub fn search_wiki(&self, query: &str) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::SearchWiki {
            query: query.into(),
        })
    }

    /// 查询最近村民的交易列表。
    pub fn villager_trades(&self, radius: Option<f64>) -> Result<ModAck> {
        self.lock_bridge()?
            .send(ModCommand::VillagerTrades { radius })
    }

    /// 与村民交易。
    pub fn trade_with_villager(
        &self,
        index: u32,
        count: Option<u32>,
        radius: Option<f64>,
    ) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::TradeWithVillager {
            index,
            count,
            radius,
        })
    }

    /// 看向指定玩家。
    pub fn look_at_player(&self, player_name: &str) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::LookAtPlayer {
            player_name: player_name.into(),
        })
    }

    /// 看向指定坐标。
    pub fn look_at_position(&self, x: f64, y: f64, z: f64) -> Result<ModAck> {
        self.lock_bridge()?
            .send(ModCommand::LookAtPosition { x, y, z })
    }

    /// 右键激活指定坐标方块。
    pub fn activate_block(&self, x: i32, y: i32, z: i32) -> Result<ModAck> {
        self.lock_bridge()?
            .send(ModCommand::ActivateBlock { x, y, z })
    }

    /// 对最近实体使用物品。
    pub fn use_on_entity(&self, entity_type: &str, radius: Option<f64>) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::UseOnEntity {
            entity_type: entity_type.into(),
            radius,
        })
    }

    /// 清空对话历史（mod 侧 ack）。
    pub fn clear_chat(&self) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::ClearChat)
    }

    /// 激活最近的指定类型方块。
    pub fn activate_nearest_block(&self, block_type: &str, radius: Option<f64>) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::ActivateNearestBlock {
            block_type: block_type.into(),
            radius,
        })
    }

    /// 查询合成计划。
    pub fn get_crafting_plan(&self, item: &str, count: u32) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::GetCraftingPlan {
            item: item.into(),
            count,
        })
    }

    /// 智能丢弃（moveAway + drop + goBack）。
    pub fn discard_smart(&self, item: &str, num: u32) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::DiscardSmart {
            item: item.into(),
            num,
        })
    }

    /// 战斗 AI（mod 侧自主走位：melee/kite/retreat，含苦力怕后撤+濒死撤退）。
    pub fn combat(&self, mode: &str, ticks: u32) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::Combat {
            mode: mode.into(),
            ticks,
        })
    }

    /// 精确看向世界坐标
    pub fn look_at(&self, x: f64, y: f64, z: f64) -> Result<()> {
        self.lock_bridge()?.send(ModCommand::LookAt { x, y, z })?;
        Ok(())
    }

    /// 绝对朝向（对齐 Mineflayer bot.look(yaw,pitch)）。
    pub fn look_abs(&self, yaw: f32, pitch: f32) -> Result<()> {
        self.lock_bridge()?
            .send(ModCommand::LookAbs { yaw, pitch })?;
        Ok(())
    }

    /// 钓鱼：手持钓鱼竿抛竿/收竿，ticks 持竿时长。
    pub fn fish(&self, ticks: u32) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::Fish { ticks })
    }

    /// 骑乘控制：mount 最近的 rideable / dismount / steer(left,forward)。
    pub fn ride(
        &self,
        action: &str,
        radius: Option<f64>,
        left: Option<f64>,
        forward: Option<f64>,
    ) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::Ride {
            action: action.to_string(),
            radius,
            left,
            forward,
        })
    }

    /// 睡觉跳夜（需要附近有床）。
    pub fn sleep(&self, radius: f64) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::Sleep {
            radius: Some(radius),
        })
    }

    /// 醒来。
    pub fn wake(&self) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::Wake)
    }

    /// 记住当前位置
    pub fn remember_here(&self, name: &str) -> String {
        let st = self.reload().ok();
        if let Some(s) = st {
            let pos = (s.position[0], s.position[1], s.position[2]);
            if let Ok(mut places) = self.lock_places() {
                places.insert(name.to_string(), pos);
            }
            format!("saved '{name}' at ({:.1},{:.1},{:.1})", pos.0, pos.1, pos.2)
        } else {
            format!("failed to save '{name}'")
        }
    }

    /// 去已记住的位置
    pub fn go_to_place(&self, name: &str) -> Result<String> {
        let pos = self.lock_places()?.get(name).cloned();
        match pos {
            Some((x, y, z)) => {
                let _ = self.move_to(x, y, z)?;
                Ok(format!("moving to '{name}' ({:.1},{:.1},{:.1})", x, y, z))
            }
            None => Ok(format!("place '{name}' not found")),
        }
    }

    /// 列出所有已保存位置
    pub fn list_places(&self) -> String {
        let Ok(places) = self.lock_places() else {
            return "failed to lock places".into();
        };
        if places.is_empty() {
            "no saved places".into()
        } else {
            places
                .iter()
                .map(|(k, (x, y, z))| format!("  {k}: ({x:.0},{y:.0},{z:.0})"))
                .collect::<Vec<_>>()
                .join("\n")
        }
    }

    /// 设置行为模式开关。
    pub fn set_mode(&self, name: &str, on: bool) {
        if let Ok(mut modes) = self.lock_modes() {
            modes.insert(name.to_string(), on);
        }
    }

    /// 查询行为模式是否开启（默认 false）。
    pub fn get_mode(&self, name: &str) -> bool {
        self.lock_modes()
            .is_ok_and(|m| *m.get(name).unwrap_or(&false))
    }

    /// 列出所有模式及状态。
    pub fn list_modes(&self) -> String {
        let Ok(modes) = self.lock_modes() else {
            return "failed to lock modes".into();
        };
        if modes.is_empty() {
            "no modes set (all default off)".into()
        } else {
            modes
                .iter()
                .map(|(k, v)| format!("  {k}: {v}"))
                .collect::<Vec<_>>()
                .join("\n")
        }
    }

    /// 合成物品（调 mod 直接操作 Inventory 扣材料加结果，零视觉依赖）。
    pub fn craft(&self, item: &str, count: u32) -> Result<()> {
        self.lock_bridge()?.send(ModCommand::Craft {
            item: item.to_string(),
            count,
        })?;
        Ok(())
    }

    /// 丢弃物品
    pub fn discard_item(&self, item: &str, num: u32) -> Result<()> {
        self.lock_bridge()?.send(ModCommand::Discard {
            item: item.to_string(),
            num,
        })?;
        Ok(())
    }

    /// 烧制物品
    pub fn smelt_item(&self, item: &str, num: u32) -> Result<()> {
        self.lock_bridge()?.send(ModCommand::Smelt {
            item: item.to_string(),
            num,
        })?;
        Ok(())
    }

    /// 附魔物品（消耗 XP 等级）
    pub fn enchant(&self, item: &str, levels: u32) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::Enchant {
            item: item.to_string(),
            levels,
        })
    }

    /// 在当前坐标建造下界传送门
    pub fn build_portal(&self) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::BuildPortal)
    }

    /// 传送到指定维度
    pub fn teleport_to(&self, dimension: &str) -> Result<ModAck> {
        self.lock_bridge()?.send(ModCommand::TeleportToDimension {
            dimension: dimension.to_string(),
        })
    }

    /// xcap 截 MC 窗口（仅用于 viewer 可视化；失败不影响主流程，返回空截图）。
    fn capture_xcap(&self) -> Screenshot {
        let windows = match xcap::Window::all() {
            Ok(w) => w,
            Err(_) => return Arc::new(Vec::new()),
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
                    return Arc::new(png);
                }
            }
        }
        Arc::new(Vec::new())
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
    if let Some(nt) = &st.nearest_threat {
        s.push_str(&format!(
            "  NEAREST THREAT: {} at {:.1}m\n",
            nt.r#type.replace("minecraft:", ""),
            nt.dist
        ));
    } else {
        s.push_str("  NEAREST THREAT: none\n");
    }
    s.push_str(&format!(
        "  Gamemode: {}  Dimension: {}  Biome: {}\n",
        st.gamemode, st.dimension, st.biome
    ));
    // 时间 + 白天黑夜（MC 时间：0=日出6:00, 6000=正午12:00, 12000=日落18:00, 18000=午夜0:00）
    let t = st.time % 24000;
    let hour = ((t / 1000 + 6) % 24) as u32;
    let minute = ((t % 1000) * 60 / 1000) as u32;
    let phase = if t < 13000 { "day" } else { "night" };
    s.push_str(&format!(
        "  Time: {:02}:{:02} ({})  tick={}\n",
        hour, minute, phase, t
    ));
    s.push_str(&format!(
        "  Light: sky={}/15 block={}/15  Weather: rain={} thunder={}\n",
        st.sky_light, st.block_light, st.raining, st.thundering
    ));
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
        .map(|i| {
            format!(
                "[{s}] {item}x{c}",
                s = i.slot + 1,
                item = i.id.replace("minecraft:", ""),
                c = i.count
            )
        })
        .collect();
    let main_inv: Vec<String> = st
        .inventory
        .iter()
        .filter(|i| i.count > 0 && i.slot >= 9)
        .map(|i| {
            format!(
                "[{s}] {item}x{c}",
                s = i.slot,
                item = i.id.replace("minecraft:", ""),
                c = i.count
            )
        })
        .collect();
    let hotbar_str = hotbar.join(", ");
    s.push_str(&format!(
        "HOTBAR (slot 1-9):  {}\n",
        if hotbar.is_empty() {
            "(empty)"
        } else {
            &hotbar_str
        }
    ));
    let main_inv_str = main_inv.join(", ");
    s.push_str(&format!(
        "INVENTORY (slots 9-):  {}\n",
        if main_inv.is_empty() {
            "(empty)"
        } else {
            &main_inv_str
        }
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
            "TARGETED: {} at ({},{},{}) distance {:.1}m\n",
            b.id.replace("minecraft:", ""),
            b.x,
            b.y,
            b.z,
            b.dist
        ));
    } else {
        s.push_str("TARGETED: none (pointing at sky or far away)\n");
    }

    // ── NEARBY BLOCKS (按类型去重：每种方块只展示最近的 MAX_PER_TYPE 个样本，避免大量同类型淹没 LLM) ──
    let mut blocks: Vec<&crate::bridge::NearbyBlock> = st.nearby_blocks.iter().collect();
    // 排序：先按优先级（功能性/资源 > 危险 > 建筑 > 其他），再按距离
    blocks.sort_by(|a, b| {
        let prio = |id: &str| -> u8 {
            // prio 0: 功能性方块（工作台/熔炉/箱子/门）+ 资源（原木/矿石）
            if id.contains("crafting_table")
                || id.contains("furnace")
                || id.contains("chest")
                || id.contains("_door")
                || id.contains("_bed")
                || id.contains("_log")
                || id.contains("_ore")
            {
                0
            // prio 1: 危险方块（水/岩浆/火）—— 安全相关信息必须让 LLM 看到
            } else if id.contains("water") || id.contains("lava") || id.contains("fire") {
                1
            // prio 2: 建筑材料（木板/石头/泥土）
            } else if id.contains("planks")
                || id.contains("stone")
                || id.contains("cobble")
                || id.contains("dirt")
            {
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
    // 按类型分组，每组只取最近的 MAX_PER_TYPE 个。同时统计每种类型的总数。
    const MAX_PER_TYPE: usize = 3;
    const MAX_TOTAL: usize = 30;
    use std::collections::BTreeMap;
    // 因 blocks 已按 prio+dist 排序，分组后组内顺序天然是"最近在前"。
    // 用 Vec<&NearbyBlock> 保留插入顺序（按首次出现顺序），同时记录每类型总数。
    let mut groups: Vec<(String, Vec<&crate::bridge::NearbyBlock>, usize)> = Vec::new();
    let mut group_idx: BTreeMap<String, usize> = BTreeMap::new();
    for b in &blocks {
        let key = b.id.replace("minecraft:", "");
        if let Some(&idx) = group_idx.get(&key) {
            groups[idx].2 += 1; // 总数 +1
            if groups[idx].1.len() < MAX_PER_TYPE {
                groups[idx].1.push(*b);
            }
        } else {
            let idx = groups.len();
            group_idx.insert(key.clone(), idx);
            groups.push((key, vec![*b], 1));
        }
    }
    // 渲染：每类型一行摘要 + 最近的几个坐标
    let mut block_lines: Vec<String> = Vec::new();
    let mut shown_total = 0;
    for (name, samples, total) in &groups {
        if shown_total >= MAX_TOTAL {
            break;
        }
        let coords: Vec<String> = samples
            .iter()
            .map(|b| {
                let hd = if b.height_diff >= 0.0 {
                    format!(" below+{:.0}", b.height_diff)
                } else {
                    format!(" above-{:.0}", -b.height_diff)
                };
                format!("({:.1},{:.1},{:.1})={:.1}m{}", b.x, b.y, b.z, b.dist, hd)
            })
            .collect();
        let suffix = if *total > samples.len() {
            format!(" (+{} more)", total - samples.len())
        } else {
            String::new()
        };
        block_lines.push(format!(
            "  {} x{}: {}{}",
            name,
            total,
            coords.join(", "),
            suffix
        ));
        shown_total += samples.len();
    }
    if !block_lines.is_empty() {
        let type_count = groups.len();
        let block_count = blocks.len();
        s.push_str(&format!(
            "NEARBY BLOCKS ({} blocks, {} types, showing up to {} per type):\n{}\n",
            block_count,
            type_count,
            MAX_PER_TYPE,
            block_lines.join("\n")
        ));
    } else {
        s.push_str("NEARBY BLOCKS: none\n");
    }

    // ── NEARBY ENTITIES (按类型去重：每种实体只展示最近的 MAX_ENT_PER_TYPE 个) ──
    if !st.entities.is_empty() {
        const MAX_ENT_PER_TYPE: usize = 3;
        let mut ents_sorted: Vec<&crate::bridge::NearbyEntity> = st.entities.iter().collect();
        ents_sorted.sort_by(|a, b| {
            a.r#type.cmp(&b.r#type).then(
                a.dist
                    .partial_cmp(&b.dist)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        });
        // 分组：每类型保留最近 MAX_ENT_PER_TYPE 个，并记录总数
        let mut ent_groups: Vec<(String, Vec<&crate::bridge::NearbyEntity>, usize)> = Vec::new();
        let mut ent_idx: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for e in &ents_sorted {
            let name = e.r#type.replace("minecraft:", "");
            if let Some(&i) = ent_idx.get(&name) {
                ent_groups[i].2 += 1;
                if ent_groups[i].1.len() < MAX_ENT_PER_TYPE {
                    ent_groups[i].1.push(*e);
                }
            } else {
                let i = ent_groups.len();
                ent_idx.insert(name.clone(), i);
                ent_groups.push((name, vec![*e], 1));
            }
        }
        let ent_lines: Vec<String> = ent_groups
            .iter()
            .map(|(name, samples, total)| {
                let coords: Vec<String> = samples
                    .iter()
                    .map(|e| {
                        format!(
                            "({:.1},{:.1},{:.1})={:.1}m hp={:.0}",
                            e.x, e.y, e.z, e.dist, e.health
                        )
                    })
                    .collect();
                let suffix = if *total > samples.len() {
                    format!(" (+{} more)", total - samples.len())
                } else {
                    String::new()
                };
                format!("  {} x{}: {}{}", name, total, coords.join(", "), suffix)
            })
            .collect();
        s.push_str(&format!(
            "NEARBY ENTITIES ({} total, {} types, up to {} per type):\n{}\n",
            ents_sorted.len(),
            ent_groups.len(),
            MAX_ENT_PER_TYPE,
            ent_lines.join("\n")
        ));
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
        // 卡住检测：记录位置，若 velocity 非零则视为正在尝试移动
        {
            if let Ok(mut d) = self.unstuck.lock() {
                let moving =
                    st.velocity[0] * st.velocity[0] + st.velocity[2] * st.velocity[2] > 0.01;
                d.record(st.position[0], st.position[2], moving);
            }
        }
        // mod 模式不需要截图——精确结构化数据已经足够，截图留给 visual_perceive
        let mut scene_desc = build_scene_desc(&st);
        if let Ok(d) = self.unstuck.lock()
            && d.is_stuck()
        {
            scene_desc.push_str("\n[WARNING] 可能卡住了（位置长时间无变化）！尝试：1) press space 跳 2) dig_at 前方方块 3) 选不同方向 move_to");
        }
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
            screenshot: Arc::new(Vec::new()),
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
                let ack = self.lock_bridge()?.send(ModCommand::Look { dx, dy })?;
                Ok(ack_to_result(ack, format!("look dx={dx} dy={dy}")))
            }
            // ServerPlayer 架构移除了 KeyMapping 模拟的 Move/Press/Mine。
            // 这些 Action 变体保留是为了兼容 core/types 的 Action 枚举，但实际调用时
            // 返回错误，引导调用方使用对应的具体方法（move_to / select_slot / dig_at 等）。
            Action::Move { dir, ticks } => Err(anyhow!(
                "ServerPlayer 架构不支持相对移动 Action::Move({dir:?}, {ticks})；请用 move_to(x,y,z) 方法"
            )),
            Action::Press { keys, ticks } => Err(anyhow!(
                "ServerPlayer 架构不支持按键模拟 Action::Press({keys}, {ticks})；请用具体方法：select_slot/use_item/dig_at 等"
            )),
            Action::Mine { ticks: _ } => {
                // 基于准星指向方块调用 dig_at（mod 侧 destroyBlock 原生破坏）
                let st = self.reload()?;
                if let Some(tb) = &st.targeted_block {
                    // 准星方块坐标 = 玩家眼睛位置 + 视线方向 * 距离，近似取整
                    let px = st.position[0];
                    let py = st.position[1] + 1.62; // 眼睛高度
                    let pz = st.position[2];
                    let yaw_rad = st.yaw.to_radians();
                    let pitch_rad = st.pitch.to_radians();
                    // MC yaw: 0=朝南(+z), 90=朝西(-x), 180=朝北(-z), 270=朝东(+x)
                    // 视线方向
                    let dx = -yaw_rad.sin() * pitch_rad.cos();
                    let dy = -pitch_rad.sin();
                    let dz = yaw_rad.cos() * pitch_rad.cos();
                    let bx = (px + dx * tb.dist).round() as i32;
                    let by = (py + dy * tb.dist).round() as i32;
                    let bz = (pz + dz * tb.dist).round() as i32;
                    let ack = self.lock_bridge()?.send(ModCommand::DigAt {
                        x: bx,
                        y: by,
                        z: bz,
                    })?;
                    let broken = ack.broken.unwrap_or(false);
                    let block_id = ack.block_id.clone().unwrap_or_default();
                    let detail = if broken {
                        format!("mine 成功，破坏 {} at ({bx},{by},{bz})", block_id)
                    } else {
                        format!("mine 未破坏方块 at ({bx},{by},{bz})（可能不可破坏或距离过远）")
                    };
                    Ok(ack_to_result(ack, detail))
                } else {
                    Err(anyhow!(
                        "mine 失败：准星未指向任何方块（targeted_block 为空）"
                    ))
                }
            }
            Action::AimAndMine { target } => {
                // 找最近匹配方块 → look_at 精确对准 → dig_at 原生破坏
                let st = self.reload()?;
                match nearest_block(&st, &target) {
                    Some(b) => {
                        let bx = b.x.round() as i32;
                        let by = b.y.round() as i32;
                        let bz = b.z.round() as i32;
                        self.lock_bridge()?.send(ModCommand::LookAt {
                            x: b.x,
                            y: b.y + 0.5,
                            z: b.z,
                        })?;
                        thread::sleep(Duration::from_millis(180));
                        let ack = self.lock_bridge()?.send(ModCommand::DigAt {
                            x: bx,
                            y: by,
                            z: bz,
                        })?;
                        let broken = ack.broken.unwrap_or(false);
                        let block_id = ack.block_id.clone().unwrap_or_default();
                        let detail = if broken {
                            format!("aim_and_mine 成功，破坏 {} at ({bx},{by},{bz})", block_id)
                        } else {
                            format!(
                                "aim_and_mine 已对准 {target} at ({bx},{by},{bz}) 但未破坏（可能需走近）"
                            )
                        };
                        Ok(ack_to_result(ack, detail))
                    }
                    None => Err(anyhow!(
                        "aim_and_mine 失败：附近未找到匹配 '{target}' 的方块"
                    )),
                }
            }
            Action::Click { element_id } => Err(anyhow!(
                "mod 模式无 2D 点击（element_id={element_id}）；改用 look/dig_at/place_at 操作 3D 世界"
            )),
        }
    }
}
