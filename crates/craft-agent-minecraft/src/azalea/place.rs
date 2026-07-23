//! 放置方块 + 打开容器（补齐 3×3 合成 / 熔炼的自主闭环断点）。
//!
//! - `do_place`：把手持物品 item 放到世界坐标 pos 旁（右键放置，需先把该物品选到手中）。
//! - `do_open_container`：打开 pos 处的容器（工作台/熔炉/箱子等），后续 craft_3x3 / smelt 才能操作。

use azalea::BlockPos;
use azalea::container::ContainerHandleRef;
use azalea::prelude::*;
use azalea_registry::builtin::ItemKind;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;

/// 找到背包里持有 item 的 hotbar 槽位（0..=8），无则 None。
fn find_hotbar_slot(inv: &ContainerHandleRef, kind: ItemKind) -> Option<u8> {
    let menu = inv.menu().ok()??;
    let range = menu.player_slots_range();
    let slots = inv.slots()?;
    // hotbar 是 player 槽范围的最后 9 个
    let hotbar: Vec<usize> = range.clone().rev().take(9).collect();
    for &s in &hotbar {
        if let Some(stack) = slots.get(s) {
            if !stack.is_empty() && stack.kind() == kind {
                // s 是绝对槽号，转换为 hotbar 索引（0..=8）
                let last = *range.end();
                let idx = (last as i32 - s as i32).unsigned_abs() as u8;
                if idx <= 8 {
                    return Some(idx);
                }
            }
        }
    }
    None
}

pub async fn do_place(bot: &Client, item: &str, pos: BlockPos) -> Result<String, String> {
    let kind = ItemKind::from_str(&normalize_item(item))
        .or_else(|_| ItemKind::from_str(item))
        .map_err(|_| format!("未知物品 {item}"))?;
    let inv = bot
        .get_inventory()
        .map_err(|e| format!("获取背包失败: {e:?}"))?;
    let slot = find_hotbar_slot(&inv, kind)
        .ok_or_else(|| format!("背包未持有 {item}（无法放置）"))?;
    bot.set_selected_hotbar_slot(slot);
    sleep(Duration::from_millis(50)).await;
    bot.block_interact(pos);
    sleep(Duration::from_millis(120)).await;
    Ok(format!("已放置 {item} 于 ({},{},{}) 旁", pos.x, pos.y, pos.z))
}

pub async fn do_open_container(bot: &Client, pos: BlockPos) -> Result<String, String> {
    let handle = bot
        .open_container_at(pos)
        .await
        .map_err(|e| format!("打开容器失败: {e:?}"))?;
    match handle {
        Some(_) => Ok(format!("已打开容器 ({},{},{})", pos.x, pos.y, pos.z)),
        None => Err(format!("({},{},{}) 处无容器或无法打开", pos.x, pos.y, pos.z)),
    }
}

fn normalize_item(item: &str) -> String {
    if item.starts_with("minecraft:") {
        item.to_string()
    } else {
        format!("minecraft:{item}")
    }
}
