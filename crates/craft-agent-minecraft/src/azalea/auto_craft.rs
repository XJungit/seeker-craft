//! 数据驱动的通用自动合成：`auto_craft(item, count)` 沿配方图递归满足全部原料，
//! 最终按方式（2×2 / 3×3 / 熔炼 / 采集）产出目标。3×3 与熔炼会自动造并打开放置的
//! 工作台/熔炉。

use azalea::BlockPos;
use azalea::prelude::*;
use azalea_registry::builtin::ItemKind;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;

use crate::azalea::ext_state::BotExtResource;
use crate::azalea::place::{do_open_container, do_place};
use crate::azalea::recipe_book::{RecipeBook, StoredRecipe};
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

/// 从 ecs 资源读取服务端下发的配方书（若有）。
fn recipe_book_of(bot: &Client) -> RecipeBook {
    bot.ecs.read().resource::<BotExtResource>().0.lock().unwrap().recipes.clone()
}

/// 统计某配方书配方所需的各原料物品数量（按网格出现次数计）。
fn recipe_input_counts(r: &StoredRecipe) -> Vec<(String, u32)> {
    use std::collections::HashMap;
    let mut map: HashMap<String, u32> = HashMap::new();
    match r {
        StoredRecipe::Shaped { grid, .. } => {
            for cell in grid {
                if let Some(ing) = cell {
                    if let Some(k) = ing.items.first() {
                        *map.entry(k.to_string()).or_insert(0) += 1;
                    }
                }
            }
        }
        StoredRecipe::Shapeless { ingredients, .. } => {
            for ing in ingredients {
                if let Some(k) = ing.items.first() {
                    *map.entry(k.to_string()).or_insert(0) += 1;
                }
            }
        }
        _ => {}
    }
    map.into_iter().collect()
}

/// 递归满足配方书配方的全部原料。
async fn ensure_recipe_inputs(bot: &Client, r: &StoredRecipe, amount: u32) -> Result<(), String> {
    for (item, per) in recipe_input_counts(r) {
        Box::pin(ensure(bot, &item, per * amount)).await?;
    }
    Ok(())
}

/// 若 `item` 可用本地配方书合成，则合成并返回 Some(结果)；否则 None（交给手写 recipes.rs）。
/// 判断配方是否需要放置的工作台（3×3 网格）。2×2 背包合成（木板/木棍/工作台/火把等）
/// 直接在玩家背包的合成格里完成，无需放置工作台——否则会陷入「造木板需要工作台、
/// 造工作台需要木板」的死循环。
fn recipe_needs_table(r: &StoredRecipe) -> bool {
    match r {
        StoredRecipe::Shaped { grid, .. } => grid.iter().enumerate().any(|(i, c)| c.is_some() && i >= 4),
        StoredRecipe::Shapeless { .. } => false,
        _ => false,
    }
}

