//! azalea tick handler：所有 bot 逻辑（命令队列排空、世界扫描、模式、动作执行）。
//! P2.2（2026-08-03）：从 azalea/mod.rs 纯移动拆出，行为与拆前逐字一致。
//!
//! 说明：handler 是 `fn` 指针（azalea 要求不捕获），队列/事件通道挂在
//! 自定义 `BotState`（Arc<Mutex<...>>，实现 Component + Default + Clone）上。

use super::{
    ActionManager, AzaleaBot, BotCommand, BotEvent, ChunkPos, EntityAgg, ObsidianTask, Priority,
    QueuedCommand, SubmitOutcome, auto_equip_best_pickaxe, count_item, count_overhead_solid,
    do_consume, do_discard, do_equip, entity_kind_name, find_hotbar_slot_for, find_item_slots,
    has_any_pickaxe_in_inventory, is_hard_block, mine_above_reached_surface,
    normalize_entity_target, parse_chat_command,
};
use azalea::BlockPos;
use azalea::core::direction::Direction;
use azalea::core::hit_result::HitResult;
use azalea::pathfinder::goals::{BlockPosGoal, RadiusGoal, YGoal};
use azalea::player::GameProfileComponent;
use azalea::prelude::*;
use azalea::protocol::packets::game::s_player_action::{
    Action as ServerboundPlayerActionKind, ServerboundPlayerAction,
};
use azalea_registry::DataRegistryKey;
use azalea_registry::builtin::{BlockKind, EntityKind, ItemKind};
use craft_agent::core::memory::{MemoryKind, MemoryPos, WorldMemory};
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// P135：读取物品耐久 (damage, max_damage)——剩余耐久 = max - damage。
/// 非工具（max_damage=0）返回 None；工具用 azalea 组件默认表兜底满耐久。
/// 解决根因：石镐反复"神秘消失"= 耐久耗尽自动销毁，LLM 却无法预知
/// （perceive 不显示耐久 → 未在损坏前换镐）。此值注入 game_state 供 perceive 展示与警示。
pub(crate) fn item_durability(st: &azalea_inventory::ItemStack) -> Option<(i32, i32)> {
    use azalea_inventory::components::{Damage, MaxDamage};
    let max = st
        .get_component::<MaxDamage>()
        .map(|c| c.amount)
        .unwrap_or(0);
    if max <= 0 {
        return None;
    }
    let dmg = st.get_component::<Damage>().map(|c| c.amount).unwrap_or(0);
    Some((dmg, max))
}

