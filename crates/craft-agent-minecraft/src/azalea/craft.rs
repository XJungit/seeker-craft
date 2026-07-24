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
use azalea::inventory::operations::PickupClick;
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
/// 网格槽位：result=0，grid=1..=9（1=左上,2=中上,3=右上,4=左中,5=中,6=右中,7=左下,8=中下,9=右下）。
/// 每个配方按 vanilla 形状给定「每格放什么原料」。
struct ShapedRecipe {
    /// (网格槽 1..=9, 原料物品 id) 列表，按 vanilla 合成形状摆放。
    cells: &'static [(usize, &'static str)],
    output_per_craft: u32,
}

const SHAPED_RECIPES: &[(&'static str, ShapedRecipe)] = &[
    // 环形：8 格同种原料
    ("furnace", ShapedRecipe { cells: &[(1,"cobblestone"),(2,"cobblestone"),(3,"cobblestone"),(4,"cobblestone"),(6,"cobblestone"),(7,"cobblestone"),(8,"cobblestone"),(9,"cobblestone")], output_per_craft: 1 }),
    ("chest", ShapedRecipe { cells: &[(1,"oak_planks"),(2,"oak_planks"),(3,"oak_planks"),(4,"oak_planks"),(6,"oak_planks"),(7,"oak_planks"),(8,"oak_planks"),(9,"oak_planks")], output_per_craft: 1 }),
    ("ladder", ShapedRecipe { cells: &[(1,"stick"),(2,"stick"),(3,"stick"),(4,"stick"),(5,"stick"),(6,"stick"),(7,"stick"),(8,"stick"),(9,"stick")], output_per_craft: 3 }),
    ("oak_trapdoor", ShapedRecipe { cells: &[(1,"oak_planks"),(2,"oak_planks"),(3,"oak_planks"),(4,"oak_planks"),(5,"oak_planks"),(6,"oak_planks")], output_per_craft: 2 }),
    // 门：两列木板
    ("oak_door", ShapedRecipe { cells: &[(1,"oak_planks"),(2,"oak_planks"),(4,"oak_planks"),(5,"oak_planks"),(7,"oak_planks"),(8,"oak_planks")], output_per_craft: 3 }),
    // 栅栏：上下木板 + 中间棍
    ("oak_fence", ShapedRecipe { cells: &[(1,"oak_planks"),(2,"oak_planks"),(4,"stick"),(5,"stick"),(7,"oak_planks"),(8,"oak_planks")], output_per_craft: 3 }),
    // 工具：木镐
    ("wooden_pickaxe", ShapedRecipe { cells: &[(1,"oak_planks"),(3,"oak_planks"),(4,"oak_planks"),(5,"stick"),(7,"stick")], output_per_craft: 1 }),
    ("wooden_axe", ShapedRecipe { cells: &[(1,"oak_planks"),(2,"oak_planks"),(4,"oak_planks"),(5,"stick"),(7,"stick")], output_per_craft: 1 }),
    ("wooden_sword", ShapedRecipe { cells: &[(2,"oak_planks"),(5,"oak_planks"),(8,"stick")], output_per_craft: 1 }),
    ("wooden_shovel", ShapedRecipe { cells: &[(2,"oak_planks"),(5,"stick"),(8,"stick")], output_per_craft: 1 }),
    ("wooden_hoe", ShapedRecipe { cells: &[(1,"oak_planks"),(2,"oak_planks"),(4,"stick"),(7,"stick")], output_per_craft: 1 }),
    // 石制工具（用 cobblestone 代替木板）
    ("stone_pickaxe", ShapedRecipe { cells: &[(1,"cobblestone"),(3,"cobblestone"),(4,"cobblestone"),(5,"stick"),(7,"stick")], output_per_craft: 1 }),
    ("stone_axe", ShapedRecipe { cells: &[(1,"cobblestone"),(2,"cobblestone"),(4,"cobblestone"),(5,"stick"),(7,"stick")], output_per_craft: 1 }),
    ("stone_sword", ShapedRecipe { cells: &[(2,"cobblestone"),(5,"cobblestone"),(8,"stick")], output_per_craft: 1 }),
    ("stone_shovel", ShapedRecipe { cells: &[(2,"cobblestone"),(5,"stick"),(8,"stick")], output_per_craft: 1 }),
    ("stone_hoe", ShapedRecipe { cells: &[(1,"cobblestone"),(2,"cobblestone"),(4,"stick"),(7,"stick")], output_per_craft: 1 }),
    // 铁制工具（需先熔炼 iron_ingot）
    ("iron_pickaxe", ShapedRecipe { cells: &[(1,"iron_ingot"),(3,"iron_ingot"),(4,"iron_ingot"),(5,"stick"),(7,"stick")], output_per_craft: 1 }),
    ("iron_axe", ShapedRecipe { cells: &[(1,"iron_ingot"),(2,"iron_ingot"),(4,"iron_ingot"),(5,"stick"),(7,"stick")], output_per_craft: 1 }),
    ("iron_sword", ShapedRecipe { cells: &[(2,"iron_ingot"),(5,"iron_ingot"),(8,"stick")], output_per_craft: 1 }),
    ("iron_shovel", ShapedRecipe { cells: &[(2,"iron_ingot"),(5,"stick"),(8,"stick")], output_per_craft: 1 }),
    ("iron_hoe", ShapedRecipe { cells: &[(1,"iron_ingot"),(2,"iron_ingot"),(4,"stick"),(7,"stick")], output_per_craft: 1 }),
    // 铁盔甲
    ("iron_helmet", ShapedRecipe { cells: &[(1,"iron_ingot"),(2,"iron_ingot"),(3,"iron_ingot"),(4,"iron_ingot"),(6,"iron_ingot")], output_per_craft: 1 }),
    ("iron_chestplate", ShapedRecipe { cells: &[(1,"iron_ingot"),(3,"iron_ingot"),(4,"iron_ingot"),(5,"iron_ingot"),(6,"iron_ingot"),(7,"iron_ingot"),(8,"iron_ingot"),(9,"iron_ingot")], output_per_craft: 1 }),
    ("iron_leggings", ShapedRecipe { cells: &[(1,"iron_ingot"),(2,"iron_ingot"),(3,"iron_ingot"),(4,"iron_ingot"),(6,"iron_ingot"),(7,"iron_ingot"),(8,"iron_ingot"),(9,"iron_ingot")], output_per_craft: 1 }),
    ("iron_boots", ShapedRecipe { cells: &[(1,"iron_ingot"),(3,"iron_ingot"),(7,"iron_ingot"),(9,"iron_ingot")], output_per_craft: 1 }),
];

fn lookup_shaped(item: &str) -> Option<ShapedRecipe> {
    let norm = normalize_item(item);
    SHAPED_RECIPES
        .iter()
        .find(|(id, _)| *id == norm)
        .map(|(_, r)| ShapedRecipe {
            cells: r.cells,
            output_per_craft: r.output_per_craft,
        })
}

pub async fn do_craft_3x3(bot: &Client, item: &str, count: u32) -> Result<String, String> {
    let recipe = lookup_shaped(item).ok_or_else(|| {
        format!("不支持的 3×3 合成目标 {item}（需先打开工作台）")
    })?;

    let inv = bot
        .get_inventory()
        .map_err(|e| format!("获取容器失败（确认已打开工作台）: {e:?}"))?;

    let output = recipe.output_per_craft;
    let crafts_needed = (count.max(1) + output - 1) / output;
    let mut crafted = 0u32;

    for _ in 0..crafts_needed {
        // 按形状把每种原料摆进对应网格槽
        for &(g, ing_id) in recipe.cells {
            let ing_kind = ItemKind::from_str(&normalize_item(ing_id))
                .map_err(|_| format!("未知原料 {ing_id}"))?;
            let src = find_source_slot(&inv, ing_kind)
                .ok_or_else(|| format!("背包缺少原料 {}", ing_id))?;
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

/// 附魔：在已打开的附魔台菜单中，给背包中的 `item` 附魔。
/// 需要背包内已有待附魔物品与青金石（lapis_lazuli）。
/// `level` 取 1/2/3，对应附魔台三个选项槽（slot 2/3/4）。
pub async fn do_enchant(
    bot: &Client,
    item: &str,
    level: u32,
) -> Result<String, String> {
    let opt_slot = match level.clamp(1, 3) {
        1 => 2usize,
        2 => 3usize,
        _ => 4usize,
    };
    let item_kind = ItemKind::from_str(&normalize_item(item))
        .map_err(|_| format!("未知物品 {item}"))?;
    let lapis_kind = ItemKind::from_str("lapis_lazuli")
        .map_err(|_| "青金石 id 解析失败".to_string())?;

    let inv = bot
        .get_inventory()
        .map_err(|e| format!("获取容器失败（确认已打开附魔台）: {e:?}"))?;

    // 把待附魔物品放进 item 槽(0)
    let src_item = find_source_slot(&inv, item_kind)
        .ok_or_else(|| format!("背包缺少待附魔物品 {item}"))?;
    move_stack(&inv, src_item, 0).await;
    // 把青金石放进 lapis 槽(1)
    let src_lapis = find_source_slot(&inv, lapis_kind)
        .ok_or_else(|| "背包缺少青金石 lapis_lazuli".to_string())?;
    move_stack(&inv, src_lapis, 1).await;

    // 等待服务端下发可用附魔选项
    sleep(Duration::from_millis(300)).await;

    // 点击所选附魔选项槽（普通左键），触发附魔（物品仍在 item 槽并带附魔）
    inv.click(PickupClick::Left { slot: Some(opt_slot as u16) });
    sleep(Duration::from_millis(200)).await;

    let enchanted = {
        let slots = inv.slots();
        slots
            .as_ref()
            .and_then(|s| s.get(0))
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    };
    if !enchanted {
        return Err(format!("附魔 {item} 失败：物品槽为空（可能等级不足或青金不够）"));
    }
    // 收回到背包
    inv.shift_click(0usize);
    sleep(Duration::from_millis(40)).await;

    Ok(format!("附魔 {item}（等级 {level}）完成"))
}
