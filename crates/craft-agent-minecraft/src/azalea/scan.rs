//! 世界扫描 helpers（纯函数 + 扫描回填）：从 handler.rs 纯移动拆出，行为一致。
//!
//! 说明：`look_at_nearest_entity` 是 async（await nearest_entities），其余为同步。
//! 调用方（handler tick）经 `super::scan::` 引用；`pub(crate)` 可见性仅为跨模块调用。

use super::{
    ActionManager, BotCommand, entity_kind_name, is_hard_block, normalize_entity_target, now_ms,
};
use azalea::BlockPos;
use azalea::player::GameProfileComponent;
use azalea::prelude::*;
use azalea_registry::builtin::BlockKind;
use craft_agent::core::memory::{MemoryKind, MemoryPos, WorldMemory};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub(crate) fn nearby_active_portal(bot: &Client, center: BlockPos) -> bool {
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
pub(crate) fn nearest_solid_block(bot: &Client, x: i32, y: i32, z: i32) -> Option<BlockPos> {
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
pub(crate) fn is_natural_mineable(kind: Option<BlockKind>) -> bool {
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
pub(crate) const GOTO_P132_AIR_SEARCH_RADIUS: i32 = 10;

pub(crate) fn nearest_standable_air(bot: &Client, x: i32, y: i32, z: i32) -> Option<BlockPos> {
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
pub(crate) fn nearest_soft_column(
    bot: &Client,
    x: i32,
    y: i32,
    z: i32,
    radius: i32,
) -> Option<BlockPos> {
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
pub(crate) const SCAN_TTL_MS: u64 = 30_000;

/// 扫描 bot 周围半径内的关键方块，回填到 WorldMemory。
/// 用 `scanned`（pos → 上次扫描时间戳）去重 + TTL 重验。
pub(crate) fn record_surroundings(
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

pub(crate) fn nearby_player_position(bot: &Client, target: Option<&str>) -> Option<azalea::Vec3> {
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
pub(crate) fn current_action_label(action_mgr: &ActionManager) -> Option<String> {
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
pub(crate) async fn look_at_nearest_entity(bot: &Client, target: &str) -> Option<(f32, f32)> {
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
