//! 2×2 背包合成（azalea 26.2 公开 API 实现，不依赖 azalea 源码改动）。
//!
//! 玩家自带 2×2 合成网格（无需工作台）。槽位布局（见 azalea-inventory
//! `Menu::Player` 宏生成）：
//! - slot 0  = 合成结果（craft_result）
//! - slot 1..=4 = 2×2 输入网格（craft）
//! - slot 5..=8 = 盔甲
//! - 其余 = 主背包 + 快捷栏（player_slots_range）
//!
//! 策略：对每个配方原料，在背包里 shift_click（QuickMove）将其填入网格
//! （服务端按当前配方自动只放进所需数量，多余留在背包）。等待服务端算出
//! 结果后 shift_click(slot 0) 把产物收进背包。循环至满足数量。

use azalea::container::ContainerHandleRef;
use azalea::prelude::*;
use azalea_registry::builtin::ItemKind;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;

/// 解析后的合成计划：原料种类 + 每次合成消耗数 + 每次产出数。
struct CraftPlan {
    ingredients: Vec<(ItemKind, u32)>,
    output_per_craft: u32,
}

/// 静态配方表：目标物品 -> (原料 id 列表, 每次产出数)。
/// 原料 id 用 `minecraft:` 命名空间，可省略前缀。
const RECIPES: &[(&'static str, &'static [(&'static str, u32)], u32)] = &[
    ("oak_planks", &[("oak_log", 1)], 4),
    ("stick", &[("oak_planks", 2)], 4),
    ("crafting_table", &[("oak_planks", 4)], 1),
    ("torch", &[("coal", 1), ("stick", 1)], 4),
    ("torch", &[("charcoal", 1), ("stick", 1)], 4),
    ("chest", &[("oak_planks", 8)], 1),
    ("wooden_pickaxe", &[("oak_planks", 3), ("stick", 2)], 1),
    ("furnace", &[("cobblestone", 8)], 1),
];

/// 把 `oak_log`/`spruce_log`/... 这类原木映射到对应木板（动态派生，无需逐条登记）。
fn planks_plan_for(log_id: &str) -> Option<CraftPlan> {
    let wood = log_id.strip_suffix("_log").or_else(|| log_id.strip_suffix("_stem"))?;
    let planks = format!("minecraft:{wood}_planks");
    // 校验木板 id 合法
    if ItemKind::from_str(&planks).is_err() {
        return None;
    }
    let kind = ItemKind::from_str(&normalize_item(log_id)).ok()?;
    Some(CraftPlan {
        ingredients: vec![(kind, 1)],
        output_per_craft: 4,
    })
}

fn lookup_recipe(item: &str) -> Option<CraftPlan> {
    let norm = normalize_item(item);
    // 木板动态派生优先（覆盖所有原木种类）
    if let Some(p) = planks_plan_for(&norm) {
        return Some(p);
    }
    RECIPES
        .iter()
        .find(|(id, _, _)| *id == norm)
        .map(|(_, ings, out)| CraftPlan {
            ingredients: ings
                .iter()
                .map(|(id, amt)| (ItemKind::from_str(&normalize_item(id)).unwrap(), *amt))
                .collect(),
            output_per_craft: *out,
        })
}

fn normalize_item(item: &str) -> String {
    if item.starts_with("minecraft:") {
        item.to_string()
    } else {
        format!("minecraft:{item}")
    }
}

/// 在玩家背包范围（排除网格/盔甲）内找到第一个含指定物品种类的槽位。
fn find_source_slot(inv: &ContainerHandleRef, kind: ItemKind) -> Option<usize> {
    let menu = inv.menu().ok()??;
    let slots = inv.slots()?;
    let range = menu.player_slots_range();
    for s in range {
        if let Some(stack) = slots.get(s) {
            if !stack.is_empty() && stack.kind() == kind {
                return Some(s);
            }
        }
    }
    None
}

/// 执行 2×2 合成。返回人类可读结果串（供 tool / 日志使用）。
pub async fn do_craft_2x2(bot: &Client, item: &str, count: u32) -> Result<String, String> {
    let plan = lookup_recipe(item).ok_or_else(|| {
        format!(
            "不支持的合成目标 {item}（当前仅支持 2×2 配方：木板/木棍/工作台/火把/箱子/木镐/熔炉等）"
        )
    })?;

    let inv = bot
        .get_inventory()
        .map_err(|e| format!("获取背包失败: {e:?}"))?;

    let output = plan.output_per_craft;
    let crafts_needed = (count.max(1) + output - 1) / output;
    let mut crafted = 0u32;

    for _ in 0..crafts_needed {
        // 1) 把每种原料 shift_click 进网格（服务端按配方填充所需数量）
        for (kind, _amt) in &plan.ingredients {
            let src = find_source_slot(&inv, *kind).ok_or_else(|| {
                format!("背包缺少原料 {}", kind.to_str())
            })?;
            inv.shift_click(src);
            // 让服务端处理点击并回填网格
            sleep(Duration::from_millis(40)).await;
        }
        // 让服务端计算合成结果（slot 0）
        sleep(Duration::from_millis(80)).await;

        // 2) 检查结果槽是否有产物
        let has_result = {
            let slots = inv.slots();
            slots
                .as_ref()
                .and_then(|slots| slots.get(0))
                .map(|s| !s.is_empty())
                .unwrap_or(false)
        };
        if !has_result {
            return Err(format!(
                "合成 {item} 失败：网格未产生结果（原料可能不足或配方不匹配）"
            ));
        }

        // 3) 收产物进背包
        inv.shift_click(0usize);
        sleep(Duration::from_millis(40)).await;
        crafted += output;
    }

    Ok(format!(
        "合成 {item} x{count} 完成（实际产出约 {crafted}，共 {crafts_needed} 次）"
    ))
}

/// 把 src 槽的整堆物品移到 dst 槽（两次 left_click：拿起→放下）。
async fn move_stack(inv: &ContainerHandleRef, src: usize, dst: usize) {
    inv.left_click(src);
    sleep(Duration::from_millis(20)).await;
    inv.left_click(dst);
    sleep(Duration::from_millis(20)).await;
}

/// 3×3 工作台合成（要求已打开工作台，即 Crafting 菜单）。
/// 网格槽位：result=0，grid=1..=9。配方为"环形摆放"（除中心外的 8 格放同种原料），
/// 覆盖 furnace / chest 等。vanilla 仅校验形状，多余数量无影响。
struct ShapedRecipe {
    /// 参与摆放的网格槽（1..=9）。
    slots: &'static [usize],
    /// 原料物品 id（所有格子用同一种）。
    ingredient: &'static str,
    output_per_craft: u32,
}

const SHAPED_RECIPES: &[(&'static str, ShapedRecipe)] = &[
    (
        "furnace",
        ShapedRecipe {
            slots: &[1, 2, 3, 4, 6, 7, 8, 9],
            ingredient: "cobblestone",
            output_per_craft: 1,
        },
    ),
    (
        "chest",
        ShapedRecipe {
            slots: &[1, 2, 3, 4, 6, 7, 8, 9],
            ingredient: "oak_planks",
            output_per_craft: 1,
        },
    ),
];

fn lookup_shaped(item: &str) -> Option<ShapedRecipe> {
    let norm = normalize_item(item);
    SHAPED_RECIPES
        .iter()
        .find(|(id, _)| *id == norm)
        .map(|(_, r)| ShapedRecipe {
            slots: r.slots,
            ingredient: r.ingredient,
            output_per_craft: r.output_per_craft,
        })
}

pub async fn do_craft_3x3(bot: &Client, item: &str, count: u32) -> Result<String, String> {
    let recipe = lookup_shaped(item).ok_or_else(|| {
        format!("不支持的 3×3 合成目标 {item}（当前仅 furnace / chest，且需先打开工作台）")
    })?;
    let ing = ItemKind::from_str(&normalize_item(recipe.ingredient))
        .map_err(|_| format!("未知原料 {}", recipe.ingredient))?;

    let inv = bot
        .get_inventory()
        .map_err(|e| format!("获取容器失败（确认已打开工作台）: {e:?}"))?;

    let output = recipe.output_per_craft;
    let crafts_needed = (count.max(1) + output - 1) / output;
    let mut crafted = 0u32;

    for _ in 0..crafts_needed {
        let src = find_source_slot(&inv, ing)
            .ok_or_else(|| format!("背包缺少原料 {}", recipe.ingredient))?;
        for &g in recipe.slots {
            move_stack(&inv, src, g).await;
        }
        sleep(Duration::from_millis(80)).await;
        let has_result = {
            let slots = inv.slots();
            slots
                .as_ref()
                .and_then(|s| s.get(0))
                .map(|s| !s.is_empty())
                .unwrap_or(false)
        };
        if !has_result {
            return Err(format!("合成 {item} 失败：网格未产生结果（原料可能不足）"));
        }
        inv.shift_click(0usize);
        sleep(Duration::from_millis(40)).await;
        crafted += output;
    }

    Ok(format!(
        "3×3 合成 {item} x{count} 完成（约 {crafted}，共 {crafts_needed} 次）"
    ))
}

/// 熔炼配方：产物 -> (输入物品 id, 每次产出数)。
struct SmeltRecipe {
    input: &'static str,
    output_per_craft: u32,
}

const SMELT_RECIPES: &[(&'static str, SmeltRecipe)] = &[
    ("iron_ingot", SmeltRecipe { input: "iron_ore", output_per_craft: 1 }),
    ("iron_ingot", SmeltRecipe { input: "raw_iron", output_per_craft: 1 }),
    ("copper_ingot", SmeltRecipe { input: "copper_ore", output_per_craft: 1 }),
    ("copper_ingot", SmeltRecipe { input: "raw_copper", output_per_craft: 1 }),
    ("gold_ingot", SmeltRecipe { input: "gold_ore", output_per_craft: 1 }),
    ("gold_ingot", SmeltRecipe { input: "raw_gold", output_per_craft: 1 }),
    ("glass", SmeltRecipe { input: "sand", output_per_craft: 1 }),
    ("stone", SmeltRecipe { input: "cobblestone", output_per_craft: 1 }),
    ("smooth_stone", SmeltRecipe { input: "stone", output_per_craft: 1 }),
    ("charcoal", SmeltRecipe { input: "oak_log", output_per_craft: 1 }),
    ("baked_potato", SmeltRecipe { input: "potato", output_per_craft: 1 }),
];

fn lookup_smelt(output: &str) -> Option<SmeltRecipe> {
    let norm = normalize_item(output);
    SMELT_RECIPES
        .iter()
        .find(|(id, _)| *id == norm)
        .map(|(_, r)| SmeltRecipe {
            input: r.input,
            output_per_craft: r.output_per_craft,
        })
}

pub async fn do_smelt(
    bot: &Client,
    output: &str,
    fuel: &str,
    count: u32,
) -> Result<String, String> {
    let recipe = lookup_smelt(output).ok_or_else(|| {
        format!("不支持的熔炼产物 {output}（当前支持 iron_ingot/copper_ingot/gold_ingot/glass/stone/charcoal 等，需先打开熔炉）")
    })?;
    let input_kind = ItemKind::from_str(&normalize_item(recipe.input))
        .map_err(|_| format!("未知输入 {}", recipe.input))?;
    let fuel_kind = ItemKind::from_str(&normalize_item(fuel))
        .map_err(|_| format!("未知燃料 {fuel}"))?;

    let inv = bot
        .get_inventory()
        .map_err(|e| format!("获取容器失败（确认已打开熔炉）: {e:?}"))?;

    // Furnace 菜单槽位：ingredient=0, fuel=1, result=2
    let output_per = recipe.output_per_craft;
    let crafts_needed = (count.max(1) + output_per - 1) / output_per;
    let mut smelted = 0u32;

    for _ in 0..crafts_needed {
        let src_in = find_source_slot(&inv, input_kind)
            .ok_or_else(|| format!("背包缺少输入 {}", recipe.input))?;
        let src_fuel = find_source_slot(&inv, fuel_kind)
            .ok_or_else(|| format!("背包缺少燃料 {fuel}"))?;
        move_stack(&inv, src_in, 0).await; // 输入槽
        move_stack(&inv, src_fuel, 1).await; // 燃料槽
        // 等待熔炼完成（粗略等待；实际可轮询结果槽）
        sleep(Duration::from_millis(1200)).await;
        let has_result = {
            let slots = inv.slots();
            slots
                .as_ref()
                .and_then(|s| s.get(2))
                .map(|s| !s.is_empty())
                .unwrap_or(false)
        };
        if !has_result {
            return Err(format!("熔炼 {output} 失败：结果槽无产物（输入/燃料不足或未完成）"));
        }
        inv.shift_click(2usize);
        sleep(Duration::from_millis(40)).await;
        smelted += output_per;
    }

    Ok(format!(
        "熔炼 {output} x{count} 完成（约 {smelted}，共 {crafts_needed} 次）"
    ))
}
