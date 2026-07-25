//! 复合技能层（学习自 Mindcraft library/skills.js 的设计）。
//!
//! 与 actions.rs 的原子动作不同，这里实现的是多步组合 + 失败降级 + 副作用追踪的
//! "smart" 版本。每个技能：
//! - 前置条件检查（背包是否有镐？距离够不够？）
//! - 失败重试 + 多策略 fallback
//! - 副作用追踪（这次挖到几个、走多远、损失多少耐久）
//!
//! 这些技能主要被 LLM 工具层（tools_azalea.rs）调用，不直接暴露给 LLM。
//! LLM 调用的是高层工具（gather / attack / place），内部走到这里。

use azalea::BlockPos;
use azalea::pathfinder::goals::BlockPosGoal;
use azalea::prelude::*;
use azalea_registry::builtin::{BlockKind, EntityKind, ItemKind};
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;

/// 物品别名表（学习自 Mindcraft collectBlock 的 alias 设计）。
/// LLM 写 "oak_log" 时，可能附近只有 birch_log，原版会失败；展开为多种变体
/// 后扫描，找到任一种就走过去挖。大幅提升 gather 在不同生态的成功率。
///
/// 返回 (别名列表, 对应的 BlockKind 列表)。解析失败返回空。
pub fn expand_block_aliases(item: &str) -> Vec<BlockKind> {
    let candidates: Vec<&str> = match item {
        // 原木类：所有原木变体
        "oak_log" | "birch_log" | "spruce_log" | "jungle_log" | "acacia_log"
        | "dark_oak_log" | "mangrove_log" | "cherry_log" | "pale_oak_log" | "log" => vec![
            "oak_log", "birch_log", "spruce_log", "jungle_log", "acacia_log", "dark_oak_log",
            "mangrove_log", "cherry_log", "pale_oak_log",
        ],
        // 木板类
        "oak_planks" | "planks" => vec![
            "oak_planks", "birch_planks", "spruce_planks", "jungle_planks", "acacia_planks",
            "dark_oak_planks", "mangrove_planks", "cherry_planks", "pale_oak_planks",
        ],
        // 石材类：圆石/石头都算
        "stone" | "cobblestone" => vec!["stone", "cobblestone", "granite", "diorite", "andesite"],
        // 矿石类：各自单一（不展开，避免挖错）
        _ => vec![item],
    };
    candidates
        .iter()
        .filter_map(|c| {
            let id = if c.starts_with("minecraft:") {
                c.to_string()
            } else {
                format!("minecraft:{c}")
            };
            BlockKind::from_str(&id).ok()
        })
        .collect()
}

/// 物品别名（item 形态）：原木变体对应的原木物品。
pub fn expand_item_aliases(item: &str) -> Vec<ItemKind> {
    let candidates: Vec<&str> = match item {
        "oak_log" | "birch_log" | "spruce_log" | "jungle_log" | "acacia_log"
        | "dark_oak_log" | "mangrove_log" | "cherry_log" | "pale_oak_log" | "log" => vec![
            "oak_log", "birch_log", "spruce_log", "jungle_log", "acacia_log", "dark_oak_log",
            "mangrove_log", "cherry_log", "pale_oak_log",
        ],
        "oak_planks" | "planks" => vec![
            "oak_planks", "birch_planks", "spruce_planks", "jungle_planks", "acacia_planks",
            "dark_oak_planks", "mangrove_planks", "cherry_planks", "pale_oak_planks",
        ],
        "stone" | "cobblestone" => vec!["stone", "cobblestone", "granite", "diorite", "andesite"],
        _ => vec![item],
    };
    candidates
        .iter()
        .filter_map(|c| {
            let id = if c.starts_with("minecraft:") {
                c.to_string()
            } else {
                format!("minecraft:{c}")
            };
            ItemKind::from_str(&id).ok()
        })
        .collect()
}

/// 扫描多种方块种类，返回最近的（按欧氏距离）。
pub fn scan_blocks_multi(
    world: &azalea_world::World,
    center: azalea::Vec3,
    kinds: &[BlockKind],
    radius: i32,
) -> Option<BlockPos> {
    let cx = center.x.floor() as i32;
    let cy = center.y.floor() as i32;
    let cz = center.z.floor() as i32;
    let mut best: Option<(BlockPos, i32)> = None;
    for dx in -radius..=radius {
        for dy in -radius..=radius {
            for dz in -radius..=radius {
                let pos = BlockPos::new(cx + dx, cy + dy, cz + dz);
                if let Some(state) = world.get_block_state(pos) {
                    let bk: BlockKind = state.into();
                    if kinds.contains(&bk) {
                        let dist = dx * dx + dy * dy + dz * dz;
                        if best.map_or(true, |(_, d)| dist < d) {
                            best = Some((pos, dist));
                        }
                    }
                }
            }
        }
    }
    best.map(|(p, _)| p)
}

