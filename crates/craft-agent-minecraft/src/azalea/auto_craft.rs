//! 数据驱动的通用自动合成：`auto_craft(item, count)` 沿配方图递归满足全部原料，
//! 最终按方式（2×2 / 3×3 / 熔炼 / 采集）产出目标。3×3 与熔炼会自动造并打开放置的
//! 工作台/熔炉。

use azalea::BlockPos;
use azalea::prelude::*;
use azalea_registry::builtin::ItemKind;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;

use crate::azalea::place::{do_open_container, do_place};
use crate::azalea::recipes::{lookup, Method};

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

/// 背包中 item 的数量。
async fn has_item(bot: &Client, item: &str) -> u32 {
    let inv = match bot.get_inventory() {
        Ok(i) => i,
        Err(_) => return 0,
    };
    let menu = match inv.menu().ok().flatten() {
        Some(m) => m,
        None => return 0,
    };
    let range = menu.player_slots_range();
    let slots = match inv.slots() {
        Some(s) => s,
        None => return 0,
    };
    let k = kind(item);
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

/// bot 头顶上方的空气格（用于临时放置工作台/熔炉）。
fn overhead_slot(bot: &Client) -> Option<BlockPos> {
    let p = bot.position().ok()?;
    Some(BlockPos::new(
        p.x.floor() as i32,
        p.y.floor() as i32 + 1,
        p.z.floor() as i32,
    ))
}

/// 确保背包有 `amount` 个 `item`：沿配方图递归满足原料。
async fn ensure(bot: &Client, item: &str, amount: u32) -> Result<(), String> {
    // 已足够则直接返回
    if has_item(bot, item).await >= amount {
        return Ok(());
    }
    let recipe = lookup(item)
        .ok_or_else(|| format!("auto_craft 无法制造 {item}（无配方且非可采集方块）"))?;

    match &recipe.method {
        Method::Gather => {
            crate::azalea::gather::do_gather(bot, item, amount).await?;
        }
        Method::Craft2x2 => {
            for (inp, amt) in recipe.inputs {
                Box::pin(ensure(bot, inp, amt * amount)).await?;
            }
            crate::azalea::craft::do_craft_2x2(bot, item, amount).await?;
        }
        Method::Smelt { fuel } => {
            for (inp, amt) in recipe.inputs {
                Box::pin(ensure(bot, inp, amt * amount)).await?;
            }
            // 确保有熔炉并放置/打开
            Box::pin(ensure(bot, "furnace", 1)).await?;
            let at = overhead_slot(bot).ok_or("无法计算放置点")?;
            do_place(bot, "furnace", at).await?;
            sleep(Duration::from_millis(200)).await;
            do_open_container(bot, at).await?;
            sleep(Duration::from_millis(200)).await;
            crate::azalea::craft::do_smelt(bot, item, fuel, amount).await?;
        }
        Method::Craft3x3 => {
            for (inp, amt) in recipe.inputs {
                Box::pin(ensure(bot, inp, amt * amount)).await?;
            }
            // 确保有工作台并放置/打开
            Box::pin(ensure(bot, "crafting_table", 1)).await?;
            let at = overhead_slot(bot).ok_or("无法计算放置点")?;
            do_place(bot, "crafting_table", at).await?;
            sleep(Duration::from_millis(200)).await;
            do_open_container(bot, at).await?;
            sleep(Duration::from_millis(200)).await;
            crate::azalea::craft::do_craft_3x3(bot, item, amount).await?;
        }
    }
    Ok(())
}

pub async fn do_auto_craft(bot: &Client, item: &str, count: u32) -> Result<String, String> {
    // 先满足产物本身（递归会处理全部原料）
    ensure(bot, item, count).await?;
    let have = has_item(bot, item).await;
    if have >= count {
        Ok(format!("auto_craft 完成：{item} x{count}（背包现 {have}）"))
    } else {
        Err(format!(
            "auto_craft 未达预期：{item} 仅 {have}/{count}（可能原料不足）"
        ))
    }
}
