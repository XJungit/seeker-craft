//! 自动采集：走到最近的指定方块并挖掘，直到背包积累足够数量。
//!
//! 用途：让 bot 能自主完成"早期游戏"第一步（砍树/挖石/挖矿），从而把
//! 采集与合成串成端到端任务，无需玩家手动给物品。

use azalea::BlockPos;
use azalea::container::ContainerHandleRef;
use azalea::pathfinder::goals::BlockPosGoal;
use azalea::prelude::*;
use azalea_registry::builtin::{BlockKind, ItemKind};
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;

/// 在半径 `radius` 内扫描给定种类的方块，返回按到中心距离升序排序的世界坐标。
fn scan_blocks(
    world: &azalea_world::World,
    center: azalea::Vec3,
    kind: BlockKind,
    radius: i32,
) -> Vec<BlockPos> {
    let cx = center.x.floor() as i32;
    let cy = center.y.floor() as i32;
    let cz = center.z.floor() as i32;
    let mut found = Vec::new();
    for dx in -radius..=radius {
        for dy in -radius..=radius {
            for dz in -radius..=radius {
                let pos = BlockPos::new(cx + dx, cy + dy, cz + dz);
                if let Some(state) = world.get_block_state(pos) {
                    let bk: BlockKind = state.into();
                    if bk == kind {
                        found.push(pos);
                    }
                }
            }
        }
    }
    // 按到中心距离排序
    found.sort_by_key(|p| {
        (p.x - cx).pow(2) + (p.y - cy).pow(2) + (p.z - cz).pow(2)
    });
    found
}

/// 统计背包中指定物品数量。
fn count_item(inv: &ContainerHandleRef, kind: ItemKind) -> u32 {
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

pub async fn do_gather(bot: &Client, item: &str, count: u32) -> Result<String, String> {
    let target = ItemKind::from_str(&normalize_item(item))
        .or_else(|_| ItemKind::from_str(item))
        .map_err(|_| format!("未知物品 {item}"))?;
    // 方块种类与物品同 id（oak_log <-> OakLog），直接复用归一化 id 解析 BlockKind。
    let block_kind = BlockKind::from_str(&normalize_item(item))
        .or_else(|_| BlockKind::from_str(item))
        .map_err(|_| format!("无法解析方块种类 {item}（采集需方块形态，如 oak_log / stone / coal_ore）"))?;

    let need = count.max(1);
    let mut gathered = 0u32;
    let max_rounds = 24;

    for _ in 0..max_rounds {
        if gathered >= need {
            break;
        }
        let pos = {
            let world = bot
                .world()
                .map_err(|e| format!("读取世界失败: {e:?}"))?;
            let w = world.read();
            let center = bot.position().map_err(|e| format!("读取坐标失败: {e:?}"))?;
            scan_blocks(&w, center, block_kind, 16)
                .into_iter()
                .next()
        };
        let Some(target_pos) = pos else {
            return Err(format!(
                "附近 16 格内找不到 {item}（已采集 {gathered}/{need}）"
            ));
        };

        // 走到方块下方一格（便于贴脸挖）
        let stand = BlockPos::new(target_pos.x, target_pos.y - 1, target_pos.z);
        bot.start_goto(BlockPosGoal(stand));
        // 等待靠近
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
        bot.stop_pathfinding(); // 停止导航，准备挖掘
        // 挖掘目标方块
        let before = {
            let inv = bot.get_inventory().map_err(|e| format!("{e:?}"))?;
            count_item(&inv, target)
        };
        bot.start_mining(target_pos);
        // 等待该方块被挖掉（背包数量增加）
        let mut done = false;
        for _ in 0..60 {
            sleep(Duration::from_millis(100)).await;
            let inv = match bot.get_inventory() {
                Ok(i) => i,
                Err(_) => continue,
            };
            let now = count_item(&inv, target);
            if now > before {
                gathered = now;
                done = true;
                break;
            }
            // 若方块已消失但数量未变（被他人捡走等），也跳出避免死等
            if let Ok(world) = bot.world() {
                if world.read().get_block_state(target_pos).is_none()
                    || world.read().get_block_state(target_pos).map(|s| s.is_air()).unwrap_or(false)
                {
                    break;
                }
            }
        }
        if !done {
            // 没采到，避免死循环：跳出
            break;
        }
    }

    if gathered >= need {
        Ok(format!("采集 {item} 完成（背包 {gathered} 个）"))
    } else {
        Err(format!("采集 {item} 未完成（仅 {gathered}/{need}，附近可能无更多）"))
    }
}

fn normalize_item(item: &str) -> String {
    if item.starts_with("minecraft:") {
        item.to_string()
    } else {
        format!("minecraft:{item}")
    }
}