fn nearby_active_portal(bot: &Client, center: BlockPos) -> bool {
    let Ok(world) = bot.world() else {
        return false;
    };
    let world = world.read();
    for dx in -5..=5 {
        for dy in -5..=5 {
            for dz in -5..=5 {
                let pos = BlockPos::new(center.x + dx, center.y + dy, center.z + dz);
                if let Some(state) = world.get_block_state(pos) {
                    let kind: BlockKind = state.into();
                    if kind == BlockKind::NetherPortal {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// 在 target 周围 4 格范围内找最近的实心方块（空气/水/岩浆排除）。
/// P101：mine 目标为空气时自动修正到最近实心方块，根治 LLM 盲猜坐标死循环。
fn nearest_solid_block(bot: &Client, x: i32, y: i32, z: i32) -> Option<BlockPos> {
    let world = bot.world().ok()?;
    let mut best: Option<(i64, BlockPos)> = None;
    for d in 1i32..=4 {
        for dx in -d..=d {
            for dy in -1..=2 {
                for dz in -d..=d {
                    let pos = BlockPos::new(x + dx, y + dy, z + dz);
                    let bk: Option<BlockKind> = world.read().get_block_state(pos).map(|b| b.into());
                    let solid = bk
                        .map(|k| {
                            k != BlockKind::Air && k != BlockKind::Water && k != BlockKind::Lava
                        })
                        .unwrap_or(false);
                    if solid {
                        let dist = (dx as i64).pow(2) + (dy as i64).pow(2) + (dz as i64).pow(2);
                        if best.as_ref().map(|(bd, _)| dist < *bd).unwrap_or(true) {
                            best = Some((dist, pos));
                        }
                    }
                }
            }
        }
        if best.is_some() {
            break;
        }
    }
    best.map(|(_, pos)| pos)
}

/// P126：goto 目标是实心矿石时是否值得"自动转直接挖掘"（等同 mine）。
/// P126b（收紧）：仅矿石（含 deepslate_*_ore）自动改挖——矿石是 LLM 明确想
/// "获得"的目标，转挖无损；P126a 曾把 stone/dirt 等岩层也纳入，实测 LLM 长距离
/// goto 时每块挡路石头都被自动挖掉 → 隧道式偏航（一路挖向 63m 外的目标），
/// 岩层阻挡应回到原拒绝逻辑让 LLM 换路线（P65/P69b 建议）。与 P101/P102 的
/// "派发时自动修正"同纪律，不影响正常 goto 到可站立目标的行为。
fn is_natural_mineable(kind: Option<BlockKind>) -> bool {
    let Some(bk) = kind else {
        return false;
    };
    if matches!(
        bk,
        BlockKind::Bedrock
            | BlockKind::Obsidian
            | BlockKind::Water
            | BlockKind::Lava
            | BlockKind::Barrier
    ) {
        return false;
    }
    let name = bk
        .to_str()
        .strip_prefix("minecraft:")
        .unwrap_or(bk.to_str());
    name.ends_with("_ore")
}

/// P132：goto 目标实心、上方无空气（P69b fallback 失效）且非矿石（P126 失效）时，
/// 找目标附近最近的可站立空气点，把 goto 目标自动修正到该点。LLM 盲猜洞穴内
/// 岩体里的坐标通常离真实可走空气很近（几格内），直接废弃坐标只会死循环
/// （实测连续 6+ 次 goto 同一岩体内坐标全失败）。与 P101/P102 "派发时自动修正"
/// 同纪律。若无障碍才走原拒绝逻辑。
const GOTO_P132_AIR_SEARCH_RADIUS: i32 = 10;

fn nearest_standable_air(bot: &Client, x: i32, y: i32, z: i32) -> Option<BlockPos> {
    let world = bot.world().ok()?;
    let mut best: Option<(i64, BlockPos)> = None;
    for d in 1i32..=GOTO_P132_AIR_SEARCH_RADIUS {
        for dx in -d..=d {
            for dz in -d..=d {
                // 跳过与 (x,z) 水平距离 >d 的格子（保持曼哈顿径向扩展顺序）
                if dx.abs().max(dz.abs()) != d {
                    continue;
                }
                for dy in -2..=2 {
                    let pos = BlockPos::new(x + dx, y + dy, z + dz);
                    let air = world
                        .read()
                        .get_block_state(pos)
                        .map(|b| b.is_air())
                        .unwrap_or(false);
                    if !air {
                        continue;
                    }
                    // 脚下必须是实心（能站立），否则洞顶空气点会让 pathfinder 绕远
                    let feet_solid = world
                        .read()
                        .get_block_state(pos.down(1))
                        .map(|b| {
                            let k: BlockKind = b.into();
                            k != BlockKind::Air && k != BlockKind::Water && k != BlockKind::Lava
                        })
                        .unwrap_or(false);
                    if !feet_solid {
                        continue;
                    }
                    let dist = (dx as i64).pow(2) + (dy as i64).pow(2) + (dz as i64).pow(2);
                    if best.as_ref().map(|(bd, _)| dist < *bd).unwrap_or(true) {
                        best = Some((dist, pos));
                    }
                }
            }
        }
        if best.is_some() {
            break;
        }
    }
    best.map(|(_, pos)| pos)
}

/// P120b：无镐时 mine_above 自动绕行的软土柱扫描。
/// 在 (x, y, z) 周围 radius 格水平范围内，找最近的"软方块列"（该列
/// 头顶 y+1..y+3 任一格是非硬方块且非空气：dirt/grass/sand/gravel/
/// sandstone 等），返回该列脚底坐标（x, y, z），供 pathfinder 绕行后
/// 从软土向上挖。徒手挖软土 ~0.25s/格 vs 硬方块 ~8s/格（差 32 倍），
/// 绕软土柱比死磕硬天花板快得多——MC 常识：无镐时走土坡/沙堆，不凿岩壁。
/// 注意：只查 y+1 曾漏掉软土在更高层的场景（probe p120b step7），
/// 放宽到 y+1..y+3 三层。
fn nearest_soft_column(bot: &Client, x: i32, y: i32, z: i32, radius: i32) -> Option<BlockPos> {
    let world = bot.world().ok()?;
    let mut best: Option<(i64, BlockPos)> = None;
    for d in 1i32..=radius {
        for dx in -d..=d {
            for dz in -d..=d {
                if dx == 0 && dz == 0 {
                    continue;
                }
                let col_x = x + dx;
                let col_z = z + dz;
                let soft = (1..=3).any(|dy| {
                    let head = BlockPos::new(col_x, y + dy, col_z);
                    world
                        .read()
                        .get_block_state(head)
                        .map(|b| {
                            let k: BlockKind = b.into();
                            k != BlockKind::Air
                                && k != BlockKind::Water
                                && k != BlockKind::Lava
                                && !is_hard_block(azalea::block::BlockState::from(k))
                        })
                        .unwrap_or(false)
                });
                if soft {
                    let dist = (dx as i64).pow(2) + (dz as i64).pow(2);
                    if best.as_ref().map(|(bd, _)| dist < *bd).unwrap_or(true) {
                        best = Some((dist, BlockPos::new(col_x, y, col_z)));
                    }
                }
            }
        }
        if best.is_some() {
            break;
        }
    }
    best.map(|(_, pos)| pos)
}

/// 把感兴趣的 BlockKind 映射为记忆元数据（item, 标签, 类别）。
/// 返回 None 表示该方块不值得记忆。
fn block_memory_meta(bk: BlockKind) -> Option<(String, &'static str, MemoryKind)> {
    // P5 修复：用 to_str() 拿到 snake_case minecraft id（如 "dark_oak_log"），
    // 原代码用 format!("{bk:?}").to_lowercase() 得到 "darkoaklog"（无下划线），
    // LLM 看到 "darkoaklog" 用 gather("dark_oak_log") 报"未知物品" → 100% 卡死。
    let name_full = bk.to_str();
    let name = name_full.strip_prefix("minecraft:").unwrap_or(name_full);
    // 原木类（oak_log / dark_oak_log / birch_log / ...）和菌丝类（crimson_stem / warped_stem）
    if name.ends_with("_log") || name.ends_with("_stem") {
        return Some((name.to_string(), "树木/原木", MemoryKind::Resource));
    }
    // 矿石类
    if name.ends_with("_ore") || name == "ancient_debris" {
        return Some((name.to_string(), "矿石", MemoryKind::Resource));
    }
    match bk {
        BlockKind::CraftingTable => {
            Some(("crafting_table".into(), "工作台", MemoryKind::Structure))
        }
        BlockKind::Furnace => Some(("furnace".into(), "熔炉", MemoryKind::Structure)),
        BlockKind::Chest => Some(("chest".into(), "箱子", MemoryKind::Container)),
        BlockKind::SmithingTable => {
            Some(("smithing_table".into(), "锻造台", MemoryKind::Structure))
        }
        BlockKind::EnchantingTable => {
            Some(("enchanting_table".into(), "附魔台", MemoryKind::Structure))
        }
        BlockKind::NetherPortal => Some(("nether_portal".into(), "下界传送门", MemoryKind::Portal)),
        BlockKind::Lava => Some(("lava".into(), "岩浆", MemoryKind::Hazard)),
        BlockKind::Water => Some(("water".into(), "水", MemoryKind::Hazard)),
        _ => None,
    }
}

/// 扫描去重 TTL：同一坐标在此时间内不再重新向服务端查询（省开销）。
/// 超过 TTL 后重新 `get_block_state` 校验，让"树被砍/方块被破坏"等世界变化
/// 能反映到记忆（消失的资源点标记 depleted，消失的结构/容器直接遗忘）。
const SCAN_TTL_MS: u64 = 30_000;

/// 扫描 bot 周围半径内的关键方块，回填到 WorldMemory。
/// 用 `scanned`（pos → 上次扫描时间戳）去重 + TTL 重验。
fn record_surroundings(
    bot: &Client,
    mem: &WorldMemory,
    center: &MemoryPos,
    scanned: &Arc<Mutex<HashMap<MemoryPos, u64>>>,
) {
    let world = match bot.world() {
        Ok(w) => w,
        Err(_) => return,
    };
    let radius = 8i32;
    let now = now_ms();
    let mut to_write: Vec<(MemoryPos, String, &'static str, MemoryKind)> = Vec::new();
    let mut to_deplete: Vec<MemoryPos> = Vec::new();
    let mut to_forget: Vec<MemoryPos> = Vec::new();
    {
        let mut scanned_g = scanned.lock().unwrap();
        for dx in -radius..=radius {
            for dy in -radius..=radius {
                for dz in -radius..=radius {
                    let pos = BlockPos::new(center.x + dx, center.y + dy, center.z + dz);
                    let mp = MemoryPos::new(pos.x, pos.y, pos.z);
                    // TTL 内已扫过：跳过（世界变化由 action 路径/B 的 forget 即时处理）
                    if let Some(&last) = scanned_g.get(&mp)
                        && now.saturating_sub(last) < SCAN_TTL_MS
                    {
                        continue;
                    }
                    scanned_g.insert(mp, now);
                    let still_memory = world
                        .read()
                        .get_block_state(pos)
                        .map(|s| block_memory_meta(s.into()));
                    match still_memory {
                        Some(Some((item, label, kind))) => {
                            to_write.push((mp, item, label, kind));
                        }
                        // 方块不再是记忆类（被挖/被破坏/变空气）：
                        // 若原记忆是资源点 → 标记 depleted（保留但不再推荐）；否则遗忘。
                        Some(None) | None => {
                            if let Some(c) = mem.get(mp) {
                                if c.kind == MemoryKind::Resource {
                                    to_deplete.push(mp);
                                } else {
                                    to_forget.push(mp);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    for (mp, item, label, kind) in to_write {
        match kind {
            MemoryKind::Resource => mem.record_resource(mp, &item, label, None),
            MemoryKind::Structure => mem.record_structure(mp, &item, label),
            MemoryKind::Container => mem.record_container(mp, label, ""),
            MemoryKind::Portal => mem.record(mp, MemoryKind::Portal, Some(&item), label, None),
            MemoryKind::Hazard => mem.record(mp, MemoryKind::Hazard, Some(&item), label, None),
            _ => mem.record(mp, kind, Some(&item), label, None),
        }
    }
    for p in to_deplete {
        mem.mark_depleted(p, true);
    }
    for p in to_forget {
        mem.forget_pos(p);
    }
}

/// P155：mine 靠近看门狗状态。
/// (目标 x, 目标 y, 目标 z, 锚点 x, 锚点 y, 锚点 z, 开始 tick, 无进展 tick 数)。
/// 锚点是上次有进展时 bot 的位置；无进展 tick 数在净移动 >1.5 格时重置为 0。
#[derive(Clone, Copy, Debug)]
pub struct MineApproachWatchdog {
    pub target: (i32, i32, i32),
    pub anchor: (f64, f64, f64),
    pub start_tick: u64,
    pub stall_ticks: u64,
}

/// P162b：goto 执行中卡住检测状态（pathfinder 重算循环）。
/// (上次位置, 连续无移动 tick 数)。
pub type GotoStuckState = (Option<(f64, f64, f64)>, u64);

/// handler 状态：持有命令队列、事件发送端与最近坐标（跨事件持久，Arc 共享）。
#[derive(Component, Clone)]
pub struct BotState {
    pub cmd_queue: Arc<Mutex<Vec<QueuedCommand>>>,
    pub evt_tx: Arc<mpsc::UnboundedSender<BotEvent>>,
    pub last_position: Arc<Mutex<Option<azalea::Vec3>>>,
    /// 持续下挖标志：收到 MineBelow 后置 true，Tick 内只要未在挖就重复触发，
    /// 对齐 POC 的持续挖矿逻辑（azalea 单次 start_mining 可能因中断失效）。
    pub mining_below: Arc<Mutex<bool>>,
    /// 持续上挖标志：收到 MineAbove 后置 true，Tick 内只要未在挖就重复触发。
    /// 用于地下脱困——头顶方块挖完后 bot 自动跳起，下一格又挖，直到头顶是空气。
    pub mining_above: Arc<Mutex<bool>>,
    /// Y at the start of the current MineAbove command. The synchronous tool
    /// completes only after actual upward movement, never on dispatch alone.
    pub mining_above_start_y: Arc<Mutex<Option<i32>>>,
    /// Direction tried by deterministic staircase ascent. Rotated whenever a
    /// concrete adjacent-up goal makes no progress.
    pub mining_above_direction: Arc<Mutex<usize>>,
    /// ActionManager：封装 pending 槽 + 按命令类型超时 + 抢占 + 快循环检测。
    /// 取代原硬编码 60-tick 超时（合成/采集/熔炼等长任务被误杀）。
    /// 字段保留 pending/pending_since/busy 的 Arc 引用，供旧代码兼容访问。
    pub action_mgr: ActionManager,
    /// 共享世界记忆库（适配器/工具/Agent 共用；handler 内扫描回填）。
    pub memory: Option<WorldMemory>,
    /// 已扫描记录的坐标 → 上次扫描时间戳（TTL 去重 + 重验世界变化）。
    pub scanned: Arc<Mutex<HashMap<MemoryPos, u64>>>,
    /// P65/P66：goto 卡死看门狗。(last_x, last_y, last_z, stall_count)。
    /// 若连续 goto 超时但 bot 净移动 <1.5 格（无论目标坐标如何变），累计 stall，
    /// 达阈值即强制脱困（地表挖开阻挡方块 / 地下 mine_above）。
    pub goto_watchdog: Arc<Mutex<(i32, i32, i32, u32)>>,
    /// P66：goto 冷却表（按 bot 当前格子）。触发脱困后冷却该格子 N tick，
    /// 期间 goto 直接拒绝，打破脚本/LLM 的 goto 死循环。
    pub goto_cooldown: Arc<Mutex<HashMap<ChunkPos, u64>>>,
    /// P162b：goto 执行中卡住检测（pathfinder 重算循环）。
    /// (上次位置, 连续无移动 tick 数)。goto 未完成且距目标 >2.5m 时每 tick 更新；
    /// 连续 100 tick（5s）净移动 <1 格 → 判定 pathfinder 不完整路径重算循环，
    /// 强制失败 + P66 脱困，避免空等 20s 超时。
    pub goto_stuck: Arc<Mutex<GotoStuckState>>,
    /// P67：全局"原地冻死"看门狗。bot 位置长时间（~20s）不变且循环仍在推进，
    /// 说明卡在某个不动作（如空转 run_script / 无效 interact）。累计到阈值即
    /// 向 LLM 推强警告，逼其换策略（pi-agent 自主止损，覆盖所有非 goto 卡死）。
    pub no_move_ticks: Arc<Mutex<u64>>,
    pub last_seen_pos: Arc<Mutex<ChunkPos>>,
    /// P67：make_obsidian 状态机。(remaining, phase, obsidian_pos)。phase: 0=找岩浆放水, 1=等黑曜石生成, 2=挖黑曜石。
    pub make_obsidian: Arc<Mutex<ObsidianTask>>,
    /// P68：跟随模式。Some(target) 表示正在跟随该玩家（None 名=跟随最近玩家）；
    /// None 表示未跟随。handler 每 tick 读取目标坐标 goto。
    pub follow_target: Arc<Mutex<Option<Option<String>>>>,
    /// P77：hunting 模式——攻击动物后自动拾取掉落物的截止 tick（0=无窗口）。
    pub hunt_pickup_until: Arc<Mutex<u64>>,
    /// P77：战斗模式请求自动装备的武器名（防重复 push Equip；None=无待装备）。
    pub combat_equip_pending: Arc<Mutex<Option<String>>>,
    /// P87：战斗走位（strafe）冷却 tick——上次走位后 40 tick（2s）内不再提交走位，
    /// 避免每轮检查都打断寻路。i64 存 ticks_connected 快照。
    pub combat_strafe_cd: Arc<Mutex<i64>>,
    /// P95：取消请求标志。外部 `AzaleaBot::cancel_commands` 置位，
    /// handler 每 tick 检查并执行真正的中止（强停寻路/清槽/回复取消）。
    pub cancel_flag: Arc<AtomicBool>,
    /// P101：当前 mine 命令的实际挖掘目标（派发时修正后）+ 派发时原目标是否空气。
    /// 解决 done 判定与反馈歧义：done 轮询用实际目标判空气（否则修正挖掘被
    /// 立即终结），done 分支据 original_air 区分"成功挖掉/修正成功/空气 no-op"。
    pub last_mine_eff: Arc<Mutex<Option<(BlockPos, bool)>>>,
    /// P116：被禁用的自动反应式模式集合（set_mode 开关）。空=全部启用。
    /// 模式名：self_preservation/self_defense/cowardice/hunting/item_collecting。
    pub mode_switches: Arc<Mutex<HashSet<String>>>,
    /// P155：mine 靠近看门狗。P150 的 mine 靠近分支（距离>2.5m 时 start_goto
    /// RadiusGoal）会 clear_pending + return，ActionManager 的 check_timeout 永远
    /// 追不到"正在靠近"状态——pathfinder 找不到路径时 mine 无限卡在靠近循环，
    /// LLM 每轮看到"目标距 X.Xm，正在靠近"却从不 start_mining（本会话反复复现）。
    /// 记录 (目标坐标, 上次位置, 无进展 tick 数, 开始 tick)。连续 120 tick（6s）
    /// 且净移动 <1.5 格 → 判定寻路失败：force_stop_pathfinding + 报错让 LLM 换策略。
    pub mine_approach_watchdog: Arc<Mutex<Option<MineApproachWatchdog>>>,
    /// P120：mine_above 无镐徒手挖警告去重（dispatch 每 tick 重入 + P60b/ceiling
    /// 持续 tick 分支都会触发）。首次警告后置 true，命令结束（done/超时）重置。
    pub mining_above_no_pick_warned: Arc<Mutex<bool>>,
    /// P120b：无镐时 mine_above 自动绕行的软土柱目标。
    /// 头顶是硬方块且无镐时，自动扫描附近软土柱（dirt/grass/sand/gravel/...），
    /// 找到即改挖软土柱（徒手 ~0.25s/格，比硬方块 8s/格快 32 倍），
    /// 避免死磕硬天花板。Some((x, y, z)) = 软土柱脚坐标。命令结束重置。
    pub mining_above_soft_column: Arc<Mutex<Option<BlockPos>>>,
    /// P160：make_obsidian 状态机启动 tick。防止状态机在"装水失败/找不到岩浆"
    /// 时无限重试（每 tick block_interact + pathfinder，拖死 viewer API）。
    /// 启动时记录 ticks_connected，状态机推进处检查 >600 tick（30s）强制失败。
    pub make_obsidian_start_tick: Arc<Mutex<Option<u64>>>,
    /// P161d：交互进行中标志（含截止 tick）。BlockInteract 对液体方块交互期间
    /// 设置，P60c 地下脱困检测到该标志时跳过上升，避免装水/装岩浆被脱困打断。
    /// (起始 tick, 截止 tick)：截止前 P60c 不触发。
    pub interact_hold_until: Arc<Mutex<Option<u64>>>,
}

impl Default for BotState {
    fn default() -> Self {
        // dummy：真实 state 总由 connect() 构造，此处仅满足 trait 约束。
        let (_, rx) = mpsc::unbounded_channel::<BotEvent>();
        drop(rx);
        BotState {
            cmd_queue: Arc::new(Mutex::new(Vec::new())),
            evt_tx: Arc::new(mpsc::unbounded_channel::<BotEvent>().0),
            last_position: Arc::new(Mutex::new(None)),
            goto_watchdog: Arc::new(Mutex::new((0, 0, 0, 0))),
            goto_cooldown: Arc::new(Mutex::new(HashMap::new())),
            goto_stuck: Arc::new(Mutex::new((None, 0))),
            no_move_ticks: Arc::new(Mutex::new(0)),
            last_seen_pos: Arc::new(Mutex::new((0, 0, 0))),
            make_obsidian: Arc::new(Mutex::new(None)),
            follow_target: Arc::new(Mutex::new(None)),
            mining_below: Arc::new(Mutex::new(false)),
            mining_above: Arc::new(Mutex::new(false)),
            mining_above_start_y: Arc::new(Mutex::new(None)),
            mining_above_direction: Arc::new(Mutex::new(0)),
            action_mgr: ActionManager::new(),
            memory: None,
            scanned: Arc::new(Mutex::new(HashMap::new())),
            hunt_pickup_until: Arc::new(Mutex::new(0)),
            combat_equip_pending: Arc::new(Mutex::new(None)),
            combat_strafe_cd: Arc::new(Mutex::new(0)),
            last_mine_eff: Arc::new(Mutex::new(None)),
            cancel_flag: Arc::new(AtomicBool::new(false)),
            mode_switches: Arc::new(Mutex::new(HashSet::new())),
            mine_approach_watchdog: Arc::new(Mutex::new(None)),
            mining_above_no_pick_warned: Arc::new(Mutex::new(false)),
            mining_above_soft_column: Arc::new(Mutex::new(None)),
            make_obsidian_start_tick: Arc::new(Mutex::new(None)),
            interact_hold_until: Arc::new(Mutex::new(None)),
        }
    }
}

impl BotState {
    /// P116：查询自动反应式模式是否被 set_mode 禁用。
    pub fn mode_disabled(&self, mode: &str) -> bool {
        self.mode_switches
            .lock()
            .map(|s| s.contains(mode))
            .unwrap_or(false)
    }
}

fn nearby_player_position(bot: &Client, target: Option<&str>) -> Option<azalea::Vec3> {
    let bot_pos = bot.position().ok();
    let players = bot.nearby_players().ok()?;
    let mut closest: Option<(f64, azalea::Vec3)> = None;
    for player in players.iter() {
        let name = player
            .component::<GameProfileComponent>()
            .map(|profile| profile.0.name.clone())
            .unwrap_or_default();
        if target.is_some_and(|wanted| name != wanted) {
            continue;
        }
        let Ok(position) = player.position() else {
            continue;
        };
        let distance = bot_pos.map_or(0.0, |origin| {
            ((origin.x - position.x).powi(2)
                + (origin.y - position.y).powi(2)
                + (origin.z - position.z).powi(2))
            .sqrt()
        });
        if closest.as_ref().is_none_or(|(best, _)| distance < *best) {
            closest = Some((distance, position));
        }
    }
    closest.map(|(_, position)| position)
}

/// 当前 pending 命令的紧凑中文标签（perceive「当前动作」行 / game_state 用，
/// P126d 对标 Mindcraft $ACTION）。无 pending 命令返回 None（调用方渲染"空闲"）。
fn current_action_label(action_mgr: &ActionManager) -> Option<String> {
    let qc = action_mgr.peek_pending()?;
    Some(match &qc.cmd {
        BotCommand::Goto { x, y, z } => format!("前往 ({x}, {y}, {z})"),
        BotCommand::GotoAnchor { name } => format!("前往锚点 {name}"),
        BotCommand::GotoPlayer { name } => {
            format!("前往玩家 {}", name.as_deref().unwrap_or("最近"))
        }
        BotCommand::Mine { x, y, z } => format!("挖掘 ({x}, {y}, {z})"),
        BotCommand::MineBelow => "向下挖矿井".to_string(),
        BotCommand::MineAbove => "向上挖通竖井".to_string(),
        BotCommand::BlockInteract { x, y, z } => format!("交互方块 ({x}, {y}, {z})"),
        BotCommand::TillAndSow { x, y, z, seed } => {
            format!("犁地播种 ({x}, {y}, {z}) {seed}")
        }
        BotCommand::Sleep => "睡觉跳夜".to_string(),
        BotCommand::Harvest => "收割作物".to_string(),
        BotCommand::Chat { content } => {
            format!("发送聊天: {}", content.chars().take(20).collect::<String>())
        }
        BotCommand::Attack { target } => format!("攻击 {target}"),
        BotCommand::Craft2x2 { item, count } => format!("合成 {item} ×{count}（2×2）"),
        BotCommand::Craft3x3 { item, count, .. } => format!("合成 {item} ×{count}（3×3）"),
        BotCommand::Smelt {
            output,
            fuel,
            count,
            ..
        } => {
            format!("熔炼 {output}（燃料 {fuel}）×{count}")
        }
        BotCommand::Gather { item, count } => format!("采集 {item} ×{count}"),
        BotCommand::MakeObsidian { count } => format!("制造黑曜石 ×{count}"),
        BotCommand::Place { item, x, y, z } => format!("放置 {item} ({x}, {y}, {z})"),
        BotCommand::OpenContainer { x, y, z } => format!("打开容器 ({x}, {y}, {z})"),
        BotCommand::AutoCraft { item, count } => format!("自动合成 {item} ×{count}"),
        BotCommand::Enchant { item, level } => format!("附魔 {item} 等级{level}"),
        BotCommand::Trade { offer } => format!("交易（报价{offer}）"),
        BotCommand::InteractEntity { kind } => format!("交互实体 {kind}"),
        BotCommand::Pickup => "拾取掉落物".to_string(),
        BotCommand::Defend => "防御".to_string(),
        BotCommand::Equip { item, slot } => format!("装备 {item} → {slot}"),
        BotCommand::Discard { item, count } => format!("丢弃 {item} ×{count}"),
        BotCommand::Consume { item } => format!("使用 {item}"),
        BotCommand::ChestView { x, y, z } => format!("查看容器 ({x}, {y}, {z})"),
        BotCommand::ChestWithdraw {
            x,
            y,
            z,
            item,
            count,
        } => {
            format!("取物 {item} ×{count} ({x}, {y}, {z})")
        }
        BotCommand::ChestDeposit {
            x,
            y,
            z,
            item,
            count,
        } => {
            format!("存物 {item} ×{count} ({x}, {y}, {z})")
        }
        BotCommand::Follow { .. } => "跟随玩家".to_string(),
        BotCommand::SearchBlock { item, radius, .. } => {
            format!("搜索 {item}（半径{radius}）")
        }
        BotCommand::MoveAway { .. } => "远离实体".to_string(),
        BotCommand::StopFollow => "停止跟随".to_string(),
        BotCommand::SetMode { mode, .. } => format!("切换模式 {mode}"),
        BotCommand::UseItem { item, .. } => format!("使用物品 {item}"),
        BotCommand::Shoot { .. } => "拉弓射箭".to_string(),
        BotCommand::Give {
            item,
            count,
            target,
        } => {
            format!("给玩家 {item} ×{count}（{target:?}）")
        }
        BotCommand::RawState => "原始状态 dump".to_string(),
        BotCommand::Memory { action, .. } => format!("记忆操作 {action}"),
    })
}

/// P119：找最近匹配目标实体，返回朝它瞄准的 (yaw, pitch)（眼睛高度近似平射）。
/// kind 匹配参考 Attack 分支：nearest = 任意非玩家生物。返回 None 表示没有可瞄准目标。
async fn look_at_nearest_entity(bot: &Client, target: &str) -> Option<(f32, f32)> {
    let Ok(entities) =
        bot.nearest_entities::<bevy_ecs::query::Without<azalea::entity::metadata::Player>>()
    else {
        return None;
    };
    let self_id = bot.entity().id();
    let requested = normalize_entity_target(target);
    let bot_pos = bot.position().ok()?;
    for e in entities.iter() {
        if e.id() == self_id {
            continue;
        }
        let Ok(kind) = e.kind() else {
            continue;
        };
        let kind = entity_kind_name(kind);
        if requested != "nearest" && kind != requested {
            continue;
        }
        if matches!(
            kind.as_str(),
            "item" | "experience_orb" | "item_frame" | "glow_item_frame"
        ) {
            continue;
        }
        let Ok(pos) = e.position() else {
            continue;
        };
        let dx = pos.x - bot_pos.x;
        let dz = pos.z - bot_pos.z;
        let dy = pos.y - bot_pos.y;
        let horiz = (dx * dx + dz * dz).sqrt();
        if horiz < 0.001 {
            continue;
        }
        let yaw = (-dx).atan2(dz).to_degrees();
        let pitch = (-dy).atan2(horiz).to_degrees();
        return Some((yaw as f32, pitch as f32));
    }
    None
}

impl AzaleaBot {
    /// azalea handler：所有 bot 逻辑在此执行（fn 指针，不捕获外部变量）。
    /// 命令从 `state.cmd_queue` 取出执行，事件经 `state.evt_tx` 转发外部。
    pub(crate) async fn handle(bot: Client, event: Event, state: BotState) -> Client {
        let cmd_queue = state.cmd_queue.clone();
        let evt_tx = state.evt_tx.clone();
        let lp = state.last_position.clone();
        match event {
            Event::Spawn => {
                if let Ok(p) = bot.position() {
                    *lp.lock().unwrap() = Some(p);
                    let _ = evt_tx.send(BotEvent::Spawn { position: p });
                }
            }
            Event::Chat(packet) => {
                // M5 修复：用 content() 方法获取纯文本字符串，而非 Debug 格式化。
                // 旧实现 format!("{:?}", p.content) 产出 "TextComponent { text: \"goto 10 64 10\", ... }"
                // 导致 strip_prefix("goto ") 等聊天命令解析全部失效。
                let content = packet.content();
                // 聊天驱动的即时指令（便于实机调试 / 玩家直接指挥 bot）：
                //   craft <物品> [数量]        2×2 背包合成
                //   craft3 <物品> [数量]       3×3 工作台合成（需已开工作台）
                //   smelt <产物> <燃料> [数量] 熔炼（需已开熔炉）
                //   gather <方块> [数量]       走到最近该方块并挖掘（如 gather oak_log 4）
                //   place <物品> <x> <y> <z>  把手持物品放到坐标旁（如 place crafting_table 10 64 10）
                //   open <x> <y> <z>          打开该坐标的容器（工作台/熔炉）
                //   autocraft <物品> [数量]   高层自动合成（木链，如 autocraft chest 1）
                //   enchant <物品> [等级]     附魔（需已开附魔台且背包有 item 与青金石，如 enchant iron_sword 2）
                //   goto <x> <y> <z> / mine <x> <y> <z> / minebelow / attack
                if let Some(cmd) = parse_chat_command(&content) {
                    let mut q = cmd_queue.lock().unwrap();
                    q.push(QueuedCommand {
                        cmd,
                        result_tx: None,
                    });
                }
                let _ = evt_tx.send(BotEvent::Chat { content });
            }
            Event::Disconnect(reason) => {
                let _ = evt_tx.send(BotEvent::Disconnect {
                    reason: format!("{reason:?}"),
                });
            }
            Event::Tick => {
                if let Ok(p) = bot.position() {
                    *lp.lock().unwrap() = Some(p);
                    // P67：全局"原地冻死"看门狗。每 tick 比对上次记录位置，
                    // 若净移动 <1 格则累加 no_move_ticks，否则清零。
                    // 累计达 400 tick(20s) 且循环仍活跃（有 pending 或队列非空）→
                    // 向 LLM 推强警告，逼其换策略（覆盖 goto 之外的所有卡死：空转脚本/无效 interact 等）。
                    {
                        let cur = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
                        let mut last = state.last_seen_pos.lock().unwrap();
                        let moved = (cur.0 - last.0).abs() > 1
                            || (cur.1 - last.1).abs() > 1
                            || (cur.2 - last.2).abs() > 1;
                        if moved {
                            *last = cur;
                            *state.no_move_ticks.lock().unwrap() = 0;
                        } else {
                            *state.no_move_ticks.lock().unwrap() += 1;
                        }
                        let nmt = *state.no_move_ticks.lock().unwrap();
                        if nmt == 400 {
                            let queue_len = state.cmd_queue.lock().unwrap().len();
                            let pending = state.action_mgr.is_idle();
                            if queue_len > 0 || !pending {
                                let _ = evt_tx.send(BotEvent::Chat {
                                    content: "【原地冻死警告】你已连续 20 秒几乎没移动，但仍在发指令——这说明卡在某个无效动作（如空转脚本、对空气 interact、反复同动作）。请立即换策略：(1) 若目标是挖矿，用 mine_below/mine_above 真正向下/向上挖；(2) 若被挡，用 mine 挖开阻挡方块；(3) 不要重复调用同一个无效工具。先 perceive 看真实状态。".to_string(),
                                });
                            }
                        }
                    }
                }
                // P68：跟随模式（每 10 tick 推进一次）。读取目标玩家坐标并 goto，
                // 实现"跟着我"。仅在当前无 pending 命令（避免打断采矿/合成等）时生效。
                {
                    let follow = state.follow_target.lock().unwrap().clone();
                    if let Some(target) = follow {
                        let tick_now = bot.ticks_connected();
                        if tick_now.is_multiple_of(10) && state.action_mgr.is_idle() {
                            let players = bot.nearby_players();
                            if let Ok(players) = players {
                                let mut chosen: Option<(f64, f64, f64, String)> = None;
                                for p in players.iter() {
                                    let uname = p
                                        .component::<GameProfileComponent>()
                                        .map(|g| g.0.name.clone())
                                        .unwrap_or_default();
                                    if let Some(t) = &target
                                        && &uname != t
                                    {
                                        continue;
                                    }
                                    if let Ok(pos) = p.position() {
                                        chosen = Some((pos.x, pos.y, pos.z, uname));
                                        if target.is_some() {
                                            break;
                                        }
                                    }
                                }
                                if let Some((px, py, pz, _uname)) = chosen {
                                    // 跟随时走到玩家脚下（略低于玩家，避免卡进身体）。
                                    let navigation = bot.goto(BlockPosGoal(BlockPos::new(
                                        px.floor() as i32,
                                        py.floor() as i32,
                                        pz.floor() as i32,
                                    )));
                                    if tokio::time::timeout(Duration::from_secs(5), navigation)
                                        .await
                                        .is_err()
                                    {
                                        bot.force_stop_pathfinding();
                                    }
                                } else {
                                    // 目标玩家不在附近：解除跟随并提示。
                                    *state.follow_target.lock().unwrap() = None;
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!(
                                            "[跟随] 找不到玩家 {}，已自动停止跟随。",
                                            target
                                                .clone()
                                                .unwrap_or_else(|| "最近的玩家".to_string())
                                        ),
                                    });
                                }
                            }
                        }
                    }
                }
                // 串行消费命令队列：每 tick 最多推进「一条」命令，等它完成才取下一条。
                // ActionManager 管理单槽 pending + 按命令类型超时 + 抢占 + 快循环检测。
                {
                    let tick_now = bot.ticks_connected();
                    // 推进队列：pending 空时从队列 pop 一条
                    if state.action_mgr.is_idle() {
                        let next = {
                            let mut q = cmd_queue.lock().unwrap();
                            // FIFO：取最早入队的命令。Vec::pop 取最后元素是 LIFO，
                            // 这里用 remove(0) 实现真 FIFO（队列长度通常 <5，O(n) 可忽略）。
                            if q.is_empty() {
                                None
                            } else {
                                Some(q.remove(0))
                            }
                        };
                        if let Some(qc) = next {
                            state.action_mgr.occupy(qc, tick_now);
                        }
                    }
                    // 轮询非阻塞命令（Goto/Mine）完成状态 + 按命令类型超时
                    if let Some(qc) = state.action_mgr.peek_pending() {
                        let done = match &qc.cmd {
                            BotCommand::Mine { x: _, y: _, z: _ } => {
                                // P101：done 判定必须用派发时确定的实际挖掘目标
                                // （可能是修正后的实心方块）。若仍按原目标 (x,y,z) 判
                                // 空气——原目标本就是空气时 done 立即成立，修正挖掘
                                // 在下一 tick 就被终结（实测 dirt 未被挖掉）。
                                // 若 last_mine_eff 为 None（命令刚入队、派发前抢占的那
                                // 一帧），不视为完成，避免 dispatch 前就误报 done。
                                let eff = *state.last_mine_eff.lock().unwrap();
                                if let Some((eff_target, _)) = eff {
                                    let (ex, ey, ez) = (eff_target.x, eff_target.y, eff_target.z);
                                    if let Ok(world) = bot.world() {
                                        let s =
                                            world.read().get_block_state(BlockPos::new(ex, ey, ez));
                                        let is_air =
                                            s.is_none() || s.map(|b| b.is_air()).unwrap_or(false);
                                        // P4 修复：start_mining 只在命令派发时调一次，但挖掘可能被
                                        // 重力/移动/伤害中断后不再恢复。这里每 20 tick 重新发起挖掘，
                                        // 确保方块还在就持续挖（对齐 MineBelow 的持续触发逻辑）。
                                        if !is_air
                                            && !bot.is_mining()
                                            && tick_now.is_multiple_of(20)
                                        {
                                            bot.start_mining(BlockPos::new(ex, ey, ez));
                                        }
                                        is_air
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            }
                            BotCommand::Goto { x, y, z } => {
                                if let Ok(p) = bot.position() {
                                    let d = ((p.x - *x as f64).powi(2)
                                        + (p.y - *y as f64).powi(2)
                                        + (p.z - *z as f64).powi(2))
                                    .sqrt();
                                    // P67：到达判定放宽 1.5→2.5m。probe 实测：bot 常停在目标 1.5-2.5m
                                    // 处（pathfinder 已认为到达）而 done 永不触发 → 空等 60s 超时，
                                    // LLM 误判"路径被阻"反复重试。2.5m 内即算到达。
                                    d < 2.5
                                } else {
                                    false
                                }
                            }
                            BotCommand::MineBelow => false,
                            BotCommand::MakeObsidian { .. } => false,
                            BotCommand::MineAbove => {
                                let start_y = *state.mining_above_start_y.lock().unwrap();
                                bot.position()
                                    .ok()
                                    .zip(start_y)
                                    .is_some_and(|(position, start)| {
                                        position.y.floor() as i32 > start
                                    })
                            }
                            // 非轮询命令（Equip/Craft/Gather/Place/...）由下方执行块处理，
                            // 这里不能标记 done=true——否则会在执行前就清空 pending，
                            // 导致 do_equip/do_craft 等从未运行（bug 表现：equip 返回"命令完成"但主手没变）。
                            _ => false,
                        };
                        // 按命令类型超时（取代原硬编码 60 tick）
                        let timed_out_cmd = state.action_mgr.check_timeout(tick_now);
                        let timed_out = timed_out_cmd.is_some();
                        // P162b（2026-08-15）：goto 执行中卡住检测。
                        // 根因：azalea pathfinder 对复杂地形（lush_caves）算出的路径是
                        // "incomplete path"（is_path_partial=true），execute 的
                        // recalculate_near_end_of_path 会在路径 <5 节点时反复重算 →
                        // GotoEvent 无限循环，bot 原地不动直到超时（probe 实测每 60ms
                        // "got goto" 重算一次、位置完全不变）。
                        // 修复：goto 未完成且距目标 >2.5m 时，跟踪 bot 净移动；连续
                        // 100 tick（5s）净移动 <1 格即判定 pathfinder 重算循环，立即
                        // 强制失败 + 触发 P66 脱困（地下 mine_above / 地表挖障碍），
                        // 而不是让 LLM 空等 20s 超时。
                        let mut goto_stuck_now: Option<(i32, i32, i32)> = None;
                        if !done
                            && !timed_out
                            && matches!(&qc.cmd, BotCommand::Goto { .. })
                            && let BotCommand::Goto { x, y, z } = &qc.cmd
                            && let Ok(p) = bot.position()
                        {
                            let d = ((p.x - *x as f64).powi(2)
                                + (p.y - *y as f64).powi(2)
                                + (p.z - *z as f64).powi(2))
                            .sqrt();
                            if d >= 2.5 {
                                let mut sg = state.goto_stuck.lock().unwrap();
                                let cur = (p.x, p.y, p.z);
                                let moved = sg.0.is_none_or(|(lx, ly, lz)| {
                                    (cur.0 - lx).abs() > 1.0
                                        || (cur.1 - ly).abs() > 1.0
                                        || (cur.2 - lz).abs() > 1.0
                                });
                                if moved {
                                    *sg = (Some(cur), 0);
                                } else {
                                    sg.1 += 1;
                                    if sg.1 >= 100 {
                                        // 5s 无移动且距目标 >2.5m → pathfinder 重算循环
                                        *sg = (None, 0);
                                        drop(sg);
                                        goto_stuck_now = Some((*x, *y, *z));
                                    }
                                }
                            }
                        }
                        if let Some((gx, gy, gz)) = goto_stuck_now {
                            if let Some(tx) = &qc.result_tx {
                                let _ = tx.send(format!(
                                    "Action output:\ngoto ({},{},{}) 执行中 5s 无移动——pathfinder 陷入不完整路径重算循环（复杂地形）。\
                                     已自动脱困：{}。请换策略：1) 若目标在附近，用 mine 挖开挡路方块再走；\
                                     2) 或 mine_above 上到地表开阔处再 goto；3) 不要重复 goto 同一片区域。",
                                    gx, gy, gz,
                                    if bot.position().map_or(true, |p| (p.y.floor() as i32) < 62) {
                                        "地下已转 mine_above 向上挖出"
                                    } else {
                                        "已尝试挖开周围阻挡方块"
                                    }
                                ));
                            }
                            // 触发与 P66 相同的脱困
                            if let Ok(p) = bot.position() {
                                if (p.y.floor() as i32) < 62 {
                                    *state.mining_above.lock().unwrap() = true;
                                    *state.mining_above_start_y.lock().unwrap() =
                                        Some(p.y.floor() as i32);
                                    *state.mining_above_direction.lock().unwrap() = 0;
                                } else if let Ok(world) = bot.world() {
                                    let world = world.read();
                                    for (bx, by, bz) in
                                        [(gx, gy, gz), (gx, gy - 1, gz), (gx, gy + 1, gz)]
                                    {
                                        if let Some(bs) =
                                            world.get_block_state(BlockPos::new(bx, by, bz))
                                            && !bs.is_air()
                                        {
                                            bot.start_mining(BlockPos::new(bx, by, bz));
                                        }
                                    }
                                }
                            }
                            bot.force_stop_pathfinding();
                            if let Ok(cp) = bot.position() {
                                let _ = state.goto_cooldown.lock().unwrap().insert(
                                    (
                                        cp.x.floor() as i32,
                                        cp.y.floor() as i32,
                                        cp.z.floor() as i32,
                                    ),
                                    bot.ticks_connected() + 300,
                                );
                            }
                            state.action_mgr.clear_pending();
                        }
                        // P65：goto 伪到达看门狗。当 goto 目标其实是脚下实心方块，
                        // bot 原地判"到达"(distance<1.5) 却从未真正移动 → 反复重发相同 goto 死循环。
                        // 检测：同一目标"done"了 2 次但 bot 实际位置(从 last_position)未变 → 强制 mine_above 脱困。
                        let mut unstick_now = false;
                        if done
                            && matches!(&qc.cmd, BotCommand::Goto { .. })
                            && let BotCommand::Goto { x, y, z } = &qc.cmd
                        {
                            let mut wd = state.goto_watchdog.lock().unwrap();
                            let moved = state.last_position.lock().unwrap().is_none_or(|lp| {
                                (lp.x - *x as f64).abs() > 1.0
                                    || (lp.y - *y as f64).abs() > 1.0
                                    || (lp.z - *z as f64).abs() > 1.0
                            });
                            if !moved && *x == wd.0 && *y == wd.1 && *z == wd.2 {
                                wd.3 += 1;
                            } else {
                                *wd = (*x, *y, *z, 0);
                            }
                            if wd.3 >= 2 {
                                *wd = (0, 0, 0, 0);
                                // 地下 → 自动转 mine_above；地表 → 也强制上挖一层绕开实心目标
                                if bot.position().map_or(true, |p| (p.y.floor() as i32) < 62) {
                                    *state.mining_above.lock().unwrap() = true;
                                    *state.mining_above_start_y.lock().unwrap() =
                                        Some(bot.position().map_or(0, |p| p.y.floor() as i32));
                                    *state.mining_above_direction.lock().unwrap() = 0;
                                    bot.force_stop_pathfinding();
                                    if let Some(tx) = &qc.result_tx {
                                        let _ = tx.send(
                                                "Action output:\ngoto 反复'到达'但 bot 未移动（目标可能是脚下实心方块）。已自动转 mine_above 向上挖出脱困。".to_string(),
                                            );
                                    }
                                    state.action_mgr.clear_pending();
                                    unstick_now = true;
                                }
                            }
                        }
                        if unstick_now {
                            // 已自行处理：强制脱困并清空 pending，跳过下方 result_msg 生成。
                        } else if done || timed_out {
                            // 统一用 Mindcraft 风格 "Action output:\n..." 让 LLM 看到一致的反馈。
                            let result_msg = match &qc.cmd {
                                BotCommand::Goto { x, y, z } if done => {
                                    let (cx, cy, cz) = bot
                                        .position()
                                        .ok()
                                        .map(|p| (p.x, p.y, p.z))
                                        .unwrap_or((0.0, 0.0, 0.0));
                                    let dist = ((cx - *x as f64).powi(2)
                                        + (cy - *y as f64).powi(2)
                                        + (cz - *z as f64).powi(2))
                                    .sqrt();
                                    format!(
                                        "Action output:\nArrived at ({},{},{}). Distance traveled: {:.1}m. Current pos: ({:.0},{:.0},{:.0}).",
                                        x, y, z, dist, cx, cy, cz
                                    )
                                }
                                BotCommand::Goto { x, y, z } => {
                                    // P66 修复：bot 反复 goto 相邻空气块却都 empty path 超时——
                                    // 无论目标坐标怎么变（LLM 每次微调），本质都是"原地导航失败"。
                                    // 改用"净移动"判定：连续 goto 超时且 bot 净移动 <1.5 格即累计 stall，
                                    // 达 3 次强制脱困 + 冷却当前格子，彻底打破 goto 洪泛（pi-agent 自主止损）。
                                    let mut wd = state.goto_watchdog.lock().unwrap();
                                    let (lx, ly, lz, _stall) = *wd;
                                    let cur = bot.position().ok().map(|p| {
                                        (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32)
                                    });
                                    let moved = cur.is_none_or(|(cx, cy, cz)| {
                                        ((cx - lx).abs() as f64 > 1.5)
                                            || ((cy - ly).abs() as f64 > 1.5)
                                            || ((cz - lz).abs() as f64 > 1.5)
                                    });
                                    if moved {
                                        *wd = (
                                            cur.unwrap_or((0, 0, 0)).0,
                                            cur.unwrap_or((0, 0, 0)).1,
                                            cur.unwrap_or((0, 0, 0)).2,
                                            0,
                                        );
                                    } else {
                                        wd.3 += 1;
                                    }
                                    let stall_count = wd.3;
                                    drop(wd);
                                    if stall_count >= 3 {
                                        // 重置并冷却当前格子 15s（300 tick）：期间任何 goto 直接拒绝。
                                        *state.goto_watchdog.lock().unwrap() = (0, 0, 0, 0);
                                        if let Some((cx, cy, cz)) = cur {
                                            state
                                                .goto_cooldown
                                                .lock()
                                                .unwrap()
                                                .insert((cx, cy, cz), bot.ticks_connected() + 300);
                                        }
                                        // 脱困：地下→mine_above；地表→挖开目标阻挡方块（若 solid）或向上挖一层
                                        if let Ok(p) = bot.position() {
                                            if (p.y.floor() as i32) < 62 {
                                                *state.mining_above.lock().unwrap() = true;
                                                *state.mining_above_start_y.lock().unwrap() =
                                                    Some(p.y.floor() as i32);
                                                *state.mining_above_direction.lock().unwrap() = 0;
                                            } else if let Ok(world) = bot.world() {
                                                let world = world.read();
                                                // 挖开目标方块（若非空气）和脚下/身旁可能阻挡的方块
                                                for (bx, by, bz) in [
                                                    (*x, *y, *z),
                                                    (*x, *y - 1, *z),
                                                    (*x, *y + 1, *z),
                                                ] {
                                                    if let Some(bs) = world
                                                        .get_block_state(BlockPos::new(bx, by, bz))
                                                        && !bs.is_air()
                                                    {
                                                        bot.start_mining(BlockPos::new(bx, by, bz));
                                                    }
                                                }
                                            }
                                        }
                                        bot.force_stop_pathfinding();
                                        format!(
                                            "Action output:\ngoto 已连续 {} 次超时且你几乎没移动——这是导航死循环！已强制停止并冷却当前位置 15s。\
                                             请：1) perceive 看清四周，用 mine 挖开挡路的实心方块再走；\
                                             2) 或 mine_above 上到地表开阔处再 goto；3) 不要重复 goto 旁边同一片区域。",
                                            stall_count
                                        )
                                    } else if let Ok(p) = bot.position() {
                                        if (p.y.floor() as i32) < 62 {
                                            *state.mining_above.lock().unwrap() = true;
                                            *state.mining_above_start_y.lock().unwrap() =
                                                Some(p.y.floor() as i32);
                                            *state.mining_above_direction.lock().unwrap() = 0;
                                            bot.force_stop_pathfinding();
                                            format!(
                                                "Action output:\ngoto ({},{},{}) 超时——bot 在地下口袋里被挡住（Y={:.0}）。已自动转为 mine_above 向上挖出脱困，到地表后请用 goto 重试目标。",
                                                x, y, z, p.y
                                            )
                                        } else {
                                            // P69a：goto 超时（地表）自动清障——树冠/密林/山体地形下
                                            // pathfinder 找不到路（empty path），LLM 换坐标也白搭。
                                            // 挖开 bot 周围挡路的实心方块（树干/树叶/石头），每格让
                                            // pathfinder 多一条路。黑名单保护容器/工作台等设施不挖。
                                            // 借鉴 Mineflayer pathfinder 的 dig 模式。
                                            let mut cleared = 0u32;
                                            if let (Ok(bp), Ok(world)) =
                                                (bot.position(), bot.world())
                                            {
                                                let bx = bp.x.floor() as i32;
                                                let by = bp.y.floor() as i32;
                                                let bz = bp.z.floor() as i32;
                                                let no_dig = |bk: &BlockKind| {
                                                    matches!(
                                                        bk,
                                                        BlockKind::Chest
                                                            | BlockKind::CraftingTable
                                                            | BlockKind::Furnace
                                                            | BlockKind::BlastFurnace
                                                            | BlockKind::Smoker
                                                            | BlockKind::Barrel
                                                            | BlockKind::Anvil
                                                            | BlockKind::EnchantingTable
                                                            | BlockKind::BrewingStand
                                                            | BlockKind::Bedrock
                                                    )
                                                };
                                                for (dx, dz) in [
                                                    (0, 1),
                                                    (0, -1),
                                                    (1, 0),
                                                    (-1, 0),
                                                    (1, 1),
                                                    (1, -1),
                                                    (-1, 1),
                                                    (-1, -1),
                                                ] {
                                                    if cleared >= 3 {
                                                        break;
                                                    }
                                                    for dy in [0i32, 1] {
                                                        let pos = BlockPos::new(
                                                            bx + dx,
                                                            by + dy,
                                                            bz + dz,
                                                        );
                                                        let solid = world
                                                            .read()
                                                            .get_block_state(pos)
                                                            .map(|b| !b.is_air())
                                                            .unwrap_or(false);
                                                        if solid {
                                                            let bk: BlockKind = world
                                                                .read()
                                                                .get_block_state(pos)
                                                                .unwrap()
                                                                .into();
                                                            if no_dig(&bk) {
                                                                break;
                                                            }
                                                            bot.start_mining(pos);
                                                            cleared += 1;
                                                            break;
                                                        }
                                                    }
                                                }
                                            }
                                            if cleared > 0 {
                                                format!(
                                                    "Action output:\ngoto ({},{},{}) 超时——路径被阻（地表）。已自动挖开 {} 个挡路方块开道，稍后请重试 goto 同一目标。",
                                                    x, y, z, cleared
                                                )
                                            } else {
                                                format!(
                                                    "Action output:\ngoto ({},{},{}) 超时——路径被阻或目标不可达（地表）。perceive 确认位置后改用更近的中间点重试（已第 {} 次净不动，连 3 次将强制停止）。",
                                                    x, y, z, stall_count
                                                )
                                            }
                                        }
                                    } else {
                                        format!(
                                            "Action output:\ngoto ({},{},{}) 超时——路径被阻或目标不可达。",
                                            x, y, z
                                        )
                                    }
                                }
                                BotCommand::Mine { x, y, z } if done => {
                                    let (cx, cy, cz) = bot
                                        .position()
                                        .ok()
                                        .map(|p| (p.x, p.y, p.z))
                                        .unwrap_or((0.0, 0.0, 0.0));
                                    // P101：done 由派发时的实际挖掘目标触发，这里取回该目标
                                    // 与原目标是否空气的记录，区分三种反馈：
                                    //   1) 原目标实心且已挖掉 → "Mined block at"（成功）
                                    //   2) 原目标空气但修正挖掉了实心方块 → 报修正成功
                                    //   3) 原目标空气且无实心可修正（no-op）→ P57 空气错误 + 建议
                                    // 旧逻辑（P57）只看 done 时原目标是否空气：挖掘成功后目标
                                    // 当然是空气，却报"该位置已是空气"——LLM 反复 mine 同一格。
                                    // P155：mine 完成/超时/取消时清空靠近看门狗，避免残留状态
                                    // 影响下一个 mine 命令。
                                    *state.mine_approach_watchdog.lock().unwrap() = None;
                                    let mine_eff = state.last_mine_eff.lock().unwrap().take();
                                    let (ex, ey, ez) = mine_eff
                                        .map(|(p, _)| (p.x, p.y, p.z))
                                        .unwrap_or((*x, *y, *z));
                                    let original_was_air =
                                        mine_eff.map(|(_, air)| air).unwrap_or(true);
                                    if original_was_air {
                                        if (ex, ey, ez) == (*x, *y, *z) {
                                            // 场景 3：原目标空气且无实心可修正（或修正失败）→ P57 建议
                                            let mut suggestions: Vec<(i32, i32, i32)> = Vec::new();
                                            if let Ok(world) = bot.world() {
                                                'outer: for d in 1i32..=4 {
                                                    for dx in -d..=d {
                                                        for dz in -d..=d {
                                                            for dy in -1..=2 {
                                                                if dx.abs() != d
                                                                    && dz.abs() != d
                                                                    && dy != -1
                                                                    && dy != 2
                                                                {
                                                                    continue;
                                                                }
                                                                let pos = BlockPos::new(
                                                                    x + dx,
                                                                    y + dy,
                                                                    z + dz,
                                                                );
                                                                let bk: Option<BlockKind> = world
                                                                    .read()
                                                                    .get_block_state(pos)
                                                                    .map(|b| b.into());
                                                                let solid = bk
                                                                    .map(|k| {
                                                                        k != BlockKind::Air
                                                                            && k != BlockKind::Water
                                                                            && k != BlockKind::Lava
                                                                    })
                                                                    .unwrap_or(false);
                                                                if solid {
                                                                    suggestions.push((
                                                                        x + dx,
                                                                        y + dy,
                                                                        z + dz,
                                                                    ));
                                                                    if suggestions.len() >= 4 {
                                                                        break 'outer;
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            let hint = if suggestions.is_empty() {
                                                "附近 4 格内无实心方块，请先 perceive 确认位置。"
                                                    .to_string()
                                            } else {
                                                format!(
                                                    "附近最近的实心方块（可挖）：{}。",
                                                    suggestions
                                                        .iter()
                                                        .map(|(sx, sy, sz)| format!(
                                                            "({sx},{sy},{sz})"
                                                        ))
                                                        .collect::<Vec<_>>()
                                                        .join(", ")
                                                )
                                            };
                                            format!(
                                                "Action output:\nmine ({},{},{}): 该位置已是空气/方块不存在（可能之前已挖掉或坐标错误）。{hint}\
                                                 直接 mine 上述坐标即可。",
                                                x, y, z
                                            )
                                        } else {
                                            // 场景 2：修正目标已被挖掉 → 报修正成功
                                            format!(
                                                "Action output:\n目标 ({},{},{}) 是空气，已自动修正挖掘最近实心方块 ({},{},{}) 并成功移除。",
                                                x, y, z, ex, ey, ez
                                            )
                                        }
                                    } else {
                                        // 场景 1：原目标实心、现已挖掉 → 正常成功
                                        format!(
                                            "Action output:\nMined block at ({},{},{}). Block removed. Bot still at ({:.0},{:.0},{:.0}) — 挖完不会自动掉进洞，无需 goto 刚挖的位置。",
                                            x, y, z, cx, cy, cz
                                        )
                                    }
                                }
                                BotCommand::Mine { x, y, z } => {
                                    // P101：命令结束（超时/取消路径）必须清空实际目标记录，
                                    // 否则残留状态会让下一个 mine 命令的 done 判定错位。
                                    // P155：同时清空靠近看门狗。
                                    *state.last_mine_eff.lock().unwrap() = None;
                                    *state.mine_approach_watchdog.lock().unwrap() = None;
                                    format!(
                                        "Action output:\nmine ({},{},{}) 超时——可能方块太硬（需更高品质镐）或距离太远。建议 gather(item=..., count=...) 自动寻路挖掘。",
                                        x, y, z
                                    )
                                }
                                BotCommand::MineAbove if done => {
                                    let y = bot
                                        .position()
                                        .ok()
                                        .map(|position| position.y.floor() as i32)
                                        .unwrap_or_default();
                                    format!(
                                        "Action output:\nMineAbove progressed to Y={y}. Call mine_above again to continue toward the surface."
                                    )
                                }
                                BotCommand::MineAbove => {
                                    *state.mining_above.lock().unwrap() = false;
                                    *state.mining_above_start_y.lock().unwrap() = None;
                                    bot.force_stop_pathfinding();
                                    // P120c：徒手硬挖超时（30s Y 未上升）说明头顶天花板太厚
                                    // 或通道被堵——同一位置死磕只会重复超时。若背包无镐，
                                    // 自动横移一格换位置（找软土柱/洞穴通道），不再原地死磕。
                                    let side_move = if !has_any_pickaxe_in_inventory(&bot).await
                                        && let Ok(p) = bot.position()
                                    {
                                        let (cx, cy, cz) = (
                                            p.x.floor() as i32,
                                            p.y.floor() as i32,
                                            p.z.floor() as i32,
                                        );
                                        if nearest_soft_column(&bot, cx, cy, cz, 4).is_none() {
                                            let mut direction =
                                                state.mining_above_direction.lock().unwrap();
                                            let directions = [(1, 0), (0, 1), (-1, 0), (0, -1)];
                                            let (dx, dz) = directions[(*direction % 4 + 4) % 4];
                                            *direction = (*direction + 1) % 4;
                                            drop(direction);
                                            let target = BlockPos::new(cx + dx, cy, cz + dz);
                                            bot.start_goto(BlockPosGoal(target));
                                            Some((cx + dx, cz + dz))
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    };
                                    match side_move {
                                        Some((nx, nz)) => format!(
                                            "Action output:\nmine_above 超时（30s）：Y 未上升，上升路径被挡或徒手挖太慢（~8秒/格）。\
                                             已自动横移到 ({nx},?,{nz}) 换位置找软土柱/洞穴通道，到达后请重试 mine_above。\
                                             若有镐可先 equip 再挖。\
                                             \nP125 提示：若背包有方块（如 cobblestone），用 place 垫方块逐格上跳（pillar 脱离）\
                                             比徒手凿岩壁快得多——原地放置、跳上、再放置，直出地表找树做镐。"
                                        ),
                                        None => "Action output:\nmine_above 超时（30s）：Y 未上升，上升路径被挡或徒手挖太慢（~8秒/格）。\
                                             建议：(1) 若有镐，先 equip 再重试；(2) 用 mine 横向挖楼梯/找软方块通道；\
                                             (3) P125：若背包有方块（如 cobblestone），用 place 垫方块逐格上跳\
                                             （pillar-up）快速脱离——无镐凿岩壁 8s/格，pillar 只需 1 格方块/跳。"
                                            .to_string(),
                                    }
                                }
                                BotCommand::Gather { item, .. } => {
                                    // P3：gather 超时时，采集 future 仍在后台运行（无法取消），
                                    // 实际可能已经/即将完成。让 LLM 用 perceive 确认背包，
                                    // 而不是直接重调 gather（会重复采集）。
                                    format!(
                                        "Action output:\ngather {item} 超时（ActionManager 120s 阈值）。\
                                    采集可能仍在后台进行——下一步请先 perceive 检查背包 {item} 数量，\
                                    若已满足需求就不要重调 gather；若确实不够，再 gather 补足差额。"
                                    )
                                }
                                BotCommand::Craft3x3 { item, .. } => {
                                    format!(
                                        "Action output:\ncraft_3x3 {item} 超时——可能工作台路径卡住或合成 UI 响应慢。\
                                    建议 perceive 确认背包是否有 {item}，若无再重试。"
                                    )
                                }
                                BotCommand::Smelt { output, .. } => {
                                    format!(
                                        "Action output:\nsmelt {output} 超时——熔炼本质慢。\
                                    建议 perceive 确认背包是否有 {output}，若无再重试。"
                                    )
                                }
                                _ if done => "Action output:\n命令完成".to_string(),
                                _ => "Action output:\n命令超时".to_string(),
                            };
                            if let Some(tx) = &qc.result_tx {
                                let _ = tx.send(result_msg);
                            }
                            // P148：mine 成功挖掉矿石后自动拾取掉落物（钻石/铁等掉进缝隙
                            // 后 item_collecting 每 200 tick 才兜底一次，且下挖时被跳过——
                            // 挖完立即入队 Pickup，确保矿石掉落物当次入包）。
                            // P151 结论：掉落物缺失根因是服务器 block_drops=false（MC 26.2
                            // gamerule 改名，doTileDrops→block_drops），非 P148 移动干扰；
                            // 已用 /gamerule block_drops true 开启。P148 保持启用。
                            if done && matches!(&qc.cmd, BotCommand::Mine { .. }) {
                                let tx_clone = qc.result_tx.clone();
                                cmd_queue.lock().unwrap().push(QueuedCommand {
                                    cmd: BotCommand::Pickup,
                                    result_tx: tx_clone,
                                });
                            }
                            if matches!(&qc.cmd, BotCommand::MineAbove) {
                                *state.mining_above_start_y.lock().unwrap() = None;
                                *state.mining_above_no_pick_warned.lock().unwrap() = false;
                                *state.mining_above_soft_column.lock().unwrap() = None;
                            }
                            state.action_mgr.clear_pending();
                        }
                    }
                    // 取走 ActionManager 的快循环警告（若有则推到事件流供 Agent 注入）
                    if let Some(nudge) = state.action_mgr.take_loop_nudge() {
                        let _ = evt_tx.send(BotEvent::Chat { content: nudge });
                    }
                }
                // P95：取消请求处理——外部 cancel_commands 置位后执行真正的中止。
                if state.cancel_flag.swap(false, Ordering::SeqCst) {
                    // 持续挖矿标志复位，防取消后仍自动下挖/上挖
                    *state.mining_below.lock().unwrap() = false;
                    *state.mining_above.lock().unwrap() = false;
                    *state.mining_above_no_pick_warned.lock().unwrap() = false;
                    *state.mining_above_soft_column.lock().unwrap() = None;
                    if !state.action_mgr.is_busy() {
                        // 非异步执行中：强停寻路 + 清槽 + 回复取消
                        if let Some(qc) = state.action_mgr.peek_pending() {
                            if matches!(
                                &qc.cmd,
                                BotCommand::Goto { .. }
                                    | BotCommand::Mine { .. }
                                    | BotCommand::MineBelow
                                    | BotCommand::MineAbove
                            ) {
                                bot.force_stop_pathfinding();
                                // P101：取消 mine 时清空实际目标记录，防止残留污染下一命令判定。
                                if matches!(&qc.cmd, BotCommand::Mine { .. }) {
                                    *state.last_mine_eff.lock().unwrap() = None;
                                }
                            }
                            if let Some(tx) = &qc.result_tx {
                                let _ = tx.send("已取消（cancel_commands）".to_string());
                            }
                        }
                        state.action_mgr.clear_pending();
                    }
                    // busy=true：异步命令执行中，不中断执行体（世界状态半途不可恢复），
                    // 等其自然完成；队列已空，完成后自然停止。
                }
                // 取当前要执行的命令：pending 里的命令每 tick 都（重）执行其 start，
                // 非阻塞命令（Goto/Mine）重复 start 是幂等的（重设同一目标），由
                // cmd_finished 轮询完成；MineBelow 在 arm 内清空中途槽。
                // 异步命令（Craft/Gather 等）执行期间 busy=true，下一 tick 跳过避免重入。
                let to_run: Option<(BotCommand, Option<std::sync::mpsc::Sender<String>>)> = {
                    if state.action_mgr.is_busy() {
                        None
                    } else if let Some(qc) = state.action_mgr.peek_pending() {
                        let is_polling = matches!(
                            &qc.cmd,
                            BotCommand::Goto { .. }
                                | BotCommand::Mine { .. }
                                | BotCommand::MineBelow
                                | BotCommand::MineAbove
                        );
                        if !is_polling {
                            state.action_mgr.set_busy(true);
                        }
                        Some((qc.cmd.clone(), qc.result_tx.clone()))
                    } else {
                        None
                    }
                };
                if let Some((cmd, result_tx)) = to_run {
                    match cmd {
                        BotCommand::RawState => {
                            // P88：原始数据 dump。与 State 快照渲染完全独立，逐槽/逐实体输出。
                            let mut out = String::new();
                            match bot.position() {
                                Ok(p) => out
                                    .push_str(&format!("pos=({:.3}, {:.3}, {:.3})", p.x, p.y, p.z)),
                                Err(e) => out.push_str(&format!("pos=ERR {e:?}")),
                            }
                            out.push_str(&format!(" health={}", bot.health().unwrap_or(-1.0)));
                            if let Ok(h) = bot.hunger() {
                                out.push_str(&format!(
                                    " food={} saturation={}",
                                    h.food, h.saturation
                                ));
                            }
                            if let Ok(xp) = bot.experience() {
                                out.push_str(&format!(
                                    " xp_level={} xp_progress={:.3}",
                                    xp.level, xp.progress
                                ));
                            }
                            out.push_str(&format!(
                                " dimension={}",
                                bot.world_name()
                                    .map(|n| n.to_string())
                                    .unwrap_or_else(|_| "unknown".into())
                            ));
                            if let Ok(p) = bot.position() {
                                let biome = bot
                                    .world()
                                    .ok()
                                    .and_then(|w| {
                                        w.read().get_biome(BlockPos::new(
                                            p.x.floor() as i32,
                                            p.y.floor() as i32,
                                            p.z.floor() as i32,
                                        ))
                                    })
                                    .and_then(|b| bot.resolve_registry_key(&b).ok().flatten())
                                    .map(|key| key.into_ident().to_string())
                                    .unwrap_or_else(|| "unknown".into());
                                out.push_str(&format!(" biome={biome}"));
                            }
                            if let Ok(d) = bot.direction() {
                                out.push_str(&format!(" dir={d:?}"));
                            }
                            match bot.get_held_item() {
                                Ok(it) if !it.is_empty() => out.push_str(&format!(
                                    " held={} x{}",
                                    it.kind().to_str(),
                                    it.count()
                                )),
                                _ => out.push_str(" held=air"),
                            }
                            out.push_str(&format!(
                                " selected_slot={}",
                                bot.selected_hotbar_slot().unwrap_or(0)
                            ));
                            out.push_str("\ninv:");
                            match bot.get_inventory() {
                                Ok(inv) => {
                                    if let Some(slots) = inv.slots() {
                                        for (i, s) in slots.iter().enumerate() {
                                            if !s.is_empty() {
                                                out.push_str(&format!(
                                                    " slot[{i}]={} x{}",
                                                    s.kind().to_str(),
                                                    s.count()
                                                ));
                                            }
                                        }
                                    }
                                }
                                Err(e) => out.push_str(&format!(" ERR {e:?}")),
                            }
                            match bot.nearest_entities::<()>() {
                                Ok(ents) => {
                                    let self_id = bot.entity().id();
                                    // P88-e：tab_list 查玩家名字，区分幽灵玩家/用户/其他 bot
                                    let tab = bot.tab_list().ok();
                                    out.push_str(&format!("\nents({}):", ents.len()));
                                    for e in ents.iter() {
                                        let kind = e
                                            .kind()
                                            .map(|k| format!("{k:?}"))
                                            .unwrap_or_else(|_| "?".into());
                                        let mut name = String::new();
                                        if kind == "Player"
                                            && let Ok(uuid) = e.uuid()
                                            && let Some(tab) = &tab
                                        {
                                            name = tab
                                                .get(&uuid)
                                                .map(|p| format!(" name={}", p.profile.name))
                                                .unwrap_or_default();
                                        }
                                        let pos = e
                                            .position()
                                            .map(|p| format!("({:.1},{:.1},{:.1})", p.x, p.y, p.z))
                                            .unwrap_or_else(|_| "?".into());
                                        let dist = e.distance_to_client().unwrap_or(-1.0);
                                        out.push_str(&format!(
                                            " id={} kind={kind}{name} pos={pos} dist={dist:.1}m{}",
                                            e.id(),
                                            if e.id() == self_id { "*self" } else { "" }
                                        ));
                                    }
                                }
                                Err(e) => out.push_str(&format!(" ERR {e:?}")),
                            }
                            if let (Ok(p), Ok(world)) = (bot.position(), bot.world()) {
                                let fx = p.x.floor() as i32;
                                let fy = (p.y - 1.0).floor() as i32;
                                let fz = p.z.floor() as i32;
                                out.push_str(&format!("\nfeet(y={fy}):"));
                                for dx in -1..=1 {
                                    for dz in -1..=1 {
                                        let bp = BlockPos::new(fx + dx, fy, fz + dz);
                                        let name = match world.read().get_block_state(bp) {
                                            Some(s) if s.is_air() => "air".to_string(),
                                            Some(s) => {
                                                let bk: BlockKind = s.into();
                                                bk.to_str().to_string()
                                            }
                                            None => "unloaded".to_string(),
                                        };
                                        out.push_str(&format!(
                                            " ({},{},{})={name}",
                                            fx + dx,
                                            fy,
                                            fz + dz
                                        ));
                                    }
                                }
                            }
                            out.push_str(&format!(
                                "\nplayers={}",
                                bot.nearby_players().map(|p| p.len()).unwrap_or(0)
                            ));
                            if let Some(tx) = &result_tx {
                                let _ = tx.send(format!("Action output:\nRAW|{out}"));
                            }
                        }
                        // P110b: probe 侧共享 WorldMemory 操作（与 LLM memory 工具同一实例）。
                        BotCommand::Memory {
                            action,
                            name,
                            x,
                            y,
                            z,
                        } => {
                            let out = match action.as_str() {
                                "anchor" => {
                                    let n = name.as_deref().unwrap_or("anchor");
                                    let p = MemoryPos::new(
                                        x.unwrap_or(0),
                                        y.unwrap_or(0),
                                        z.unwrap_or(0),
                                    );
                                    if let Some(mem) = state.memory.as_ref() {
                                        mem.set_anchor(n, Some(p), n);
                                        format!("已设锚点 {n} @({},{},{})", p.x, p.y, p.z)
                                    } else {
                                        "无共享 WorldMemory".to_string()
                                    }
                                }
                                "query" => {
                                    if let Some(mem) = state.memory.as_ref() {
                                        let anchors = mem.anchors();
                                        if anchors.is_empty() {
                                            "无锚点".to_string()
                                        } else {
                                            anchors
                                                .iter()
                                                .map(|a| {
                                                    let p = a
                                                        .pos
                                                        .map(|p| {
                                                            format!("({},{},{})", p.x, p.y, p.z)
                                                        })
                                                        .unwrap_or_default();
                                                    format!("{} {p}", a.name)
                                                })
                                                .collect::<Vec<_>>()
                                                .join(", ")
                                        }
                                    } else {
                                        "无锚点".to_string()
                                    }
                                }
                                other => format!("memory 未知动作 {other}"),
                            };
                            if let Some(tx) = &result_tx {
                                let _ = tx.send(format!("Action output:\n{out}"));
                            }
                            state.action_mgr.clear_pending();
                        }
                        // P110: 按锚点名导航（goto home）。解析锚点 → 替换 pending 槽为 Goto
                        //（busy=true 已置，clear_pending 后下一 tick Goto 正常轮询执行）。
                        // 锚点不存在 → 明确报错。锚点解析失败自动回退 x/y/z 分支。
                        BotCommand::GotoAnchor { name } => {
                            let pos = state
                                .memory
                                .as_ref()
                                .and_then(|mem| mem.find_anchor(&name).and_then(|a| a.pos));
                            match pos {
                                Some(p) => {
                                    // 直接接管 pending 槽为 Goto（保留原 result_tx 以便结果回传）。
                                    *state.action_mgr.pending.lock().unwrap() =
                                        Some(QueuedCommand {
                                            cmd: BotCommand::Goto {
                                                x: p.x,
                                                y: p.y,
                                                z: p.z,
                                            },
                                            result_tx: result_tx.clone(),
                                        });
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!(
                                            "[goto] 锚点 {name} -> ({},{},{})，开始导航",
                                            p.x, p.y, p.z
                                        ),
                                    });
                                }
                                None => {
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\n锚点 {name} 不存在。可用 memory action=query 查看全部锚点。"
                                        ));
                                    }
                                    state.action_mgr.clear_pending();
                                }
                            }
                        }
                        // P111: 按玩家名单次导航（gotoplayer <名字>）。复用 P110 模式：
                        // 解析玩家当前坐标 → 替换 pending 槽为 Goto（保留原 result_tx）。
                        // 玩家移动时到达点即目标当时坐标；不持续跟随（持续跟随用 follow）。
                        BotCommand::GotoPlayer { name } => {
                            let name_ref = name.as_deref();
                            match nearby_player_position(&bot, name_ref) {
                                Some(p) => {
                                    let (px, py, pz) = (
                                        p.x.floor() as i32,
                                        p.y.floor() as i32,
                                        p.z.floor() as i32,
                                    );
                                    *state.action_mgr.pending.lock().unwrap() =
                                        Some(QueuedCommand {
                                            cmd: BotCommand::Goto {
                                                x: px,
                                                y: py,
                                                z: pz,
                                            },
                                            result_tx: result_tx.clone(),
                                        });
                                    let who = name_ref.unwrap_or("最近的玩家");
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!(
                                            "[goto] 玩家 {who} @ ({px},{py},{pz})，开始导航"
                                        ),
                                    });
                                }
                                None => {
                                    let who = name_ref.unwrap_or("任何玩家");
                                    let msg = format!("未找到玩家 {who}（不在附近扫描范围）。");
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\n{msg}"));
                                    }
                                    state.action_mgr.clear_pending();
                                }
                            }
                        }
                        BotCommand::Goto { x, y, z } => {
                            *state.mining_below.lock().unwrap() = false;
                            // P66：冷却拦截。按 bot 当前格子检查冷却（而非目标坐标，
                            // 因为 LLM 会微调目标逃避同一坐标冷却）。在冷却期内任何 goto 直接拒绝，
                            // 强制 LLM/脚本换策略（挖开阻挡或上地表），打破 goto 洪泛。
                            {
                                if let Ok(p) = bot.position() {
                                    let cell = (
                                        p.x.floor() as i32,
                                        p.y.floor() as i32,
                                        p.z.floor() as i32,
                                    );
                                    let cd = state.goto_cooldown.lock().unwrap();
                                    if let Some(&until) = cd.get(&cell)
                                        && until > bot.ticks_connected()
                                    {
                                        if let Some(tx) = &result_tx {
                                            let _ = tx.send(format!(
                                                    "Action output:\ngoto ({},{},{}) 被拒绝——你当前位置仍在导航冷却中（之前连续 goto 超时且没移动）。\
                                                     请改用 mine 挖开挡路方块，或 mine_above 上到地表开阔处，不要继续 goto 旁边区域。",
                                                    x, y, z
                                                ));
                                        }
                                        state.action_mgr.clear_pending();
                                        return bot;
                                    }
                                }
                            }
                            // 距离限制：>32 格的 goto 拒绝执行，让 LLM 拆成多段。
                            // 原因：azalea pathfinder 的 A* 在长距离/复杂地形上计算量大，
                            // 每 tick 发 MovePlayerPos+PlayerInput 包会拖死 vanilla 服 TPS，
                            // 导致同服真实玩家 WASD 输入丢失（服务器来不及处理）。
                            let p = bot.position().ok();
                            if let Some(p) = p {
                                let dist = ((p.x - x as f64).powi(2)
                                    + (p.y - y as f64).powi(2)
                                    + (p.z - z as f64).powi(2))
                                .sqrt();
                                if dist > 32.0 {
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                        "Action output:\ngoto ({},{},{}) 距离 {:.0}m 过远（>32m），\
                                         请拆成多段：先 goto 中间点（距当前 16-24m），到达后再 goto 目标。",
                                        x, y, z, dist
                                    ));
                                    }
                                    state.action_mgr.clear_pending();
                                    return bot;
                                }
                                // P65 修复：goto 目标是实心方块（脚下/身旁矿脉）时，bot 站旁边即被判
                                // "到达"(distance<1.5) 却永远挖不进/进不去 → 反复 goto 同一坐标死循环。
                                // 检测目标方块是否 solid：solid 则直接拒绝并（地下时）自动 mine_above 脱困。
                                let target_solid = if let Ok(world) = bot.world() {
                                    let world = world.read();
                                    world
                                        .get_block_state(BlockPos::new(x, y, z))
                                        .map(|b| !b.is_air())
                                        .unwrap_or(false)
                                } else {
                                    false
                                };
                                if target_solid {
                                    // P69b：目标实心（树干/树叶/山体/树冠）时不再直接拒绝——
                                    // LLM 在密林里看不见地面，经常选到树冠/树干坐标。
                                    // 自动向上找最近的可站立空气点，修正目标继续前往。
                                    // 若上方 8 格全是实心（如地下岩体）才走原拒绝逻辑。
                                    let mut fallback: Option<(i32, i32, i32)> = None;
                                    if let Ok(world) = bot.world() {
                                        for k in 1..=8 {
                                            let up = BlockPos::new(x, y + k, z);
                                            let is_air = world
                                                .read()
                                                .get_block_state(up)
                                                .map(|b| b.is_air())
                                                .unwrap_or(false);
                                            if is_air {
                                                fallback = Some((x, y + k, z));
                                                break;
                                            }
                                        }
                                    }
                                    if let Some((fx, fy, fz)) = fallback {
                                        if let Some(tx) = &result_tx {
                                            let _ = tx.send(format!(
                                                "Action output:\ngoto ({},{},{}) 目标方块是实心（树干/树叶/山体），已自动修正为上方可站立点 ({},{},{}) 继续前往。",
                                                x, y, z, fx, fy, fz
                                            ));
                                        }
                                        bot.start_goto(BlockPosGoal(BlockPos::new(fx, fy, fz)));
                                        state.action_mgr.clear_pending();
                                        return bot;
                                    }
                                    // P126：上方无站立点且目标是矿石时，自动改挖该方块
                                    // （等同 mine）。原拒绝建议"改用附近空气坐标"对嵌入岩体的矿石
                                    // 是死路——矿石四周全是实心，LLM 会反复 goto 周边坐标死循环
                                    // （实测连续 6+ 次 goto 矿石/邻格全实心失败）。派发时自动修正。
                                    let target_kind = bot.world().ok().and_then(|w| {
                                        w.read()
                                            .get_block_state(BlockPos::new(x, y, z))
                                            .map(|b| b.into())
                                    });
                                    if is_natural_mineable(target_kind) {
                                        if let Some(tx) = &result_tx {
                                            let _ = tx.send(format!(
                                                "Action output:\ngoto ({},{},{}) 的目标方块是矿石（实心、上方无站立点），\
                                                已自动改为直接挖掘该方块（等同 mine {} {} {}）。若掉落物离你较远，挖完后用 pickup 拾取。",
                                                x, y, z, x, y, z
                                            ));
                                        }
                                        let tx_clone = result_tx.clone();
                                        state.action_mgr.clear_pending();
                                        cmd_queue.lock().unwrap().push(QueuedCommand {
                                            cmd: BotCommand::Mine { x, y, z },
                                            result_tx: tx_clone,
                                        });
                                        return bot;
                                    }
                                    // P132：非矿石实体（岩壁/山体/树干）目标，P69b 上方无空气且
                                    // P126 非矿石时，自动修正到目标附近最近的可站立空气点。
                                    // LLM 盲猜岩体内坐标离真实洞穴/地表通常几格内，修正后
                                    // pathfinder 能到达；直接拒绝会让 LLM 反复换坐标死循环
                                    // （实测同一坐标连续 6+ 次失败，换坐标继续失败）。
                                    if let Some(air_pos) = nearest_standable_air(&bot, x, y, z) {
                                        let (fx, fy, fz) = (air_pos.x, air_pos.y, air_pos.z);
                                        if let Some(tx) = &result_tx {
                                            let _ = tx.send(format!(
                                                "Action output:\ngoto ({},{},{}) 的目标方块是实心（岩壁/山体，上方无站立空气），\
                                                 已自动修正到最近可站立空气点 ({},{},{}) 继续前往。",
                                                x, y, z, fx, fy, fz
                                            ));
                                        }
                                        bot.start_goto(BlockPosGoal(BlockPos::new(fx, fy, fz)));
                                        state.action_mgr.clear_pending();
                                        return bot;
                                    }
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\ngoto ({},{},{}) 失败——目标方块是实心方块（不能站在里面）。请改用附近的空气方块坐标，或若在地下请用 mine_above 向上挖出脱困。",
                                            x, y, z
                                        ));
                                    }
                                    if (p.y as i32) < 62 {
                                        *state.mining_above.lock().unwrap() = true;
                                        *state.mining_above_start_y.lock().unwrap() =
                                            Some(p.y.floor() as i32);
                                        *state.mining_above_direction.lock().unwrap() = 0;
                                        bot.force_stop_pathfinding();
                                    }
                                    state.action_mgr.clear_pending();
                                    return bot;
                                }
                            }
                            // P60: 自动挖回地表后再 goto——当 bot 被实心方块封闭（头顶实心，
                            // 无空气通道）时，pathfinder 无法穿墙导航，需挖出再 goto。
                            // 注意：判定标准是「被埋」而非 Y<62——洞穴/地下开放空间（头顶空气、
                            // 可水平寻路）是 bot 的合法活动区（工作台/熔炉/矿点都在地下），
                            // 若一入 Y<62 就强制挖回地表，bot 永远无法在层间导航（P147 修复）。
                            let mut needs_surface = false;
                            if let (Ok(p), Ok(world)) = (bot.position(), bot.world()) {
                                let world = world.read();
                                // 头顶 1 格与 2 格是否全是实心：头顶被封闭 = 被埋，需挖出
                                let head_pos = BlockPos::new(
                                    p.x.floor() as i32,
                                    p.y.floor() as i32 + 1,
                                    p.z.floor() as i32,
                                );
                                let head2_pos = BlockPos::new(
                                    p.x.floor() as i32,
                                    p.y.floor() as i32 + 2,
                                    p.z.floor() as i32,
                                );
                                let head_block = world.get_block_state(head_pos);
                                let head2_block = world.get_block_state(head2_pos);
                                let head_solid = head_block.map(|b| !b.is_air()).unwrap_or(false);
                                let head2_solid = head2_block.map(|b| !b.is_air()).unwrap_or(false);
                                // 仅当头顶（及上方第 2 格）都被实心方块封闭时才判定被埋。
                                // 头顶是空气（洞穴/室内/矿井竖井）→ 可正常 goto 导航。
                                if head_solid && head2_solid {
                                    needs_surface = true;
                                }
                            }
                            if needs_surface {
                                // 启动自动挖回地表模式（handler 的 tick 循环会处理持续上挖）
                                *state.mining_above.lock().unwrap() = true;
                                // 快速返回：让 handler 在接下来的 tick 中执行 mine_above
                                // 同时启动一个延迟检查：当 Y>=62 时自动执行原始 goto
                                // 注：goto 目标坐标保存在 pending cmd 中，不会被清除
                                if let Some(tx) = &result_tx {
                                    let _ = tx.send(format!(
                                        "Action output:
goto ({},{},{}) ——bot 在地下，先自动挖回地表。mine_above 已启动。",
                                        x, y, z
                                    ));
                                }
                                state.action_mgr.clear_pending();
                                return bot;
                            }
                            // P59: 快速可达性检测——检查目标是否在同一 Y 层被实心方块包围
                            if let Ok(p) = bot.position() {
                                let dy = (y as f64 - p.y).abs();
                                let dxz =
                                    ((p.x - x as f64).powi(2) + (p.z - z as f64).powi(2)).sqrt();
                                if dxz < 5.0
                                    && dy < 2.0
                                    && let Ok(world) = bot.world()
                                {
                                    let world = world.read();
                                    let head_pos = BlockPos::new(
                                        p.x.floor() as i32,
                                        p.y.floor() as i32 + 1,
                                        p.z.floor() as i32,
                                    );
                                    if let Some(head_block) = world.get_block_state(head_pos) {
                                        let bk: azalea_registry::builtin::BlockKind =
                                            head_block.into();
                                        if bk != azalea_registry::builtin::BlockKind::Air {
                                            if let Some(tx) = &result_tx {
                                                let _ = tx.send(format!(
                                                    "Action output:
goto ({},{},{}) 失败——bot 头上有方块（可能被埋）。
先用 perceive 确认位置，若被实心方块封闭需用 mine_above 挖出。",
                                                    x, y, z
                                                ));
                                            }
                                            state.action_mgr.clear_pending();
                                            return bot;
                                        }
                                    }
                                }
                            }
                            // P59: 快速可达性检测——检查目标是否在同一 Y 层被实心方块包围
                            if let Ok(p) = bot.position() {
                                let dy = (y as f64 - p.y).abs();
                                let dxz =
                                    ((p.x - x as f64).powi(2) + (p.z - z as f64).powi(2)).sqrt();
                                if dxz < 5.0
                                    && dy < 2.0
                                    && let Ok(world) = bot.world()
                                {
                                    let world = world.read();
                                    let head_pos = BlockPos::new(
                                        p.x.floor() as i32,
                                        p.y.floor() as i32 + 1,
                                        p.z.floor() as i32,
                                    );
                                    if let Some(head_block) = world.get_block_state(head_pos) {
                                        let bk: azalea_registry::builtin::BlockKind =
                                            head_block.into();
                                        if bk != azalea_registry::builtin::BlockKind::Air {
                                            if let Some(tx) = &result_tx {
                                                let _ = tx.send(format!(
                                                    "Action output:
goto ({},{},{}) 失败——bot 头上有方块（可能在地下）。
先用 perceive 确认位置，若 Y<62 说明在地下，需用 mine_above 挖回地表。",
                                                    x, y, z
                                                ));
                                            }
                                            state.action_mgr.clear_pending();
                                            return bot;
                                        }
                                    }
                                }
                            }
                            // P59: 快速可达性检测——检查目标是否在同一 Y 层被实心方块包围
                            if let Ok(p) = bot.position() {
                                let dy = (y as f64 - p.y).abs();
                                let dxz =
                                    ((p.x - x as f64).powi(2) + (p.z - z as f64).powi(2)).sqrt();
                                if dxz < 5.0
                                    && dy < 2.0
                                    && let Ok(world) = bot.world()
                                {
                                    let world = world.read();
                                    let head_pos = BlockPos::new(
                                        p.x.floor() as i32,
                                        p.y.floor() as i32 + 1,
                                        p.z.floor() as i32,
                                    );
                                    if let Some(head_block) = world.get_block_state(head_pos) {
                                        let bk: azalea_registry::builtin::BlockKind =
                                            head_block.into();
                                        if bk != azalea_registry::builtin::BlockKind::Air {
                                            if let Some(tx) = &result_tx {
                                                let _ = tx.send(format!(
                                                    "Action output:
goto ({},{},{}) 失败——bot 头上有方块（可能在地下）。
先用 perceive 确认位置，若 Y<62 说明在地下，需用 mine_above 挖回地表。",
                                                    x, y, z
                                                ));
                                            }
                                            state.action_mgr.clear_pending();
                                            return bot;
                                        }
                                    }
                                }
                            }
                            bot.start_goto(BlockPosGoal(BlockPos::new(x, y, z)));
                            // P93：goto 进度流式事件（每 20 tick 一次）
                            if bot.ticks_connected().is_multiple_of(20)
                                && let Ok(p) = bot.position()
                            {
                                let dist = ((p.x - x as f64).powi(2)
                                    + (p.y - y as f64).powi(2)
                                    + (p.z - z as f64).powi(2))
                                .sqrt();
                                let _ = evt_tx.send(BotEvent::Progress {
                                    command: format!("goto ({x},{y},{z})"),
                                    detail: format!(
                                        "位置 ({:.1},{:.1},{:.1})，距目标 {:.1}m",
                                        p.x, p.y, p.z, dist
                                    ),
                                });
                            }
                        }
                        BotCommand::Mine { x, y, z } => {
                            *state.mining_below.lock().unwrap() = false;
                            // P5 修复：挖矿前自动装备最好的镐。否则 bot 拿面包挖石头
                            // 既慢又不掉落物，且 LLM 不会主动 equip（挖矿工具隐含前提）。
                            let _ = auto_equip_best_pickaxe(&bot).await;
                            // P101 修复：目标格是空气时自动修正到最近实心方块。
                            // 实机观测：LLM 盲猜坐标连续 15+ 次 mine 空气格（每次换坐标，
                            // 死循环检测不触发），工具返回的"最近实心方块"提示被无视。
                            // 与其报错让 LLM 猜，不如直接挖最近的实心方块——行为不变契约：
                            // 正常情况目标即实心方块，修正仅在空气目标时生效。
                            let mine_eff = if let Ok(world) = bot.world() {
                                let target_is_air = world
                                    .read()
                                    .get_block_state(BlockPos::new(x, y, z))
                                    .map(|b| b.is_air())
                                    .unwrap_or(true);
                                if target_is_air {
                                    let eff = nearest_solid_block(&bot, x, y, z)
                                        .unwrap_or(BlockPos::new(x, y, z));
                                    (eff, true)
                                } else {
                                    (BlockPos::new(x, y, z), false)
                                }
                            } else {
                                (BlockPos::new(x, y, z), false)
                            };
                            let (mine_pos, original_was_air) = mine_eff;
                            let (mx, my, mz) = (mine_pos.x, mine_pos.y, mine_pos.z);
                            // P101：记录实际挖掘目标——done 轮询判定与完成反馈都依赖它
                            // （否则空气原目标会让 done 立即成立，修正挖掘被终结）。
                            let mut eff_guard = state.last_mine_eff.lock().unwrap();
                            let already_recorded =
                                eff_guard.map(|(p, _)| (p.x, p.y, p.z) == (mx, my, mz));
                            *eff_guard = Some((BlockPos::new(mx, my, mz), original_was_air));
                            drop(eff_guard);
                            // P101 修正通知走事件流（瞬态），不消费 result_tx——
                            // 最终成功/超时结果必须由 done 分支发送。且每 tick 重复派发
                            // 会重复发通知（实测 14 次），只在目标变更的首帧发一次。
                            if (mx, my, mz) != (x, y, z) && already_recorded != Some(true) {
                                let _ = evt_tx.send(BotEvent::Chat {
                                    content: format!(
                                        "目标 ({x},{y},{z}) 已是空气，自动修正为最近实心方块 ({mx},{my},{mz}) 开始挖掘"
                                    ),
                                });
                            }
                            // P150：mine 目标距 bot 过远时先靠近再挖——否则掉落物生成在远处
                            // 拾取不到（实机：挖原木/泥土/dirt 掉落物丢失，实体列表无 item）。
                            // 参考 P100 交互贴脸纪律：先 goto 到目标旁（水平+垂直均靠近），
                            // 靠近后下一 tick 自然触发 start_mining。
                            let mine_dist = bot
                                .position()
                                .ok()
                                .map(|p| {
                                    ((p.x - mx as f64).powi(2)
                                        + (p.y - my as f64).powi(2)
                                        + (p.z - mz as f64).powi(2))
                                    .sqrt()
                                })
                                .unwrap_or(f64::MAX);
                            // P167：mine 目标距 bot 过远（>40m）或 Y 差过大（>12）时，
                            // pathfinder 全图寻路会严重漂移（实机：mine 距 3m 的目标被拉到
                            // 9-24m 外，Y 从 93 漂到 85）——盲目 goto 反而更远。
                            // 直接报错提示，让 LLM 用 /tp 或 goto 分小段接近。
                            let now_pos_y = bot.position().ok().map(|p| p.y).unwrap_or(my as f64);
                            if mine_dist > 40.0 || (now_pos_y - my as f64).abs() > 12.0 {
                                *state.mine_approach_watchdog.lock().unwrap() = None;
                                if let Some(tx) = &result_tx {
                                    let _ = tx.send(format!(
                                        "Action output:\nmine ({mx},{my},{mz}) 距 bot {:.0}m（Y 差 {:.0}），过远——pathfinder 全图寻路会漂移。\
                                        建议：1) 先 /tp 到目标附近（确认可站立）再 mine 2) 或分小段 goto 接近。",
                                        mine_dist, (now_pos_y - my as f64).abs()
                                    ));
                                }
                                state.action_mgr.clear_pending();
                                return bot;
                            }
                            if mine_dist > 2.5 {
                                // P155：mine 靠近看门狗——pathfinder 找不到路径时
                                // start_goto(RadiusGoal) 永不到达，靠近分支无限循环。
                                // 连续 120 tick（6s）且 bot 净移动 <1.5 格 → 判定寻路失败。
                                let tick = bot.ticks_connected();
                                let now_pos = bot.position().ok();
                                let mut wd = state.mine_approach_watchdog.lock().unwrap();
                                match *wd {
                                    Some(ref w) if w.target == (mx, my, mz) => {
                                        let (wx, wy, wz) = w.target;
                                        let (lx, ly, lz) = w.anchor;
                                        let start_tick = w.start_tick;
                                        // 净移动检测：bot 实际位置距上次锚点的位移
                                        let moved = now_pos
                                            .map(|p| {
                                                ((p.x - lx).powi(2)
                                                    + (p.y - ly).powi(2)
                                                    + (p.z - lz).powi(2))
                                                .sqrt()
                                            })
                                            .unwrap_or(0.0)
                                            > 1.5;
                                        if moved {
                                            // 有进展：重置无进展计数，更新锚点
                                            *wd = Some(MineApproachWatchdog {
                                                target: (wx, wy, wz),
                                                anchor: (
                                                    now_pos.as_ref().map(|p| p.x).unwrap_or(lx),
                                                    now_pos.as_ref().map(|p| p.y).unwrap_or(ly),
                                                    now_pos.as_ref().map(|p| p.z).unwrap_or(lz),
                                                ),
                                                start_tick: tick,
                                                stall_ticks: 0,
                                            });
                                        } else if tick.saturating_sub(start_tick) >= 120 {
                                            // 判定寻路失败：强停 + 报错
                                            bot.force_stop_pathfinding();
                                            *wd = None;
                                            if let Some(tx) = &result_tx {
                                                let _ = tx.send(format!(
                                                    "Action output:\nmine ({mx},{my},{mz}) 靠近失败——6s 内 bot 几乎没移动（路径被阻或目标不可达）。已停止靠近。建议：1) 用 goto 分小段接近目标 2) 换一个更近/可达的方块 3) 先 perceive 确认位置。"
                                                ));
                                            }
                                            state.action_mgr.clear_pending();
                                            drop(wd);
                                            return bot;
                                        }
                                    }
                                    _ => {
                                        *wd = Some(MineApproachWatchdog {
                                            target: (mx, my, mz),
                                            anchor: (
                                                now_pos.as_ref().map(|p| p.x).unwrap_or(mx as f64),
                                                now_pos.as_ref().map(|p| p.y).unwrap_or(my as f64),
                                                now_pos.as_ref().map(|p| p.z).unwrap_or(mz as f64),
                                            ),
                                            start_tick: tick,
                                            stall_ticks: 0,
                                        });
                                    }
                                }
                                drop(wd);
                                // 走到目标旁：用 RadiusGoal 让 pathfinder 靠近到 2m 内
                                bot.start_goto(RadiusGoal {
                                    pos: azalea::Vec3::new(mx as f64, my as f64, mz as f64),
                                    radius: 2.0,
                                });
                                // P152：靠近分支必须给 result_tx 发中间结果——否则命令结果
                                // 通道为空，bot_tool 报 "channel is empty"。靠近是异步的，
                                // 下一 tick 重新派发时距离 ≤2.5m 自然触发 start_mining。
                                if let Some(tx) = &result_tx {
                                    let _ = tx.send(format!(
                                        "Action output:\nmine ({mx},{my},{mz}) 目标距 {:.1}m，正在靠近（≤2.5m 后自动挖掘）。",
                                        mine_dist
                                    ));
                                }
                                state.action_mgr.clear_pending();
                                return bot;
                            } else {
                                // 距离已足够：清看门狗
                                *state.mine_approach_watchdog.lock().unwrap() = None;
                            }
                            // P151：挖矿前必须 look_at 目标方块中心——azalea mine 不强制视线，
                            // 但不看向方块时服务端不完整认可这次破坏（table_flow.rs P34 同款：
                            // "look_at 提高挖掘成功率"）。实机：mine 不 look_at 时方块被破坏但
                            // 掉落物从不生成（probe + 主 bot 双验证，cobblestone/dirt/coal 均不变）。
                            let mine_center = azalea::Vec3::new(
                                mx as f64 + 0.5,
                                my as f64 + 0.5,
                                mz as f64 + 0.5,
                            );
                            bot.look_at(mine_center);
                            bot.start_mining(mine_pos);
                            // P93：mine 进度流式事件（每 20 tick 一次）
                            if bot.ticks_connected().is_multiple_of(20)
                                && let Ok(p) = bot.position()
                            {
                                let dist = ((p.x - mx as f64).powi(2)
                                    + (p.y - my as f64).powi(2)
                                    + (p.z - mz as f64).powi(2))
                                .sqrt();
                                let _ = evt_tx.send(BotEvent::Progress {
                                    command: format!("mine ({mx},{my},{mz})"),
                                    detail: format!(
                                        "挖掘目标距当前位置 {:.1}m，bot 位置 ({:.1},{:.1},{:.1})",
                                        dist, p.x, p.y, p.z
                                    ),
                                });
                            }
                        }
                        BotCommand::MineBelow => {
                            *state.mining_below.lock().unwrap() = true;
                            // 同 Mine：下挖也要装备镐
                            let _ = auto_equip_best_pickaxe(&bot).await;
                            if let Ok(p) = bot.position() {
                                let foot = BlockPos::new(
                                    p.x.floor() as i32,
                                    (p.y - 1.0).floor() as i32,
                                    p.z.floor() as i32,
                                );
                                bot.start_mining(foot);
                            }
                            if let Some(tx) = &result_tx {
                                let _ = tx.send("已开始向下挖掘".to_string());
                            }
                            state.action_mgr.clear_pending();
                        }
                        BotCommand::MakeObsidian { count } => {
                            // P67：初始化造黑曜石状态机 (remaining, phase, obsidian_pos)。
                            // 注意：tick handler 内严禁 await（会冻结整个事件循环导致 120s 超时）。
                            // 装备 bucket / 装水 / 找岩浆全部在状态机内每 tick 同步推进，不做任何 .await。
                            *state.make_obsidian.lock().unwrap() = Some((count.max(1), 0, None));
                            // P160：记录启动 tick，状态机推进处检查 >600 tick（30s）强制失败，
                            // 防止"装水失败/找不到岩浆"无限重试拖死 viewer。
                            *state.make_obsidian_start_tick.lock().unwrap() =
                                Some(bot.ticks_connected());
                            // 立即回报"已开始"，让工具层不阻塞等待（真正的完成由状态机结束帧回报）。
                            if let Some(tx) = &result_tx {
                                let _ = tx.send(format!(
                                    "已开始造黑曜石 x{}：状态机会自动装备水桶、找水源装水、再找岩浆造黑曜石。",
                                    count
                                ));
                            }
                            state.action_mgr.clear_pending();
                        }
                        BotCommand::MineAbove => {
                            // P5 新增：向上挖脱困。从 bot 头顶逐格挖到空气或达到 64 格上限。
                            // 持续触发模式（同 MineBelow）：mining_above 标志位驱动后续 tick 重复发起。
                            *state.mining_below.lock().unwrap() = false;
                            // P5 修复：原代码无脑要求"必须有镐"，但 dirt/grass/sand/gravel/sandstone
                            // 等软方块徒手就能挖。只有挖 stone/deepslate/ores 等硬方块才必须用镐。
                            // 现在改为：先看头顶方块类型，软方块直接挖；硬方块才检查镐。
                            let head_pos = bot.position().ok().map(|p| {
                                BlockPos::new(
                                    p.x.floor() as i32,
                                    (p.y + 1.0).floor() as i32,
                                    p.z.floor() as i32,
                                )
                            });
                            let head_state = head_pos.and_then(|pos| {
                                let world = bot.world().ok()?;
                                let world = world.read();
                                world.get_block_state(pos)
                            });
                            let head_is_air = head_state.is_some_and(|block| block.is_air());
                            // Surface pre-check: if already on surface (Y>=62 + air column),
                            // return immediately instead of starting 10s timeout.
                            if head_is_air && let Ok(p) = bot.position() {
                                let y = p.y.floor() as i32;
                                if y >= 62
                                    && let Ok(world) = bot.world()
                                {
                                    let cx = p.x.floor() as i32;
                                    let cz = p.z.floor() as i32;
                                    let world = world.read();
                                    let mut five_air = true;
                                    for dy in 1..=5 {
                                        let check = BlockPos::new(cx, y + dy, cz);
                                        let is_air = world
                                            .get_block_state(check)
                                            .map(|s| s.is_air())
                                            .unwrap_or(false);
                                        if !is_air {
                                            five_air = false;
                                            break;
                                        }
                                    }
                                    drop(world);
                                    if mine_above_reached_surface(y, true, five_air) {
                                        if let Some(tx) = &result_tx {
                                            let _ = tx.send(format!(
                                                        "Action output:\nMineAbove done at Y={y} (已到地表，头顶是空气)。当前坐标 ({:.0},{y},{:.0})，可继续探索。",
                                                        p.x, p.z
                                                    ));
                                        }
                                        *state.mining_above.lock().unwrap() = false;
                                        state.action_mgr.clear_pending();
                                        return bot;
                                    }
                                }
                            }
                            let head_is_hard = head_state.map(is_hard_block).unwrap_or(true); // 不确定时按硬方块处理
                            // P120（2026-08-07）：无镐徒手挖硬方块是可行的逃生路径（~8s/格，
                            // 不掉落物品）。此前这里直接 abort 并报错，导致地下死锁——
                            // 无镐无木时 自动路径全被拦（mine_above/gather 都拒绝徒手），
                            // LLM 只能靠 Mine 逐格慢挖。逃生不需要掉落物，挖穿即可。
                            // 改为警告后继续徒手挖（不做硬拒绝）。dispatch 每 tick 重入，
                            // 用 mining_above_no_pick_warned 去重（命令结束重置）。
                            // P120b：无镐且头顶硬方块时，先自动找附近软土柱（dirt/sand/...）
                            // 绕行——徒手挖软土 ~0.25s/格 vs 硬方块 ~8s/格（快 32 倍）。
                            // 找到软土柱则优先绕行（避免死磕硬天花板），找不到才徒手硬挖。
                            if head_is_hard
                                && !head_is_air
                                && !has_any_pickaxe_in_inventory(&bot).await
                                && state.mining_above_soft_column.lock().unwrap().is_none()
                                && let Ok(p) = bot.position()
                            {
                                let (cx, cy, cz) =
                                    (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
                                if let Some(col) = nearest_soft_column(&bot, cx, cy, cz, 4)
                                    .or_else(|| nearest_soft_column(&bot, cx, cy, cz, 8))
                                    .or_else(|| nearest_soft_column(&bot, cx, cy, cz, 16))
                                {
                                    *state.mining_above_soft_column.lock().unwrap() = Some(col);
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!(
                                            "⚠️ 头顶是硬方块且背包无镐：已自动绕行到最近软土柱 ({},{},{}) 从软土向上挖（徒手 ~0.25s/格，比硬方块快 32 倍）。",
                                            col.x, col.y, col.z
                                        ),
                                    });
                                } else if !*state.mining_above_no_pick_warned.lock().unwrap() {
                                    *state.mining_above_no_pick_warned.lock().unwrap() = true;
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: "⚠️ 头顶是硬方块且背包无镐：附近无软土柱，将徒手慢速挖掘（~8秒/格，不掉落物品）作为逃生通道。\
                                                  如多个 MineAbove 都未成功，请先合成镐（wooden_pickaxe 2×2：3 planks+2 stick）或用 mine 横向找泥土地/沙地软通道。".to_string(),
                                        });
                                }
                            }
                            let was_active = *state.mining_above.lock().unwrap();
                            *state.mining_above.lock().unwrap() = true;
                            if !was_active && let Ok(position) = bot.position() {
                                *state.mining_above_start_y.lock().unwrap() =
                                    Some(position.y.floor() as i32);
                                *state.mining_above_direction.lock().unwrap() = 0;
                            }
                            let _ = auto_equip_best_pickaxe(&bot).await;
                            if !head_is_air
                                && state.mining_above_soft_column.lock().unwrap().is_none()
                                && let Some(pos) = head_pos
                                && !bot.is_mining()
                            {
                                bot.start_mining(pos);
                            }
                        }
                        BotCommand::BlockInteract { x, y, z } => {
                            *state.mining_below.lock().unwrap() = false;
                            // P161：水源/岩浆等液体方块用 force_block 右键会被服务端静默拒收
                            // （force_block 构造的 BlockHitResult 方向固定 Up，服务端不识别为
                            // "点击液体"）。改用"面向目标 + start_use_item"（真实 hit_result 交互）：
                            // 手持 bucket 面向水源右键 → 装水；面向岩浆 → 装岩浆。
                            let target_is_liquid = bot
                                .world()
                                .ok()
                                .and_then(|w| {
                                    let w = w.read();
                                    w.get_block_state(BlockPos::new(x, y, z))
                                })
                                .map(|bs| {
                                    let k: azalea_registry::builtin::BlockKind = bs.into();
                                    k == azalea_registry::builtin::BlockKind::Water
                                        || k == azalea_registry::builtin::BlockKind::Lava
                                        || k == azalea_registry::builtin::BlockKind::PowderSnow
                                })
                                .unwrap_or(false);
                            // P161c 诊断：输出目标方块类型与交互路径（viewer stderr 可查）
                            let held_diag = bot
                                .get_held_item()
                                .ok()
                                .filter(|s| !s.is_empty())
                                .map(|s| s.kind().to_string())
                                .unwrap_or_else(|| "empty".into());
                            eprintln!(
                                "[P161] interact ({x},{y},{z}) liquid={target_is_liquid} held={held_diag}"
                            );
                            if target_is_liquid {
                                if let Ok(p) = bot.position() {
                                    // P161d：交互期间暂停 P60c 地下脱困（截止 +10 tick），
                                    // 避免装水/装岩浆刚发出就被脱困拉走。
                                    *state.interact_hold_until.lock().unwrap() =
                                        Some(bot.ticks_connected() + 10);
                                    // P161b：先确保手持 bucket（P156 自动装备镐可能把主手切走）。
                                    // 目标液体是水/岩浆时，交互需要手持对应桶：水→bucket(空桶装水)，
                                    // 岩浆→bucket(装岩浆) 或已持有的 lava_bucket。此处统一确保空 bucket 在主手。
                                    let held_kind = bot
                                        .get_held_item()
                                        .ok()
                                        .filter(|s| !s.is_empty())
                                        .map(|s| s.kind());
                                    let need_bucket = !held_kind.is_some_and(|k| {
                                        k == azalea_registry::builtin::ItemKind::Bucket
                                    });
                                    if need_bucket
                                        && let Ok(inv) = bot.get_inventory()
                                        && let Some(h) = find_hotbar_slot_for(
                                            &inv,
                                            azalea_registry::builtin::ItemKind::Bucket,
                                        )
                                    {
                                        bot.set_selected_hotbar_slot(h);
                                        tokio::time::sleep(std::time::Duration::from_millis(150))
                                            .await;
                                    }
                                    let dx = x as f64 + 0.5 - p.x;
                                    let dy = y as f64 + 0.5 - p.y;
                                    let dz = z as f64 + 0.5 - p.z;
                                    let dist = (dx * dx + dy * dy + dz * dz).sqrt().max(0.001);
                                    let yaw =
                                        (dz.atan2(dx).to_degrees() + 90.0).rem_euclid(360.0) as f32;
                                    let pitch =
                                        (-dy / dist).asin().to_degrees().clamp(-89.0, 89.0) as f32;
                                    let _ = bot.set_direction(yaw, pitch);
                                    // 等方向生效（P118：set_direction 后 150ms 内 raycast 仍用旧朝向）
                                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                                    bot.start_use_item();
                                } else {
                                    bot.block_interact(BlockPos::new(x, y, z));
                                }
                            } else {
                                bot.block_interact(BlockPos::new(x, y, z));
                            }
                            if let Some(tx) = &result_tx {
                                let _ = tx.send(format!("已交互 ({},{},{})", x, y, z));
                            }
                        }
                        BotCommand::TillAndSow { x, y, z, seed } => {
                            *state.mining_below.lock().unwrap() = false;
                            match crate::azalea::till::do_till_and_sow(&bot, x, y, z, &seed).await {
                                Ok(msg) => {
                                    let chat = format!("[种植] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\n{msg}"));
                                    }
                                }
                                Err(e) => {
                                    let chat = format!("[种植失败] {e}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\n❌ {e}"));
                                    }
                                }
                            }
                        }
                        BotCommand::Sleep => {
                            *state.mining_below.lock().unwrap() = false;
                            match crate::azalea::sleep::do_sleep(&bot).await {
                                Ok(msg) => {
                                    let chat = format!("[睡觉] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\n{msg}"));
                                    }
                                }
                                Err(e) => {
                                    let chat = format!("[睡觉失败] {e}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\n❌ {e}"));
                                    }
                                }
                            }
                        }
                        BotCommand::Harvest => {
                            *state.mining_below.lock().unwrap() = false;
                            match crate::azalea::harvest::do_harvest(&bot, 24).await {
                                Ok(msg) => {
                                    let chat = format!("[收割] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\n{msg}"));
                                    }
                                }
                                Err(e) => {
                                    let chat = format!("[收割失败] {e}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\n❌ {e}"));
                                    }
                                }
                            }
                        }
                        BotCommand::Chat { content } => {
                            bot.chat(&content);
                            if let Some(tx) = &result_tx {
                                let _ = tx.send(format!("Action output:\nSent chat: {content}"));
                            }
                        }
                        BotCommand::Attack { target } => {
                            if let Ok(entities) =
                            bot.nearest_entities::<bevy_ecs::query::Without<azalea::entity::metadata::Player>>()
                        {
                            let self_id = bot.entity().id();
                            let requested = normalize_entity_target(&target);
                            // 记录攻击前的生命，便于反馈损血
                            let health_before = bot.health().unwrap_or(20.0);
                            let mut hit_kind: Option<String> = None;
                            let mut nearest_match: Option<(i32, i32, i32, f64)> = None;
                            for e in entities.iter() {
                                if e.id() == self_id { continue; }
                                let Ok(kind) = e.kind() else { continue; };
                                let kind = entity_kind_name(kind);
                                if requested != "nearest" && requested != "chat" && kind != requested {
                                    continue;
                                }
                                if matches!(kind.as_str(), "item" | "experience_orb" | "item_frame" | "glow_item_frame") {
                                    continue;
                                }
                                let Ok(distance) = e.distance_to_client() else { continue; };
                                if nearest_match.is_none()
                                    && let Ok(position) = e.position()
                                {
                                    nearest_match = Some((
                                        position.x.floor() as i32,
                                        position.y.floor() as i32,
                                        position.z.floor() as i32,
                                        distance,
                                    ));
                                }
                                if distance > 4.5 {
                                    continue;
                                }
                                let indexed = bot
                                    .query_self::<&azalea::entity::indexing::EntityIdIndex, _>(|index| {
                                        index.contains_ecs_entity(e.id())
                                    })
                                    .unwrap_or(false);
                                if !indexed {
                                    continue;
                                }
                                e.attack();
                                hit_kind = Some(kind);
                                break;
                            }
                            let health_after = bot.health().unwrap_or(20.0);
                            let msg = match hit_kind {
                                Some(k) => {
                                    let dmg = (health_before - health_after).max(0.0);
                                    if dmg > 0.0 {
                                        format!("Action output:\nAttacked {k}. Took {dmg:.0} damage. Health: {health_after:.0}/20.")
                                    } else {
                                        format!("Action output:\nAttacked {k}. Health: {health_after:.0}/20.")
                                    }
                                }
                                None => match nearest_match {
                                    Some((x, y, z, distance)) => {
                                        // P76：远处实体攻击失败时按类型引导——LLM 曾连续 3+ 回合
                                        // 在树冠上追远处僵尸（11-22m），全 wasted（每回合 30-60s）。
                                        let is_hostile = matches!(
                                            requested.as_str(),
                                            "zombie"
                                                | "skeleton"
                                                | "creeper"
                                                | "spider"
                                                | "cave_spider"
                                                | "enderman"
                                                | "pillager"
                                                | "phantom"
                                                | "witch"
                                                | "drowned"
                                                | "husk"
                                                | "stray"
                                        );
                                        let guidance = if is_hostile {
                                            format!(
                                                "不要追击远处{requested}——追击引怪且浪费回合；远离它继续主线（如采集/合成/挖矿），它进入 4 格内时系统会自动反击。"
                                            )
                                        } else {
                                            format!(
                                                "动物在 {distance:.0}m 外：goto({x},{y},{z}) 靠近到 4 格内再 attack；动物会逃跑，靠近后立即攻击。"
                                            )
                                        };
                                        format!(
                                            "Action output:\nCould not attack {requested}: nearest match is {distance:.1} blocks away at ({x},{y},{z}). {guidance}"
                                        )
                                    }
                                    None => format!(
                                        "Action output:\nCould not find a valid {requested}. Use perceive to choose another action or flee if unsafe."
                                    ),
                                },
                            };
                            if let Some(tx) = &result_tx { let _ = tx.send(msg.clone()); }
                            let _ = evt_tx.send(BotEvent::Chat { content: msg });
                        }
                        }
                        // P113: 向远离指定实体的方向移动（moveaway [实体名] [距离]）。
                        // 定位最近的目标实体（无参=最近非玩家实体，排除物品/经验球），
                        // 计算水平反向向量 → goto bot_pos + 反向*distance（y 保持 bot 当前层）。
                        BotCommand::MoveAway { target, distance } => {
                            let distance = distance.clamp(4, 64);
                            let entity_pos: Option<(azalea::Vec3, String)> = {
                                let entities = bot.nearest_entities::<()>().ok();
                                let self_id = bot.entity().id();
                                let wanted = target.as_deref();
                                let mut best: Option<(f64, azalea::Vec3, String)> = None;
                                if let Some(entities) = entities {
                                    for e in entities.iter() {
                                        if e.id() == self_id {
                                            continue;
                                        }
                                        let Ok(kind) = e.kind() else {
                                            continue;
                                        };
                                        let kind = entity_kind_name(kind);
                                        if matches!(
                                            kind.as_str(),
                                            "item"
                                                | "experience_orb"
                                                | "item_frame"
                                                | "glow_item_frame"
                                        ) {
                                            continue;
                                        }
                                        let Ok(position) = e.position() else {
                                            continue;
                                        };
                                        let Ok(d) = e.distance_to_client() else {
                                            continue;
                                        };
                                        if let Some(wanted) = wanted {
                                            let player_name = e
                                                .component::<azalea::player::GameProfileComponent>()
                                                .map(|profile| profile.name.clone())
                                                .unwrap_or_default();
                                            if player_name != wanted && kind != wanted {
                                                continue;
                                            }
                                        }
                                        let replace = match &best {
                                            Some((bd, _, _)) => d < *bd,
                                            None => true,
                                        };
                                        if replace {
                                            best = Some((d, position, kind));
                                        }
                                    }
                                }
                                best.map(|(_, p, k)| (p, k))
                            };
                            match entity_pos {
                                Some((ep, kind)) => match bot.position() {
                                    Ok(bot_pos) => {
                                        let dx = bot_pos.x - ep.x;
                                        let dz = bot_pos.z - ep.z;
                                        let dist = (dx * dx + dz * dz).sqrt();
                                        let (nx, nz) = if dist < 0.01 {
                                            (1.0, 0.0)
                                        } else {
                                            (dx / dist, dz / dist)
                                        };
                                        let tx_x = bot_pos.x + nx * distance as f64;
                                        let tx_z = bot_pos.z + nz * distance as f64;
                                        *state.action_mgr.pending.lock().unwrap() =
                                            Some(QueuedCommand {
                                                cmd: BotCommand::Goto {
                                                    x: tx_x.floor() as i32,
                                                    y: bot_pos.y.floor() as i32,
                                                    z: tx_z.floor() as i32,
                                                },
                                                result_tx: result_tx.clone(),
                                            });
                                        let _ = evt_tx.send(BotEvent::Chat {
                                                content: format!(
                                                    "[远离] 远离 {kind} -> 反向 {distance}m 目标 ({},{},{})",
                                                    tx_x.floor() as i32,
                                                    bot_pos.y.floor() as i32,
                                                    tx_z.floor() as i32
                                                ),
                                            });
                                    }
                                    Err(_) => {
                                        let _ = evt_tx.send(BotEvent::Chat {
                                            content: "[远离失败] 读取坐标失败".to_string(),
                                        });
                                        state.action_mgr.clear_pending();
                                    }
                                },
                                None => {
                                    let who =
                                        target.clone().unwrap_or_else(|| "任何实体".to_string());
                                    let msg = format!("附近找不到目标实体 {who}，无需远离。");
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\n{msg}"));
                                    }
                                    state.action_mgr.clear_pending();
                                }
                            }
                        }
                        // P116: 开关自动反应式模式（setmode <模式> on|off / setmode list）。
                        // mode="list" 仅查询当前禁用集合，不修改。
                        BotCommand::SetMode { mode, enabled } => {
                            const SWITCHABLE: [&str; 5] = [
                                "self_preservation",
                                "self_defense",
                                "cowardice",
                                "hunting",
                                "item_collecting",
                            ];
                            let (msg, chat): (String, String) = if mode == "list" {
                                let disabled = state.mode_switches.lock().unwrap().clone();
                                if disabled.is_empty() {
                                    ("全部自动模式已启用".to_string(), String::new())
                                } else {
                                    let list = {
                                        let mut v: Vec<String> = disabled.iter().cloned().collect();
                                        v.sort();
                                        v.join(", ")
                                    };
                                    (format!("已禁用的自动模式: {list}"), String::new())
                                }
                            } else if SWITCHABLE.contains(&mode.as_str()) {
                                if enabled {
                                    let removed =
                                        state.mode_switches.lock().unwrap().remove(mode.as_str());
                                    if removed {
                                        (
                                            format!("已启用自动模式 {mode}"),
                                            format!("[模式] 自动模式 {mode} 已启用"),
                                        )
                                    } else {
                                        (format!("自动模式 {mode} 本来就是启用的"), String::new())
                                    }
                                } else {
                                    let added =
                                        state.mode_switches.lock().unwrap().insert(mode.clone());
                                    if added {
                                        (
                                            format!("已禁用自动模式 {mode}"),
                                            format!("[模式] 自动模式 {mode} 已禁用"),
                                        )
                                    } else {
                                        (format!("自动模式 {mode} 本来就被禁用"), String::new())
                                    }
                                }
                            } else {
                                (
                                    format!(
                                        "不支持的自动模式: {mode}（可开关: {}）",
                                        SWITCHABLE.join("/")
                                    ),
                                    String::new(),
                                )
                            };
                            if let Some(tx) = &result_tx {
                                let _ = tx.send(format!("Action output:\n{msg}"));
                            }
                            if !chat.is_empty() {
                                let _ = evt_tx.send(BotEvent::Chat { content: chat });
                            }
                            state.action_mgr.clear_pending();
                        }
                        // P118：使用/投掷手持物品（末影之眼定位要塞等）。
                        // 装备 → 可选转视角 → 右键使用一次 → 验证物品消耗。
                        BotCommand::UseItem { item, yaw, pitch } => {
                            match ItemKind::from_str(&crate::azalea::recipe_book::normalize_item(
                                &item,
                            ))
                            .or_else(|_| ItemKind::from_str(&item))
                            {
                                Ok(kind) => {
                                    let eq = do_equip(&bot, &item, "hand").await;
                                    if !eq.starts_with("已装备") {
                                        let msg = format!("使用 {item} 失败：{eq}");
                                        if let Some(tx) = &result_tx {
                                            let _ = tx.send(format!("Action output:\n{msg}"));
                                        }
                                        let _ = evt_tx.send(BotEvent::Chat {
                                            content: format!("[使用] {msg}"),
                                        });
                                        state.action_mgr.clear_pending();
                                    } else {
                                        let orig = bot.direction().ok();
                                        if let (Some(y), Some(p)) = (yaw, pitch) {
                                            let _ = bot.set_direction(y, p);
                                            sleep(Duration::from_millis(150)).await;
                                        }
                                        // P118 修复：azalea 的 start_use_item() 会 raycast，
                                        // 命中方块时发 ServerboundUseItemOn（右键方块），服务端
                                        // 不会消耗/投掷投掷物（末影之眼等）——表现为"数量未变化"。
                                        // 检测 hit_result，命中方块/实体时自动改向上瞄准（P8 同款），
                                        // 保证发 ServerboundUseItem（右键空气）。
                                        let mut aim_up = false;
                                        if let Ok(hit) = bot.hit_result() {
                                            match hit {
                                                HitResult::Block(r) => aim_up = !r.miss,
                                                HitResult::Entity(_) => aim_up = true,
                                            }
                                        }
                                        if aim_up {
                                            let y = orig.map(|o| o.y_rot()).unwrap_or(0.0);
                                            let _ = bot.set_direction(y, -89.0);
                                            sleep(Duration::from_millis(150)).await;
                                        }
                                        let before = count_item(&bot, kind);
                                        bot.start_use_item();
                                        // 服务端消耗同步可能有延迟，最多等 1.5s 确认消耗
                                        let mut after = count_item(&bot, kind);
                                        for _ in 0..5 {
                                            if after < before {
                                                break;
                                            }
                                            sleep(Duration::from_millis(300)).await;
                                            after = count_item(&bot, kind);
                                        }
                                        if let Some(o) = orig {
                                            let _ = bot.set_direction(o.y_rot(), o.x_rot());
                                        }
                                        let msg = if after < before {
                                            format!("已使用 {item}（消耗 1，背包剩余 {after}）")
                                        } else if aim_up {
                                            format!(
                                                "已使用 {item}（朝向命中方块/实体，已自动改向上使用；物品数量未变化，可能未消耗）"
                                            )
                                        } else {
                                            format!(
                                                "已右键使用 {item}（物品数量未变化，可能未消耗）"
                                            )
                                        };
                                        if let Some(tx) = &result_tx {
                                            let _ = tx.send(format!("Action output:\n{msg}"));
                                        }
                                        let _ = evt_tx.send(BotEvent::Chat {
                                            content: format!("[使用] {msg}"),
                                        });
                                        state.action_mgr.clear_pending();
                                    }
                                }
                                Err(_) => {
                                    let msg = format!("未知物品 {item}");
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\n{msg}"));
                                    }
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!("[使用] {msg}"),
                                    });
                                    state.action_mgr.clear_pending();
                                }
                            }
                        }
                        // P119：拉弓射箭（龙战远程必需）。装备弓 → 检查箭 → 可选转向目标
                        // → 拉弦 ~1s（循环 start_use_item，P8 模式）→ 放箭（stop_use_item，
                        // azalea 魔改新增的 ReleaseUseItem 支持）→ 验证箭数消耗。
                        BotCommand::Shoot { target } => {
                            let eq = do_equip(&bot, "bow", "hand").await;
                            if !eq.starts_with("已装备") {
                                let msg = format!("射击失败：{eq}（需要弓）");
                                if let Some(tx) = &result_tx {
                                    let _ = tx.send(format!("Action output:\n{msg}"));
                                }
                                let _ = evt_tx.send(BotEvent::Chat {
                                    content: format!("[射击] {msg}"),
                                });
                                state.action_mgr.clear_pending();
                            }
                            let arrow_kind =
                                ItemKind::from_str("arrow").expect("arrow is a valid item kind");
                            let arrows_before = count_item(&bot, arrow_kind);
                            if arrows_before == 0 {
                                let msg = "射击失败：背包没有箭（arrow）。请先合成/获取箭（flint + stick + feather）。"
                                    .to_string();
                                if let Some(tx) = &result_tx {
                                    let _ = tx.send(format!("Action output:\n{msg}"));
                                }
                                let _ = evt_tx.send(BotEvent::Chat {
                                    content: format!("[射击] {msg}"),
                                });
                                state.action_mgr.clear_pending();
                            }
                            // 可选：转向目标实体（眼睛高度差 + 弹道平射近似）。
                            let orig = bot.direction().ok();
                            let mut aimed_at: Option<String> = None;
                            if let Some(t) = target.clone()
                                && let Some((yaw, pitch)) = look_at_nearest_entity(&bot, &t).await
                            {
                                let _ = bot.set_direction(yaw, pitch);
                                sleep(Duration::from_millis(150)).await;
                                aimed_at = Some(t.clone());
                            }
                            // P118 教训：命中方块时 start_use_item 发 ServerboundUseItemOn，
                            // 服务端不拉弓。射箭必须瞄准空旷处，命中方块时明确报错。
                            let mut blocked = false;
                            if let Ok(hit) = bot.hit_result()
                                && let HitResult::Block(r) = hit
                            {
                                blocked = !r.miss;
                            }
                            if blocked {
                                let msg = "射击失败：当前朝向命中方块，无法拉弓。请先移动到开阔处或调整视角再射。".to_string();
                                if let Some(o) = orig {
                                    let _ = bot.set_direction(o.y_rot(), o.x_rot());
                                }
                                if let Some(tx) = &result_tx {
                                    let _ = tx.send(format!("Action output:\n{msg}"));
                                }
                                let _ = evt_tx.send(BotEvent::Chat {
                                    content: format!("[射击] {msg}"),
                                });
                                state.action_mgr.clear_pending();
                            }
                            // 拉弦：循环 start_use_item 模拟按住右键（P8 模式，~1s 满蓄力）。
                            for _ in 0..20 {
                                bot.start_use_item();
                                sleep(Duration::from_millis(50)).await;
                            }
                            // 松弦：直接发 ReleaseUseItem 包（azalea 官方 API write_packet，
                            // 不依赖本地 vendor 扩展——vendor 保持官方 c35b57e 可随上游更新）。
                            bot.write_packet(ServerboundPlayerAction {
                                action: ServerboundPlayerActionKind::ReleaseUseItem,
                                pos: BlockPos::new(0, 0, 0),
                                direction: Direction::Down,
                                seq: 0,
                            });
                            // 放箭后服务端同步箭数可能有延迟，轮询最多 1.5s。
                            let mut arrows_after = count_item(&bot, arrow_kind);
                            for _ in 0..5 {
                                if arrows_after < arrows_before {
                                    break;
                                }
                                sleep(Duration::from_millis(300)).await;
                                arrows_after = count_item(&bot, arrow_kind);
                            }
                            if let Some(o) = orig {
                                let _ = bot.set_direction(o.y_rot(), o.x_rot());
                            }
                            let msg = match aimed_at {
                                Some(t) => {
                                    if arrows_after < arrows_before {
                                        format!(
                                            "已朝 {t} 射出一支箭（消耗 1，背包剩余 {arrows_after}）"
                                        )
                                    } else {
                                        format!(
                                            "已朝 {t} 放箭（箭数未变化 {arrows_after}，可能未命中目标或未消耗）"
                                        )
                                    }
                                }
                                None => {
                                    if arrows_after < arrows_before {
                                        format!(
                                            "已朝当前方向射出一支箭（消耗 1，背包剩余 {arrows_after}）"
                                        )
                                    } else {
                                        format!(
                                            "已朝当前方向放箭（箭数未变化 {arrows_after}，可能未消耗）"
                                        )
                                    }
                                }
                            };
                            if let Some(tx) = &result_tx {
                                let _ = tx.send(format!("Action output:\n{msg}"));
                            }
                            let _ = evt_tx.send(BotEvent::Chat {
                                content: format!("[射击] {msg}"),
                            });
                            state.action_mgr.clear_pending();
                        }
                        BotCommand::Craft2x2 { item, count } => {
                            match crate::azalea::craft::do_craft_2x2(&bot, &item, count).await {
                                Ok(msg) => {
                                    let chat = format!("[合成] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\nSuccessfully crafted {item}, you now have it. ({msg})"));
                                    }
                                }
                                Err(e) => {
                                    let chat = format!("[合成失败] {e}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nFailed to craft {item}: {e}"
                                        ));
                                    }
                                }
                            }
                        }
                        BotCommand::Craft3x3 {
                            item,
                            count,
                            table_pos,
                        } => {
                            let hint = table_pos.map(|(x, y, z)| BlockPos::new(x, y, z));
                            // P1-4：自动放收桌流程（确保桌开 → 合成 → 关桌）
                            let table_open = crate::azalea::table_flow::ensure_table_open(
                                &bot,
                                "crafting_table",
                                hint,
                            )
                            .await;
                            let result = match table_open {
                                Ok(tp) => {
                                    let r = crate::azalea::craft::do_craft_3x3(
                                        &bot,
                                        &item,
                                        count,
                                        Some(tp),
                                    )
                                    .await;
                                    let _ =
                                        crate::azalea::table_flow::close_container_if_open(&bot);
                                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                                    r.map(|msg| {
                                        format!(
                                            "{msg}\n(桌位: ({},{},{}), 已自动关闭)",
                                            tp.x, tp.y, tp.z
                                        )
                                    })
                                }
                                Err(e) => Err(e),
                            };
                            match result {
                                Ok(msg) => {
                                    let chat = format!("[合成] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\nSuccessfully crafted {item}, you now have it. ({msg})"));
                                    }
                                }
                                Err(e) => {
                                    let chat = format!("[合成失败] {e}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nFailed to craft {item}: {e}"
                                        ));
                                    }
                                }
                            }
                        }
                        BotCommand::Smelt {
                            output,
                            fuel,
                            count,
                            table_pos,
                        } => {
                            let hint = table_pos.map(|(x, y, z)| BlockPos::new(x, y, z));
                            // P1-4：自动放收炉流程（确保炉开 → 熔炼 → 关炉）
                            let table_open =
                                crate::azalea::table_flow::ensure_table_open(&bot, "furnace", hint)
                                    .await;
                            let result = match table_open {
                                Ok(tp) => {
                                    let r =
                                        crate::azalea::craft::do_smelt(&bot, &output, &fuel, count)
                                            .await;
                                    let _ =
                                        crate::azalea::table_flow::close_container_if_open(&bot);
                                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                                    r.map(|msg| {
                                        format!(
                                            "{msg}\n(炉位: ({},{},{}), 已自动关闭)",
                                            tp.x, tp.y, tp.z
                                        )
                                    })
                                }
                                Err(e) => Err(e),
                            };
                            match result {
                                Ok(msg) => {
                                    let chat = format!("[熔炼] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\nSuccessfully smelted {output}, you now have it. ({msg})"));
                                    }
                                }
                                Err(e) => {
                                    let chat = format!("[熔炼失败] {e}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nFailed to smelt {output}: {e}"
                                        ));
                                    }
                                }
                            }
                        }
                        BotCommand::Gather { item, count } => {
                            // 用 smart_actions::collect_block_smart 替代 gather::do_gather：
                            // 支持别名展开（"oak_log" 匹配 9 种原木变体），多轮渐扩半径扫描。
                            match crate::azalea::smart_actions::collect_block_smart(
                                &bot, &item, count,
                            )
                            .await
                            {
                                Ok(msg) => {
                                    let chat = format!("[采集] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nSuccessfully gathered {item}, {msg}"
                                        ));
                                    }
                                }
                                Err(e) => {
                                    let chat = format!("[采集失败] {e}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nFailed to gather {item}: {e}"
                                        ));
                                    }
                                }
                            }
                        }
                        // P112: 搜块返回坐标（searchblock <方块> [半径]）。只扫描不挖掘，
                        // 输出按距离升序的坐标列表供 LLM 规划（对齐 Mindcraft searchForBlock）。
                        BotCommand::SearchBlock { item, radius } => {
                            let radius = radius.clamp(4, 96) as i32;
                            match crate::azalea::smart_actions::search_block_coords(
                                &bot, &item, radius, 8,
                            )
                            .await
                            {
                                Ok(hits) => {
                                    let lines: Vec<String> = hits
                                        .iter()
                                        .map(|(p, d)| {
                                            format!(
                                                "{item} @ ({},{},{}) 距离 {d:.1}m",
                                                p.x, p.y, p.z
                                            )
                                        })
                                        .collect();
                                    let msg = format!(
                                        "半径 {radius} 内找到 {} 处 {item}：\n{}",
                                        lines.len(),
                                        lines.join("\n")
                                    );
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!("[搜索] {msg}"),
                                    });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\n{msg}"));
                                    }
                                }
                                Err(e) => {
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!("[搜索失败] {e}"),
                                    });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\n{e}"));
                                    }
                                }
                            }
                            state.action_mgr.clear_pending();
                        }
                        BotCommand::Place { item, x, y, z } => {
                            match crate::azalea::place::do_place(
                                &bot,
                                &item,
                                BlockPos::new(x, y, z),
                            )
                            .await
                            {
                                Ok(msg) => {
                                    let chat = format!("[放置] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    // P9 修复（2026-07-26）：do_place 返回的 msg 已包含实际放置坐标
                                    // （可能因自动重定位与 LLM 给的 x,y,z 不同）。原代码在外面包一层
                                    // "Placed {item} at ({x},{y},{z})" 用的是 LLM 原始坐标，导致 LLM
                                    // 记住错误坐标 → 后续 open(原始坐标) 必然失败。
                                    // 现在直接透传 msg，让 LLM 看到真实放置坐标。
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(msg);
                                    }
                                }
                                Err(e) => {
                                    let chat = format!("[放置失败] {e}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\nFailed to place {item} at ({},{},{}): {e}", x, y, z));
                                    }
                                }
                            }
                        }
                        BotCommand::OpenContainer { x, y, z } => {
                            match crate::azalea::place::do_open_container(
                                &bot,
                                BlockPos::new(x, y, z),
                            )
                            .await
                            {
                                Ok(msg) => {
                                    let chat = format!("[开容器] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\nOpened container at ({},{},{}). ({msg})", x, y, z));
                                    }
                                }
                                Err(e) => {
                                    let chat = format!("[开容器失败] {e}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\nFailed to open container at ({},{},{}): {e}", x, y, z));
                                    }
                                }
                            }
                        }
                        BotCommand::AutoCraft { item, count } => {
                            match crate::azalea::auto_craft::do_auto_craft(&bot, &item, count).await
                            {
                                Ok(msg) => {
                                    let chat = format!("[自动合成] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nAuto-crafted {item}. ({msg})"
                                        ));
                                    }
                                }
                                Err(e) => {
                                    let chat = format!("[自动合成失败] {e}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nFailed to auto-craft {item}: {e}"
                                        ));
                                    }
                                }
                            }
                        }
                        BotCommand::Enchant { item, level } => {
                            match crate::azalea::craft::do_enchant(&bot, &item, level).await {
                                Ok(msg) => {
                                    let chat = format!("[附魔] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\nEnchanted {item} at level {level}. ({msg})"));
                                    }
                                }
                                Err(e) => {
                                    let chat = format!("[附魔失败] {e}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nFailed to enchant {item}: {e}"
                                        ));
                                    }
                                }
                            }
                        }
                        BotCommand::Trade { offer } => {
                            let ext = bot
                                .ecs
                                .read()
                                .resource::<crate::azalea::ext_state::BotExtResource>()
                                .0
                                .clone();
                            match crate::azalea::trade::do_trade(&bot, &ext, offer).await {
                                Ok(msg) => {
                                    let chat = format!("[交易] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nTrade offer {offer} completed. ({msg})"
                                        ));
                                    }
                                }
                                Err(e) => {
                                    let chat = format!("[交易失败] {e}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nFailed to trade offer {offer}: {e}"
                                        ));
                                    }
                                }
                            }
                        }
                        BotCommand::InteractEntity { kind } => {
                            let target = match kind.to_ascii_lowercase().as_str() {
                                "villager" => crate::azalea::trade::find_nearest_villager(&bot)
                                    .ok_or_else(|| "附近没有村民".to_string()),
                                other => {
                                    Err(format!("暂不支持的实体种类 {other}（目前仅 villager）"))
                                }
                            };
                            match target {
                                Ok(e) => {
                                    bot.entity_interact(e);
                                    let chat = format!("[交互] 已右键 {kind}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nInteracted with {kind}."
                                        ));
                                    }
                                }
                                Err(e) => {
                                    let chat = format!("[交互失败] {e}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nFailed to interact with {kind}: {e}"
                                        ));
                                    }
                                }
                            }
                        }
                        BotCommand::Pickup => {
                            match crate::azalea::smart_actions::pickup_nearby_items(&bot).await {
                                Ok(msg) => {
                                    let chat = format!("[捡物] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\n{msg}"));
                                    }
                                }
                                Err(e) => {
                                    if let Some(tx) = &result_tx {
                                        let _ =
                                            tx.send(format!("Action output:\nPickup failed: {e}"));
                                    }
                                }
                            }
                        }
                        BotCommand::Defend => {
                            match crate::azalea::smart_actions::defend_self(&bot).await {
                                Ok(msg) => {
                                    let chat = format!("[防御] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\n{msg}"));
                                    }
                                }
                                Err(e) => {
                                    if let Some(tx) = &result_tx {
                                        let _ =
                                            tx.send(format!("Action output:\nDefend failed: {e}"));
                                    }
                                }
                            }
                        }
                        BotCommand::Equip { item, slot } => {
                            let msg = do_equip(&bot, &item, &slot).await;
                            if let Some(tx) = &result_tx {
                                let _ = tx.send(format!("Action output:\n{msg}"));
                            }
                            let _ = evt_tx.send(BotEvent::Chat {
                                content: format!("[装备] {msg}"),
                            });
                        }
                        BotCommand::Discard { item, count } => {
                            let msg = do_discard(&bot, &item, count).await;
                            if let Some(tx) = &result_tx {
                                let _ = tx.send(format!("Action output:\n{msg}"));
                            }
                            let _ = evt_tx.send(BotEvent::Chat {
                                content: format!("[丢弃] {msg}"),
                            });
                        }
                        BotCommand::Consume { item } => {
                            let msg = do_consume(&bot, &item).await;
                            if let Some(tx) = &result_tx {
                                let _ = tx.send(format!("Action output:\n{msg}"));
                            }
                            let _ = evt_tx.send(BotEvent::Chat {
                                content: format!("[消耗] {msg}"),
                            });
                        }
                        BotCommand::ChestView { x, y, z } => {
                            match crate::azalea::chest::do_chest_view(&bot, BlockPos::new(x, y, z))
                                .await
                            {
                                Ok(msg) => {
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\n{msg}"));
                                    }
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!("[查看容器] {msg}"),
                                    });
                                }
                                Err(e) => {
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nFailed to view chest: {e}"
                                        ));
                                    }
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!("[查看容器失败] {e}"),
                                    });
                                }
                            }
                        }
                        BotCommand::ChestWithdraw {
                            x,
                            y,
                            z,
                            item,
                            count,
                        } => {
                            match crate::azalea::chest::do_chest_withdraw(
                                &bot,
                                BlockPos::new(x, y, z),
                                &item,
                                count,
                            )
                            .await
                            {
                                Ok(msg) => {
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\n{msg}"));
                                    }
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!("[取出] {msg}"),
                                    });
                                }
                                Err(e) => {
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nFailed to withdraw: {e}"
                                        ));
                                    }
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!("[取出失败] {e}"),
                                    });
                                }
                            }
                        }
                        BotCommand::ChestDeposit {
                            x,
                            y,
                            z,
                            item,
                            count,
                        } => {
                            match crate::azalea::chest::do_chest_deposit(
                                &bot,
                                BlockPos::new(x, y, z),
                                &item,
                                count,
                            )
                            .await
                            {
                                Ok(msg) => {
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\n{msg}"));
                                    }
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!("[存入] {msg}"),
                                    });
                                }
                                Err(e) => {
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nFailed to deposit: {e}"
                                        ));
                                    }
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!("[存入失败] {e}"),
                                    });
                                }
                            }
                        }
                        // P68：跟随玩家。设置 follow_target，handler 每 tick 读取目标坐标 goto。
                        BotCommand::Follow { target } => {
                            *state.follow_target.lock().unwrap() = Some(target.clone());
                            let who = target.clone().unwrap_or_else(|| "最近的玩家".to_string());
                            let msg = format!(
                                "已开始跟随 {who}（每 tick 自动走到其身边）。说 \"stop\" 或聊天 stop 可解除。"
                            );
                            if let Some(tx) = &result_tx {
                                let _ = tx.send(format!("Action output:\n{msg}"));
                            }
                            let _ = evt_tx.send(BotEvent::Chat {
                                content: format!("[跟随] {msg}"),
                            });
                            state.action_mgr.clear_pending();
                        }
                        // P68：停止跟随。
                        BotCommand::StopFollow => {
                            *state.follow_target.lock().unwrap() = None;
                            let msg = "已停止跟随。";
                            if let Some(tx) = &result_tx {
                                let _ = tx.send(format!("Action output:\n{msg}"));
                            }
                            let _ = evt_tx.send(BotEvent::Chat {
                                content: format!("[跟随] {msg}"),
                            });
                            state.action_mgr.clear_pending();
                        }
                        // P68：把物品丢在玩家脚边。基于 Discard 能力，但目标坐标改为玩家位置。
                        BotCommand::Give {
                            item,
                            count,
                            target,
                        } => {
                            let target_name = target.as_deref();
                            match nearby_player_position(&bot, target_name) {
                                Some(initial_target) => {
                                    let initial_distance = bot
                                        .position()
                                        .ok()
                                        .map(|position| {
                                            ((position.x - initial_target.x).powi(2)
                                                + (position.y - initial_target.y).powi(2)
                                                + (position.z - initial_target.z).powi(2))
                                            .sqrt()
                                        })
                                        .unwrap_or(f64::INFINITY);
                                    if initial_distance > 2.0 {
                                        let goal = RadiusGoal {
                                            pos: initial_target,
                                            radius: 1.5,
                                        };
                                        let navigation = bot.goto(goal);
                                        if tokio::time::timeout(Duration::from_secs(10), navigation)
                                            .await
                                            .is_err()
                                        {
                                            bot.force_stop_pathfinding();
                                        }
                                    }

                                    // The player may move during navigation. Re-read both
                                    // positions and refuse to drop at a stale destination.
                                    let final_target = nearby_player_position(&bot, target_name);
                                    let final_distance = bot.position().ok().zip(final_target).map(
                                        |(position, player)| {
                                            ((position.x - player.x).powi(2)
                                                + (position.y - player.y).powi(2)
                                                + (position.z - player.z).powi(2))
                                            .sqrt()
                                        },
                                    );
                                    if !final_distance.is_some_and(|distance| distance <= 2.0) {
                                        let distance = final_distance
                                            .map(|value| format!("{value:.1}m"))
                                            .unwrap_or_else(|| "未知".to_string());
                                        let msg = format!(
                                            "给予失败：导航后仍距玩家 {distance}，为避免把物品丢在远处未执行 discard。"
                                        );
                                        if let Some(tx) = &result_tx {
                                            let _ = tx.send(format!("Action output:\n{msg}"));
                                        }
                                        let _ = evt_tx.send(BotEvent::Chat {
                                            content: format!("[给予失败] {msg}"),
                                        });
                                    } else {
                                        let dmsg = do_discard(&bot, &item, count).await;
                                        let msg = format!(
                                            "已把 {item} x{count} 丢在玩家附近（距离确认 <=2m）：{dmsg}"
                                        );
                                        if let Some(tx) = &result_tx {
                                            let _ = tx.send(format!("Action output:\n{msg}"));
                                        }
                                        let _ = evt_tx.send(BotEvent::Chat {
                                            content: format!("[给予] {msg}"),
                                        });
                                    }
                                }
                                None => {
                                    let msg = "附近没有可给予的其他玩家（需同一世界且可见）。";
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\n{msg}"));
                                    }
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!("[给予失败] {msg}"),
                                    });
                                }
                            }
                            state.action_mgr.clear_pending();
                        }
                    }
                    // 非轮询命令（异步/即时）执行完即清空中途槽与 busy，让队列推进下一条。
                    {
                        if let Some(qc) = state.action_mgr.peek_pending()
                            && !matches!(
                                &qc.cmd,
                                BotCommand::Goto { .. }
                                    | BotCommand::Mine { .. }
                                    | BotCommand::MineBelow
                                    | BotCommand::MineAbove
                            )
                        {
                            state.action_mgr.clear_pending();
                        }
                    }
                }
                // 持续下挖：只要标志为真且当前未在挖，就续挖（对齐 POC 逻辑，
                // 避免单次 start_mining 因中断失效导致 bot 停在原地不下降）。
                // **Y 下限保护**：Y<=-61 是深板岩+基岩层（1.18+ 基岩层 Y=-64~-59），
                // 继续下挖毫无意义且徒手挖深板岩极慢。到达后自动停止 mining_below 并提示。
                if *state.mining_below.lock().unwrap()
                    && !bot.is_mining()
                    && let Ok(p) = bot.position()
                {
                    let y = p.y.floor() as i32;
                    if y <= -61 {
                        // 到达深岩层，停止下挖
                        *state.mining_below.lock().unwrap() = false;
                        let _ = state.evt_tx.send(BotEvent::Chat {
                            content: format!(
                                "Action output:\nMineBelow stopped at Y={y} (深板岩/基岩层，继续下挖无意义)。\
                                 当前坐标 ({:.0},{y},{:.0})。建议改用 mine(x,y,z) 精确挖附近矿石，或 goto 上返回地面。",
                                p.x, p.z
                            ),
                        });
                    } else {
                        let foot = BlockPos::new(
                            p.x.floor() as i32,
                            (p.y - 1.0).floor() as i32,
                            p.z.floor() as i32,
                        );
                        bot.start_mining(foot);
                        // P93：mine_below 进度（每 20 tick）
                        if bot.ticks_connected().is_multiple_of(20) {
                            let _ = state.evt_tx.send(BotEvent::Progress {
                                command: "mine_below".into(),
                                detail: format!("当前 Y={y}，持续下挖中"),
                            });
                        }
                    }
                }
                // 持续上挖：mining_above 标志为真时，让 pathfinder 自动挖通头顶并 ascend。
                // **关键修复**：1x1 竖井里 bot 跳跃无法上升（物理限制），必须用 pathfinder
                //              的 ascend_move 让 bot 走到旁边一格的上方。pathfinder allow_mining=true
                //              会自动挖通 head + head+1 + 旁边方块让 bot ascend。
                // **YGoal**：用 YGoal(y+5) 而不是 BlockPosGoal，让 pathfinder 在水平方向自由选择
                //            最容易挖通的柱子，避免 1x1 竖井里 BlockPosGoal 算不出路径。
                // **Y 上限保护**：Y>=62（地表海平面）认为脱困，停止。
                // **大 timeout**：挖通深板岩需要计算长路径，默认 5s 不够，改为 30s。
                if *state.mining_above.lock().unwrap()
                    && let Ok(p) = bot.position()
                {
                    let t = bot.ticks_connected();
                    let y = p.y.floor() as i32;
                    let cx = p.x.floor() as i32;
                    let cz = p.z.floor() as i32;
                    // P120b：无镐软土柱绕行。目标软土柱已设置时，先走到柱脚下
                    // （水平距离 >1.5 格就 goto），到达后清除目标，让上方
                    // YGoal 循环从软土柱向上挖（软土徒手 ~0.25s/格）。
                    let soft_col = *state.mining_above_soft_column.lock().unwrap();
                    if let Some(col) = soft_col {
                        let d2 = (col.x - cx).pow(2) + (col.z - cz).pow(2);
                        if d2 > 2 {
                            // 每 20 tick 重新发起 goto（pathfinder 可能被硬墙挡回）
                            if t.is_multiple_of(20)
                                && !bot.is_calculating_path()
                                && !bot.is_executing_path()
                            {
                                use azalea::pathfinder::PathfinderOpts;
                                use std::time::Duration;
                                let opts = PathfinderOpts::new()
                                    .allow_mining(true)
                                    .min_timeout(Duration::from_secs(2))
                                    .max_timeout(Duration::from_secs(30));
                                bot.start_goto_with_opts(
                                    BlockPosGoal(BlockPos::new(col.x, y, col.z)),
                                    opts,
                                );
                            }
                        } else {
                            // 已到达软土柱脚下：清除目标，正常 YGoal 逻辑接管。
                            *state.mining_above_soft_column.lock().unwrap() = None;
                        }
                    }
                    // Throttle surface detection to every 5 ticks to reduce per-tick
                    // world reads (6 block reads per check) and avoid GameTick lag.
                    if t.is_multiple_of(5) {
                        // Air alone only proves that the bot entered a cave. Require a
                        // plausible overworld surface elevation before ending ascent.
                        let head_pos = BlockPos::new(cx, y + 1, cz);
                        let head_is_air = bot
                            .world()
                            .ok()
                            .and_then(|w| w.read().get_block_state(head_pos))
                            .map(|s| s.is_air())
                            .unwrap_or(false);
                        // Check an open column so a two-block tunnel at sea level does
                        // not get reported as the surface.
                        let mut five_air = true;
                        for dy in 1..=5 {
                            let check = BlockPos::new(cx, y + dy, cz);
                            let is_air = bot
                                .world()
                                .ok()
                                .and_then(|w| w.read().get_block_state(check))
                                .map(|s| s.is_air())
                                .unwrap_or(false);
                            if !is_air {
                                five_air = false;
                                break;
                            }
                        }
                        if mine_above_reached_surface(y, head_is_air, five_air) {
                            *state.mining_above.lock().unwrap() = false;
                            let _ = state.evt_tx.send(BotEvent::Chat {
                                content: format!(
                                    "Action output:\nMineAbove done at Y={y} (已到地表，头顶是空气)。\
                                     当前坐标 ({:.0},{y},{:.0})，可继续探索。",
                                    p.x, p.z
                                ),
                            });
                        } else if y >= 320 {
                            *state.mining_above.lock().unwrap() = false;
                            let _ = state.evt_tx.send(BotEvent::Chat {
                                content: format!(
                                    "Action output:\nMineAbove stopped at Y={y} (建筑高度上限)。\
                                     当前坐标 ({:.0},{y},{:.0})。",
                                    p.x, p.z
                                ),
                            });
                        }
                    }
                    // auto_equip is expensive (inventory scan), throttle to every 20 ticks.
                    if t.is_multiple_of(20) {
                        let _ = auto_equip_best_pickaxe(&bot).await;
                        // P93：mine_above 进度（与 auto_equip 同节流）
                        let _ = state.evt_tx.send(BotEvent::Progress {
                            command: "mine_above".into(),
                            detail: format!("当前 Y={y}，正在向上挖掘"),
                        });
                    }
                    // P60b: 强制楼梯脱困。当 bot 在 2 格高空气袋里（头顶是空气），
                    // pathfinder 用 YGoal 算出的路径"reached"却不会真正上升（因为
                    // 上方 y+2 是实心方块，bot 无法踏入）。这里每 4 tick 主动挖掉
                    // 头顶上方那格 (y+2)，打开竖井，让 bot 能站到 y+1；
                    // 同时发起一个 goto 到自身上方一格，触发真正的上升。
                    let p60b_head_air = bot
                        .world()
                        .ok()
                        .and_then(|w| w.read().get_block_state(BlockPos::new(cx, y + 1, cz)))
                        .map(|s| s.is_air())
                        .unwrap_or(false);
                    // P120b：正在绕行软土柱时跳过正常上挖逻辑（P60b/ceiling/YGoal
                    // 会覆盖绕行 goto 或徒手硬挖硬块——绕行期只走位不挖）。
                    let soft_col_active = state.mining_above_soft_column.lock().unwrap().is_some();
                    if p60b_head_air && !soft_col_active {
                        let above_head = BlockPos::new(cx, y + 2, cz);
                        let above_is_solid = bot
                            .world()
                            .ok()
                            .and_then(|w| w.read().get_block_state(above_head))
                            .map(|s| !s.is_air())
                            .unwrap_or(false);
                        if above_is_solid && !bot.is_mining() {
                            // P105：P60b 挖 y+2 前检查镐。入口的镐检查只看头顶（head_is_hard），
                            // 头顶是空气时被跳过——但 y+2 可能是硬方块，无镐徒手挖不动
                            // （~8s/格）→ 空转 10s 后超时，且失败消息误导 LLM 横向找路。
                            // P120：不再提前终止——徒手挖硬方块是可行逃生路径（慢但挖得动），
                            // 警告后继续挖；由 MineAbove 超时兜底，不做硬拒绝。
                            let above_hard = bot
                                .world()
                                .ok()
                                .and_then(|w| w.read().get_block_state(above_head))
                                .map(is_hard_block)
                                .unwrap_or(false);
                            if above_hard
                                && t.is_multiple_of(20)
                                && !has_any_pickaxe_in_inventory(&bot).await
                                && !*state.mining_above_no_pick_warned.lock().unwrap()
                            {
                                *state.mining_above_no_pick_warned.lock().unwrap() = true;
                                let _ = state.evt_tx.send(BotEvent::Chat {
                                    content: "⚠️ 上方 y+2 是硬方块且背包无镐：徒手慢速挖掘（~8秒/格，不掉落物）。逃生通道可行但慢。".to_string(),
                                });
                            }
                            bot.start_mining(above_head);
                        } else if !above_is_solid {
                            // P107: 头顶 y+2 也是空气——可能身处高穹顶洞穴腔体
                            // （空气袋不止 2 格高，上方远处才是硬方块天花板）。
                            // 实机复现（2026-08-03 tier3_bread）：mine_above 在
                            // lush_caves 腔体 10s 空转超时，pathfinder 反复
                            // "incomplete path"——LLM 盲猜 goto 地表坐标反而更糟。
                            // 这里向上扫描 y+2..y+8 找第一个实心方块（天花板）：
                            //   1. 天花板存在 → 挖穿它（若硬方块且无镐，P105 同款
                            //      提前终止给明确反馈，不再空转 10s）；
                            //   2. 0..8 全空气 → 已到开阔空间/地表，交给 YGoal 上升
                            //      （P106 原逻辑）。
                            // 每 4 tick 扫描一次（与 P60b 原节流一致）。
                            let ceiling = (2..=8).find_map(|dy| {
                                let check = BlockPos::new(cx, y + dy, cz);
                                bot.world()
                                    .ok()
                                    .and_then(|w| w.read().get_block_state(check))
                                    .filter(|s| !s.is_air())
                                    .map(|_| check)
                            });
                            if let Some(_cpos) = ceiling {
                                // 有天花板：优先挖穿（软硬方块都用镐/徒手，由
                                // is_hard_block + 镐检查决定是否值得挖）。
                                // P120：无镐不再提前终止——徒手挖穿（慢但可行），
                                // 由 MineAbove 超时兜底，不做硬拒绝。
                                let cpos = _cpos;
                                let chard = bot
                                    .world()
                                    .ok()
                                    .and_then(|w| w.read().get_block_state(cpos))
                                    .map(is_hard_block)
                                    .unwrap_or(false);
                                if chard
                                    && t.is_multiple_of(20)
                                    && !has_any_pickaxe_in_inventory(&bot).await
                                    && !*state.mining_above_no_pick_warned.lock().unwrap()
                                {
                                    *state.mining_above_no_pick_warned.lock().unwrap() = true;
                                    let _ = state.evt_tx.send(BotEvent::Chat {
                                        content: format!(
                                            "⚠️ 上方 y+{} 是硬方块天花板且背包无镐：徒手慢速挖穿（~8秒/格，不掉落物）。逃生通道可行但慢。",
                                            cpos.y - y
                                        ),
                                    });
                                }
                                if !bot.is_mining() {
                                    bot.start_mining(cpos);
                                }
                            } else if t.is_multiple_of(4) {
                                // 头顶上方已空：强制上升，真正脱困。P106：绝不能用
                                // BlockPosGoal(y+1)——目标格是 bot 头部所在格（空气），
                                // pathfinder 算不出站立路径 → empty path 卡满 10s，
                                // 且反复 goto 阻塞 40-tick 主循环的 YGoal(y+5) 兜底
                                // （L121 "Y did not increase" 真实根因）。
                                // 用 YGoal 只要求到达 y+2 高度（任意水平位置），
                                // pathfinder 可自由挖墙/找楼梯上升（同 P60 主循环）。
                                if !bot.is_calculating_path() && !bot.is_executing_path() {
                                    use azalea::pathfinder::PathfinderOpts;
                                    use std::time::Duration;
                                    let opts = PathfinderOpts::new()
                                        .allow_mining(true)
                                        .min_timeout(Duration::from_secs(1))
                                        .max_timeout(Duration::from_secs(10));
                                    bot.start_goto_with_opts(
                                        YGoal::from(BlockPos::new(cx, y + 2, cz)),
                                        opts,
                                    );
                                }
                            }
                        }
                    }
                    // An active goal with no calculation or execution can be
                    // permanent no-path retry. Reset it periodically instead of
                    // letting it suppress every future ascent attempt.
                    if !bot.is_calculating_path()
                        && !bot.is_executing_path()
                        && t.is_multiple_of(40)
                        && state.mining_above_soft_column.lock().unwrap().is_none()
                    {
                        use azalea::pathfinder::PathfinderOpts;
                        use std::time::Duration;
                        bot.force_stop_pathfinding();
                        let mut direction = state.mining_above_direction.lock().unwrap();
                        let directions = [(1, 0), (0, 1), (-1, 0), (0, -1)];
                        let (dx, dz) = directions[*direction % directions.len()];
                        *direction = (*direction + 1) % 4;
                        drop(direction);
                        // P60 关键修复：1x1 竖井里用 YGoal(y+5) 而不是 BlockPosGoal。
                        // BlockPosGoal 指向特定侧方方块，pathfinder 在 1x1 竖井里
                        // 算不出通往该固定坐标的路径（每根柱子都只有 1 格宽），
                        // 导致"reached end of path"却原地不动、永久卡死。
                        // YGoal 只要求到达 y+5 任意水平位置，pathfinder 可自由选择
                        // 最容易挖通的柱子上升，从而真正脱困。
                        let target = BlockPos::new(cx + dx, y + 5, cz + dz);
                        let opts = PathfinderOpts::new()
                            .allow_mining(true)
                            .min_timeout(Duration::from_secs(2))
                            .max_timeout(Duration::from_secs(30));
                        bot.start_goto_with_opts(YGoal::from(target), opts);
                    }
                }
                // P67：make_obsidian 状态机。每 tick 推进：
                //  phase 0：找附近（半径12）岩浆源；装备 water_bucket+diamond_pickaxe；
                //          在岩浆旁的空气块右键放水（block_interact 手持 water_bucket）→ 生成黑曜石。
                //  phase 1：等 ~4s（黑曜石生成）。
                //  phase 2：用 diamond_pickaxe 挖下黑曜石；remaining-1；回 phase 0。
                //  完成 remaining==0 或找不到岩浆/没水 → 结束并发结果。
                if let Some((remaining, phase, ob_pos)) = *state.make_obsidian.lock().unwrap() {
                    let t = bot.ticks_connected();
                    // P160：状态机超时护栏——启动后 >600 tick（30s）仍未完成（典型：
                    // 装水失败循环 / 找不到岩浆反复重试 / pathfinder 卡死），强制失败，
                    // 避免无限重试拖死 viewer API（每 tick block_interact+pathfinder）。
                    let start_tick = *state.make_obsidian_start_tick.lock().unwrap();
                    if start_tick.is_some_and(|s| t.saturating_sub(s) > 600) {
                        *state.make_obsidian_start_tick.lock().unwrap() = None;
                        *state.make_obsidian.lock().unwrap() = None;
                        let _ = state.evt_tx.send(BotEvent::Chat {
                            content: "Action output:\nmake_obsidian 超时（30s 未完成）：装水失败或附近无岩浆源。请确认手持 bucket 已装备、且 goto 到水源+岩浆源都在 16m/12m 内再重试。".to_string(),
                        });
                        // 跳出本轮（状态机已清空）
                    } else {
                        match phase {
                            0 => {
                                // P67c 同步装备水桶：tick handler 内严禁 await，这里用
                                // set_selected_hotbar_slot 同步把 bucket 切到主手（不等待服务端轮询）。
                                // 若 bucket 不在 hotbar，则同步 shift_click 到空 hotbar 槽。
                                if bot
                                    .get_held_item()
                                    .map(|s| {
                                        let k: azalea_registry::builtin::ItemKind = s.kind();
                                        k != azalea_registry::builtin::ItemKind::Bucket
                                            && k != azalea_registry::builtin::ItemKind::WaterBucket
                                    })
                                    .unwrap_or(true)
                                    && let Ok(inv) = bot.get_inventory()
                                {
                                    if let Some(h) = find_hotbar_slot_for(
                                        &inv,
                                        azalea_registry::builtin::ItemKind::Bucket,
                                    ) {
                                        bot.set_selected_hotbar_slot(h);
                                    } else if let Some(srcs) = Some(find_item_slots(
                                        &inv,
                                        azalea_registry::builtin::ItemKind::Bucket,
                                    )) && !srcs.is_empty()
                                    {
                                        let menu = inv.menu().ok().flatten();
                                        if let Some(menu) = menu {
                                            let hotbar_range = menu.hotbar_slots_range();
                                            if let Some(slots) = inv.slots() {
                                                let mut placed = false;
                                                for hb in hotbar_range {
                                                    if slots
                                                        .get(hb)
                                                        .map(|s| s.is_empty())
                                                        .unwrap_or(false)
                                                    {
                                                        inv.left_click(*srcs.first().unwrap());
                                                        inv.left_click(hb);
                                                        placed = true;
                                                        break;
                                                    }
                                                }
                                                let _ = placed;
                                            }
                                        }
                                    }
                                }
                                // 检查手持 water_bucket；没有则自动找水源装水（已装备 bucket）。
                                let held = bot
                                    .get_held_item()
                                    .map(|s| s.kind().to_string())
                                    .unwrap_or_default();
                                if !held.contains("water_bucket") {
                                    // 自动装水：扫描半径 16 内水源，对水块 block_interact（持 bucket 右键水→装水）
                                    if let (Ok(p), Ok(world)) = (bot.position(), bot.world()) {
                                        let wp = p.x.floor() as i32;
                                        let wy = p.y.floor() as i32;
                                        let wz = p.z.floor() as i32;
                                        let world = world.read();
                                        let mut water: Option<(i32, i32, i32)> = None;
                                        'wscan: for r in 1..=16i32 {
                                            for dx in -r..=r {
                                                for dy in -3..=4i32 {
                                                    for dz in -r..=r {
                                                        let wx = wp + dx;
                                                        let wy2 = wy + dy;
                                                        let wz2 = wz + dz;
                                                        if let Some(bs) = world.get_block_state(
                                                            BlockPos::new(wx, wy2, wz2),
                                                        ) {
                                                            let kind: azalea_registry::builtin::BlockKind =
                                                            bs.into();
                                                            if kind
                                                            == azalea_registry::builtin::BlockKind::Water
                                                        {
                                                            water = Some((wx, wy2, wz2));
                                                            break 'wscan;
                                                        }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        drop(world);
                                        match water {
                                            Some((wx, wy2, wz2)) => {
                                                bot.block_interact(BlockPos::new(wx, wy2, wz2));
                                                // 装水后下一 tick 再检查手持，进入岩浆逻辑
                                                *state.make_obsidian.lock().unwrap() =
                                                    Some((remaining, 0, None));
                                            }
                                            None => {
                                                let _ = state.evt_tx.send(BotEvent::Chat {
                                                content: "Action output:\nmake_obsidian 失败：附近（半径16）未找到水源。请先 goto 到河流/湖泊附近再调用。".to_string(),
                                            });
                                                *state.make_obsidian.lock().unwrap() = None;
                                            }
                                        }
                                    } else {
                                        *state.make_obsidian.lock().unwrap() = None;
                                    }
                                } else if let (Ok(p), Ok(world)) = (bot.position(), bot.world()) {
                                    let wp = p.x.floor() as i32;
                                    let wy = p.y.floor() as i32;
                                    let wz = p.z.floor() as i32;
                                    let world = world.read();
                                    // 扫描半径 12 内岩浆方块（Lava）；视作岩浆源处理。
                                    let mut found: Option<(i32, i32, i32)> = None;
                                    'scan: for r in 1..=12i32 {
                                        for dx in -r..=r {
                                            for dy in -2..=4i32 {
                                                for dz in -r..=r {
                                                    let lx = wp + dx;
                                                    let ly = wy + dy;
                                                    let lz = wz + dz;
                                                    if let Some(bs) = world
                                                        .get_block_state(BlockPos::new(lx, ly, lz))
                                                    {
                                                        let kind: azalea_registry::builtin::BlockKind =
                                                        bs.into();
                                                        if kind
                                                        == azalea_registry::builtin::BlockKind::Lava
                                                    {
                                                        // 找岩浆旁的空气邻居放水
                                                        for (nx, ny, nz) in [
                                                            (lx + 1, ly, lz),
                                                            (lx - 1, ly, lz),
                                                            (lx, ly, lz + 1),
                                                            (lx, ly, lz - 1),
                                                            (lx, ly + 1, lz),
                                                        ] {
                                                            if let Some(nb) = world.get_block_state(
                                                                BlockPos::new(nx, ny, nz),
                                                            ) && nb.is_air()
                                                            {
                                                                found = Some((nx, ny, nz));
                                                                break 'scan;
                                                            }
                                                        }
                                                    }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    drop(world);
                                    match found {
                                        Some((nx, ny, nz)) => {
                                            // 右键该空气块放水→黑曜石（需手持 water_bucket，由 LLM 保证）
                                            bot.block_interact(BlockPos::new(nx, ny, nz));
                                            *state.make_obsidian.lock().unwrap() =
                                                Some((remaining, 1, Some((nx, ny, nz))));
                                        }
                                        None => {
                                            let _ = state.evt_tx.send(BotEvent::Chat {
                                            content: "Action output:\nmake_obsidian 失败：附近（半径12）未找到岩浆源。请先 goto 到岩浆湖附近再调用。".to_string(),
                                        });
                                            *state.make_obsidian.lock().unwrap() = None;
                                        }
                                    }
                                }
                            }
                            1 => {
                                // 等 ~80 tick(4s) 让水与岩浆反应生成黑曜石。
                                // 用 ob_pos 记录起始 tick 比较麻烦，这里简单用 ticks%80==0 推进到挖阶段。
                                if t.is_multiple_of(80) || ob_pos.is_none() {
                                    if let Some((_nx, _ny, _nz)) = ob_pos {
                                        *state.make_obsidian.lock().unwrap() =
                                            Some((remaining, 2, ob_pos));
                                    } else {
                                        *state.make_obsidian.lock().unwrap() =
                                            Some((remaining, 0, None));
                                    }
                                }
                            }
                            2 => {
                                if let Some((nx, ny, nz)) = ob_pos {
                                    // 黑曜石生成在岩浆源处（邻居的反方向）。尝试挖 (nx, ny-1, nz) 及 ob_pos 自身。
                                    let targets = [(nx, ny - 1, nz), (nx, ny, nz)];
                                    let mut mined = false;
                                    if let Ok(world) = bot.world() {
                                        let world = world.read();
                                        for (tx, ty, tz) in targets {
                                            if let Some(bs) =
                                                world.get_block_state(BlockPos::new(tx, ty, tz))
                                            {
                                                let kind: azalea_registry::builtin::BlockKind =
                                                    bs.into();
                                                if kind
                                                    == azalea_registry::builtin::BlockKind::Obsidian
                                                {
                                                    bot.start_mining(BlockPos::new(tx, ty, tz));
                                                    mined = true;
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                    if mined {
                                        let _ = state.evt_tx.send(BotEvent::Chat {
                                            content: format!(
                                                "[造黑曜石] 已挖下 1 块黑曜石，剩余 {}",
                                                remaining.saturating_sub(1)
                                            ),
                                        });
                                        let left = remaining.saturating_sub(1);
                                        if left == 0 {
                                            let _ = state.evt_tx.send(BotEvent::Chat {
                                            content: "Action output:\nmake_obsidian 完成：已收集所需黑曜石。可用于搭建下界传送门框架。".to_string(),
                                        });
                                            *state.make_obsidian.lock().unwrap() = None;
                                        } else {
                                            *state.make_obsidian.lock().unwrap() =
                                                Some((left, 0, None));
                                        }
                                    } else {
                                        // 没生成黑曜石（可能水没流到岩浆），重试
                                        *state.make_obsidian.lock().unwrap() =
                                            Some((remaining, 0, None));
                                    }
                                } else {
                                    *state.make_obsidian.lock().unwrap() =
                                        Some((remaining, 0, None));
                                }
                            }
                            _ => {
                                *state.make_obsidian.lock().unwrap() = None;
                            }
                        }
                    } // P160 else 闭合（超时未触发时的正常推进）
                }
                // P60c: 地下强制楼梯脱困（无条件运行，不依赖 LLM 是否调用 mine_above）。
                // 当 bot 在地下 (Y<62) 且头顶是空气（处于 2 格高空气袋），持续挖掉头顶上方
                // 那格并走到上方一格，保证 bot 真正上升——即使 LLM 反复下发无效的地下
                // goto/mine，bot 也能稳定爬出竖井，避免永久困死在 Y=12。
                if let Ok(p) = bot.position() {
                    let y = p.y.floor() as i32;
                    // P161d：交互进行中（interact_hold_until 未过期）时暂停地下脱困，
                    // 避免装水/装岩浆/右键液体被 P60c 强制上升打断。
                    let interact_holding = state
                        .interact_hold_until
                        .lock()
                        .unwrap()
                        .is_some_and(|until| bot.ticks_connected() < until);
                    if y < 62 && !interact_holding {
                        let cx = p.x.floor() as i32;
                        let cz = p.z.floor() as i32;
                        let head_air = bot
                            .world()
                            .ok()
                            .and_then(|w| w.read().get_block_state(BlockPos::new(cx, y + 1, cz)))
                            .map(|s| s.is_air())
                            .unwrap_or(false);
                        if head_air && !bot.is_executing_path() && !bot.is_calculating_path() {
                            let above_head = BlockPos::new(cx, y + 2, cz);
                            let above_is_solid = bot
                                .world()
                                .ok()
                                .and_then(|w| w.read().get_block_state(above_head))
                                .map(|s| !s.is_air())
                                .unwrap_or(false);
                            if above_is_solid && !bot.is_mining() {
                                bot.start_mining(above_head);
                            } else if !above_is_solid {
                                // 头顶上方已空：强制上升。P106：同 P60b，用 YGoal 而非
                                // BlockPosGoal(y+1)（目标格是空气，pathfinder empty path）。
                                use azalea::pathfinder::PathfinderOpts;
                                use std::time::Duration;
                                let opts = PathfinderOpts::new()
                                    .allow_mining(true)
                                    .min_timeout(Duration::from_secs(1))
                                    .max_timeout(Duration::from_secs(10));
                                bot.start_goto_with_opts(
                                    YGoal::from(BlockPos::new(cx, y + 2, cz)),
                                    opts,
                                );
                            }
                        }
                        // 看门狗：完全卡死（头顶是实心、无法 ascent）时退回 mining_above 模式。
                        if !head_air
                            && !*state.mining_above.lock().unwrap()
                            && !bot.is_mining()
                            && bot.ticks_connected().is_multiple_of(20)
                        {
                            *state.mining_above.lock().unwrap() = true;
                            *state.mining_above_start_y.lock().unwrap() = Some(y);
                            *state.mining_above_direction.lock().unwrap() = 0;
                        }
                    }
                }
                // 每 20 tick 推送状态快照。
                let t = bot.ticks_connected();
                if t.is_multiple_of(20)
                    && let Ok(p) = bot.position()
                {
                    // 全量背包：列出所有非空格，**按物品 ID 聚合后输出**（旧版每个槽位单独
                    // 输出，导致 `dirt:46, dirt:64, leaflitter:64, leaflitter:26` 这种重复条目，
                    // LLM 困惑且浪费 token）。聚合后输出 `dirt:110, leaflitter:90`。
                    let (inventory, armor_str, hotbar_str) = match bot.get_inventory() {
                        Ok(inv) => match inv.slots() {
                            Some(slots) => {
                                // P56：Player 菜单槽位布局（azalea declare_menus!）：
                                // 0=craft_result, 1-4=craft, 5-8=armor(helmet/chestplate/
                                // leggings/boots), 9-44=inventory, 45=offhand。
                                // 原实现把 armor 槽混入"背包"聚合 → LLM 以为甲还在背包，
                                // 反复 equip 又因 find_item_slots(9-44) 找不到而报"背包未持有"
                                // → 死循环（实测甲已上身仍被反复驱赶）。现跳过 armor 槽并
                                // 单独产出装备摘要行。仅 Player 菜单布局固定，容器菜单跳过。
                                let is_player_menu = inv
                                    .menu()
                                    .ok()
                                    .flatten()
                                    .map(|m| matches!(m, azalea::inventory::Menu::Player(_)))
                                    .unwrap_or(false);
                                let mut agg: std::collections::HashMap<String, u32> =
                                    std::collections::HashMap::new();
                                let mut armor: [String; 4] = Default::default();
                                // P124：hotbar 槽位（azalea Player 菜单 36-44）单独汇总，
                                // 让 LLM 看到"hotbar 空/已有哪些物品"——装备主手只需 set_selected_hotbar_slot
                                // 或 shift_click 入 hotbar，无需先清空背包（P8 已处理 hotbar 满自动腾位）。
                                let mut hotbar_items: Vec<String> = Vec::new();
                                for (idx, s) in slots.iter().enumerate() {
                                    if s.is_empty() {
                                        continue;
                                    }
                                    // P5 关键修复：用 to_str() 返回 minecraft id（如 "minecraft:crafting_table"），
                                    // 然后 strip "minecraft:" 前缀得到 "crafting_table"。
                                    // 原代码用 format!("{:?}", s.kind()).to_lowercase() 得到 enum Debug 名
                                    // （如 "CraftingTable".to_lowercase() = "craftingtable"，无下划线），
                                    // 与工具/craft 配方表期望的 snake_case id 不匹配 → LLM 看到 "craftingtable"
                                    // 却 craft("crafting_table") 报"无此物品" → 100% 卡死。
                                    let kind_full = s.kind().to_str();
                                    let kind =
                                        kind_full.strip_prefix("minecraft:").unwrap_or(kind_full);
                                    let cnt = s.count() as u32;
                                    if is_player_menu && (5..=8).contains(&idx) {
                                        armor[idx - 5] = kind.to_string();
                                    } else {
                                        *agg.entry(kind.to_string()).or_insert(0) += cnt;
                                    }
                                    if is_player_menu && (36..=44).contains(&idx) {
                                        hotbar_items.push(format!("{kind} x{cnt}"));
                                    }
                                }
                                let hotbar_str = if hotbar_items.is_empty() {
                                    "空".to_string()
                                } else {
                                    hotbar_items.join(", ")
                                };
                                let inv_str = if agg.is_empty() {
                                    "空背包".to_string()
                                } else {
                                    // 按数量降序输出（多的在前，LLM 重点看前几个）
                                    let mut items: Vec<(String, u32)> = agg.into_iter().collect();
                                    items.sort_by_key(|x| std::cmp::Reverse(x.1));
                                    items
                                        .iter()
                                        .map(|(k, c)| format!("{k}:{c}"))
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                };
                                let display = |s: &String| {
                                    if s.is_empty() {
                                        "无".to_string()
                                    } else {
                                        s.clone()
                                    }
                                };
                                let armor_summary = format!(
                                    "头盔: {}, 胸甲: {}, 护腿: {}, 靴子: {}",
                                    display(&armor[0]),
                                    display(&armor[1]),
                                    display(&armor[2]),
                                    display(&armor[3])
                                );
                                (inv_str, armor_summary, hotbar_str)
                            }
                            None => (
                                "slots=None".to_string(),
                                "头盔: 无, 胸甲: 无, 护腿: 无, 靴子: 无".to_string(),
                                "空".to_string(),
                            ),
                        },
                        Err(_) => (
                            "获取失败".to_string(),
                            "头盔: 无, 胸甲: 无, 护腿: 无, 靴子: 无".to_string(),
                            "空".to_string(),
                        ),
                    };
                    let player_count = bot.nearby_players().map(|pp| pp.len()).unwrap_or(0);
                    // 朝向（yaw/pitch，度数）：从 LookDirection 的 Debug 输出解析（azalea 字段为私有，不改动库）。
                    let (yaw, pitch) = bot
                        .direction()
                        .map(|d| {
                            let s = format!("{d:?}");
                            let y = s
                                .split("y_rot: ")
                                .nth(1)
                                .and_then(|x| x.split(',').next())
                                .and_then(|x| x.trim().parse::<f64>().ok())
                                .unwrap_or(0.0);
                            let pi = s
                                .split("x_rot: ")
                                .nth(1)
                                .and_then(|x| x.split('}').next())
                                .and_then(|x| x.trim().parse::<f64>().ok())
                                .unwrap_or(0.0);
                            (y, pi)
                        })
                        .unwrap_or((0.0, 0.0));
                    // 脚下方块 + 前方 1 格方块（用于 bot 判断脚下是否悬空/面前是否墙）。
                    let block_name = |bp: BlockPos| -> String {
                        if let Ok(world) = bot.world() {
                            match world.read().get_block_state(bp) {
                                Some(s) if !s.is_air() => {
                                    let bk: BlockKind = s.into();
                                    // P5 修复：用 to_str() 拿到 minecraft id（如 "minecraft:stone"）。
                                    // 原代码 format!("{bk:?}").to_lowercase() 得到 "stone"（无前缀），
                                    // 但对于多词方块如 "GrassBlock".to_lowercase() = "grassblock"（无下划线），
                                    // 与工具/mem 期望的 snake_case id 不匹配。
                                    let k = bk.to_str();
                                    k.strip_prefix("minecraft:").unwrap_or(k).to_string()
                                }
                                _ => "air".to_string(),
                            }
                        } else {
                            "?".to_string()
                        }
                    };
                    let foot_y = (p.y - 1.0).floor() as i32;
                    let block_under = block_name(BlockPos::new(
                        p.x.floor() as i32,
                        foot_y,
                        p.z.floor() as i32,
                    ));
                    // 前方方块：由 yaw/pitch 推算视线落点（水平 1 格 + 俯仰修正）。
                    let rad = yaw.to_radians();
                    let dx = -rad.sin(); // 与 azalea 约定一致：yaw 0 朝 +Z
                    let dz = rad.cos();
                    let horiz = 1.0_f64.max((pitch.abs() / 90.0) * 2.0);
                    let ahead_x = (p.x + dx * horiz).floor() as i32;
                    let ahead_z = (p.z + dz * horiz).floor() as i32;
                    let ahead_y = (p.y + -(pitch / 90.0)).floor() as i32;
                    let block_ahead = block_name(BlockPos::new(ahead_x, ahead_y, ahead_z));
                    // 生命/饱食/主手/群系/附近方块
                    let health = bot.health().unwrap_or(20.0);
                    let hunger = bot.hunger().ok();
                    let food = hunger.as_ref().map(|h| h.food).unwrap_or(20);
                    let saturation = hunger.as_ref().map(|h| h.saturation).unwrap_or(5.0);
                    let held_item = match bot.get_held_item() {
                        Ok(item) if !item.is_empty() => {
                            // P5 修复：用 to_str() 拿到 minecraft id（同背包聚合逻辑）。
                            let k = item.kind().to_str();
                            k.strip_prefix("minecraft:").unwrap_or(k).to_string()
                        }
                        _ => "air".to_string(),
                    };
                    // biome 通过 registry 解析为可读 Identifier（如 "minecraft:dark_forest"）。
                    // 旧实现 `format!("{b:?}")` 会输出 "biome { id: 30 }" 这种调试串，LLM 看不懂。
                    let biome = bot
                        .world()
                        .ok()
                        .and_then(|w| {
                            w.read().get_biome(BlockPos::new(
                                p.x.floor() as i32,
                                p.y.floor() as i32,
                                p.z.floor() as i32,
                            ))
                        })
                        .and_then(|b| bot.resolve_registry_key(&b).ok().flatten())
                        .map(|key| key.into_ident().to_string())
                        .map(|s| {
                            // "minecraft:dark_forest" → "dark_forest"
                            s.strip_prefix("minecraft:")
                                .map(|x| x.to_string())
                                .unwrap_or(s)
                        })
                        .unwrap_or_else(|| "unknown".to_string());
                    // 附近方块摘要：3x3 地面区域
                    let nearby = {
                        let foot_x = p.x.floor() as i32;
                        let foot_z = p.z.floor() as i32;
                        let mut counts: HashMap<String, u32> = HashMap::new();
                        let world = bot.world().ok();
                        for dx in -1..=1 {
                            for dz in -1..=1 {
                                if let Some(ref w) = world {
                                    let bp = BlockPos::new(foot_x + dx, foot_y, foot_z + dz);
                                    let name = match w.read().get_block_state(bp) {
                                        Some(s) if !s.is_air() => {
                                            let bk: BlockKind = s.into();
                                            // P5 修复：用 to_str() 拿到 snake_case minecraft id
                                            let k = bk.to_str();
                                            k.strip_prefix("minecraft:").unwrap_or(k).to_string()
                                        }
                                        _ => "air".to_string(),
                                    };
                                    *counts.entry(name).or_insert(0) += 1;
                                }
                            }
                        }
                        let parts: Vec<String> = counts
                            .into_iter()
                            .filter(|(k, _)| k != "air")
                            .map(|(k, v)| format!("{k}:{v}"))
                            .collect();
                        if parts.is_empty() {
                            "air".to_string()
                        } else {
                            parts.join(", ")
                        }
                    };
                    // 结构化游戏状态 JSON（前端面板可视化）
                    let game_state = {
                        let inv_slots: Vec<serde_json::Value> = match bot.get_inventory() {
                            Ok(inv) => match inv.slots() {
                                Some(slots) => slots
                                    .iter()
                                    .enumerate()
                                    .map(|(i, s)| {
                                        let id = if s.is_empty() {
                                            "minecraft:air".to_string()
                                        } else {
                                            // P5 修复：to_str() 已返回 "minecraft:xxx"，不需要拼前缀
                                            s.kind().to_str().to_string()
                                        };
                                        let cnt = if s.is_empty() { 0 } else { s.count() };
                                        // P135：工具耐久（damage/max_damage），非工具为 0。
                                        // 供 perceive 显示主手/背包工具剩余耐久，避免耐久耗尽
                                        // 自动销毁（镐"神秘消失"）前 LLM 毫不知情。
                                        let (dmg, max) = item_durability(s).unwrap_or((0, 0));
                                        serde_json::json!({
                                            "slot": i,
                                            "id": id,
                                            "count": cnt,
                                            "dmg": dmg,
                                            "max": max,
                                        })
                                    })
                                    .collect(),
                                None => vec![],
                            },
                            Err(_) => vec![],
                        };
                        let xp = bot.experience().ok();
                        let dimension = bot
                            .world_name()
                            .map(|name| name.to_string())
                            .unwrap_or_else(|_| "unknown".to_string());
                        let portal_active = nearby_active_portal(
                            &bot,
                            BlockPos::new(
                                p.x.floor() as i32,
                                p.y.floor() as i32,
                                p.z.floor() as i32,
                            ),
                        );
                        let kill_counts = bot
                            .ecs
                            .read()
                            .resource::<crate::azalea::ext_state::BotExtResource>()
                            .0
                            .lock()
                            .unwrap()
                            .kill_counts
                            .clone();
                        serde_json::json!({
                            "inventory": inv_slots,
                            // P126d：当前执行动作标签（perceive「当前动作」行用，
                            // 对标 Mindcraft $ACTION）。无 pending 命令时为空串。
                            "current_action": current_action_label(&state.action_mgr).unwrap_or_default(),
                            // P124：hotbar（Player 菜单 36-44）摘要，供面板/API 查看。
                            "hotbar": inv_slots
                                .iter()
                                .filter(|s| {
                                    s.get("slot")
                                        .and_then(|v| v.as_u64())
                                        .map(|i| (36..=44).contains(&i) && s["count"].as_u64().unwrap_or(0) > 0)
                                        .unwrap_or(false)
                                })
                                .cloned()
                                .collect::<Vec<_>>(),
                            // P56：盔甲槽位（Player 菜单 5-8）单独列出，与背包区分。
                            "armor": inv_slots
                                .iter()
                                .filter(|s| {
                                    s.get("slot")
                                        .and_then(|v| v.as_u64())
                                        .map(|i| (5..=8).contains(&i))
                                        .unwrap_or(false)
                                })
                                .cloned()
                                .collect::<Vec<_>>(),
                            "experience_level": xp.as_ref().map(|e| e.level).unwrap_or(0),
                            "experience_progress": xp.as_ref().map(|e| e.progress).unwrap_or(0.0),
                            "held_item": held_item,
                            "selected_slot": bot.selected_hotbar_slot().unwrap_or(0),
                            "dimension": dimension,
                            "portal_active": portal_active,
                            "kill_counts": kill_counts,
                        })
                    };
                    // 回填世界记忆：更新当前位置锚点 + 扫描周边关键方块
                    if let Some(mem) = &state.memory {
                        let mp = MemoryPos::new(
                            p.x.floor() as i32,
                            p.y.floor() as i32,
                            p.z.floor() as i32,
                        );
                        mem.set_anchor("__self__", Some(mp), "当前位置");
                        record_surroundings(&bot, mem, &mp, &state.scanned);
                    }
                    // 10x10 范围方块扫描：列出所有非空气方块类型及计数
                    let nearby_blocks = {
                        let mut counts: HashMap<String, u32> = HashMap::new();
                        let world = bot.world().ok();
                        let cx = p.x.floor() as i32;
                        let cy = p.y.floor() as i32;
                        let cz = p.z.floor() as i32;
                        for dx in -5..=5 {
                            for dy in -5..=5 {
                                for dz in -5..=5 {
                                    if let Some(ref w) = world {
                                        let bp = BlockPos::new(cx + dx, cy + dy, cz + dz);
                                        let name = match w.read().get_block_state(bp) {
                                            Some(s) if !s.is_air() => {
                                                let bk: BlockKind = s.into();
                                                // P5 修复：用 to_str() 拿到 snake_case minecraft id
                                                let k = bk.to_str();
                                                k.strip_prefix("minecraft:")
                                                    .unwrap_or(k)
                                                    .to_string()
                                            }
                                            _ => continue,
                                        };
                                        *counts.entry(name).or_insert(0) += 1;
                                    }
                                }
                            }
                        }
                        let mut items: Vec<_> = counts.into_iter().collect();
                        items.sort_by_key(|x| std::cmp::Reverse(x.1));
                        items
                            .iter()
                            .map(|(k, v)| format!("{k}:{v}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    // 资源分类摘要：把 10x10 里的方块按 wood/stone/ore/other 分组，
                    // 让 WorldInfo 的 find_match_line 能为每类找到独立的 label 行，
                    // 避免【场景提示】里 Wood/Stone/Ore 三条都粘同一份 10x10 字符串。
                    let _resource_summary = {
                        let wood_kinds = [
                            "oaklog",
                            "darkoaklog",
                            "birchlog",
                            "sprucelog",
                            "acalog",
                            "junglelog",
                            "mangrovelog",
                            "cherrylog",
                            "oakplanks",
                            "darkoakplanks",
                        ];
                        let stone_kinds = [
                            "stone",
                            "cobblestone",
                            "dirt",
                            "grassblock",
                            "sand",
                            "gravel",
                            "andesite",
                            "granite",
                            "diorite",
                        ];
                        let ore_kinds = [
                            "coalore",
                            "ironore",
                            "copperore",
                            "goldore",
                            "diamondore",
                            "emeraldore",
                            "redstoneore",
                            "lapisore",
                            "netherquartzore",
                        ];
                        let mut wood = Vec::new();
                        let mut stone = Vec::new();
                        let mut ore = Vec::new();
                        for (k, v) in nearby_blocks.split(", ").map(|s| {
                            let mut it = s.split(':');
                            (
                                it.next().unwrap_or("").to_string(),
                                it.next().and_then(|x| x.parse::<u32>().ok()).unwrap_or(0),
                            )
                        }) {
                            if wood_kinds.iter().any(|x| *x == k) {
                                wood.push(format!("{k}:{v}"));
                            } else if stone_kinds.iter().any(|x| *x == k) {
                                stone.push(format!("{k}:{v}"));
                            } else if ore_kinds.iter().any(|x| *x == k) {
                                ore.push(format!("{k}:{v}"));
                            }
                        }
                        let mut lines = Vec::new();
                        if !wood.is_empty() {
                            lines.push(format!("木材: {}", wood.join(", ")));
                        }
                        if !stone.is_empty() {
                            lines.push(format!("石头: {}", stone.join(", ")));
                        }
                        if !ore.is_empty() {
                            lines.push(format!("矿石: {}", ore.join(", ")));
                        }
                        lines.join("\n")
                    };
                    // 附近实体列表：按类型分组计数 + 最小距离（仅感知半径内，避免 LLM 追逐远处实体）
                    // P74：加最近实例坐标——LLM 想找动物狩猎/避开怪物时可直接 goto，
                    // 此前只有距离没有方向（实测 LLM 在树冠上找不到食物来源）。
                    let nearby_entities = {
                        const PERCEPTION_RADIUS: f64 = 24.0;
                        let mut kinds: EntityAgg = HashMap::new();
                        if let Ok(entities) = bot.nearest_entities::<bevy_ecs::query::Without<azalea::entity::metadata::Player>>() {
                            let self_id = bot.entity().id();
                            for e in entities.iter() {
                                if e.id() == self_id { continue; }
                                let Ok(distance) = e.distance_to_client() else { continue; };
                                if distance > PERCEPTION_RADIUS { continue; }
                                let name = entity_kind_name(e.kind().unwrap_or(EntityKind::Pig));
                                let pos = e.position().ok().map(|p| {
                                    (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32)
                                });
                                let entry = kinds.entry(name).or_insert((0, distance, pos.unwrap_or((0,0,0))));
                                entry.0 += 1;
                                if distance < entry.1 {
                                    entry.1 = distance;
                                    if let Some(p) = pos { entry.2 = p; }
                                }
                            }
                        }
                        // 玩家分开计数
                        let player_count = bot.nearby_players().map(|pp| pp.len()).unwrap_or(0);
                        let mut parts: Vec<String> = Vec::new();
                        if player_count > 0 {
                            parts.push(format!("player:{}", player_count));
                        }
                        let mut items: Vec<_> = kinds.into_iter().collect();
                        items.sort_by_key(|x| std::cmp::Reverse(x.1.0));
                        for (k, (v, d, pos)) in items {
                            if v > 0 {
                                parts.push(format!("{k}:{v}@{d:.0}m@{pos:?}"));
                            }
                        }
                        if parts.is_empty() {
                            "无".to_string()
                        } else {
                            parts.join(", ")
                        }
                    };
                    // P83：头顶连续实心方块数——LLM 判断"能否 mine_above 挖出"的关键信号。
                    // bot 在深洞时 n 大（深埋），在洞穴/地表时 n=0。
                    let overhead_solid = {
                        let wx = p.x.floor() as i32;
                        let wz = p.z.floor() as i32;
                        let head_y = p.y.floor() as i32;
                        match bot.world() {
                            Ok(world) => {
                                let w = world.read();
                                count_overhead_solid(|bp| w.get_block_state(bp), wx, head_y, wz)
                            }
                            Err(_) => 0,
                        }
                    };
                    let _ = evt_tx.send(BotEvent::State {
                        position: p,
                        inventory,
                        hotbar: hotbar_str,
                        armor: armor_str,
                        player_count,
                        yaw,
                        pitch,
                        block_under,
                        block_ahead,
                        health,
                        food,
                        saturation,
                        held_item,
                        biome,
                        nearby,
                        nearby_blocks,
                        nearby_entities,
                        overhead_solid,
                        game_state,
                    });
                }
                // ===== 反应式 modes（每 tick 检查，直接执行动作，不依赖 LLM）=====
                // self_preservation：检测火/岩浆，自动脱困
                // 使用 ActionManager 的 High 优先级抢占当前 pending（如正在合成时着火立即打断）
                // P116：set_mode 可禁用（mode_switches 集合）。
                if !state.mode_disabled("self_preservation")
                    && let Ok(p) = bot.position()
                {
                    let foot = BlockPos::new(
                        p.x.floor() as i32,
                        (p.y - 1.0).floor() as i32,
                        p.z.floor() as i32,
                    );
                    let head =
                        BlockPos::new(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
                    if let Ok(world) = bot.world() {
                        let under = world.read().get_block_state(foot);
                        let at = world.read().get_block_state(head);
                        let is_danger = |s: Option<azalea::block::BlockState>| -> bool {
                            s.map(|s| {
                                let bk: BlockKind = s.into();
                                matches!(
                                    bk,
                                    BlockKind::Lava | BlockKind::Fire | BlockKind::MagmaBlock
                                )
                            })
                            .unwrap_or(false)
                        };
                        if is_danger(under) || is_danger(at) {
                            let escape_cmd = BotCommand::Goto {
                                x: p.x.floor() as i32 + 5,
                                y: p.y.floor() as i32 + 1,
                                z: p.z.floor() as i32 + 5,
                            };
                            // 高优先级提交：若当前 pending 是 Normal（合成/采集等）则抢占
                            let tick_now = bot.ticks_connected();
                            let outcome = state.action_mgr.submit(
                                escape_cmd,
                                Priority::High,
                                &cmd_queue,
                                tick_now,
                            );
                            let preempt_msg = match outcome {
                                SubmitOutcome::Preempted(old) => {
                                    format!(
                                        "[MODE] 检测到火/岩浆，抢占当前命令 ({:?}) 自动脱困",
                                        old
                                    )
                                }
                                _ => "[MODE] 检测到火/岩浆，自动脱困".to_string(),
                            };
                            let _ = evt_tx.send(BotEvent::Chat {
                                content: preempt_msg,
                            });
                        }
                    }
                }
                // auto_eat：饥饿 ≤14 且背包有安全食物 → 自动进食（每 80 tick ≈4s 检查一次）。
                // P58：借鉴 Mindcraft autoEat（startAt=14 + bannedFood）。此前靠 LLM 手动
                // consume（30-60s/回合延迟），且 LLM 吃过 rotten_flesh（食物中毒风险）。
                // 仅空闲时执行（不打断 LLM 的 goto/挖矿/合成），安全白名单排除毒物。
                {
                    let hunger_now = bot.hunger().ok().map(|h| h.food).unwrap_or(20);
                    let auto_eat_ok = hunger_now <= 14
                        && state.action_mgr.is_idle()
                        && bot.ticks_connected().is_multiple_of(80);
                    if auto_eat_ok && let Ok(inv) = bot.get_inventory() {
                        const SAFE_FOODS: [&str; 20] = [
                            "cooked_beef",
                            "cooked_porkchop",
                            "cooked_chicken",
                            "cooked_mutton",
                            "cooked_rabbit",
                            "cooked_cod",
                            "cooked_salmon",
                            "bread",
                            "apple",
                            "golden_apple",
                            "baked_potato",
                            "mushroom_stew",
                            "rabbit_stew",
                            "pumpkin_pie",
                            "cookie",
                            "melon_slice",
                            "sweet_berries",
                            "glow_berries",
                            "cake",
                            "dried_kelp",
                        ];
                        let found = SAFE_FOODS.iter().find_map(|name| {
                            ItemKind::from_str(name)
                                .ok()
                                .filter(|k| !find_item_slots(&inv, *k).is_empty())
                        });
                        if let Some(k) = found {
                            let item_name = k
                                .to_str()
                                .strip_prefix("minecraft:")
                                .unwrap_or_else(|| k.to_str());
                            let msg = do_consume(&bot, item_name).await;
                            if !msg.contains("失败") && !msg.contains("未持有") {
                                let _ = evt_tx.send(BotEvent::Chat {
                                    content: format!(
                                        "[MODE:auto_eat] 饥饿 {hunger_now}/20，自动进食 {item_name}"
                                    ),
                                });
                            }
                        }
                    }
                }
                // hunting：空闲时自动狩猎附近动物（P77，Mindcraft 移植）。
                // Mindcraft modes.js hunting: 8m 内 isHuntable 动物自动 attackEntity，
                // 掉落物靠 item_collecting 模式拾取。此前我们只有 LLM 决策层提示
                // （30-60s/回合），动物跑了/没 LLM 关注就没有食物来源。
                // 实现：100 tick 节流 + is_idle + hp≥10（濒死让位 cowardice）；
                // 攻击后 5s 拾取窗口内自动 pickup 掉落物。
                // P116：set_mode 可禁用（mode_switches 集合）。
                if !state.mode_disabled("hunting")
                    && bot.ticks_connected().is_multiple_of(100)
                    && state.action_mgr.is_idle()
                    && !*state.mining_below.lock().unwrap()
                    && bot.health().unwrap_or(20.0) >= 10.0
                    && let Ok(entities) = bot
                        .nearest_entities::<bevy_ecs::query::Without<azalea::entity::metadata::Player>>()
                    {
                        let self_id = bot.entity().id();
                        let self_pos = bot.position().ok();
                        'hunt: for e in entities.iter() {
                            if e.id() == self_id {
                                continue;
                            }
                            let Ok(kind) = e.kind() else { continue };
                            let huntable = matches!(
                                kind,
                                EntityKind::Cow
                                    | EntityKind::Pig
                                    | EntityKind::Chicken
                                    | EntityKind::Sheep
                                    | EntityKind::Rabbit
                                    | EntityKind::Mooshroom
                            );
                            if !huntable {
                                continue;
                            }
                            let (Some(sp), Ok(ep)) = (self_pos, e.position()) else {
                                continue;
                            };
                            let d = ((sp.x - ep.x).powi(2)
                                + (sp.y - ep.y).powi(2)
                                + (sp.z - ep.z).powi(2))
                                .sqrt();
                            if d <= 8.0 {
                                let indexed = bot
                                    .query_self::<&azalea::entity::indexing::EntityIdIndex, _>(
                                        |index| index.contains_ecs_entity(e.id()),
                                    )
                                    .unwrap_or(false);
                                if indexed
                                    && e.get_component::<azalea::entity::EntityKindComponent>()
                                        .is_some()
                                {
                                    e.attack();
                                    let tick = bot.ticks_connected();
                                    *state.hunt_pickup_until.lock().unwrap() = tick + 100;
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!(
                                            "[MODE:hunting] 自动狩猎 {kind:?}（食物来源）"
                                        ),
                                    });
                                    break 'hunt;
                                }
                            }
                        }
                    }
                // hunting 拾取窗口：攻击动物后自动捡掉落物（每 20 tick 一次，直到窗口结束）。
                {
                    let tick = bot.ticks_connected();
                    let until = *state.hunt_pickup_until.lock().unwrap();
                    if until > 0
                        && tick < until
                        && tick.is_multiple_of(20)
                        && state.action_mgr.is_idle()
                    {
                        let _ = crate::azalea::smart_actions::pickup_nearby_items(&bot).await;
                    }
                    if until > 0 && tick >= until {
                        *state.hunt_pickup_until.lock().unwrap() = 0;
                    }
                }
                // item_collecting：自动拾取附近掉落物（P80，Mindcraft 移植）。
                // Mindcraft modes.js item_collecting: 8m 内 item 实体 + 空闲 + 背包有空位
                // → pickupNearbyItems。此前只有 hunting 后 5s 窗口 + LLM pickup 工具
                // （30-60s/回合注意不到地上 raw_iron/diamond——实测 item:6@5m 无人捡）。
                // 每 200 tick（~10s）：空闲时 8m 内有 item 实体 → 自动拾取。
                // 背包空位保护：空槽 <2 时跳过（避免捡垃圾占满背包）。
                // P116：set_mode 可禁用（mode_switches 集合）。
                if !state.mode_disabled("item_collecting")
                    && bot.ticks_connected().is_multiple_of(200)
                    && state.action_mgr.is_idle()
                    && !*state.mining_below.lock().unwrap()
                {
                    let has_item_near = bot
                        .nearest_entities::<bevy_ecs::query::Without<azalea::entity::metadata::Player>>()
                        .map(|entities| {
                            let self_id = bot.entity().id();
                            let self_pos = bot.position().ok();
                            entities.iter().any(|e| {
                                if e.id() == self_id {
                                    return false;
                                }
                                if let Ok(kind) = e.kind() {
                                    if kind != EntityKind::Item {
                                        return false;
                                    }
                                    if let (Some(sp), Ok(ep)) = (self_pos, e.position()) {
                                        let d = ((sp.x - ep.x).powi(2)
                                            + (sp.y - ep.y).powi(2)
                                            + (sp.z - ep.z).powi(2))
                                            .sqrt();
                                        return d <= 8.0;
                                    }
                                }
                                false
                            })
                        })
                        .unwrap_or(false);
                    if has_item_near {
                        let free_slots = bot
                            .get_inventory()
                            .ok()
                            .and_then(|inv| {
                                inv.menu().ok().flatten().and_then(|m| {
                                    inv.slots().map(|slots| {
                                        m.player_slots_range()
                                            .filter(|&s| {
                                                slots
                                                    .get(s)
                                                    .map(|st| st.is_empty())
                                                    .unwrap_or(false)
                                            })
                                            .count()
                                    })
                                })
                            })
                            .unwrap_or(9);
                        if free_slots >= 2 {
                            let _ = crate::azalea::smart_actions::pickup_nearby_items(&bot).await;
                        }
                    }
                }
                // auto_armor：自动穿甲（P79，对齐 Mindcraft armorManager.equipAll()）。
                // 实机高频问题：装备持续退化（头盔/靴子缺失、甲损坏），LLM 回合
                // 30-60s 管不过来；且 P56 后 do_equip 幂等（目标槽同款直接成功）。
                // 每 200 tick（~10s）：空闲时逐槽位检查，槽空或现有甲材料更差、
                // 且背包有更高档同类甲 → 自动装备。材料优先级：netherite>diamond>
                // iron>chainmail>gold>leather（MC 基础防御排序）。
                if bot.ticks_connected().is_multiple_of(200)
                    && state.action_mgr.is_idle()
                    && let Ok(inv) = bot.get_inventory()
                    && let Some(slots) = inv.slots()
                {
                    let tier_rank = |id: &str| -> u8 {
                        if id.contains("netherite") {
                            5
                        } else if id.contains("diamond") {
                            4
                        } else if id.contains("iron") {
                            3
                        } else if id.contains("chainmail") {
                            2
                        } else if id.contains("gold") {
                            1
                        } else if id.contains("leather") {
                            0
                        } else {
                            255
                        }
                    };
                    const ARMOR_SLOTS: [(&str, usize); 4] = [
                        ("helmet", 5),
                        ("chestplate", 6),
                        ("leggings", 7),
                        ("boots", 8),
                    ];
                    for (slot_name, slot_idx) in ARMOR_SLOTS {
                        let worn_id = slots.get(slot_idx).and_then(|st| {
                            if st.is_empty() {
                                None
                            } else {
                                Some(st.kind().to_str().to_string())
                            }
                        });
                        let worn_rank = worn_id.as_deref().map(tier_rank).unwrap_or(255);
                        let best = [
                            ("netherite", slot_name),
                            ("diamond", slot_name),
                            ("iron", slot_name),
                            ("chainmail", slot_name),
                            ("golden", slot_name),
                            ("leather", slot_name),
                        ]
                        .iter()
                        .find_map(|(mat, slot)| {
                            let item_id = format!("{mat}_{slot}");
                            ItemKind::from_str(&item_id)
                                .ok()
                                .filter(|k| !find_item_slots(&inv, *k).is_empty())
                        });
                        if let Some(k) = best {
                            let item_id = k
                                .to_str()
                                .strip_prefix("minecraft:")
                                .unwrap_or_else(|| k.to_str())
                                .to_string();
                            if tier_rank(&item_id) < worn_rank {
                                let msg = do_equip(&bot, &item_id, slot_name).await;
                                let _ = evt_tx.send(BotEvent::Chat {
                                    content: format!("[MODE:auto_armor] {msg}"),
                                });
                            }
                        }
                    }
                }
                // cowardice：hp 低 + 附近有敌对 → 自动逃离（Mindcraft 移植）。
                // self_defense 只攻击 4 格内敌人，而僵尸/骷髅 16m 外扑来时 LLM 回合
                // 30-60s 太慢（实测 hp=1 濒死时 LLM 想撤退但 goto 连续失败，被僵尸追死）。
                // P77：阈值 hp<6→hp<10（骷髅 2 箭 7-9 伤害就能破 6，之前的阈值太晚；
                // Mindcraft 是无条件 16m 逃，我们保留 hp 门槛避免 bot 见怪就放弃主线）。
                // 地下→自动向上挖洞逃生（僵尸不会挖方块）；地表→向远离敌人方向走 20 格。
                // 优先于 self_defense：hp<10 时 self_defense 的攻击会被跳过。
                // P116：set_mode 可禁用（mode_switches 集合）。
                if !state.mode_disabled("cowardice") && bot.ticks_connected().is_multiple_of(100) {
                    let health = bot.health().unwrap_or(20.0);
                    if health < 10.0 {
                        let mut flee_dir: Option<(f64, f64)> = None;
                        if let Ok(entities) = bot
                            .nearest_entities::<bevy_ecs::query::Without<azalea::entity::metadata::Player>>()
                        {
                            let self_id = bot.entity().id();
                            let self_pos = bot.position().ok();
                            for e in entities.iter() {
                                if e.id() == self_id {
                                    continue;
                                }
                                if flee_dir.is_some() {
                                    break;
                                }
                                if let Ok(kind) = e.kind() {
                                    let hostile = matches!(
                                        kind,
                                        EntityKind::Zombie
                                            | EntityKind::Skeleton
                                            | EntityKind::Creeper
                                            | EntityKind::Spider
                                            | EntityKind::CaveSpider
                                            | EntityKind::Enderman
                                            | EntityKind::Pillager
                                            | EntityKind::Phantom
                                            | EntityKind::Witch
                                            | EntityKind::Drowned
                                            | EntityKind::Husk
                                            | EntityKind::Stray
                                            // P77：下界/末地敌对（dragon 主线的自动防御保障）
                                            | EntityKind::Blaze
                                            | EntityKind::Ghast
                                            | EntityKind::Piglin
                                            | EntityKind::PiglinBrute
                                            | EntityKind::ZombifiedPiglin
                                            | EntityKind::Guardian
                                            | EntityKind::ElderGuardian
                                            | EntityKind::Shulker
                                            | EntityKind::Vex
                                            | EntityKind::Wither
                                            | EntityKind::WitherSkeleton
                                            | EntityKind::MagmaCube
                                    );
                                    if hostile
                                        && let (Some(sp), Ok(ep)) = (self_pos, e.position()) {
                                            let dx = sp.x - ep.x;
                                            let dz = sp.z - ep.z;
                                            let d = (dx * dx + dz * dz).sqrt();
                                            // 20m 半径：僵尸 18m 外徘徊时也要提前逃
                                            // （实测 hp=1 时僵尸 18m 处 bot 原地等死，LLM 回合太慢）
                                            if d <= 20.0 && d > 0.01 {
                                                flee_dir = Some((dx / d, dz / d));
                                            }
                                        }
                                }
                            }
                        }
                        if let (Some((fx, fz)), Ok(p)) = (flee_dir, bot.position()) {
                            let head_blocked = bot
                                .world()
                                .ok()
                                .map(|w| {
                                    w.read()
                                        .get_block_state(BlockPos::new(
                                            p.x.floor() as i32,
                                            p.y.floor() as i32 + 1,
                                            p.z.floor() as i32,
                                        ))
                                        .map(|b| !b.is_air())
                                        .unwrap_or(false)
                                })
                                .unwrap_or(false);
                            if (p.y.floor() as i32) < 62 || head_blocked {
                                // 地下：向上挖逃生
                                if !*state.mining_above.lock().unwrap() {
                                    *state.mining_above.lock().unwrap() = true;
                                    *state.mining_above_start_y.lock().unwrap() =
                                        Some(p.y.floor() as i32);
                                    *state.mining_above_direction.lock().unwrap() = 0;
                                    bot.force_stop_pathfinding();
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!(
                                            "[MODE:cowardice] HP {health:.0}/20 过低且附近有敌对生物，自动向上挖洞逃生（mine_above）"
                                        ),
                                    });
                                }
                            } else {
                                // 地表：向远离方向走 20 格
                                let tx = (p.x + fx * 20.0).floor() as i32;
                                let ty = p.y.floor() as i32;
                                let tz = (p.z + fz * 20.0).floor() as i32;
                                let escape_cmd = BotCommand::Goto {
                                    x: tx,
                                    y: ty,
                                    z: tz,
                                };
                                let tick_now = bot.ticks_connected();
                                let _ = state.action_mgr.submit(
                                    escape_cmd,
                                    Priority::High,
                                    &cmd_queue,
                                    tick_now,
                                );
                                let _ = evt_tx.send(BotEvent::Chat {
                                    content: format!(
                                        "[MODE:cowardice] HP {health:.0}/20 过低且附近有敌对生物，自动向 ({tx},{ty},{tz}) 逃离"
                                    ),
                                });
                            }
                        }
                    }
                }
                // self_defense：空闲或寻路途中自动攻击附近敌对生物（每 100 tick ≈5s 检查一次）
                // 距离限制：只攻击 8 格内实体（P77：对齐 Mindcraft 的 8m；4m 太近——
                // 僵尸走到 4m 内往往已经开始扑击，8m 能提前两轮出手）。
                // 用 is_busy() 而非 is_idle()：Goto/Mine 等轮询命令执行期间 pending 非空但 busy=false，
                // 此时仍应自卫（否则 bot 寻路途中被僵尸攻击不还手——H3 bug）。
                // 只在异步命令（Craft/Gather/Smelt）执行中（busy=true）跳过，避免抢占。
                // hp<10 时不攻击（cowardice 逃跑优先，避免濒死还硬刚被补刀——P77 随 cowardice 同步 6→10）。
                // P77：主手非武器且背包有剑/斧 → 自动装备（Mindcraft pvp 插件默认行为），
                // 装备请求期间跳过攻击（等 5s 后下一轮装备好再打）。
                // P77：creeper ≤3m（爆炸半径）→ 撤离优先于攻击。
                // P87-2：移除「mining_below 时跳过 self_defense」——bot 深挖（MineBelow）期间
                // 僵尸贴脸会站桩挨打（实机验证 P87 时暴露：LLM 持续 mine_below，8m 内僵尸无人管）。
                // 挖矿中同样先保命：仅 busy=true（Craft/Gather/Smelt 异步命令）时退出。
                // P88-b：检查间隔 100 tick(5s) → 20 tick(1s)。实机验证 P88 暴露：
                // 僵尸贴脸 5s 内咬 5 击（~12 伤害），等 100 tick 检查时 bot 已 hp<10，
                // self_defense 让位 cowardice 逃跑——永远来不及反击，站桩被咬死。
                // 1s 检查一次，僵尸贴脸后第一轮就能攻击（攻击冷却由 MC 服务端控制）。
                // P88-c：hp 条件移到内层——低血只放弃「远处逼近」，
                // 贴脸（≤3.2m）怪照打：cowardice 逃跑途中被贴脸怪追着咬，
                // 不反击只会越逃越死（实机验证 P88-b 时 bot 8/20 全程逃跑被追）。
                // P116：set_mode 可禁用（mode_switches 集合）。
                if !state.mode_disabled("self_defense")
                    && !state.action_mgr.is_busy()
                    && bot.ticks_connected().is_multiple_of(20)
                    && let Ok(entities) = bot.nearest_entities::<bevy_ecs::query::Without<azalea::entity::metadata::Player>>() {
                    let self_id = bot.entity().id();
                    let self_pos = bot.position().ok();
                    let mut attacked = false;
                    for e in entities.iter() {
                        if e.id() == self_id { continue; }
                        if attacked { break; }
                        if let Ok(kind) = e.kind() {
                            let hostile = matches!(kind,
                                EntityKind::Zombie | EntityKind::Skeleton | EntityKind::Creeper
                                | EntityKind::Spider | EntityKind::CaveSpider | EntityKind::Enderman
                                | EntityKind::Pillager | EntityKind::Phantom | EntityKind::Witch
                                | EntityKind::Drowned | EntityKind::Husk | EntityKind::Stray
                                // P77：下界/末地敌对（dragon 主线的自动防御保障）
                                | EntityKind::Blaze | EntityKind::Ghast | EntityKind::Piglin
                                | EntityKind::PiglinBrute | EntityKind::ZombifiedPiglin
                                | EntityKind::Guardian | EntityKind::ElderGuardian
                                | EntityKind::Shulker | EntityKind::Vex | EntityKind::Wither
                                | EntityKind::WitherSkeleton | EntityKind::MagmaCube
                            );
                            if hostile {
                                // creeper 3m 内：爆炸半径内，撤离优先（High 优先级 goto 8m 外）
                                if kind == EntityKind::Creeper
                                    && let (Some(sp), Ok(ep)) = (self_pos, e.position()) {
                                        let d = ((sp.x - ep.x).powi(2)
                                            + (sp.y - ep.y).powi(2)
                                            + (sp.z - ep.z).powi(2)).sqrt();
                                        if d <= 3.0 {
                                            let mut dx = sp.x - ep.x;
                                            let mut dz = sp.z - ep.z;
                                            let dl = (dx * dx + dz * dz).sqrt();
                                            if dl < 0.1 { dx = 1.0; dz = 0.0; } else { dx /= dl; dz /= dl; }
                                            let tx = (sp.x + dx * 8.0).floor() as i32;
                                            let ty = sp.y.floor() as i32;
                                            let tz = (sp.z + dz * 8.0).floor() as i32;
                                            let tick_now = bot.ticks_connected();
                                            let _ = state.action_mgr.submit(
                                                BotCommand::Goto { x: tx, y: ty, z: tz },
                                                Priority::High,
                                                &cmd_queue,
                                                tick_now,
                                            );
                                            let _ = evt_tx.send(BotEvent::Chat {
                                                content: format!("[MODE] creeper {d:.1}m 内即将爆炸，自动撤离 ({tx},{ty},{tz})"),
                                            });
                                            break;
                                        }
                                    }
                                // 距离检查：8 格内才处理（远距离敌人由 LLM 决策是否拉近或撤退）
                                let in_range = if let Some(sp) = self_pos {
                                    if let Ok(ep) = e.position() {
                                        let d = ((sp.x - ep.x).powi(2)
                                            + (sp.y - ep.y).powi(2)
                                            + (sp.z - ep.z).powi(2)).sqrt();
                                        d <= 8.0
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                };
                                if !in_range { continue; }
                                // P88：MC 近战 reach=3.0，4~8m 直接 e.attack() 是无效攻击
                                // （包发出但服务器判定 miss）。实机验证 P87-2 暴露：僵尸 4.6m
                                // 时 MODE 攻击 + strafe 全触发但僵尸一直不死——攻击全部 miss。
                                // 改为：>3.2m 先 High 优先级 goto 逼近到僵尸 2m 处，下一轮再打。
                                // P88-b：垂直差 >4m 不逼近（phantom/ghast 在头顶几十格时
                                // goto 空中目标必然失败乱跑——飞行怪交给 LLM 决策）。
                                let dist = self_pos.and_then(|sp| {
                                    e.position()
                                        .ok()
                                        .map(|ep| {
                                            ((sp.x - ep.x).powi(2)
                                                + (sp.y - ep.y).powi(2)
                                                + (sp.z - ep.z).powi(2))
                                                .sqrt()
                                        })
                                });
                                if let Some(d) = dist
                                    && d > 3.2
                                {
                                    // P88-d：>3.2m 时只有两种结局——(a) 满足条件就 goto 逼近，
                                    // (b) 不满足（低血/垂直差大）就 continue 跳过攻击——
                                    // 否则会走到下方攻击分支，在 4~8m 直接 e.attack() 必 miss
                                    // （实机验证 P88-c：bot 8/20 时对 4m+ 苦力怕连续攻击 7 次
                                    // 全 miss——低血不逼近但又直接攻击，无效输出）。
                                    if let (Some(sp), Ok(ep)) = (self_pos, e.position())
                                        && (ep.y - sp.y).abs() <= 4.0
                                        // P88-c：低血不逼近远处怪（cowardice 逃跑优先，
                                        // 逼近途中反而吃更多攻击）；贴脸怪照打（下方分支）。
                                        && bot.health().unwrap_or(0.0) >= 10.0
                                    {
                                        let mut dx = sp.x - ep.x;
                                        let mut dz = sp.z - ep.z;
                                        let dl = (dx * dx + dz * dz).sqrt();
                                        if dl > 0.1 {
                                            dx /= dl;
                                            dz /= dl;
                                            let tx = (ep.x + dx * 2.0).floor() as i32;
                                            let ty = ep.y.floor() as i32;
                                            let tz = (ep.z + dz * 2.0).floor() as i32;
                                            let tick_now = bot.ticks_connected();
                                            let _ = state.action_mgr.submit(
                                                BotCommand::Goto { x: tx, y: ty, z: tz },
                                                Priority::High,
                                                &cmd_queue,
                                                tick_now,
                                            );
                                            let _ = evt_tx.send(BotEvent::Chat {
                                                content: format!(
                                                    "[MODE:self_defense] 敌人 {kind:?} {d:.1}m 超近战范围，逼近 ({tx},{ty},{tz})"
                                                ),
                                            });
                                            continue;
                                        }
                                    }
                                    // 无法逼近：>3.2m 攻击必 miss，直接跳过（交给 LLM/cowardice）
                                    continue;
                                }
                                // 至此 d ≤ 3.2m：近战范围内，直接攻击（贴脸怪低血也打）
                                // 自动换武器：主手非武器且背包有剑/斧 → Equip（防重复，本轮跳过攻击）
                                let held_is_weapon = bot
                                    .get_held_item()
                                    .ok()
                                    .map(|s| {
                                        let id = s.kind().to_str();
                                        id.ends_with("_sword") || id.ends_with("_axe")
                                    })
                                    .unwrap_or(false);
                                if !held_is_weapon {
                                    // P87 修复：背包无武器时（best=None）也要徒手攻击——
                                    // 原实现无条件 continue，导致无武器时每 100 tick 都跳过攻击、
                                    // bot 面对僵尸站桩挨打永不还手（probe 实测复现）。
                                    let mut equip_submitted = false;
                                    {
                                        let mut pending = state.combat_equip_pending.lock().unwrap();
                                        if pending.is_none()
                                            && let Ok(inv) = bot.get_inventory() {
                                                let best = [
                                                    "diamond_sword", "iron_sword", "stone_sword",
                                                    "wooden_sword", "diamond_axe", "iron_axe",
                                                    "stone_axe", "wooden_axe",
                                                ]
                                                .iter()
                                                .find_map(|n| {
                                                    ItemKind::from_str(n).ok().filter(|k| {
                                                        !find_item_slots(&inv, *k).is_empty()
                                                    })
                                                });
                                                if let Some(k) = best {
                                                    let name = k.to_str()
                                                        .strip_prefix("minecraft:")
                                                        .unwrap_or_else(|| k.to_str())
                                                        .to_string();
                                                    *pending = Some(name.clone());
                                                    let tick_now = bot.ticks_connected();
                                                    let _ = state.action_mgr.submit(
                                                        BotCommand::Equip {
                                                            item: name.clone(),
                                                            slot: "hand".into(),
                                                        },
                                                        Priority::Normal,
                                                        &cmd_queue,
                                                        tick_now,
                                                    );
                                                    let _ = evt_tx.send(BotEvent::Chat {
                                                        content: format!("[MODE:self_defense] 主手无武器，自动装备 {name}"),
                                                    });
                                                    equip_submitted = true;
                                                }
                                            }
                                    }
                                    if equip_submitted {
                                        // 等装备完成，本轮不攻击
                                        continue;
                                    }
                                    // 背包无武器 → 徒手攻击（打总比站桩挨打好）
                                } else {
                                    *state.combat_equip_pending.lock().unwrap() = None;
                                }
                                // 攻击前检查实体是否存活（get_component 失败说明已消失）
                                let indexed = bot
                                    .query_self::<&azalea::entity::indexing::EntityIdIndex, _>(|index| {
                                        index.contains_ecs_entity(e.id())
                                    })
                                    .unwrap_or(false);
                                if indexed && e.get_component::<azalea::entity::EntityKindComponent>().is_some() {
                                    e.attack();
                                    attacked = true;
                                    // P87：战斗走位（strafe）——攻击后围绕敌人侧向移动，
                                    // 目标点 = 敌人位置 + 径向 1.2m + 切向 1.5m（保持 ~1.9m 战斗距离，
                                    // 在近战 reach 3m 内能打中，又不贴脸吃全部挥击）。
                                    // y 取敌人所在层（P88：原 sp.y 在竖井里会差层，走位到 3.6m
                                    // 依然打不中——实机验证 P87-2 时的 miss 根因之一）。
                                    // 冷却 40 tick 防打断寻路。
                                    let strafe_cd = *state.combat_strafe_cd.lock().unwrap();
                                    if (bot.ticks_connected() as i64) >= strafe_cd
                                        && let (Some(_sp), Ok(ep)) = (self_pos, e.position())
                                    {
                                        let mut dx = _sp.x - ep.x;
                                        let mut dz = _sp.z - ep.z;
                                        let dl = (dx * dx + dz * dz).sqrt();
                                        if dl > 0.1 {
                                            dx /= dl;
                                            dz /= dl;
                                            let tx = (ep.x + dx * 1.2 + -dz * 1.5).floor() as i32;
                                            let ty = ep.y.floor() as i32;
                                            let tz = (ep.z + dz * 1.2 + dx * 1.5).floor() as i32;
                                            let tick_now = bot.ticks_connected();
                                            let _ = state.action_mgr.submit(
                                                BotCommand::Goto { x: tx, y: ty, z: tz },
                                                Priority::High,
                                                &cmd_queue,
                                                tick_now,
                                            );
                                            *state.combat_strafe_cd.lock().unwrap() = tick_now as i64 + 40;
                                            let _ = evt_tx.send(BotEvent::Chat {
                                                content: format!("[MODE:self_defense] strafe 走位 ({tx},{ty},{tz})"),
                                            });
                                        }
                                    }
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!("[MODE] 攻击 {kind:?}"),
                                    });
                                }
                            }
                        }
                    }
                }
                // P153：空闲时强制停止移动——实测 bot 在无命令时会持续漂移
                // （25s 漂移 11 格，Y 持续变化），导致 tp/goto/mine 后位置不稳定、
                // 深挖作业无法持续。根因疑似 azalea 残留 move_direction 或跳跃状态。
                // 仅在真正空闲（无 pending 命令、非 busy、无战斗）时复位移动方向，
                // 不影响 goto/mine/战斗中的正常移动。
                if state.action_mgr.is_idle() && !state.action_mgr.is_busy() {
                    bot.walk(azalea::WalkDirection::None);
                    let _ = bot.set_jumping(false);
                }
            }
            _ => {}
        }
        bot
    }
}
