//! 高层自动合成管道：把"采集→2×2合成→放置工作台→开→3×3合成"串成一条指令。
//!
//! 当前覆盖**纯木制品链**（oak_planks / stick / crafting_table / chest），这是早期游戏
//! 最常见需求。LLM 只需 `auto_craft chest 1`，bot 自主完成全链路。
//! 其他目标回退到分步工具（gather/craft/place/open）。

use azalea::BlockPos;
use azalea::prelude::*;
use azalea_registry::builtin::ItemKind;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;

use crate::azalea::craft::{do_craft_2x2, do_craft_3x3};
use crate::azalea::gather::do_gather;
use crate::azalea::place::do_place;

fn kind(id: &str) -> ItemKind {
    ItemKind::from_str(&normalize(id)).unwrap()
}

fn normalize(item: &str) -> String {
    if item.starts_with("minecraft:") {
        item.to_string()
    } else {
        format!("minecraft:{item}")
    }
}

/// 统计背包中指定物品数量（仅玩家主背包+热栏）。
fn count_item(inv: &azalea::container::ContainerHandleRef, k: ItemKind) -> u32 {
    let menu = match inv.menu().ok().flatten() {
        Some(m) => m,
        None => return 0,
    };
    let range = menu.player_slots_range();
    let slots = match inv.slots() {
        Some(s) => s,
        None => return 0,
    };
    let mut t = 0u32;
    for s in range {
        if let Some(st) = slots.get(s) {
            if !st.is_empty() && st.kind() == k {
                t += st.count().max(0) as u32;
            }
        }
    }
    t
}

/// 估算目标所需 oak_planks 数量（木链）。
fn planks_needed(item: &str, count: u32) -> Option<u32> {
    match item {
        "oak_planks" => Some(count), // 1 log→4 planks，这里 count 视为期望 planks 数
        "stick" => Some((count * 2).div_ceil(4) * 4), // 每 2 planks→4 sticks，向上取整到 4 的倍数
        "crafting_table" => Some(4 * count),
        "chest" => Some(8 * count),
        _ => None,
    }
}

pub async fn do_auto_craft(bot: &Client, item: &str, count: u32) -> Result<String, String> {
    let need = planks_needed(item, count)
        .ok_or_else(|| format!("auto_craft 暂仅支持木制品：oak_planks/stick/crafting_table/chest（{item} 请用分步工具）"))?;

    // 1) 确保有足够 oak_planks
    let planks = kind("oak_planks");
    let logs = kind("oak_log");
    loop {
        let inv = bot.get_inventory().map_err(|e| format!("{e:?}"))?;
        let have = count_item(&inv, planks);
        if have >= need {
            break;
        }
        // 还需要 planks，先确保有 log
        let have_logs = count_item(&inv, logs);
        if have_logs == 0 {
            do_gather(bot, "oak_log", 1).await?;
        }
        // 把 1 个 log 转 4 planks
        do_craft_2x2(bot, "oak_planks", 4).await?;
    }

    // 2) 按目标收尾
    match item {
        "oak_planks" => Ok(format!("auto_craft 完成：oak_planks 已备足（{need}）")),
        "stick" => {
            do_craft_2x2(bot, "stick", count).await?;
            Ok(format!("auto_craft 完成：stick x{count}"))
        }
        "crafting_table" => {
            do_craft_2x2(bot, "crafting_table", count).await?;
            Ok(format!("auto_craft 完成：crafting_table x{count}（在背包，可用 place 放下）"))
        }
        "chest" => {
            // 需要工作台：背包若无则造一个（消耗 4 planks），放下并打开，再 3x3 合成 chest
            let inv = bot.get_inventory().map_err(|e| format!("{e:?}"))?;
            if count_item(&inv, kind("crafting_table")) == 0 {
                do_craft_2x2(bot, "crafting_table", 1).await?;
            }
            // 放到 bot 头顶上方的空气格，并打开
            let p = bot.position().map_err(|e| format!("{e:?}"))?;
            let fx = p.x.floor() as i32;
            let fy = p.y.floor() as i32;
            let fz = p.z.floor() as i32;
            let at = BlockPos::new(fx, fy + 1, fz);
            do_place(bot, "crafting_table", at).await?;
            sleep(Duration::from_millis(200)).await;
            // 打开刚放置的工作台
            crate::azalea::place::do_open_container(bot, at).await?;
            sleep(Duration::from_millis(200)).await;
            do_craft_3x3(bot, "chest", count).await?;
            Ok(format!("auto_craft 完成：chest x{count}（工作台已留在世界 ({},{},{})）", at.x, at.y, at.z))
        }
        _ => unreachable!(),
    }
}