async fn ensure_via_book(bot: &Client, item: &str, amount: u32) -> Option<Result<(), String>> {
    let r = recipe_book_of(bot).get_by_result(item)?.clone();
    match &r {
        StoredRecipe::Shaped { .. } | StoredRecipe::Shapeless { .. } => {
            if let Err(e) = ensure_recipe_inputs(bot, &r, amount).await {
                return Some(Err(e));
            }
            if recipe_needs_table(&r) {
                // 3×3：确保有工作台并放置/打开后合成
                if item != "crafting_table" {
                    if let Err(e) = Box::pin(ensure(bot, "crafting_table", 1)).await {
                        return Some(Err(e));
                    }
                }
                let at = match overhead_slot(bot) {
                    Some(a) => a,
                    None => return Some(Err("无法计算放置点".to_string())),
                };
                if let Err(e) = do_place(bot, "crafting_table", at).await {
                    return Some(Err(e));
                }
                sleep(Duration::from_millis(200)).await;
                if let Err(e) = do_open_container(bot, at).await {
                    return Some(Err(e));
                }
                sleep(Duration::from_millis(200)).await;
                Some(crate::azalea::craft::do_craft_3x3_recipe(bot, &r, amount).await.map(|_| ()))
            } else {
                // 2×2：直接在玩家背包合成格里完成（手写表已覆盖木板/木棍/工作台/火把）
                Some(
                    crate::azalea::craft::do_craft_2x2(bot, item, amount)
                        .await
                        .map(|_| ()),
                )
            }
        }
        StoredRecipe::Smithing { .. } => {
            if let Err(e) = ensure_recipe_inputs(bot, &r, amount).await {
                return Some(Err(e));
            }
            if item != "smithing_table" {
                if let Err(e) = Box::pin(ensure(bot, "smithing_table", 1)).await {
                    return Some(Err(e));
                }
            }
            let at = match overhead_slot(bot) {
                Some(a) => a,
                None => return Some(Err("无法计算放置点".to_string())),
            };
            if let Err(e) = do_place(bot, "smithing_table", at).await {
                return Some(Err(e));
            }
            sleep(Duration::from_millis(200)).await;
            if let Err(e) = do_open_container(bot, at).await {
                return Some(Err(e));
            }
            sleep(Duration::from_millis(200)).await;
            Some(
                crate::azalea::craft::do_craft_smithing(bot, &r, amount)
                    .await
                    .map(|_| ()),
            )
        }
        StoredRecipe::Stonecutter { .. } => {
            if let Err(e) = ensure_recipe_inputs(bot, &r, amount).await {
                return Some(Err(e));
            }
            if item != "stonecutter" {
                if let Err(e) = Box::pin(ensure(bot, "stonecutter", 1)).await {
                    return Some(Err(e));
                }
            }
            let at = match overhead_slot(bot) {
                Some(a) => a,
                None => return Some(Err("无法计算放置点".to_string())),
            };
            if let Err(e) = do_place(bot, "stonecutter", at).await {
                return Some(Err(e));
            }
            sleep(Duration::from_millis(200)).await;
            if let Err(e) = do_open_container(bot, at).await {
                return Some(Err(e));
            }
            sleep(Duration::from_millis(200)).await;
            Some(
                crate::azalea::craft::do_craft_stonecutter(bot, &r, amount)
                    .await
                    .map(|_| ()),
            )
        }
        StoredRecipe::Brewing { .. } => {
            if let Err(e) = ensure_recipe_inputs(bot, &r, amount).await {
                return Some(Err(e));
            }
            if item != "brewing_stand" {
                if let Err(e) = Box::pin(ensure(bot, "brewing_stand", 1)).await {
                    return Some(Err(e));
                }
            }
            let at = match overhead_slot(bot) {
                Some(a) => a,
                None => return Some(Err("无法计算放置点".to_string())),
            };
            if let Err(e) = do_place(bot, "brewing_stand", at).await {
                return Some(Err(e));
            }
            sleep(Duration::from_millis(200)).await;
            if let Err(e) = do_open_container(bot, at).await {
                return Some(Err(e));
            }
            sleep(Duration::from_millis(200)).await;
            Some(crate::azalea::craft::do_brew(bot, &r, amount).await.map(|_| ()))
        }
        StoredRecipe::Furnace { .. } => {
            if let Err(e) = ensure_recipe_inputs(bot, &r, amount).await {
                return Some(Err(e));
            }
            if item != "furnace" {
                if let Err(e) = Box::pin(ensure(bot, "furnace", 1)).await {
                    return Some(Err(e));
                }
            }
            let at = match overhead_slot(bot) {
                Some(a) => a,
                None => return Some(Err("无法计算放置点".to_string())),
            };
            if let Err(e) = do_place(bot, "furnace", at).await {
                return Some(Err(e));
            }
            sleep(Duration::from_millis(200)).await;
            if let Err(e) = do_open_container(bot, at).await {
                return Some(Err(e));
            }
            sleep(Duration::from_millis(200)).await;
            // 取熔炼原料（配方书第一条为原料）
            let ing = match &r {
                StoredRecipe::Furnace { ingredient, .. } => ingredient.items.first().copied(),
                _ => None,
            };
            let fuel = match &r {
                StoredRecipe::Furnace { fuel, .. } => fuel.items.first().copied(),
                _ => None,
            };
            match (ing, fuel) {
                (Some(i), Some(f)) => {
                    let ik = i.to_string();
                    let fk = f.to_string();
                    Some(
                        crate::azalea::craft::do_smelt(bot, &ik, &fk, amount)
                            .await
                            .map(|_| ()),
                    )
                }
                _ => Some(Err("锻造配方原料解析失败".to_string())),
            }
        }
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
    // 优先用本地配方书（覆盖 3×3 合成 / 锻造 / 熔炼等），免手写表。
    if let Some(res) = ensure_via_book(bot, item, amount).await {
        return res;
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
            // 优先用服务端配方书（精确原料，免手写表）；否则走手写 SHAPED_RECIPES
            {
                let book = recipe_book_of(bot);
                if let Some(r) = book.get_by_result(item) {
                    match r {
                        StoredRecipe::Shaped { .. } | StoredRecipe::Shapeless { .. } => {
                            // 先满足配方书里的全部原料
                            ensure_recipe_inputs(bot, r, amount).await?;
                            // 确保有工作台并放置/打开
                            Box::pin(ensure(bot, "crafting_table", 1)).await?;
                            let at = overhead_slot(bot).ok_or("无法计算放置点")?;
                            do_place(bot, "crafting_table", at).await?;
                            sleep(Duration::from_millis(200)).await;
                            do_open_container(bot, at).await?;
                            sleep(Duration::from_millis(200)).await;
                            return crate::azalea::craft::do_craft_3x3_recipe(bot, r, amount)
                                .await
                                .map(|_| ());
                        }
                        StoredRecipe::Smithing { .. } => {
                            // 先满足模板/基础/附加三类原料
                            ensure_recipe_inputs(bot, r, amount).await?;
                            // 确保有锻造台并放置/打开
                            Box::pin(ensure(bot, "smithing_table", 1)).await?;
                            let at = overhead_slot(bot).ok_or("无法计算放置点")?;
                            do_place(bot, "smithing_table", at).await?;
                            sleep(Duration::from_millis(200)).await;
                            do_open_container(bot, at).await?;
                            sleep(Duration::from_millis(200)).await;
                            return crate::azalea::craft::do_craft_smithing(bot, r, amount)
                                .await
                                .map(|_| ());
                        }
                        _ => {}
                    }
                }
            }
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
