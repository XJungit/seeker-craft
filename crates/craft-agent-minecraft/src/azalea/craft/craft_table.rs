//! 2×2 与 3×3 合成：配方表 + 合成执行（craft_table 域）。
use super::*;

pub(crate) struct CraftPlan {
    pub(crate) ingredients: Vec<(ItemKind, u32)>,
    pub(crate) output_per_craft: u32,
}

/// 顺序填充配方（单原料）条目：(目标, 原料列表, 每次产出数)
type RecipeEntry = (&'static str, &'static [(&'static str, u32)], u32);
/// 2×2 形状配方条目：(目标, (槽位, 原料) 列表, 每次产出数)
type ShapedEntry = (&'static str, &'static [(usize, &'static str)], u32);
/// 2×2 形状配方条目：(目标, (槽位, 原料) 列表, 每次产出数)
const RECIPES: &[RecipeEntry] = &[
    ("oak_planks", &[("oak_log", 1)], 4),
    ("stick", &[("oak_planks", 2)], 4),
    ("crafting_table", &[("oak_planks", 4)], 1),
    ("torch", &[("coal", 1), ("stick", 1)], 4),
    ("torch", &[("charcoal", 1), ("stick", 1)], 4),
    // P104: mushroom_stew 是 shapeless 2×2（bowl + red_mushroom + brown_mushroom）。
    // 此前仅 prompt 知识层教 LLM 做蘑菇炖菜、harness 无配方 → "RecipeBook 和手写配方表均无此配方"
    // 失败误导 LLM（P83 知识→能力断裂）。顺序填充 3 格即可（shapeless 任意排列匹配）。
    (
        "mushroom_stew",
        &[("bowl", 1), ("red_mushroom", 1), ("brown_mushroom", 1)],
        1,
    ),
];

/// 2×2 形状配方表（P12 新增，2026-07-26）。
///
/// 槽位编号（Player 菜单 2×2 网格）：1=左上, 2=右上, 3=左下, 4=右下
///
/// vanilla 中 stick 和 torch 是竖直配方（["P","P"] / ["C","S"]），
/// 原 do_craft_2x2 顺序填充会把原料横放在 slot1+slot2，服务端按 shape 匹配失败，
/// 表现为「网格未产生结果：slot1=coal, slot2=stick, slot3=空, slot4=空」。
/// 改用显式 (slot, ingredient) 映射，把原料放在正确竖列上。
///
/// 同一目标可有多个候选（如 torch 同时支持 coal/charcoal），按表中顺序尝试。
const SHAPED_2X2: &[ShapedEntry] = &[
    // stick: 2 planks 竖直（左列）—— vanilla shape ["P","P"]
    ("stick", &[(1, "oak_planks"), (3, "oak_planks")], 4),
    // torch (coal 变体): coal 在上, stick 在下 —— vanilla shape ["C","S"]
    ("torch", &[(1, "coal"), (3, "stick")], 4),
    // torch (charcoal 变体): charcoal 在上, stick 在下
    ("torch", &[(1, "charcoal"), (3, "stick")], 4),
];

/// 查找 2×2 形状配方的所有候选（按表中顺序，coal 优先于 charcoal）。
/// 返回 (cells, output_per_craft) 列表；空表示该物品无形状配方，应回退到顺序填充。
pub(crate) fn lookup_shaped_2x2(item: &str) -> Vec<(&'static [(usize, &'static str)], u32)> {
    let b = bare(item);
    SHAPED_2X2
        .iter()
        .filter(|(id, _, _)| *id == b)
        .map(|(_, cells, out)| (*cells, *out))
        .collect()
}

/// 去掉 `minecraft:` 前缀，便于比较裸 id。
pub(crate) fn planks_plan_for(planks_id: &str) -> Option<CraftPlan> {
    let wood = bare(planks_id).strip_suffix("_planks")?;
    let log = format!("minecraft:{wood}_log");
    // 校验原木 id 合法（覆盖 oak/spruce/birch/...）
    let kind = match ItemKind::from_str(&log) {
        Ok(k) => k,
        Err(_) => return None,
    };
    Some(CraftPlan {
        ingredients: vec![(kind, 1)],
        output_per_craft: 4,
    })
}

pub(crate) fn lookup_recipe(item: &str) -> Option<CraftPlan> {
    let b = bare(item).to_string();
    // 显式配方优先；否则对木板做动态派生（覆盖所有原木种类）
    if let Some(p) = RECIPES
        .iter()
        .find(|(id, _, _)| *id == b)
        .map(|(_, ings, out)| CraftPlan {
            ingredients: ings
                .iter()
                .map(|(id, amt)| (ItemKind::from_str(&normalize_item(id)).unwrap(), *amt))
                .collect(),
            output_per_craft: *out,
        })
    {
        return Some(p);
    }
    if let Some(p) = planks_plan_for(&b) {
        return Some(p);
    }
    None
}

fn count_empty_player_slots(inv: &ContainerHandleRef) -> u32 {
    let Some(menu) = inv.menu().ok().flatten() else {
        return 0;
    };
    let Some(slots) = inv.slots() else {
        return 0;
    };
    let range = menu.player_slots_range();
    slots
        .iter()
        .enumerate()
        .filter(|(i, _)| range.contains(i))
        .filter(|(_, s)| s.is_empty())
        .count() as u32
}

/// 找玩家背包（player_slots_range，含 hotbar）里第一个空槽位。
fn dump_player_inventory(inv: &ContainerHandleRef) -> String {
    let Some(menu) = inv.menu().ok().flatten() else {
        return "(无法读取菜单)".into();
    };
    let Some(slots) = inv.slots() else {
        return "(无法读取槽位)".into();
    };
    let range = menu.player_slots_range();
    let mut items: Vec<String> = Vec::new();
    let mut count = 0;
    for s in range {
        if let Some(st) = slots.get(s)
            && !st.is_empty()
        {
            let k = st.kind().to_str();
            let bare = k.strip_prefix("minecraft:").unwrap_or(k);
            items.push(format!("slot{s}={bare}x{}", st.count()));
            count += 1;
            if count >= 20 {
                break;
            }
        }
    }
    if items.is_empty() {
        "(空背包)".into()
    } else {
        items.join(", ")
    }
}

/// P16 修复（2026-07-26）：合成前自动丢弃垃圾方块腾出空位。
///
/// 背景：session 中 craft_3x3 stone_pickaxe 失败，原因 "背包完全满（player_slots
/// 无空位），产物无法收集"。背包塞满 dirt/cobblestone/granite/diorite/tuff 等
/// 垃圾方块，LLM 又不主动 discard，导致每次 craft 都因产物无法收集而失败。
///
/// 修复：合成前若空位 < 2，自动丢弃常见垃圾方块（保留少量有用的）：
/// - dirt, granite, diorite, andesite, tuff, gravel, sand, red_sand, clay_ball:
///   全丢（早期游戏无用，可随时再挖）
/// - cobblestone, cobbled_deepslate: 保留 16 个（合成石镐/石斧/熔炉需要）
/// - oak_sapling: 保留 4 个（种树用）
/// - flint: 保留 8 个（箭矢/打火石用）
/// - string: 保留 4 个（弓用）
/// - oak_planks, oak_log: 不丢（合成原料）
/// - 所有工具/矿物/食物/燃料: 不丢
///
/// 返回 (丢弃的物品描述, 腾出的空位数)。
async fn auto_discard_junk(bot: &Client) -> (String, u32) {
    let inv = match bot.get_inventory() {
        Ok(i) => i,
        Err(_) => return (String::new(), 0),
    };
    let empty_before = count_empty_player_slots(&inv);
    // P22 修复（2026-07-27）：阈值从 2 提高到 6。
    // 原阈值 2 太晚——craft 时产物 + 原料交换需要至少 2 空位，
    // 但 bot 在两次 craft 之间会拾取垃圾，等 empty<2 才清理时已经积满 100+ 垃圾。
    // 现在阈值 6：始终保持至少 6 个空位，给 craft 留足缓冲。
    if empty_before >= 6 {
        return (String::new(), 0); // 空位足够，无需丢弃
    }

    // (物品 kind, 保留数量) — 保留数量 0 表示全丢
    const JUNK_KEEP: &[(&str, u32)] = &[
        // 全丢的纯垃圾
        ("dirt", 0),
        ("grass_block", 0),
        ("sand", 0),
        ("red_sand", 0),
        ("gravel", 0),
        ("granite", 0),
        ("diorite", 0),
        ("andesite", 0),
        ("tuff", 0),
        ("clay_ball", 0),
        ("netherrack", 0),
        ("basalt", 0),
        ("blackstone", 0),
        ("end_stone", 0),
        ("podzol", 0),
        ("mycelium", 0),
        ("coarse_dirt", 0),
        ("rooted_dirt", 0),
        ("moss_block", 0),
        // P22 新增：更多垃圾方块
        ("terracotta", 0),
        ("sandstone", 0),
        ("red_sandstone", 0),
        ("quartz_block", 0),
        ("calcite", 0),
        ("dripstone_block", 0),
        ("pointed_dripstone", 0),
        ("smooth_basalt", 0),
        ("deepslate", 0),
        ("stone", 0),        // 石头挖掉得到 cobblestone，原石本身无用
        ("cobblestone", 32), // P22: 保留数从 16 提到 32（合成熔炉/石镐需要 8+）
        ("cobbled_deepslate", 32),
        // 保留少量的有用方块
        ("oak_sapling", 4),
        ("flint", 8),
        ("string", 4),
        ("stick", 16), // 合成原料，保留 16 个够用
        // 丢弃多余的同类工具（如多把 iron_hoe）
        ("iron_hoe", 0),
        ("wooden_hoe", 0),
        ("stone_hoe", 0),
        ("wooden_axe", 0), // P22: 升级后旧工具丢弃
        ("stone_axe", 0),
        ("wooden_pickaxe", 0),
        ("stone_pickaxe", 0),
        ("wooden_shovel", 0),
        ("stone_shovel", 0),
    ];

    let mut dropped_log: Vec<String> = Vec::new();
    let mut total_dropped: u32 = 0;

    for (item_name, keep) in JUNK_KEEP {
        let kind = match ItemKind::from_str(item_name) {
            Ok(k) => k,
            Err(_) => continue,
        };
        // 重新读 inv（每次丢弃后状态变化）
        let inv = match bot.get_inventory() {
            Ok(i) => i,
            Err(_) => break,
        };
        let Some(menu) = inv.menu().ok().flatten() else {
            break;
        };
        let Some(slots) = inv.slots() else {
            break;
        };
        let range = menu.player_slots_range();

        // 收集所有该类物品的 (slot, count)
        let mut stacks: Vec<(usize, u32)> = Vec::new();
        for s in range {
            if let Some(st) = slots.get(s)
                && !st.is_empty()
                && st.kind() == kind
            {
                stacks.push((s, st.count() as u32));
            }
        }
        if stacks.is_empty() {
            continue;
        }

        let total: u32 = stacks.iter().map(|(_, c)| *c).sum();
        if total <= *keep {
            continue; // 总量不超过保留数，不丢
        }
        let mut to_drop = total - keep;

        for (s, stack_count) in &stacks {
            if to_drop == 0 {
                break;
            }
            let drop_from_this = (*stack_count).min(to_drop);
            if drop_from_this >= *stack_count {
                // 丢整堆
                inv.click(ThrowClick::All { slot: *s as u16 });
                sleep(Duration::from_millis(80)).await;
            } else {
                // 丢指定数量
                for _ in 0..drop_from_this {
                    inv.click(ThrowClick::Single { slot: *s as u16 });
                    sleep(Duration::from_millis(40)).await;
                }
            }
            to_drop -= drop_from_this;
            total_dropped += drop_from_this;
        }
        dropped_log.push(format!("{item_name}x{}", total - keep));
    }

    // 等待服务端同步背包
    sleep(Duration::from_millis(300)).await;
    let inv = match bot.get_inventory() {
        Ok(i) => i,
        Err(_) => return (dropped_log.join(", "), total_dropped),
    };
    let empty_after = count_empty_player_slots(&inv);
    let freed = empty_after.saturating_sub(empty_before);

    if !dropped_log.is_empty() {
        eprintln!(
            "[craft] auto_discard_junk: 丢弃 {}，腾出 {} 个空位",
            dropped_log.join(", "),
            freed
        );
    }
    (dropped_log.join(", "), freed)
}

/// 在玩家背包**以及合成网格**内找原料槽位。
///
/// P9 修复：`find_source_slot` 只搜 `player_slots_range`，当上一次合成在网格里
/// 留了残料时找不到它，会误报「背包缺少原料 X」并触发多余的自动合成。
/// 这里先搜玩家背包（优先用背包里的整堆），再兜底搜网格槽（1..=9）。
///
/// P23 修复（2026-07-27）：木板/原木别名替换。
/// LLM 常传 "oak_planks" 但 bot 实际有 birch_log/birch_planks——原 code 只找
/// oak_planks → "背包缺少原料 oak_planks" → craft 100% 失败。
/// 现在：找不到目标 kind 时，按别名表尝试所有变体（oak/birch/spruce/...）。
fn find_ingredient_slot(
    inv: &ContainerHandleRef,
    kind: ItemKind,
    grid_slots: std::ops::RangeInclusive<usize>,
) -> Option<usize> {
    // 先精确找
    if let Some(s) = find_source_slot(inv, kind) {
        return Some(s);
    }
    // 兜底：网格里可能有上次残留的同种原料
    let slots = inv.slots()?;
    for s in grid_slots.clone() {
        if let Some(stack) = slots.get(s)
            && !stack.is_empty()
            && stack.kind() == kind
        {
            return Some(s);
        }
    }
    // P23: 别名替换——找不到精确 kind 时，尝试同类其他变体
    for alt_kind in expand_ingredient_aliases(kind) {
        if alt_kind == kind {
            continue;
        }
        if let Some(s) = find_source_slot(inv, alt_kind) {
            eprintln!(
                "[craft] P23 别名替换：{} -> {}（背包无前者，用后者替代）",
                kind.to_str(),
                alt_kind.to_str()
            );
            return Some(s);
        }
        for s in grid_slots.clone() {
            if let Some(stack) = slots.get(s)
                && !stack.is_empty()
                && stack.kind() == alt_kind
            {
                eprintln!(
                    "[craft] P23 别名替换（网格）：{} -> {}",
                    kind.to_str(),
                    alt_kind.to_str()
                );
                return Some(s);
            }
        }
    }
    None
}

/// P23 新增（2026-07-27）：返回原料的别名列表（同种类不同变体）。
///
/// 学习自 mindcraft 的 getItemId/grindstone：mindcraft 用一个 mapping 表把
/// "oak_planks" 映射到所有 planks 变体。本项目手动列出 vanilla 9 种木材。
///
/// 规则：
/// - oak_planks → [birch_planks, spruce_planks, jungle_planks, acacia_planks, ...]
/// - oak_log → [birch_log, spruce_log, jungle_log, acacia_log, ...]
/// - 其他物品：返回空（无别名）
fn expand_ingredient_aliases(kind: ItemKind) -> Vec<ItemKind> {
    let name = kind.to_str();
    let bare = name.strip_prefix("minecraft:").unwrap_or(name);
    let aliases: Vec<&str> = if bare.ends_with("_planks") {
        vec![
            "oak_planks",
            "birch_planks",
            "spruce_planks",
            "jungle_planks",
            "acacia_planks",
            "dark_oak_planks",
            "mangrove_planks",
            "cherry_planks",
            "pale_oak_planks",
        ]
    } else if bare.ends_with("_log") {
        vec![
            "oak_log",
            "birch_log",
            "spruce_log",
            "jungle_log",
            "acacia_log",
            "dark_oak_log",
            "mangrove_log",
            "cherry_log",
            "pale_oak_log",
        ]
    } else if bare.ends_with("_wood") {
        vec![
            "oak_wood",
            "birch_wood",
            "spruce_wood",
            "jungle_wood",
            "acacia_wood",
            "dark_oak_wood",
            "mangrove_wood",
            "cherry_wood",
            "pale_oak_wood",
        ]
    } else if matches!(bare, "coal" | "charcoal") {
        // 火把配方同时支持 coal 和 charcoal
        vec!["coal", "charcoal"]
    } else {
        return Vec::new();
    };
    aliases
        .iter()
        .filter_map(|s| ItemKind::from_str(&format!("minecraft:{s}")).ok())
        .collect()
}

/// 清空合成网格里的残留物品（回背包）。返回是否已全部清空。
///
/// P9 修复：残留物品会污染下一次合成——服务端按「网格现有内容」匹配配方，
/// 多出来的格子会让配方不匹配，表现为「网格未产生结果」。
///
/// 两种方式依次尝试：
/// 1. `shift_click`（QuickMove）——Minecraft 标准的从网格回背包方式；
/// 2. `left_click` 拿起 + `left_click` 空背包槽放下——shift_click 对合成网格
///    偶尔无效时的兜底。
async fn place_one(inv: &ContainerHandleRef, src: usize, dst: usize) {
    for attempt in 0..3u8 {
        let expected_kind = inv
            .slots()
            .as_ref()
            .and_then(|s| s.get(src))
            .filter(|st| !st.is_empty())
            .map(|st| st.kind());

        inv.left_click(src);
        sleep(Duration::from_millis(80)).await;
        inv.right_click(dst);
        sleep(Duration::from_millis(80)).await;
        inv.left_click(src);
        sleep(Duration::from_millis(80)).await;

        let dst_ok = inv
            .slots()
            .as_ref()
            .and_then(|s| s.get(dst))
            .map(|st| {
                if st.is_empty() {
                    false
                } else if let Some(k) = expected_kind {
                    st.kind() == k
                } else {
                    true
                }
            })
            .unwrap_or(false);
        if dst_ok {
            return;
        }

        let wait_ms = 100u64 * (attempt as u64 + 1);
        eprintln!(
            "[place_one] attempt {} failed: src={src}, dst={dst}（dst 空或物品不对），{wait_ms}ms 后重试",
            attempt + 1
        );
        sleep(Duration::from_millis(wait_ms)).await;

        let src_has_item = inv
            .slots()
            .as_ref()
            .and_then(|s| s.get(src))
            .map(|st| !st.is_empty())
            .unwrap_or(false);
        if !src_has_item {
            eprintln!("[place_one] src={src} 已空，无法重试");
            return;
        }
    }
    eprintln!("[place_one] 警告：3 次尝试后仍未能把物品放入 dst={dst}（src={src}）");
}

/// 执行 2×2 合成。返回人类可读结果串（供 tool / 日志使用）。
///
/// P12 修复（2026-07-26）：优先使用 SHAPED_2X2 形状配方（正确竖列摆放），
/// 没有形状配方时回退到顺序填充（仅对单原料配方如 planks 安全）。
/// 原 code 顺序填充 stick/torch 把原料横放在 slot1+slot2，
/// 服务端按 vanilla shape ["P","P"]/["C","S"] 匹配失败 → 100% 失败。
pub async fn do_craft_2x2(bot: &Client, item: &str, count: u32) -> Result<String, String> {
    // Player 菜单的 2×2 合成网格是 slot 1..=4（slot 0 是结果槽）。
    const GRID: std::ops::RangeInclusive<usize> = 1..=4;

    // P16 修复（2026-07-26）：合成前自动丢弃垃圾方块腾出空位。
    // 避免"背包完全满，产物无法收集"的失败。
    let (discard_log, freed) = auto_discard_junk(bot).await;
    if freed > 0 {
        eprintln!("[craft 2x2] 预清理: {discard_log}, 腾出 {freed} 空位");
    }

    // P13 修复（2026-07-26）：若容器（工作台/熔炉/箱子）还开着，2×2 网格槽位 1..=4
    // 实际指向的是容器内的格子（不是玩家 2×2 网格），所有 click 都打到错误位置，
    // place_one 误判成功（dst 看起来有东西，其实是容器槽的旧数据），最后 shift_click(0)
    // 把容器内的物品移走，导致「合成报告成功但背包没东西」的假成功。
    // 修复：开工前强制关闭任何已打开的容器，等菜单变回 Player 再继续。
    if crate::azalea::table_flow::is_container_open(bot) {
        crate::azalea::table_flow::close_container_if_open(bot);
        // 等菜单变回 Player（最多 1s）
        for _ in 0..10 {
            sleep(Duration::from_millis(100)).await;
            if let Ok(inv) = bot.get_inventory()
                && let Ok(Some(menu)) = inv.menu()
                && matches!(menu, azalea::inventory::Menu::Player(_))
            {
                break;
            }
        }
    }

    let inv = bot
        .get_inventory()
        .map_err(|e| format!("获取背包失败: {e:?}"))?;

    // 二次确认：菜单必须是 Player（2×2 网格只在此菜单下有效）
    if let Ok(Some(menu)) = inv.menu()
        && !matches!(menu, azalea::inventory::Menu::Player(_))
    {
        return Err(format!(
            "合成 {item} 失败：当前打开的不是玩家背包（2×2 网格不可用）。\
                 请先关闭已打开的容器再调用 craft。"
        ));
    }

    // 决定 placement：[(slot, ItemKind)] 列表 + output_per_craft
    // 优先级：SHAPED_2X2 候选（含 coal/charcoal 多变体）> 顺序填充（lookup_recipe）
    let placement: Vec<(usize, ItemKind)>;
    let output: u32;

    let shaped_candidates = lookup_shaped_2x2(item);
    if !shaped_candidates.is_empty() {
        // 尝试每个候选，选第一个原料齐全的（coal 优先于 charcoal）
        let mut chosen: Option<(&'static [(usize, &'static str)], u32)> = None;
        let mut missing_ingredient: Option<String> = None;
        for (cells, out) in &shaped_candidates {
            let mut all_present = true;
            for (_, ing_id) in *cells {
                let kind = ItemKind::from_str(&normalize_item(ing_id))
                    .map_err(|_| format!("未知原料 {ing_id}"))?;
                if find_ingredient_slot(&inv, kind, GRID).is_none() {
                    all_present = false;
                    if missing_ingredient.is_none() {
                        missing_ingredient = Some(ing_id.to_string());
                    }
                    break;
                }
            }
            if all_present {
                chosen = Some((*cells, *out));
                break;
            }
        }
        let (cells, out) = chosen.ok_or_else(|| {
            format!(
                "合成 {item} 失败：背包缺少原料 {}（已尝试 {} 个候选配方均缺料）",
                missing_ingredient.as_deref().unwrap_or("?"),
                shaped_candidates.len()
            )
        })?;
        output = out;
        placement = cells
            .iter()
            .map(|(slot, ing_id)| {
                (
                    *slot,
                    ItemKind::from_str(&normalize_item(ing_id)).unwrap_or(ItemKind::Air),
                )
            })
            .collect();
    } else {
        // 回退：无形状配方 → 顺序填充（仅对单原料配方安全，如 planks/crafting_table）
        let plan = lookup_recipe(item).ok_or_else(|| {
            // P17 修复（2026-07-27）：原错误消息提到"木镐"是 2×2 配方，
            // 但 wooden_pickaxe 实际是 3×3 配方——误导 LLM 用 craft(2×2) 合成木镐，
            // 必然失败。改为明确告诉 LLM 用 craft_3x3。
            format!(
                "不支持的 2×2 合成目标 {item}（2×2 仅支持：木板/木棍/工作台/火把/箱子等单原料配方）。\
                 若目标是工具（木镐/石镐/铁镐/剑/斧/锹/锄）或 3×3 配方（furnace/chest/ladder 等），\
                 请改用 craft_3x3 工具（需先 place 工作台或由 craft_3x3 自动放桌）。"
            )
        })?;
        output = plan.output_per_craft;
        let mut seq: Vec<(usize, ItemKind)> = Vec::new();
        let mut grid_slot = *GRID.start();
        for (kind, amt) in &plan.ingredients {
            for _ in 0..*amt {
                if grid_slot > *GRID.end() {
                    return Err(format!(
                        "合成 {item} 失败：配方需要的格子数超过 2×2 网格容量"
                    ));
                }
                seq.push((grid_slot, *kind));
                grid_slot += 1;
            }
        }
        placement = seq;
    }

    let crafts_needed = count.max(1).div_ceil(output);
    let mut crafted = 0u32;

    for round in 0..crafts_needed {
        // 0) 清空上一轮/上一次合成的网格残留，否则服务端按「网格现有内容」
        //    匹配配方时会因多出的格子而不匹配（表现为「网格未产生结果」）。
        clear_cursor(&inv).await;
        if !clear_grid(&inv, GRID).await {
            return Err(format!(
                "合成 {item} 失败：无法清空 2×2 网格残留（第 {} 轮）。建议先关闭再重开容器。",
                round + 1
            ));
        }

        // 1) 按 placement 放料（形状配方用显式 slot，顺序配方用 1→2→3→4）
        //    P8：不能用 shift_click——服务端自行决定落到哪一格，不按配方形状；
        //    也不能整堆塞一格，否则同种原料的其他格找不到料。
        for (slot, kind) in &placement {
            let src = find_ingredient_slot(&inv, *kind, GRID)
                .ok_or_else(|| format!("背包缺少原料 {}", kind.to_str()))?;
            place_one(&inv, src, *slot).await;
        }

        // 2) 等服务端算出结果并检查结果槽（最多等 2s）
        let mut has_result = false;
        for _ in 0..20 {
            sleep(Duration::from_millis(100)).await;
            let r = inv
                .slots()
                .as_ref()
                .and_then(|s| s.first())
                .map(|s| !s.is_empty())
                .unwrap_or(false);
            if r {
                has_result = true;
                break;
            }
        }
        if !has_result {
            let diag = inv
                .slots()
                .map(|s| {
                    s.iter()
                        .take(5)
                        .enumerate()
                        .map(|(i, st)| {
                            format!(
                                "slot{i}={}",
                                if st.is_empty() {
                                    "空".to_string()
                                } else {
                                    format!("{}x{}", st.kind().to_str(), st.count())
                                }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_else(|| "无法读取 slots".to_string());
            // 失败也要清网格，否则残料会污染下一次合成
            let _ = clear_grid(&inv, GRID).await;
            return Err(format!(
                "合成 {item} 失败：网格未产生结果（等待 2s slot 0 仍空）。网格状态: [{diag}]。\
                 可能原因：原料类型/位置不对、原料不足、或服务端回包延迟"
            ));
        }

        // 3) 收产物进背包（P15 修复：用 left_click 直接收集，绕过 azalea shift_click 只移主背包的 bug）
        //    P14 用 shift_click(0) + left_click 兜底，但 azalea 的 CraftResult shift_click
        //    只移到 Player::INVENTORY_SLOTS（主背包 9..=35，不含 hotbar）。主背包满时
        //    shift_click 失败，left_click 兜底又用 stale slots_data 找空位，导致 100% 失败。
        //    P15 改用 left_click(0) 拿起 result + left_click(empty_player_slot) 放下，
        //    遍历 player_slots_range（含 hotbar）找空位，每次操作后重新读 inv 拿最新 state。
        let target_kind = ItemKind::from_str(&normalize_item(item))
            .map_err(|_| format!("未知目标物品 {item}"))?;

        // P22 修复（2026-07-27）：检测 result slot 实际类型。
        // 原 code 只数 target_kind（如 oak_planks），但 LLM 可能传 "oak_planks"
        // 而背包只有 birch_log → 服务端产出 birch_planks → count(oak_planks) 永远 0 → 误报失败。
        // 学习自 P20 对 3x3 的修复：读 slot 0 实际类型，按实际类型计数。
        let actual_kind: Option<ItemKind> = inv
            .slots()
            .as_ref()
            .and_then(|s| s.first())
            .filter(|st| !st.is_empty())
            .map(|st| st.kind());
        let count_kind = actual_kind.unwrap_or(target_kind);
        if let Some(ak) = actual_kind
            && ak != target_kind
        {
            let ak_name = ak.to_str();
            let tk_name = target_kind.to_str();
            eprintln!(
                "[craft 2x2] 警告：result slot 是 {} 而非 {}（可能 LLM 传了别名，按实际类型计数）",
                ak_name.strip_prefix("minecraft:").unwrap_or(ak_name),
                tk_name.strip_prefix("minecraft:").unwrap_or(tk_name),
            );
        }
        let before_count = count_item_in_player_slots(&inv, count_kind);

        // 先尝试 shift_click(0)（如果主背包有空位，这是最快的）
        let empty_before = count_empty_player_slots(&inv);
        if empty_before > 0 {
            inv.shift_click(0usize);
            sleep(Duration::from_millis(200)).await;
            let after_count = count_item_in_player_slots(&inv, count_kind);
            if after_count > before_count {
                crafted += output;
                continue;
            }
        }

        // shift_click 失败或主背包满：用 left_click 手动收集（含 hotbar）
        // 1. left_click(0) 拿起 result
        inv.left_click(0usize);
        sleep(Duration::from_millis(150)).await;

        // 2. 重新读 inv 找空槽（每次都读最新的，避免 stale state）
        let inv2 = bot
            .get_inventory()
            .map_err(|e| format!("读取背包失败: {e:?}"))?;
        match find_empty_player_slot(&inv2) {
            Some(empty_slot) => {
                inv2.left_click(empty_slot);
                sleep(Duration::from_millis(150)).await;
            }
            None => {
                // 背包完全满：把 result 放回 slot 0（避免丢失），清网格，报错
                inv2.left_click(0usize);
                sleep(Duration::from_millis(100)).await;
                let _ = clear_grid(&inv2, GRID).await;
                let dump = dump_player_inventory(&inv2);
                return Err(format!(
                    "合成 {item} 失败：背包完全满（player_slots 无空位），产物无法收集。\
                     当前背包: {dump}\n\
                     建议：先 discard 丢弃垃圾物品（dirt/cobblestone/gravel/clay_ball/tuff/granite/diorite/flint 等），\
                     腾出至少 1 个空位后再重试 craft。"
                ));
            }
        }

        // 3. 验证 count 增加
        let inv3 = bot
            .get_inventory()
            .map_err(|e| format!("验证时读取背包失败: {e:?}"))?;
        let after_count2 = count_item_in_player_slots(&inv3, count_kind);
        if after_count2 > before_count {
            crafted += output;
            continue;
        }

        // P22 新增（2026-07-27）：clear_grid + 关背包兜底收集（学习自 P20 对 3x3 的修复）。
        // shift_click + left_click 都失败时，清网格让服务端把光标/网格上的物品回背包。
        // 对 2x2 没有"关容器"概念（Player 菜单始终打开），但 clear_grid 会触发服务端
        // 重新计算 result slot，可能让产物自动进入背包。
        eprintln!(
            "[craft 2x2] shift_click + left_click 均失败，尝试 clear_grid 兜底收集 (before={before_count})"
        );
        let _ = clear_grid(&inv3, GRID).await;
        sleep(Duration::from_millis(400)).await;
        let inv4 = bot
            .get_inventory()
            .map_err(|e| format!("兜底后读取背包失败: {e:?}"))?;
        let after_close = count_item_in_player_slots(&inv4, count_kind);
        if after_close > before_count {
            eprintln!(
                "[craft 2x2] clear_grid 兜底成功：{} x{} 进入背包 (before={before_count} after={after_close})",
                item, output
            );
            crafted += output;
            continue;
        }

        // 都失败：清网格，报错
        let _ = clear_grid(&inv4, GRID).await;
        return Err(format!(
            "合成 {item} 失败：产物无法从结果槽移入背包（shift_click、left_click、clear_grid 兜底 均未让背包产物增加，\
             before={before_count} after_left={after_count2} after_close={after_close}）。\
             建议：关闭背包再重新打开后重试，或先 discard 腾出空位。"
        ));
    }

    // 收尾：清掉可能残留的网格与光标，保证下次合成从干净状态开始
    clear_cursor(&inv).await;
    let _ = clear_grid(&inv, GRID).await;

    Ok(format!(
        "合成 {item} x{count} 完成（实际产出约 {crafted}，共 {crafts_needed} 次）"
    ))
}

/// 把 src 槽的整堆物品移到 dst 槽（两次 left_click：拿起→放下）。
pub(crate) struct ShapedRecipe {
    /// (网格槽 1..=9, 原料物品 id) 列表，按 vanilla 合成形状摆放。
    pub(crate) cells: &'static [(usize, &'static str)],
    pub(crate) output_per_craft: u32,
}

const SHAPED_RECIPES: &[(&str, ShapedRecipe)] = &[
    // P16 修复（2026-07-26）：2×2 配方也加入 3×3 表。
    // vanilla 中这些配方在 2×2 和 3×3 网格中都能合成（形状放在左上角），
    // 但原 lookup_shaped 只查 SHAPED_RECIPES，craft_3x3 对这些物品报
    // "不支持的 3×3 合成目标"。LLM 常误用 craft_3x3 合成 crafting_table，
    // 导致 100% 失败。加入这些配方让 craft_3x3 也能处理。
    // 槽位编号：1 2 3 / 4 5 6 / 7 8 9，2×2 形状放在 1,2,4,5。
    (
        "oak_planks",
        ShapedRecipe {
            cells: &[(1, "oak_log")],
            output_per_craft: 4,
        },
    ),
    (
        "stick",
        ShapedRecipe {
            cells: &[(1, "oak_planks"), (4, "oak_planks")],
            output_per_craft: 4,
        },
    ),
    (
        "crafting_table",
        ShapedRecipe {
            cells: &[
                (1, "oak_planks"),
                (2, "oak_planks"),
                (4, "oak_planks"),
                (5, "oak_planks"),
            ],
            output_per_craft: 1,
        },
    ),
    (
        "torch",
        ShapedRecipe {
            cells: &[(1, "coal"), (4, "stick")],
            output_per_craft: 4,
        },
    ),
    (
        "torch_charcoal",
        ShapedRecipe {
            cells: &[(1, "charcoal"), (4, "stick")],
            output_per_craft: 4,
        },
    ),
    // 环形：8 格同种原料
    (
        "furnace",
        ShapedRecipe {
            cells: &[
                (1, "cobblestone"),
                (2, "cobblestone"),
                (3, "cobblestone"),
                (4, "cobblestone"),
                (6, "cobblestone"),
                (7, "cobblestone"),
                (8, "cobblestone"),
                (9, "cobblestone"),
            ],
            output_per_craft: 1,
        },
    ),
    (
        "chest",
        ShapedRecipe {
            cells: &[
                (1, "oak_planks"),
                (2, "oak_planks"),
                (3, "oak_planks"),
                (4, "oak_planks"),
                (6, "oak_planks"),
                (7, "oak_planks"),
                (8, "oak_planks"),
                (9, "oak_planks"),
            ],
            output_per_craft: 1,
        },
    ),
    (
        "ladder",
        ShapedRecipe {
            cells: &[
                (1, "stick"),
                (2, "stick"),
                (3, "stick"),
                (4, "stick"),
                (5, "stick"),
                (6, "stick"),
                (7, "stick"),
                (8, "stick"),
                (9, "stick"),
            ],
            output_per_craft: 3,
        },
    ),
    (
        "oak_trapdoor",
        ShapedRecipe {
            cells: &[
                (1, "oak_planks"),
                (2, "oak_planks"),
                (3, "oak_planks"),
                (4, "oak_planks"),
                (5, "oak_planks"),
                (6, "oak_planks"),
            ],
            output_per_craft: 2,
        },
    ),
    // 门：两列木板
    (
        "oak_door",
        ShapedRecipe {
            cells: &[
                (1, "oak_planks"),
                (2, "oak_planks"),
                (4, "oak_planks"),
                (5, "oak_planks"),
                (7, "oak_planks"),
                (8, "oak_planks"),
            ],
            output_per_craft: 3,
        },
    ),
    // 栅栏：上下木板 + 中间棍
    (
        "oak_fence",
        ShapedRecipe {
            cells: &[
                (1, "oak_planks"),
                (2, "oak_planks"),
                (4, "stick"),
                (5, "stick"),
                (7, "oak_planks"),
                (8, "oak_planks"),
            ],
            output_per_craft: 3,
        },
    ),
    // 工具类的 vanilla 形状（3×3 网格编号：1 2 3 / 4 5 6 / 7 8 9）
    //   镐  XXX / .S. / .S.  → 头部占 1,2,3；柄占 5,8
    //   斧  XX. / XS. / .S.  → 头部占 1,2,4；柄占 5,8
    //   剑  .X. / .X. / .S.  → 刃占 2,5；柄占 8
    //   锹  .X. / .S. / .S.  → 头占 2；柄占 5,8
    //   锄  XX. / .S. / .S.  → 头占 1,2；柄占 5,8
    // 旧版把柄写成 5,7（锄写成 4,7）——柄不在同一竖列，服务端配方匹配失败，
    // 是「网格未产生结果」的一个独立成因。
    (
        "wooden_pickaxe",
        ShapedRecipe {
            cells: &[
                (1, "oak_planks"),
                (2, "oak_planks"),
                (3, "oak_planks"),
                (5, "stick"),
                (8, "stick"),
            ],
            output_per_craft: 1,
        },
    ),
    (
        "wooden_axe",
        ShapedRecipe {
            cells: &[
                (1, "oak_planks"),
                (2, "oak_planks"),
                (4, "oak_planks"),
                (5, "stick"),
                (8, "stick"),
            ],
            output_per_craft: 1,
        },
    ),
    (
        "wooden_sword",
        ShapedRecipe {
            cells: &[(2, "oak_planks"), (5, "oak_planks"), (8, "stick")],
            output_per_craft: 1,
        },
    ),
    (
        "wooden_shovel",
        ShapedRecipe {
            cells: &[(2, "oak_planks"), (5, "stick"), (8, "stick")],
            output_per_craft: 1,
        },
    ),
    (
        "wooden_hoe",
        ShapedRecipe {
            cells: &[
                (1, "oak_planks"),
                (2, "oak_planks"),
                (5, "stick"),
                (8, "stick"),
            ],
            output_per_craft: 1,
        },
    ),
    // 石制工具（用 cobblestone 代替木板）
    (
        "stone_pickaxe",
        ShapedRecipe {
            cells: &[
                (1, "cobblestone"),
                (2, "cobblestone"),
                (3, "cobblestone"),
                (5, "stick"),
                (8, "stick"),
            ],
            output_per_craft: 1,
        },
    ),
    (
        "stone_axe",
        ShapedRecipe {
            cells: &[
                (1, "cobblestone"),
                (2, "cobblestone"),
                (4, "cobblestone"),
                (5, "stick"),
                (8, "stick"),
            ],
            output_per_craft: 1,
        },
    ),
    (
        "stone_sword",
        ShapedRecipe {
            cells: &[(2, "cobblestone"), (5, "cobblestone"), (8, "stick")],
            output_per_craft: 1,
        },
    ),
    (
        "stone_shovel",
        ShapedRecipe {
            cells: &[(2, "cobblestone"), (5, "stick"), (8, "stick")],
            output_per_craft: 1,
        },
    ),
    (
        "stone_hoe",
        ShapedRecipe {
            cells: &[
                (1, "cobblestone"),
                (2, "cobblestone"),
                (5, "stick"),
                (8, "stick"),
            ],
            output_per_craft: 1,
        },
    ),
    // 铁制工具（需先熔炼 iron_ingot）
    (
        "iron_pickaxe",
        ShapedRecipe {
            cells: &[
                (1, "iron_ingot"),
                (2, "iron_ingot"),
                (3, "iron_ingot"),
                (5, "stick"),
                (8, "stick"),
            ],
            output_per_craft: 1,
        },
    ),
    (
        "iron_axe",
        ShapedRecipe {
            cells: &[
                (1, "iron_ingot"),
                (2, "iron_ingot"),
                (4, "iron_ingot"),
                (5, "stick"),
                (8, "stick"),
            ],
            output_per_craft: 1,
        },
    ),
    (
        "iron_sword",
        ShapedRecipe {
            cells: &[(2, "iron_ingot"), (5, "iron_ingot"), (8, "stick")],
            output_per_craft: 1,
        },
    ),
    (
        "iron_shovel",
        ShapedRecipe {
            cells: &[(2, "iron_ingot"), (5, "stick"), (8, "stick")],
            output_per_craft: 1,
        },
    ),
    (
        "iron_hoe",
        ShapedRecipe {
            cells: &[
                (1, "iron_ingot"),
                (2, "iron_ingot"),
                (5, "stick"),
                (8, "stick"),
            ],
            output_per_craft: 1,
        },
    ),
    // 铁盔甲
    (
        "iron_helmet",
        ShapedRecipe {
            cells: &[
                (1, "iron_ingot"),
                (2, "iron_ingot"),
                (3, "iron_ingot"),
                (4, "iron_ingot"),
                (6, "iron_ingot"),
            ],
            output_per_craft: 1,
        },
    ),
    (
        "iron_chestplate",
        ShapedRecipe {
            cells: &[
                (1, "iron_ingot"),
                (3, "iron_ingot"),
                (4, "iron_ingot"),
                (5, "iron_ingot"),
                (6, "iron_ingot"),
                (7, "iron_ingot"),
                (8, "iron_ingot"),
                (9, "iron_ingot"),
            ],
            output_per_craft: 1,
        },
    ),
    (
        "iron_leggings",
        ShapedRecipe {
            cells: &[
                (1, "iron_ingot"),
                (2, "iron_ingot"),
                (3, "iron_ingot"),
                (4, "iron_ingot"),
                (6, "iron_ingot"),
                (7, "iron_ingot"),
                (8, "iron_ingot"),
                (9, "iron_ingot"),
            ],
            output_per_craft: 1,
        },
    ),
    (
        "iron_boots",
        ShapedRecipe {
            cells: &[
                (1, "iron_ingot"),
                (3, "iron_ingot"),
                (7, "iron_ingot"),
                (9, "iron_ingot"),
            ],
            output_per_craft: 1,
        },
    ),
];

pub(crate) fn lookup_shaped(item: &str) -> Option<ShapedRecipe> {
    // P12 修复（2026-07-26）：原代码用 normalize_item(item) 把 id 统一加上 "minecraft:" 前缀，
    // 但 SHAPED_RECIPES 表里存的是裸 id（如 "stone_pickaxe"），导致查找永远不匹配，
    // craft_3x3 100% 失败。改用 bare() 去掉前缀后比较。
    let norm = bare(item);
    SHAPED_RECIPES
        .iter()
        .find(|(id, _)| *id == norm)
        .map(|(_, r)| ShapedRecipe {
            cells: r.cells,
            output_per_craft: r.output_per_craft,
        })
}

/// 3×3 工作台合成。**要求调用方已打开工作台**（Crafting 菜单）。
///
/// 调用方（`BotCommand::Craft3x3` 处理分支）负责整个「确保桌开 → 合成 → 关桌」
/// 流程：先 [`crate::azalea::table_flow::ensure_table_open`]（背包无桌时会自动
/// 合成并放置一个），再调用本函数，最后 `close_container_if_open`。
///
/// `table_pos` 是 `ensure_table_open` 返回的实际桌位，仅用于日志/错误信息定位；
/// 合成本身只操作当前已打开的容器菜单。
///
/// 网格槽位：0=结果，1..=9=3×3 网格（1=左上、2=中上、3=右上、4=左中、5=正中、
/// 6=右中、7=左下、8=中下、9=右下）。
pub async fn do_craft_3x3(
    bot: &Client,
    item: &str,
    count: u32,
    table_pos: Option<BlockPos>,
) -> Result<String, String> {
    // P16 修复（2026-07-26）：合成前自动丢弃垃圾方块腾出空位。
    // 避免"背包完全满，产物无法收集"的失败。
    let (discard_log, freed) = auto_discard_junk(bot).await;
    if freed > 0 {
        eprintln!("[craft 3x3] 预清理: {discard_log}, 腾出 {freed} 空位");
    }

    // P48 方向 C（2026-07-27）：RecipeBook 优先，手写表作 fallback。
    //
    // 学习自 mindcraft：mindcraft 用 mineflayer-prismarine-recipe 全量配方，
    // 不需要手写配方表。本项目 azalea 无 prismarine-recipe，但 RecipeBook 是等价物。
    //
    // 原 P17 逻辑：手写表优先 → RecipeBook fallback。
    // 问题：手写表只有 ~30 个配方，LLM 调用 craft_3x3 合成表外物品时走 RecipeBook，
    // 但 RecipeBook 的 grid 映射可能不如手写表精确（如 stick/torch 竖直形状）。
    //
    // P48 反转：RecipeBook 优先（覆盖 vanilla 全量配方），手写表作 fallback
    // （仅在手写表有但 RecipeBook 没有时用，或 RecipeBook grid 解析失败时用）。
    // 这样：1) 全量配方覆盖；2) 手写表的精确形状仍保留作兜底。
    let book = crate::azalea::auto_craft::recipe_book_of(bot);
    let recipe = if let Some(stored) = book.get_by_result(item) {
        // RecipeBook 命中，走 RecipeBook 路径
        eprintln!(
            "[craft 3x3] P48: '{item}' 命中 RecipeBook ({})，走 book 路径",
            stored.kind()
        );
        return do_craft_3x3_recipe(bot, stored, count).await;
    } else {
        // RecipeBook 未命中，查手写表
        match lookup_shaped(item) {
            Some(r) => {
                eprintln!("[craft 3x3] P48: '{item}' 不在 RecipeBook，回退到手写 SHAPED_RECIPES");
                r
            }
            None => {
                // P104: 3×3 失败时若该物品实际是 2×2 配方，明确引导改用 craft，
                // 避免"RecipeBook 和手写配方表均无此配方"误导 LLM（mushroom_stew 教训）。
                let is_2x2 = lookup_shaped_2x2(item).is_empty() && lookup_recipe(item).is_some();
                if is_2x2 {
                    return Err(format!(
                        "不支持的 3×3 合成目标 {item}：该物品是 2×2 配方（无需工作台）。\
                         请改用 craft(item, count) 工具（2×2 玩家网格合成）。"
                    ));
                }
                return Err(format!(
                    "不支持的 3×3 合成目标 {item}（RecipeBook 和手写配方表均无此配方）。\
                     可能原因：1) 物品名拼写错误；2) 该物品不可合成（如 air/bedrock）；\
                     3) 该物品是熔炼/切石产物，请用 smelt 工具。"
                ));
            }
        }
    };

    let inv = bot.get_inventory().map_err(|e| {
        let at = table_pos
            .map(|p| format!("（桌位 ({},{},{})）", p.x, p.y, p.z))
            .unwrap_or_default();
        format!("获取容器失败{at}（确认已打开工作台）: {e:?}")
    })?;

    let output = recipe.output_per_craft;
    let crafts_needed = count.max(1).div_ceil(output);
    let mut crafted = 0u32;

    // 工作台菜单：slot 0 = 结果，1..=9 = 3×3 网格
    const GRID: std::ops::RangeInclusive<usize> = 1..=9;

    for round in 0..crafts_needed {
        // 0) 清网格 + 清光标（P9：残留物品会让服务端配方匹配失败）
        clear_cursor(&inv).await;
        if !clear_grid(&inv, GRID).await {
            return Err(format!(
                "合成 {item} 失败：无法清空 3×3 网格残留（第 {} 轮）。\
                 建议关闭工作台再重新打开后重试。",
                round + 1
            ));
        }

        // 1) 按 vanilla 形状逐格放料，每格 1 个（P8：不能 shift_click / 不能整堆塞一格）
        for &(g, ing_id) in recipe.cells {
            let ing_kind = ItemKind::from_str(&normalize_item(ing_id))
                .map_err(|_| format!("未知原料 {ing_id}"))?;
            let src = find_ingredient_slot(&inv, ing_kind, GRID)
                .ok_or_else(|| format!("背包缺少原料 {ing_id}"))?;
            place_one(&inv, src, g).await;
        }

        // 2) 等服务端算出结果（最多 2s）
        let mut has_result = false;
        for _ in 0..20 {
            sleep(Duration::from_millis(100)).await;
            let r = inv
                .slots()
                .as_ref()
                .and_then(|s| s.first())
                .map(|s| !s.is_empty())
                .unwrap_or(false);
            if r {
                has_result = true;
                break;
            }
        }
        if !has_result {
            let diag = inv
                .slots()
                .map(|s| {
                    s.iter()
                        .take(10)
                        .enumerate()
                        .map(|(i, st)| {
                            format!(
                                "slot{i}={}",
                                if st.is_empty() {
                                    "空".to_string()
                                } else {
                                    format!("{}x{}", st.kind().to_str(), st.count())
                                }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_else(|| "无法读取 slots".to_string());
            let _ = clear_grid(&inv, GRID).await;
            return Err(format!(
                "合成 {item} 失败：网格未产生结果（等待 2s slot 0 仍空）。网格状态: [{diag}]。\
                 可能原因：原料类型/位置不对、原料不足、服务端配方未注册、或回包延迟"
            ));
        }

        // 3) 收产物（P15 修复：用 left_click 直接收集，绕过 azalea shift_click 只移主背包的 bug）
        // P20 修复（2026-07-27）：增加 result 类型验证 + 关容器兜底收集策略。
        // 原代码 shift_click 和 left_click 都失败时直接报错，但实际产物可能还在光标/网格上。
        // 关闭容器时服务端会自动把光标和网格上的物品返回玩家背包——这是最可靠的兜底。
        let target_kind = ItemKind::from_str(&normalize_item(item))
            .map_err(|_| format!("未知目标物品 {item}"))?;
        let before_count = count_item_in_player_slots(&inv, target_kind);

        // P20 新增：验证 result slot 里确实是 target_kind（不只是非空）
        // 防止网格摆错导致产出错误物品时，count 永远不增加，误报"收集失败"
        let result_kind = inv
            .slots()
            .as_ref()
            .and_then(|s| s.first())
            .filter(|st| !st.is_empty())
            .map(|st| st.kind());
        if let Some(rk) = result_kind
            && rk != target_kind
        {
            let rk_name = rk.to_str();
            eprintln!(
                "[craft 3x3] 警告：result slot 是 {} 而非 {}（网格可能摆错）",
                rk_name, item
            );
            // 不直接报错，继续尝试收集——可能 LLM 指定了别名
        }

        // 先尝试 shift_click(0)（如果主背包有空位，这是最快的）
        let empty_before = count_empty_player_slots(&inv);
        if empty_before > 0 {
            inv.shift_click(0usize);
            sleep(Duration::from_millis(200)).await;
            let after_count = count_item_in_player_slots(&inv, target_kind);
            if after_count > before_count {
                crafted += output;
                continue;
            }
        }

        // shift_click 失败或主背包满：用 left_click 手动收集（含 hotbar）
        inv.left_click(0usize);
        sleep(Duration::from_millis(150)).await;

        let inv2 = bot
            .get_inventory()
            .map_err(|e| format!("读取背包失败: {e:?}"))?;
        match find_empty_player_slot(&inv2) {
            Some(empty_slot) => {
                inv2.left_click(empty_slot);
                sleep(Duration::from_millis(150)).await;
            }
            None => {
                inv2.left_click(0usize);
                sleep(Duration::from_millis(100)).await;
                let _ = clear_grid(&inv2, GRID).await;
                let dump = dump_player_inventory(&inv2);
                return Err(format!(
                    "合成 {item} 失败：背包完全满（player_slots 无空位），产物无法收集。\
                     当前背包: {dump}\n\
                     建议：先 discard 丢弃垃圾物品（dirt/cobblestone/gravel/clay_ball/tuff/granite/diorite/flint 等），\
                     腾出至少 1 个空位后再重试 craft_3x3。"
                ));
            }
        }

        let inv3 = bot
            .get_inventory()
            .map_err(|e| format!("验证时读取背包失败: {e:?}"))?;
        let after_count2 = count_item_in_player_slots(&inv3, target_kind);
        if after_count2 > before_count {
            crafted += output;
            continue;
        }

        // P20 关键修复：shift_click 和 left_click 都失败时，关闭容器让服务端
        // 自动把光标/网格上的物品返回背包。这是最可靠的兜底——服务端在
        // ContainerClose 时会强制归还所有悬空物品。
        // 学习自 vanilla 客户端行为：玩家按 E 关闭工作台时，所有物品自动回背包。
        eprintln!(
            "[craft 3x3] shift_click + left_click 均失败，尝试关闭容器兜底收集 (before={before_count})"
        );
        // 先清网格（把网格上的原料回背包），再关容器（把光标上的产物回背包）
        let _ = clear_grid(&inv3, GRID).await;
        // 关闭容器
        crate::azalea::table_flow::close_container_if_open(bot);
        sleep(Duration::from_millis(400)).await;
        // 检查关容器后产物是否进入背包
        let inv4 = bot
            .get_inventory()
            .map_err(|e| format!("关容器后读取背包失败: {e:?}"))?;
        let after_close = count_item_in_player_slots(&inv4, target_kind);
        if after_close > before_count {
            eprintln!(
                "[craft 3x3] 关容器兜底成功：{} x{} 进入背包 (before={before_count} after={after_close})",
                item, output
            );
            crafted += output;
            // 重新打开工作台继续下一轮（如果有）
            if crafted < crafts_needed * output {
                if let Some(tp) = table_pos {
                    // 走到桌旁并重新打开
                    use azalea::Vec3;
                    use azalea::pathfinder::goals::RadiusGoal;
                    let target = Vec3::new(tp.x as f64 + 0.5, tp.y as f64 + 0.5, tp.z as f64 + 0.5);
                    let goto_fut = bot.goto(RadiusGoal {
                        pos: target,
                        radius: 1.5,
                    });
                    let _ = tokio::time::timeout(Duration::from_secs(5), goto_fut).await;
                    match bot.open_container_at(tp).await {
                        Ok(Some(h)) => {
                            std::mem::forget(h);
                        }
                        _ => {
                            return Err(format!(
                                "合成 {item} 部分完成（{crafted}）但重新打开工作台失败，无法继续"
                            ));
                        }
                    }
                    sleep(Duration::from_millis(300)).await;
                } else {
                    return Err(format!(
                        "合成 {item} 部分完成（{crafted}）但缺少 table_pos，无法重新打开工作台继续"
                    ));
                }
            }
            continue;
        }

        // 关容器也没能收集到产物——真正的失败
        let dump = dump_player_inventory(&inv4);
        return Err(format!(
            "合成 {item} 失败：产物无法从结果槽移入背包（shift_click、left_click、关容器兜底 均未让背包产物增加，\
             before={before_count} after_left={after_count2} after_close={after_close}）。\
             当前背包: {dump}\n\
             建议：1) 先 discard 腾出空位；2) 关闭工作台再重新打开后重试；3) 检查背包是否同步正常。"
        ));
    }

    clear_cursor(&inv).await;
    let _ = clear_grid(&inv, GRID).await;

    Ok(format!(
        "3×3 合成 {item} x{count} 完成（约 {crafted}，共 {crafts_needed} 次）"
    ))
}

/// 按配方书（服务端下发）做 3×3 合成：shaped 按网格摆放，shapeless 顺序摆放。
/// 需已打开工作台（Crafting 菜单）。返回完成信息。
pub async fn do_craft_3x3_recipe(
    bot: &Client,
    recipe: &crate::azalea::recipe_book::StoredRecipe,
    count: u32,
) -> Result<String, String> {
    use crate::azalea::recipe_book::StoredRecipe;
    let (grid_items, label) = match recipe {
        StoredRecipe::Shaped {
            width,
            height,
            grid,
            ..
        } => {
            // 把 width*height 的网格映射到 3×3 工作台槽位（1..=9，行优先）
            let mut placed: Vec<(usize, ItemKind)> = Vec::new();
            let w = *width as usize;
            let h = *height as usize;
            for r in 0..h {
                for c in 0..w {
                    let idx = r * w + c;
                    if let Some(Some(ing)) = grid.get(idx)
                        && let Some(k) = ing.items.first()
                    {
                        // 工作台槽位：row*3+col+1
                        placed.push((r * 3 + c + 1, *k));
                    }
                }
            }
            (placed, "shaped")
        }
        StoredRecipe::Shapeless { ingredients, .. } => {
            let mut placed: Vec<(usize, ItemKind)> = Vec::new();
            for (i, ing) in ingredients.iter().enumerate() {
                if let Some(k) = ing.items.first() {
                    placed.push((i + 1, *k));
                }
            }
            (placed, "shapeless")
        }
        _ => return Err("该配方不是 3×3 合成（请用 smelt/smithing 路径）".to_string()),
    };

    if grid_items.is_empty() {
        return Err("配方书无可用原料（可能是 tag 原料未解析）".to_string());
    }

    let inv = bot
        .get_inventory()
        .map_err(|e| format!("获取容器失败（确认已打开工作台）: {e:?}"))?;

    let crafts_needed = count.max(1);
    let mut crafted = 0u32;

    // 工作台菜单：slot 0 = 结果，1..=9 = 3×3 网格
    const GRID: std::ops::RangeInclusive<usize> = 1..=9;

    for round in 0..crafts_needed {
        // P9：先清网格与光标，残留会让服务端配方匹配失败
        clear_cursor(&inv).await;
        if !clear_grid(&inv, GRID).await {
            return Err(format!(
                "配方书合成失败：无法清空 3×3 网格残留（第 {} 轮）",
                round + 1
            ));
        }

        // P8：逐格放 1 个（shift_click 不按形状、整堆塞一格会让其他格缺料）
        for &(g, k) in &grid_items {
            let src =
                find_ingredient_slot(&inv, k, GRID).ok_or_else(|| format!("背包缺少原料 {k:?}"))?;
            place_one(&inv, src, g).await;
        }

        // 等服务端算结果（最多 2s）
        let mut has_result = false;
        for _ in 0..20 {
            sleep(Duration::from_millis(100)).await;
            let r = inv
                .slots()
                .as_ref()
                .and_then(|s| s.first())
                .map(|s| !s.is_empty())
                .unwrap_or(false);
            if r {
                has_result = true;
                break;
            }
        }
        if !has_result {
            let _ = clear_grid(&inv, GRID).await;
            return Err(
                "配方书合成失败：网格未产生结果（等待 2s slot 0 仍空，原料可能不足或形状不匹配）"
                    .to_string(),
            );
        }
        // P50 改进（2026-07-27）：对齐 do_craft_3x3 的 P20 + P49 验证逻辑。
        // 原 bug：shift_click(0) 后直接 crafted += 1，背包满时 shift_click 静默失败，
        // 产物仍在结果槽，crafted 虚增。
        // 修复：shift_click 后验证结果槽是否空；不空则 left_click 兜底；仍不空则报错。
        let target_kind = match recipe {
            crate::azalea::recipe_book::StoredRecipe::Shaped { result, .. }
            | crate::azalea::recipe_book::StoredRecipe::Shapeless { result, .. } => *result,
            _ => {
                let _ = clear_grid(&inv, GRID).await;
                return Err("配方书合成失败：不支持的非合成配方类型".to_string());
            }
        };
        let before_count = count_item_in_player_slots(&inv, target_kind);

        // 先 shift_click(0)
        inv.shift_click(0usize);
        sleep(Duration::from_millis(200)).await;
        let after_count = count_item_in_player_slots(&inv, target_kind);
        if after_count > before_count {
            crafted += 1;
            continue;
        }

        // shift_click 失败，用 left_click 兜底
        inv.left_click(0usize);
        sleep(Duration::from_millis(150)).await;
        let inv2 = match bot.get_inventory() {
            Ok(i) => i,
            Err(e) => return Err(format!("left_click 后读取背包失败: {e:?}")),
        };
        match find_empty_player_slot(&inv2) {
            Some(empty) => {
                inv2.left_click(empty);
                sleep(Duration::from_millis(150)).await;
            }
            None => {
                let _ = clear_grid(&inv2, GRID).await;
                return Err("配方书合成失败：背包完全满，产物无法收集。\
                     建议：先 discard 丢弃垃圾物品腾出空位后再重试。"
                    .to_string());
            }
        }
        let inv3 = match bot.get_inventory() {
            Ok(i) => i,
            Err(e) => return Err(format!("验证时读取背包失败: {e:?}")),
        };
        let after_count2 = count_item_in_player_slots(&inv3, target_kind);
        if after_count2 > before_count {
            crafted += 1;
            continue;
        }

        // 都失败了
        let _ = clear_grid(&inv3, GRID).await;
        return Err(
            "配方书合成失败：产物无法从结果槽移入背包（shift_click + left_click 均失败）。\
             建议：1) 先 discard 腾出空位；2) 关闭工作台再重新打开后重试。"
                .to_string(),
        );
    }

    clear_cursor(&inv).await;
    let _ = clear_grid(&inv, GRID).await;

    Ok(format!(
        "3×3 合成（配方书 {label}）x{count} 完成（约 {crafted} 次）"
    ))
}