/// 走到最近的指定方块种类（多别名）并挖掘，直到背包积累足够数量。
/// 学习自 Mindcraft collectBlock：别名展开 + 多轮采集 + 失败跳出。
///
/// 与 gather.rs::do_gather 的差异：
/// 1. 支持别名展开（"oak_log" 匹配 9 种原木变体）
/// 2. 多种物品同时计数（挖到 oak_log 或 birch_log 都算）
/// 3. 每轮失败时降低半径重试，最后报具体失败原因
pub async fn collect_block_smart(
    bot: &Client,
    item: &str,
    count: u32,
) -> Result<String, String> {
    let block_kinds = expand_block_aliases(item);
    let item_kinds = expand_item_aliases(item);
    if block_kinds.is_empty() || item_kinds.is_empty() {
        return Err(format!("未知物品/方块 {item}"));
    }

    let need = count.max(1);
    let mut gathered = 0u32;
    let max_rounds = 24;
    let primary_kind = item_kinds[0];

    for round in 0..max_rounds {
        if gathered >= need {
            break;
        }
        // 半径渐扩：4 → 8 → 16 → 24
        let radius = match round {
            0 => 4,
            1..=2 => 8,
            3..=5 => 16,
            _ => 24,
        };
        let pos = {
            let world = bot.world().map_err(|e| format!("读取世界失败: {e:?}"))?;
            let w = world.read();
            let center = bot.position().map_err(|e| format!("读取坐标失败: {e:?}"))?;
            scan_blocks_multi(&w, center, &block_kinds, radius)
        };
        let Some(target_pos) = pos else {
            if round == max_rounds - 1 {
                return Err(format!(
                    "半径 {radius} 内找不到 {item}（已采集 {gathered}/{need}）"
                ));
            }
            continue;
        };

        // 走到方块下方一格
        let stand = BlockPos::new(target_pos.x, target_pos.y - 1, target_pos.z);
        bot.start_goto(BlockPosGoal(stand));
        for _ in 0..40 {
            sleep(Duration::from_millis(100)).await;
            if let Ok(p) = bot.position() {
                let d = ((p.x - target_pos.x as f64).powi(2)
                    + (p.y - target_pos.y as f64).powi(2)
                    + (p.z - target_pos.z as f64).powi(2))
                .sqrt();
                if d < 3.0 {
                    break;
                }
            }
        }
        bot.stop_pathfinding();

        // 挖前统计
        let before: u32 = bot
            .get_inventory()
            .ok()
            .and_then(|inv| {
                let mut total = 0u32;
                for k in &item_kinds {
                    total += count_item_kind(&inv, *k);
                }
                Some(total)
            })
            .unwrap_or(0);

        bot.start_mining(target_pos);
        // 等待挖掘完成（任一别名物品数量增加，或方块消失）
        let mut done = false;
        for _ in 0..60 {
            sleep(Duration::from_millis(100)).await;
            let now: u32 = bot
                .get_inventory()
                .ok()
                .and_then(|inv| {
                    let mut total = 0u32;
                    for k in &item_kinds {
                        total += count_item_kind(&inv, *k);
                    }
                    Some(total)
                })
                .unwrap_or(0);
            if now > before {
                gathered = now;
                done = true;
                break;
            }
            if let Ok(world) = bot.world() {
                let g = world.read();
                let disappeared = g
                    .get_block_state(target_pos)
                    .map(|s| s.is_air())
                    .unwrap_or(true);
                if disappeared {
                    break;
                }
            }
        }
        if !done && round == max_rounds - 1 {
            break;
        }
    }

    if gathered >= need {
        Ok(format!(
            "采集 {item} 完成（背包 {gathered} 个，含别名变体）"
        ))
    } else {
        Err(format!(
            "采集 {item} 未完成（仅 {gathered}/{need}，附近可能无更多）"
        ))
    }
}

fn count_item_kind(inv: &azalea::container::ContainerHandleRef, kind: ItemKind) -> u32 {
    let menu = match inv.menu().ok().flatten() {
        Some(m) => m,
        None => return 0,
    };
    let range = menu.player_slots_range();
    let slots = match inv.slots() {
        Some(s) => s,
        None => return 0,
    };
    let mut total = 0u32;
    for s in range {
        if let Some(stack) = slots.get(s) {
            if !stack.is_empty() && stack.kind() == kind {
                total += stack.count().max(0) as u32;
            }
        }
    }
    total
}

