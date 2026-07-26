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
    let already = has_item(bot, item).await;
    if already >= amount {
        return Ok(());
    }
    // P11 修复（2026-07-26）：原代码无视背包已有的中间产物数量，
    // 总是按 `amt * amount` 全量采集原料——例如要 4 planks 但已有 1 plank，
    // 仍去采 4 logs（多采了 1 个 log，浪费 + 在地下找不到 log 时误报 100% 失败）。
    // 修复：实际需要补足 `amount - already` 个，原料量按此差值计算。
    let needed = amount.saturating_sub(already);

    // 优先用本地配方书（覆盖 3×3 合成 / 锻造 / 熔炼等），免手写表。
    if let Some(res) = ensure_via_book(bot, item, needed).await {
        return res;
    }
    let recipe = lookup(item)
        .ok_or_else(|| format!("auto_craft 无法制造 {item}（无配方且非可采集方块）"))?;

    match &recipe.method {
        Method::Gather => {
            // P11 修复：地下（Y < 60）采集 surface 资源（oak_log/sand/sugar_cane 等）几乎必失败。
            // 提前给出可操作建议：去地表/换区域，而不是浪费 32 格半径的搜索。
            if is_surface_resource(item) && is_bot_underground(bot) {
                return Err(format!(
                    "auto_craft 失败：需要采集 {item}（地表资源）但 bot 当前在地下（Y<60），32 格内找不到。\
                     建议：1) 先 go 到地表（Y>62）再 auto_craft；\
                     2) 或换一个不依赖 {item} 的合成路径；\
                     3) 或用 craft_3x3/craft_2x2 手动合成（背包已有部分原料时）。",
                ));
            }
            crate::azalea::gather::do_gather(bot, item, needed).await?;
        }
        Method::Craft2x2 => {
            for (inp, amt) in recipe.inputs {
                // P11 修复：按 needed（差值）计算原料需求，而非全量 amount
                Box::pin(ensure(bot, inp, amt * needed)).await?;
            }
            crate::azalea::craft::do_craft_2x2(bot, item, needed).await?;
        }
        Method::Smelt { fuel } => {
            for (inp, amt) in recipe.inputs {
                Box::pin(ensure(bot, inp, amt * needed)).await?;
            }
            // 确保有熔炉并放置/打开
            Box::pin(ensure(bot, "furnace", 1)).await?;
            let at = overhead_slot(bot).ok_or("无法计算放置点")?;
            do_place(bot, "furnace", at).await?;
            sleep(Duration::from_millis(200)).await;
            do_open_container(bot, at).await?;
            sleep(Duration::from_millis(200)).await;
            crate::azalea::craft::do_smelt(bot, item, fuel, needed).await?;
        }
        Method::Craft3x3 => {
            // 优先用服务端配方书（精确原料，免手写表）；否则走手写 SHAPED_RECIPES
            {
                let book = recipe_book_of(bot);
                if let Some(r) = book.get_by_result(item) {
                    match r {
                        StoredRecipe::Shaped { .. } | StoredRecipe::Shapeless { .. } => {
                            // 先满足配方书里的全部原料
                            ensure_recipe_inputs(bot, r, needed).await?;
                            // 确保有工作台并放置/打开
                            Box::pin(ensure(bot, "crafting_table", 1)).await?;
                            let at = overhead_slot(bot).ok_or("无法计算放置点")?;
                            do_place(bot, "crafting_table", at).await?;
                            sleep(Duration::from_millis(200)).await;
                            do_open_container(bot, at).await?;
                            sleep(Duration::from_millis(200)).await;
                            return crate::azalea::craft::do_craft_3x3_recipe(bot, r, needed)
                                .await
                                .map(|_| ());
                        }
                        StoredRecipe::Smithing { .. } => {
                            // 先满足模板/基础/附加三类原料
                            ensure_recipe_inputs(bot, r, needed).await?;
                            // 确保有锻造台并放置/打开
                            Box::pin(ensure(bot, "smithing_table", 1)).await?;
                            let at = overhead_slot(bot).ok_or("无法计算放置点")?;
                            do_place(bot, "smithing_table", at).await?;
                            sleep(Duration::from_millis(200)).await;
                            do_open_container(bot, at).await?;
                            sleep(Duration::from_millis(200)).await;
                            return crate::azalea::craft::do_craft_smithing(bot, r, needed)
                                .await
                                .map(|_| ());
                        }
                        _ => {}
                    }
                }
            }
            for (inp, amt) in recipe.inputs {
                Box::pin(ensure(bot, inp, amt * needed)).await?;
            }
            // 确保有工作台并放置/打开
            Box::pin(ensure(bot, "crafting_table", 1)).await?;
            let at = overhead_slot(bot).ok_or("无法计算放置点")?;
            do_place(bot, "crafting_table", at).await?;
            sleep(Duration::from_millis(200)).await;
            do_open_container(bot, at).await?;
            sleep(Duration::from_millis(200)).await;
            crate::azalea::craft::do_craft_3x3(bot, item, needed, None).await?;
        }
    }
    Ok(())
}

/// P11 新增：判断 item 是否为「地表资源」（地下找不到）。
///
/// 用于 auto_craft 提前检测「地下采集 oak_log 等地表资源」的徒劳场景，
/// 给 LLM 一个可操作的错误（"先 go 到地表"），而不是浪费 32 格搜索后失败。
fn is_surface_resource(item: &str) -> bool {
    let b = item.strip_prefix("minecraft:").unwrap_or(item);
    matches!(
        b,
        "oak_log" | "spruce_log" | "birch_log" | "jungle_log"
            | "acacia_log" | "dark_oak_log" | "mangrove_log" | "cherry_log"
            | "pale_oak_log" | "bamboo" | "sugar_cane" | "cactus"
            | "sand" | "red_sand" | "lily_pad" | "vine" | "moss_block"
            | "grass_block" | "tall_grass" | "fern" | "large_fern"
            | "oak_leaves" | "spruce_leaves" | "birch_leaves" | "jungle_leaves"
            | "acacia_leaves" | "dark_oak_leaves" | "mangrove_leaves" | "cherry_leaves"
    )
}

/// P11 新增：判断 bot 是否在地下（Y < 60 且头顶非空气）。
fn is_bot_underground(bot: &Client) -> bool {
    let Ok(p) = bot.position() else { return false; };
    if p.y >= 60.0 {
        return false;
    }
    // 进一步检查：bot 头顶上方 1-3 格是否有实心方块（地道/洞穴也可能 Y<60 但头顶是空气）
    let Ok(world_lock) = bot.world() else { return false; };
    let world = world_lock.read();
    let bx = p.x.floor() as i32;
    let by = p.y.floor() as i32;
    let bz = p.z.floor() as i32;
    // 检查头顶 2-5 格是否有非空气方块（地表覆盖判定）
    for dy in 2..=5 {
        let pos = BlockPos::new(bx, by + dy, bz);
        if world.get_block_state(pos).map(|s| !s.is_air()).unwrap_or(false) {
            return true; // 头顶有方块遮挡 → 在地下
        }
    }
    false
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