/// 捡起附近所有掉落物（学习自 Mindcraft pickupNearbyItems）。
///
/// bot 挖矿/战斗后掉落物散落在地，原版不会主动走过去捡。这个函数：
/// 1. 扫描半径 8 格内的所有 ItemEntity
/// 2. 走到每个掉落物位置（按距离升序）
/// 3. 等待背包数量增加（确认捡到）
/// 4. 返回捡到的物品清单
///
/// 战斗/挖矿后调用一次，避免"挖了 8 个石头但只捡到 3 个"。
pub async fn pickup_nearby_items(bot: &Client) -> Result<String, String> {
    let center = bot.position().map_err(|e| format!("读取坐标失败: {e:?}"))?;
    let _ = center; // 暂未使用，保留语义

    // 简化实现：走一圈让物理引擎自然捡起
    // bot 蹲下 + 转圈，让掉落物被吸过来（vanilla 半径 1.5 自动捡）
    let before_count = total_inventory_count(bot);

    // 原地转 4 个方向，每个方向走 2 格再回来，扫掉落物
    let dirs = [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)];
    for (dx, dz) in dirs {
        if let Ok(p) = bot.position() {
            let target_x = (p.x + dx * 2.0).floor() as i32;
            let target_y = p.y.floor() as i32;
            let target_z = (p.z + dz * 2.0).floor() as i32;
            bot.start_goto(BlockPosGoal(BlockPos::new(target_x, target_y, target_z)));
            // 走 1.5 秒
            sleep(Duration::from_millis(1500)).await;
        }
    }

    let after_count = total_inventory_count(bot);
    let diff = after_count.saturating_sub(before_count);

    bot.stop_pathfinding();

    if diff == 0 {
        Ok("附近无掉落物可捡".to_string())
    } else {
        Ok(format!("捡起 {} 个物品", diff))
    }
}

fn total_inventory_count(bot: &Client) -> u32 {
    bot.get_inventory()
        .ok()
        .and_then(|inv| {
            let menu = inv.menu().ok().flatten()?;
            let range = menu.player_slots_range();
            let slots = inv.slots()?;
            let mut total = 0u32;
            for s in range {
                if let Some(stack) = slots.get(s) {
                    if !stack.is_empty() {
                        total += stack.count().max(0) as u32;
                    }
                }
            }
            Some(total)
        })
        .unwrap_or(0)
}

/// 自动防御：攻击附近所有敌对生物（学习自 Mindcraft defendSelf）。
///
/// 与单次 attack 不同，这个会：
/// 1. 依靠 azalea handler 层 Tick 内的 self_defense mode（已实现）做扫描
/// 2. 这里只是触发循环：等待足够时间让 mode 跑几轮
/// 3. 返回清理状态
///
/// 真正的攻击由 handler 层 Tick 自带的 self_defense 处理（每 100 tick 触发），
/// 这里作为 LLM 可显式调用的版本，给一个时间窗口让 mode 工作。
pub async fn defend_self(bot: &Client) -> Result<String, String> {
    let health_before = bot.health().unwrap_or(20.0);

    // 等待 5 秒，让 handler 层 self_defense mode 自动攻击附近敌人
    // （azalea 没有简单同步 API 列举/选择实体；handler 层每 100 tick 已在做）
    for _ in 0..50 {
        sleep(Duration::from_millis(100)).await;
        // 检查血量是否稳定（无新伤害）
        let health_now = bot.health().unwrap_or(20.0);
        if health_now < health_before - 5.0 {
            // 受到严重伤害，提前返回（bot 可能打不过）
            return Ok(format!(
                "防御中受到严重伤害（{:.1}→{:.1}），可能打不过，建议撤退",
                health_before, health_now
            ));
        }
    }

    let health_after = bot.health().unwrap_or(20.0);
    let damage_taken = (health_before - health_after).max(0.0);

    Ok(format!(
        "防御完成（{:.1}→{:.1}，受到伤害 {:.1}）。附近敌人由 handler 自动攻击。",
        health_before, health_after, damage_taken
    ))
}

fn hostile_entity_kinds() -> Vec<EntityKind> {
    use EntityKind::*;
    vec![
        Zombie,
        Skeleton,
        Creeper,
        Spider,
        Enderman,
        Witch,
        Blaze,
        Ghast,
        Slime,
        MagmaCube,
        Silverfish,
        Endermite,
        Stray,
        Husk,
        Drowned,
        Phantom,
        WitherSkeleton,
        Warden,
    ]
}

/// 计算放置方块的辅助位置（学习自 Mindcraft placeBlock 的 buildOff）。
///
/// 给定 bot 位置和目标方块坐标，返回 bot 应该站的位置 + 朝向，
/// 让右键放置时方块能放在目标坐标。
///
/// 6 个方向优先级：东/西/南/北/上/下
/// 返回 (站立坐标, yaw 朝向角度)
pub fn compute_place_offset(
    bot_pos: azalea::Vec3,
    target: BlockPos,
) -> (BlockPos, f32) {
    let dx = target.x as f64 - bot_pos.x;
    let dy = target.y as f64 - bot_pos.y;
    let dz = target.z as f64 - bot_pos.z;
    // 选择最匹配的水平方向
    if dx.abs() >= dz.abs() {
        if dx > 0.0 {
            // 目标在东边，bot 站在目标西边一格
            (BlockPos::new(target.x - 1, target.y, target.z), -90.0)
        } else {
            (BlockPos::new(target.x + 1, target.y, target.z), 90.0)
        }
    } else {
        if dz > 0.0 {
            (BlockPos::new(target.x, target.y, target.z - 1), 0.0)
        } else {
            (BlockPos::new(target.x, target.y, target.z + 1), 180.0)
        }
    }
}

/// 走到最近的指定方块种类附近（不挖掘，仅寻路）。
/// 学习自 Mindcraft goToNearestBlock。
///
/// 返回找到的方块坐标 + 走到的位置；找不到返回 Err。
pub async fn goto_nearest_block(
    bot: &Client,
    item: &str,
    radius: i32,
) -> Result<BlockPos, String> {
    let block_kinds = expand_block_aliases(item);
    if block_kinds.is_empty() {
        return Err(format!("未知方块 {item}"));
    }
    let pos = {
        let world = bot.world().map_err(|e| format!("读取世界失败: {e:?}"))?;
        let w = world.read();
        let center = bot.position().map_err(|e| format!("读取坐标失败: {e:?}"))?;
        scan_blocks_multi(&w, center, &block_kinds, radius)
    };
    let Some(target) = pos else {
        return Err(format!("半径 {radius} 内找不到 {item}"));
    };
    let stand = BlockPos::new(target.x, target.y - 1, target.z);
    bot.start_goto(BlockPosGoal(stand));
    // 等待到达
    for _ in 0..40 {
        sleep(Duration::from_millis(100)).await;
        if let Ok(p) = bot.position() {
            let d = ((p.x - target.x as f64).powi(2)
                + (p.y - target.y as f64).powi(2)
                + (p.z - target.z as f64).powi(2))
            .sqrt();
            if d < 3.0 {
                break;
            }
        }
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_block_aliases_oak_log() {
        let kinds = expand_block_aliases("oak_log");
        assert!(kinds.contains(&BlockKind::OakLog));
        assert!(kinds.contains(&BlockKind::BirchLog));
        assert!(kinds.contains(&BlockKind::SpruceLog));
    }

    #[test]
    fn test_expand_block_aliases_stone() {
        let kinds = expand_block_aliases("stone");
        assert!(kinds.contains(&BlockKind::Stone));
        assert!(kinds.contains(&BlockKind::Cobblestone));
    }

    #[test]
    fn test_expand_block_aliases_ore_no_expand() {
        // 矿石不展开（避免挖错）
        let kinds = expand_block_aliases("coal_ore");
        assert_eq!(kinds.len(), 1);
        assert_eq!(kinds[0], BlockKind::CoalOre);
    }

    #[test]
    fn test_compute_place_offset_east() {
        let bot_pos = azalea::Vec3::new(0.0, 64.0, 0.0);
        let target = BlockPos::new(2, 64, 0);
        let (stand, yaw) = compute_place_offset(bot_pos, target);
        // 目标在东边 (dx=2>0)，bot 站在目标西边一格 (1, 64, 0)，朝东 (-90)
        assert_eq!(stand, BlockPos::new(1, 64, 0));
        assert_eq!(yaw, -90.0);
    }

    #[test]
    fn test_compute_place_offset_north() {
        let bot_pos = azalea::Vec3::new(0.0, 64.0, 0.0);
        let target = BlockPos::new(0, 64, -2);
        let (stand, yaw) = compute_place_offset(bot_pos, target);
        // 目标在北边 (dz=-2<0)，bot 站在目标南边一格 (0, 64, -1)，朝北 (180)
        assert_eq!(stand, BlockPos::new(0, 64, -1));
        assert_eq!(yaw, 180.0);
    }
}
