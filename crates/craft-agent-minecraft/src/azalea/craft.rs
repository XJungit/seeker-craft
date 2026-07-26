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
use azalea::inventory::operations::{PickupClick, ThrowClick};
use azalea::prelude::*;
use azalea::BlockPos;
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
/// 注意：此表只描述「需要哪些原料、各多少个」，不描述网格形状。
/// 顺序填充（slot1, slot2, slot3, slot4）只对单原料配方（如 planks）正确；
/// 竖直配方（stick/torch）必须走 SHAPED_2X2，否则横放导致服务端配方不匹配。
const RECIPES: &[(&'static str, &'static [(&'static str, u32)], u32)] = &[
    ("oak_planks", &[("oak_log", 1)], 4),
    ("stick", &[("oak_planks", 2)], 4),
    ("crafting_table", &[("oak_planks", 4)], 1),
    ("torch", &[("coal", 1), ("stick", 1)], 4),
    ("torch", &[("charcoal", 1), ("stick", 1)], 4),
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
const SHAPED_2X2: &[(&'static str, &'static [(usize, &'static str)], u32)] = &[
    // stick: 2 planks 竖直（左列）—— vanilla shape ["P","P"]
    ("stick", &[(1, "oak_planks"), (3, "oak_planks")], 4),
    // torch (coal 变体): coal 在上, stick 在下 —— vanilla shape ["C","S"]
    ("torch", &[(1, "coal"), (3, "stick")], 4),
    // torch (charcoal 变体): charcoal 在上, stick 在下
    ("torch", &[(1, "charcoal"), (3, "stick")], 4),
];

/// 查找 2×2 形状配方的所有候选（按表中顺序，coal 优先于 charcoal）。
/// 返回 (cells, output_per_craft) 列表；空表示该物品无形状配方，应回退到顺序填充。
fn lookup_shaped_2x2(item: &str) -> Vec<(&'static [(usize, &'static str)], u32)> {
    let b = bare(item);
    SHAPED_2X2
        .iter()
        .filter(|(id, _, _)| *id == b)
        .map(|(_, cells, out)| (*cells, *out))
        .collect()
}

/// 去掉 `minecraft:` 前缀，便于比较裸 id。
fn bare(id: &str) -> &str {
    id.strip_prefix("minecraft:").unwrap_or(id)
}

/// 把 `oak_planks`/`spruce_planks`/... 这类木板动态派生为「由对应原木合成」的配方，
/// 免去逐条登记。若查询本身不是木板（如原木），返回 None，避免自引用死循环。
fn planks_plan_for(planks_id: &str) -> Option<CraftPlan> {
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

fn lookup_recipe(item: &str) -> Option<CraftPlan> {
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

/// 统计玩家背包（player_slots_range，含 hotbar）里指定物品的总数。
///
/// P14 修复（2026-07-26）：P13 引入的产物收集验证用「slot 0 是否为空」判断成功，
/// 但 azalea 本地乐观更新对 result slot 的 QuickMove 语义不完整——服务端实际已把
/// 产物给了玩家、消耗了原料，但本地 slot 0 可能仍显示非空（被服务端重新算出 result
/// 填回，或本地 state 没同步）。导致 P13 验证逻辑误判「shift_click 失败」，
/// 走兜底 left_click 反而把网格里的原料拿出来污染背包，craft 失败率从 14% 飙到 77%。
///
/// 正确做法：**验证背包里产物数量是否增加**（ground truth），而非 slot 0 是否为空。
fn count_item_in_player_slots(inv: &ContainerHandleRef, kind: ItemKind) -> u32 {
    let Some(menu) = inv.menu().ok().flatten() else { return 0; };
    let Some(slots) = inv.slots() else { return 0; };
    let range = menu.player_slots_range();
    slots
        .iter()
        .enumerate()
        .filter(|(i, _)| range.contains(i))
        .filter(|(_, s)| !s.is_empty() && s.kind() == kind)
        .map(|(_, s)| s.count() as u32)
        .sum()
}

/// 统计玩家背包（player_slots_range，含 hotbar）里的空槽位数。
///
/// P15 修复（2026-07-26）：azalea 的 `PlayerMenuLocation::CraftResult` shift_click
/// 只移到 `Player::INVENTORY_SLOTS`（主背包 9..=35），**不含 hotbar**。当主背包满
/// 但 hotbar 有空位时，shift_click(result) 失败，result 留在 slot 0，craft 报错
/// "产物无法移入背包"。实际上 hotbar 还能放，但 azalea 不会移过去。
///
/// 本函数用于 craft 前检查空位，以及 craft 失败时给出明确诊断（背包满→让 LLM discard）。
fn count_empty_player_slots(inv: &ContainerHandleRef) -> u32 {
    let Some(menu) = inv.menu().ok().flatten() else { return 0; };
    let Some(slots) = inv.slots() else { return 0; };
    let range = menu.player_slots_range();
    slots
        .iter()
        .enumerate()
        .filter(|(i, _)| range.contains(i))
        .filter(|(_, s)| s.is_empty())
        .count() as u32
}

/// 找玩家背包（player_slots_range，含 hotbar）里第一个空槽位。
fn find_empty_player_slot(inv: &ContainerHandleRef) -> Option<usize> {
    let menu = inv.menu().ok()??;
    let slots = inv.slots()?;
    let range = menu.player_slots_range();
    for s in range {
        if let Some(st) = slots.get(s) {
            if st.is_empty() {
                return Some(s);
            }
        }
    }
    None
}

/// 列出背包内容（用于错误诊断），格式 "slot_idx=itemxN, ..."，最多 20 个非空槽。
fn dump_player_inventory(inv: &ContainerHandleRef) -> String {
    let Some(menu) = inv.menu().ok().flatten() else { return "(无法读取菜单)".into(); };
    let Some(slots) = inv.slots() else { return "(无法读取槽位)".into(); };
    let range = menu.player_slots_range();
    let mut items: Vec<String> = Vec::new();
    let mut count = 0;
    for s in range {
        if let Some(st) = slots.get(s) {
            if !st.is_empty() {
                let k = st.kind().to_str();
                let bare = k.strip_prefix("minecraft:").unwrap_or(k);
                items.push(format!("slot{s}={bare}x{}", st.count()));
                count += 1;
                if count >= 20 { break; }
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
    if empty_before >= 2 {
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
        // 保留少量的有用方块
        ("cobblestone", 16),
        ("cobbled_deepslate", 16),
        ("oak_sapling", 4),
        ("flint", 8),
        ("string", 4),
        ("stick", 16), // 合成原料，保留 16 个够用
        // 丢弃多余的同类工具（如多把 iron_hoe）
        ("iron_hoe", 0),
        ("wooden_hoe", 0),
        ("stone_hoe", 0),
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
        let Some(menu) = inv.menu().ok().flatten() else { break; };
        let Some(slots) = inv.slots() else { break; };
        let range = menu.player_slots_range();

        // 收集所有该类物品的 (slot, count)
        let mut stacks: Vec<(usize, u32)> = Vec::new();
        for s in range {
            if let Some(st) = slots.get(s) {
                if !st.is_empty() && st.kind() == kind {
                    stacks.push((s, st.count() as u32));
                }
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
fn find_ingredient_slot(
    inv: &ContainerHandleRef,
    kind: ItemKind,
    grid_slots: std::ops::RangeInclusive<usize>,
) -> Option<usize> {
    if let Some(s) = find_source_slot(inv, kind) {
        return Some(s);
    }
    // 兜底：网格里可能有上次残留的同种原料
    let slots = inv.slots()?;
    for s in grid_slots {
        if let Some(stack) = slots.get(s) {
            if !stack.is_empty() && stack.kind() == kind {
                return Some(s);
            }
        }
    }
    None
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
async fn clear_grid(inv: &ContainerHandleRef, slots: std::ops::RangeInclusive<usize>) -> bool {
    let mut all_clear = true;
    for s in slots {
        let non_empty = inv
            .slots()
            .as_ref()
            .and_then(|all| all.get(s))
            .map(|st| !st.is_empty())
            .unwrap_or(false);
        if !non_empty {
            continue;
        }
        inv.shift_click(s);
        sleep(Duration::from_millis(80)).await;
        let still_non_empty = inv
            .slots()
            .as_ref()
            .and_then(|all| all.get(s))
            .map(|st| !st.is_empty())
            .unwrap_or(false);
        if !still_non_empty {
            continue;
        }
        // 兜底：手动拿起再放到第一个空背包槽
        inv.left_click(s);
        sleep(Duration::from_millis(80)).await;
        if let Some(menu) = inv.menu().ok().flatten() {
            let player_range = menu.player_slots_range();
            if let Some(slots_data) = inv.slots() {
                for ps in player_range {
                    let empty = slots_data.get(ps).map(|st| st.is_empty()).unwrap_or(false);
                    if empty {
                        inv.left_click(ps);
                        sleep(Duration::from_millis(80)).await;
                        break;
                    }
                }
            }
        }
        let still_non_empty = inv
            .slots()
            .as_ref()
            .and_then(|all| all.get(s))
            .map(|st| !st.is_empty())
            .unwrap_or(false);
        if still_non_empty {
            all_clear = false;
        }
    }
    all_clear
}

/// 把光标上可能残留的物品放回背包。
///
/// `place_one` 是「拿起→放下→放回」三步，中途失败会让物品留在光标上，
/// 污染后续所有点击（服务端认为手里有东西）。azalea 没有查询光标的 API，
/// 因此用 left_click 一个空背包槽来尝试放下（光标为空时是安全的 no-op）。
async fn clear_cursor(inv: &ContainerHandleRef) {
    for s in 9..=35usize {
        let empty = inv
            .slots()
            .as_ref()
            .and_then(|all| all.get(s))
            .map(|st| st.is_empty())
            .unwrap_or(false);
        if empty {
            inv.left_click(s);
            sleep(Duration::from_millis(80)).await;
            return;
        }
    }
}

/// 从 src 槽取 **1 个** 物品放到 dst 槽。
///
/// 三步：`left_click(src)` 拿起整堆 → `right_click(dst)` 放 1 个 → `left_click(src)` 放回剩余。
///
/// P8 修复：不能用 `shift_click`/`move_stack` 填网格——前者由服务端决定去哪一格
/// （不按配方形状），后者会把整堆塞进一格，导致同种原料的其他格找不到料，
/// 表现为 furnace/chest/工具类（镐斧剑）3×3 合成全部失败。
///
/// P10 修复：每个 click 间隔 80ms（>1 server tick=50ms）。azalea 的 `click()` 是
/// fire-and-forget，且服务端用 `state_id` 做 desync 检测；20ms 间隔连发三个 click
/// 会让后续 click 带着过期 state_id 被服务端静默拒绝，表现为 stone_pickaxe 配方
/// 的 slot5（中中）总是空的。完成后校验 dst 槽，失败则退避重试最多 3 次。
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
            if let Ok(inv) = bot.get_inventory() {
                if let Ok(Some(menu)) = inv.menu() {
                    if matches!(menu, azalea::inventory::Menu::Player(_)) {
                        break;
                    }
                }
            }
        }
    }

    let inv = bot
        .get_inventory()
        .map_err(|e| format!("获取背包失败: {e:?}"))?;

    // 二次确认：菜单必须是 Player（2×2 网格只在此菜单下有效）
    if let Ok(Some(menu)) = inv.menu() {
        if !matches!(menu, azalea::inventory::Menu::Player(_)) {
            return Err(format!(
                "合成 {item} 失败：当前打开的不是玩家背包（2×2 网格不可用）。\
                 请先关闭已打开的容器再调用 craft。"
            ));
        }
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
                    ItemKind::from_str(&normalize_item(ing_id))
                        .unwrap_or_else(|_| ItemKind::Air),
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

    let crafts_needed = (count.max(1) + output - 1) / output;
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
                .and_then(|s| s.get(0))
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
        let before_count = count_item_in_player_slots(&inv, target_kind);

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
        // 1. left_click(0) 拿起 result
        inv.left_click(0usize);
        sleep(Duration::from_millis(150)).await;

        // 2. 重新读 inv 找空槽（每次都读最新的，避免 stale state）
        let inv2 = bot.get_inventory().map_err(|e| format!("读取背包失败: {e:?}"))?;
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
        let inv3 = bot.get_inventory().map_err(|e| format!("验证时读取背包失败: {e:?}"))?;
        let after_count2 = count_item_in_player_slots(&inv3, target_kind);
        if after_count2 > before_count {
            crafted += output;
            continue;
        }

        // 都失败：清网格，报错
        let _ = clear_grid(&inv3, GRID).await;
        return Err(format!(
            "合成 {item} 失败：产物无法从结果槽移入背包（shift_click 与 left_click 均未让背包产物增加，\
             before={before_count} after_left={after_count2}）。\
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
    // P16 修复（2026-07-26）：2×2 配方也加入 3×3 表。
    // vanilla 中这些配方在 2×2 和 3×3 网格中都能合成（形状放在左上角），
    // 但原 lookup_shaped 只查 SHAPED_RECIPES，craft_3x3 对这些物品报
    // "不支持的 3×3 合成目标"。LLM 常误用 craft_3x3 合成 crafting_table，
    // 导致 100% 失败。加入这些配方让 craft_3x3 也能处理。
    // 槽位编号：1 2 3 / 4 5 6 / 7 8 9，2×2 形状放在 1,2,4,5。
    ("oak_planks", ShapedRecipe { cells: &[(1,"oak_log")], output_per_craft: 4 }),
    ("stick", ShapedRecipe { cells: &[(1,"oak_planks"),(4,"oak_planks")], output_per_craft: 4 }),
    ("crafting_table", ShapedRecipe { cells: &[(1,"oak_planks"),(2,"oak_planks"),(4,"oak_planks"),(5,"oak_planks")], output_per_craft: 1 }),
    ("torch", ShapedRecipe { cells: &[(1,"coal"),(4,"stick")], output_per_craft: 4 }),
    ("torch_charcoal", ShapedRecipe { cells: &[(1,"charcoal"),(4,"stick")], output_per_craft: 4 }),
    // 环形：8 格同种原料
    ("furnace", ShapedRecipe { cells: &[(1,"cobblestone"),(2,"cobblestone"),(3,"cobblestone"),(4,"cobblestone"),(6,"cobblestone"),(7,"cobblestone"),(8,"cobblestone"),(9,"cobblestone")], output_per_craft: 1 }),
    ("chest", ShapedRecipe { cells: &[(1,"oak_planks"),(2,"oak_planks"),(3,"oak_planks"),(4,"oak_planks"),(6,"oak_planks"),(7,"oak_planks"),(8,"oak_planks"),(9,"oak_planks")], output_per_craft: 1 }),
    ("ladder", ShapedRecipe { cells: &[(1,"stick"),(2,"stick"),(3,"stick"),(4,"stick"),(5,"stick"),(6,"stick"),(7,"stick"),(8,"stick"),(9,"stick")], output_per_craft: 3 }),
    ("oak_trapdoor", ShapedRecipe { cells: &[(1,"oak_planks"),(2,"oak_planks"),(3,"oak_planks"),(4,"oak_planks"),(5,"oak_planks"),(6,"oak_planks")], output_per_craft: 2 }),
    // 门：两列木板
    ("oak_door", ShapedRecipe { cells: &[(1,"oak_planks"),(2,"oak_planks"),(4,"oak_planks"),(5,"oak_planks"),(7,"oak_planks"),(8,"oak_planks")], output_per_craft: 3 }),
    // 栅栏：上下木板 + 中间棍
    ("oak_fence", ShapedRecipe { cells: &[(1,"oak_planks"),(2,"oak_planks"),(4,"stick"),(5,"stick"),(7,"oak_planks"),(8,"oak_planks")], output_per_craft: 3 }),
    // 工具类的 vanilla 形状（3×3 网格编号：1 2 3 / 4 5 6 / 7 8 9）
    //   镐  XXX / .S. / .S.  → 头部占 1,2,3；柄占 5,8
    //   斧  XX. / XS. / .S.  → 头部占 1,2,4；柄占 5,8
    //   剑  .X. / .X. / .S.  → 刃占 2,5；柄占 8
    //   锹  .X. / .S. / .S.  → 头占 2；柄占 5,8
    //   锄  XX. / .S. / .S.  → 头占 1,2；柄占 5,8
    // 旧版把柄写成 5,7（锄写成 4,7）——柄不在同一竖列，服务端配方匹配失败，
    // 是「网格未产生结果」的一个独立成因。
    ("wooden_pickaxe", ShapedRecipe { cells: &[(1,"oak_planks"),(2,"oak_planks"),(3,"oak_planks"),(5,"stick"),(8,"stick")], output_per_craft: 1 }),
    ("wooden_axe", ShapedRecipe { cells: &[(1,"oak_planks"),(2,"oak_planks"),(4,"oak_planks"),(5,"stick"),(8,"stick")], output_per_craft: 1 }),
    ("wooden_sword", ShapedRecipe { cells: &[(2,"oak_planks"),(5,"oak_planks"),(8,"stick")], output_per_craft: 1 }),
    ("wooden_shovel", ShapedRecipe { cells: &[(2,"oak_planks"),(5,"stick"),(8,"stick")], output_per_craft: 1 }),
    ("wooden_hoe", ShapedRecipe { cells: &[(1,"oak_planks"),(2,"oak_planks"),(5,"stick"),(8,"stick")], output_per_craft: 1 }),
    // 石制工具（用 cobblestone 代替木板）
    ("stone_pickaxe", ShapedRecipe { cells: &[(1,"cobblestone"),(2,"cobblestone"),(3,"cobblestone"),(5,"stick"),(8,"stick")], output_per_craft: 1 }),
    ("stone_axe", ShapedRecipe { cells: &[(1,"cobblestone"),(2,"cobblestone"),(4,"cobblestone"),(5,"stick"),(8,"stick")], output_per_craft: 1 }),
    ("stone_sword", ShapedRecipe { cells: &[(2,"cobblestone"),(5,"cobblestone"),(8,"stick")], output_per_craft: 1 }),
    ("stone_shovel", ShapedRecipe { cells: &[(2,"cobblestone"),(5,"stick"),(8,"stick")], output_per_craft: 1 }),
    ("stone_hoe", ShapedRecipe { cells: &[(1,"cobblestone"),(2,"cobblestone"),(5,"stick"),(8,"stick")], output_per_craft: 1 }),
    // 铁制工具（需先熔炼 iron_ingot）
    ("iron_pickaxe", ShapedRecipe { cells: &[(1,"iron_ingot"),(2,"iron_ingot"),(3,"iron_ingot"),(5,"stick"),(8,"stick")], output_per_craft: 1 }),
    ("iron_axe", ShapedRecipe { cells: &[(1,"iron_ingot"),(2,"iron_ingot"),(4,"iron_ingot"),(5,"stick"),(8,"stick")], output_per_craft: 1 }),
    ("iron_sword", ShapedRecipe { cells: &[(2,"iron_ingot"),(5,"iron_ingot"),(8,"stick")], output_per_craft: 1 }),
    ("iron_shovel", ShapedRecipe { cells: &[(2,"iron_ingot"),(5,"stick"),(8,"stick")], output_per_craft: 1 }),
    ("iron_hoe", ShapedRecipe { cells: &[(1,"iron_ingot"),(2,"iron_ingot"),(5,"stick"),(8,"stick")], output_per_craft: 1 }),
    // 铁盔甲
    ("iron_helmet", ShapedRecipe { cells: &[(1,"iron_ingot"),(2,"iron_ingot"),(3,"iron_ingot"),(4,"iron_ingot"),(6,"iron_ingot")], output_per_craft: 1 }),
    ("iron_chestplate", ShapedRecipe { cells: &[(1,"iron_ingot"),(3,"iron_ingot"),(4,"iron_ingot"),(5,"iron_ingot"),(6,"iron_ingot"),(7,"iron_ingot"),(8,"iron_ingot"),(9,"iron_ingot")], output_per_craft: 1 }),
    ("iron_leggings", ShapedRecipe { cells: &[(1,"iron_ingot"),(2,"iron_ingot"),(3,"iron_ingot"),(4,"iron_ingot"),(6,"iron_ingot"),(7,"iron_ingot"),(8,"iron_ingot"),(9,"iron_ingot")], output_per_craft: 1 }),
    ("iron_boots", ShapedRecipe { cells: &[(1,"iron_ingot"),(3,"iron_ingot"),(7,"iron_ingot"),(9,"iron_ingot")], output_per_craft: 1 }),
];

fn lookup_shaped(item: &str) -> Option<ShapedRecipe> {
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

    // P17 修复（2026-07-27）：手写 SHAPED_RECIPES 表只有 ~30 个配方，
    // LLM 调用 craft_3x3 合成表外物品（如 bread/cake/bowl 等）时 100% 失败。
    // 改为：先查手写表，未命中则回退到 RecipeBook（vanilla 26.2 全量配方书），
    // 调用 do_craft_3x3_recipe 走 RecipeBook 路径。
    // 学习自 mindcraft：mindcraft 用 mineflayer-prismarine-recipe 全量配方，
    // 不需要手写配方表。本项目 azalea 无 prismarine-recipe，但 RecipeBook 是等价物。
    let recipe = match lookup_shaped(item) {
        Some(r) => r,
        None => {
            // 回退到 RecipeBook
            let book = crate::azalea::auto_craft::recipe_book_of(bot);
            match book.get_by_result(item) {
                Some(stored) => {
                    eprintln!(
                        "[craft 3x3] '{item}' 不在手写表，回退到 RecipeBook ({})",
                        stored.kind()
                    );
                    return do_craft_3x3_recipe(bot, stored, count).await;
                }
                None => {
                    return Err(format!(
                        "不支持的 3×3 合成目标 {item}（手写配方表和 RecipeBook 均无此配方）。\
                         可能原因：1) 物品名拼写错误；2) 该物品不可合成（如 air/bedrock）；\
                         3) 该物品是熔炼/切石产物，请用 smelt 工具。"
                    ));
                }
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
    let crafts_needed = (count.max(1) + output - 1) / output;
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
                .and_then(|s| s.get(0))
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
        let target_kind = ItemKind::from_str(&normalize_item(item))
            .map_err(|_| format!("未知目标物品 {item}"))?;
        let before_count = count_item_in_player_slots(&inv, target_kind);

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

        let inv2 = bot.get_inventory().map_err(|e| format!("读取背包失败: {e:?}"))?;
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

        let inv3 = bot.get_inventory().map_err(|e| format!("验证时读取背包失败: {e:?}"))?;
        let after_count2 = count_item_in_player_slots(&inv3, target_kind);
        if after_count2 > before_count {
            crafted += output;
            continue;
        }

        let _ = clear_grid(&inv3, GRID).await;
        return Err(format!(
            "合成 {item} 失败：产物无法从结果槽移入背包（shift_click 与 left_click 均未让背包产物增加，\
             before={before_count} after_left={after_count2}）。\
             建议：关闭工作台再重新打开后重试，或先 discard 腾出空位。"
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
        StoredRecipe::Shaped { width, height, grid, .. } => {
            // 把 width*height 的网格映射到 3×3 工作台槽位（1..=9，行优先）
            let mut placed: Vec<(usize, ItemKind)> = Vec::new();
            let w = *width as usize;
            let h = *height as usize;
            for r in 0..h {
                for c in 0..w {
                    let idx = r * w + c;
                    if let Some(Some(ing)) = grid.get(idx) {
                        if let Some(k) = ing.items.first() {
                            // 工作台槽位：row*3+col+1
                            placed.push((r * 3 + c + 1, *k));
                        }
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
            let src = find_ingredient_slot(&inv, k, GRID)
                .ok_or_else(|| format!("背包缺少原料 {k:?}"))?;
            place_one(&inv, src, g).await;
        }

        // 等服务端算结果（最多 2s）
        let mut has_result = false;
        for _ in 0..20 {
            sleep(Duration::from_millis(100)).await;
            let r = inv
                .slots()
                .as_ref()
                .and_then(|s| s.get(0))
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
        inv.shift_click(0usize);
        sleep(Duration::from_millis(150)).await;
        crafted += 1;
    }

    clear_cursor(&inv).await;
    let _ = clear_grid(&inv, GRID).await;

    Ok(format!(
        "3×3 合成（配方书 {label}）x{count} 完成（约 {crafted} 次）"
    ))
}

/// 在已打开的锻造台菜单中，按配方书 Smithing 配方合成（template/base/addition 已就绪）。
pub async fn do_craft_smithing(
    bot: &Client,
    recipe: &crate::azalea::recipe_book::StoredRecipe,
    count: u32,
) -> Result<String, String> {
    use crate::azalea::recipe_book::StoredRecipe;
    let (template, base, addition) = match recipe {
        StoredRecipe::Smithing {
            template,
            base,
            addition,
            ..
        } => (template.items.first().copied(), base.items.first().copied(), addition.items.first().copied()),
        _ => return Err("do_craft_smithing 仅支持 Smithing 配方".to_string()),
    };
    let inv = bot
        .get_inventory()
        .map_err(|e| format!("获取容器失败（确认已打开锻造台）: {e:?}"))?;

    let mut made = 0u32;
    for _ in 0..count.max(1) {
        if let Some(k) = template {
            let src = find_source_slot(&inv, k).ok_or_else(|| format!("背包缺少模板 {}", k))?;
            move_stack(&inv, src, 0).await; // template 槽
        }
        if let Some(k) = base {
            let src = find_source_slot(&inv, k).ok_or_else(|| format!("背包缺少基础物品 {}", k))?;
            move_stack(&inv, src, 1).await; // base 槽
        }
        if let Some(k) = addition {
            let src = find_source_slot(&inv, k).ok_or_else(|| format!("背包缺少附加物品 {}", k))?;
            move_stack(&inv, src, 2).await; // additional 槽
        }
        sleep(Duration::from_millis(80)).await;
        let has_result = inv
            .slots()
            .as_ref()
            .and_then(|s| s.get(3))
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        if !has_result {
            return Err("锻造失败：结果槽无产物（模板/基础/附加可能不足）".to_string());
        }
        inv.shift_click(3usize); // 取结果
        sleep(Duration::from_millis(40)).await;
        made += 1;
    }
    Ok(format!("锻造合成 x{count} 完成（约 {made} 次）"))
}

/// 切石机合成：把 input 放入槽 1，结果出现在槽 1（与 input 同号，先放后取），
/// 重复 count 次。切石机只有一个输出选项，故直接取结果槽。
pub async fn do_craft_stonecutter(
    bot: &Client,
    recipe: &crate::azalea::recipe_book::StoredRecipe,
    count: u32,
) -> Result<String, String> {
    use crate::azalea::recipe_book::StoredRecipe;
    let input = match recipe {
        StoredRecipe::Stonecutter { input, .. } => input.items.first().copied(),
        _ => return Err("do_craft_stonecutter 仅支持 Stonecutter 配方".to_string()),
    };
    let inv = bot
        .get_inventory()
        .map_err(|e| format!("获取容器失败（确认已打开切石机）: {e:?}"))?;
    let mut made = 0u32;
    for _ in 0..count.max(1) {
        if let Some(k) = input {
            let src = find_source_slot(&inv, k)
                .ok_or_else(|| format!("背包缺少切石机原料 {}", k))?;
            move_stack(&inv, src, 1).await; // input 槽
        }
        sleep(Duration::from_millis(80)).await;
        let has_result = inv
            .slots()
            .as_ref()
            .and_then(|s| s.get(1))
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        if !has_result {
            return Err("切石失败：结果槽无产物（原料可能不足）".to_string());
        }
        inv.shift_click(1usize); // 取结果
        sleep(Duration::from_millis(40)).await;
        made += 1;
    }
    Ok(format!("切石合成 x{count} 完成（约 {made} 次）"))
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

fn lookup_smelt_all(output: &str) -> Vec<SmeltRecipe> {
    // P17 修复（2026-07-27）：原代码用 normalize_item(output) 把 id 变成
    // "minecraft:iron_ingot"，但 SMELT_RECIPES 存的是裸 id "iron_ingot"，
    // 比较永远 false → lookup_smelt 100% 返回 None → smelt 报"不支持 iron_ingot"
    // 但错误消息里又列出 iron_ingot 作为支持项，自相矛盾。
    // 修复：用 bare() 去前缀（与 P12 lookup_shaped 修复同理）。
    //
    // P18 修复（2026-07-27）：返回**所有**候选配方（不再 .find().first()）。
    // SMELT_RECIPES 中 iron_ingot 有两条候选：iron_ore 和 raw_iron。
    // vanilla 中挖 iron_ore 掉 raw_iron（不是 iron_ore 本身），所以 bot
    // 背包里通常是 raw_iron。原 lookup_smelt 只返回第一个（iron_ore），
    // do_smelt 报"背包缺少输入 iron_ore"——但 bot 实际有 raw_iron×7！
    // 修复：返回所有候选，do_smelt 优先选背包里有的原料。
    let norm = bare(output);
    SMELT_RECIPES
        .iter()
        .filter(|(id, _)| *id == norm)
        .map(|(_, r)| SmeltRecipe {
            input: r.input,
            output_per_craft: r.output_per_craft,
        })
        .collect()
}

pub async fn do_smelt(
    bot: &Client,
    output: &str,
    fuel: &str,
    count: u32,
) -> Result<String, String> {
    // P18 修复：从所有候选配方中选一个 bot 背包里有的原料。
    let candidates = lookup_smelt_all(output);
    if candidates.is_empty() {
        return Err(format!(
            "不支持的熔炼产物 {output}（当前支持 iron_ingot/copper_ingot/gold_ingot/glass/stone/charcoal 等，需先打开熔炉）"
        ));
    }

    let inv = bot
        .get_inventory()
        .map_err(|e| format!("获取容器失败（确认已打开熔炉）: {e:?}"))?;

    // 优先选背包里有的原料；若都没有，选第一个候选并报错（列出所有候选原料）
    let recipe = {
        let mut chosen: Option<SmeltRecipe> = None;
        for c in &candidates {
            let kind = match ItemKind::from_str(&normalize_item(c.input)) {
                Ok(k) => k,
                Err(_) => continue,
            };
            if find_source_slot(&inv, kind).is_some() {
                chosen = Some(SmeltRecipe {
                    input: c.input,
                    output_per_craft: c.output_per_craft,
                });
                break;
            }
        }
        match chosen {
            Some(r) => r,
            None => {
                let inputs: Vec<&str> = candidates.iter().map(|c| c.input).collect();
                return Err(format!(
                    "背包缺少熔炼 {output} 的原料（候选: {}）。\
                     vanilla 规则：挖 {} 矿掉的是 raw_xxx（不是 ore 本身）\
                     ——请用 perceive 查看背包实际拥有的原料 id。",
                    inputs.join(" / "),
                    output
                ));
            }
        }
    };

    let input_kind = ItemKind::from_str(&normalize_item(recipe.input))
        .map_err(|_| format!("未知输入 {}", recipe.input))?;
    let fuel_kind = ItemKind::from_str(&normalize_item(fuel))
        .map_err(|_| format!("未知燃料 {fuel}"))?;

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

        // P10 修复：vanilla 单次熔炼需 200 ticks = 10s（燃料点燃还有额外延迟），
        // 原来只等 1.2s，结果槽必然是空的 → smelt 稳定失败（实测 80% 失败率）。
        // 改为轮询结果槽最多 30s，一有产物立刻继续，不必等满。
        let mut has_result = false;
        for _ in 0..300 {
            sleep(Duration::from_millis(100)).await;
            let r = inv
                .slots()
                .as_ref()
                .and_then(|s| s.get(2))
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
                        .take(3)
                        .enumerate()
                        .map(|(i, st)| {
                            let name = match i {
                                0 => "输入",
                                1 => "燃料",
                                _ => "产物",
                            };
                            format!(
                                "{name}={}",
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
            return Err(format!(
                "熔炼 {output} 失败：等待 30s 后结果槽仍无产物。熔炉状态: [{diag}]。\
                 可能原因：燃料不足（1 煤=8 次熔炼）、输入物品不可熔炼、或打开的不是熔炉"
            ));
        }

        // 取产物：shift_click 后要给服务端时间回包，否则下一轮读到的还是旧状态
        inv.shift_click(2usize);
        sleep(Duration::from_millis(150)).await;
        smelted += output_per;
    }

    Ok(format!(
        "熔炼 {output} x{count} 完成（约 {smelted}，共 {crafts_needed} 次）"
    ))
}

/// 酿造：在已打开的酿造台菜单中，把 `base`（默认 water_bottle）用 `ingredient` 酿成结果。
/// 酿造台槽位：ingredient=0, fuel=1, bottles=3/4/5（产物回到瓶槽）。
/// 一次最多 3 瓶；每瓶耗 1 份 ingredient，每轮约 20s（400 ticks）。
pub async fn do_brew(
    bot: &Client,
    recipe: &crate::azalea::recipe_book::StoredRecipe,
    count: u32,
) -> Result<String, String> {
    use crate::azalea::recipe_book::StoredRecipe;
    let (ingredient, base) = match recipe {
        StoredRecipe::Brewing {
            ingredient, base, ..
        } => (ingredient.items.first().copied(), base.items.first().copied()),
        _ => return Err("do_brew 仅支持 Brewing 配方".to_string()),
    };
    let ing_kind = ingredient.ok_or("酿造配方缺少原料".to_string())?;
    let base_kind = base.ok_or("酿造配方缺少基底（如 water_bottle）".to_string())?;
    let fuel_kind = ItemKind::from_str("blaze_powder").map_err(|_| "blaze_powder 解析失败".to_string())?;

    let inv = bot
        .get_inventory()
        .map_err(|e| format!("获取容器失败（确认已打开酿造台）: {e:?}"))?;

    let mut made = 0u32;
    let total = count.max(1);
    while made < total {
        let batch = (total - made).min(3);
        // 燃料（blaze_powder）放槽 1
        if let Some(src) = find_source_slot(&inv, fuel_kind) {
            move_stack(&inv, src, 1).await;
        }
        // 原料放槽 0
        let src_ing = find_source_slot(&inv, ing_kind)
            .ok_or_else(|| format!("背包缺少酿造原料 {}", ing_kind))?;
        move_stack(&inv, src_ing, 0).await;
        // 基底瓶放槽 3/4/5
        for slot in 3..3 + batch {
            let src = find_source_slot(&inv, base_kind)
                .ok_or_else(|| format!("背包缺少基底 {}", base_kind))?;
            move_stack(&inv, src, slot as usize).await;
        }
        // 等待酿造完成（一轮 400 ticks ≈ 20s）
        sleep(Duration::from_millis(21000)).await;
        // 收回瓶槽产物
        for slot in 3..3 + batch {
            inv.shift_click(slot as usize);
            sleep(Duration::from_millis(40)).await;
        }
        made += batch;
    }
    Ok(format!("酿造 x{total} 完成（约 {made} 瓶）"))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// P12 回归测试：lookup_shaped 必须能查到 SHAPED_RECIPES 表中的所有工具配方。
    /// 历史bug：原 lookup_shaped 用 normalize_item() 给 id 加 "minecraft:" 前缀，
    /// 但表里存的是裸 id（如 "stone_pickaxe"），导致查找永远不匹配 → craft_3x3 100% 失败。
    #[test]
    fn regression_lookup_shaped_finds_pickaxe_recipes() {
        // 裸 id 必须能查到
        assert!(lookup_shaped("wooden_pickaxe").is_some(), "wooden_pickaxe 必须可查");
        assert!(lookup_shaped("stone_pickaxe").is_some(), "stone_pickaxe 必须可查");
        assert!(lookup_shaped("iron_pickaxe").is_some(), "iron_pickaxe 必须可查");
        // 带 minecraft: 前缀也必须能查到（LLM 经常输出带前缀的形式）
        assert!(lookup_shaped("minecraft:stone_pickaxe").is_some(), "minecraft:stone_pickaxe 必须可查");
        assert!(lookup_shaped("minecraft:wooden_axe").is_some(), "minecraft:wooden_axe 必须可查");
        // 熔炉/箱子等环形配方
        assert!(lookup_shaped("furnace").is_some(), "furnace 必须可查");
        assert!(lookup_shaped("chest").is_some(), "chest 必须可查");
        // 不存在的物品应返回 None
        assert!(lookup_shaped("nonexistent_item").is_none());
        // P16 修复（2026-07-26）：2×2 配方（crafting_table/torch/oak_planks/stick）
        // 也加入 3×3 表，让 craft_3x3 能处理 LLM 误用 craft_3x3 合成这些物品的情况。
        assert!(lookup_shaped("crafting_table").is_some(), "crafting_table 应在 3×3 表中（P16）");
        assert!(lookup_shaped("torch").is_some(), "torch 应在 3×3 表中（P16）");
        assert!(lookup_shaped("oak_planks").is_some(), "oak_planks 应在 3×3 表中（P16）");
        assert!(lookup_shaped("stick").is_some(), "stick 应在 3×3 表中（P16）");
    }

    /// P12 回归测试：lookup_shaped 返回的 cells 必须是 vanilla 正确形状。
    /// 镐形状：头部占 1,2,3（顶行），柄占 5,8（中列）。
    #[test]
    fn regression_lookup_shaped_pickaxe_shape_is_vanilla_correct() {
        let r = lookup_shaped("stone_pickaxe").expect("stone_pickaxe 必须可查");
        // 顶部 3 格 cobblestone
        assert!(r.cells.contains(&(1, "cobblestone")), "slot1 应为 cobblestone");
        assert!(r.cells.contains(&(2, "cobblestone")), "slot2 应为 cobblestone");
        assert!(r.cells.contains(&(3, "cobblestone")), "slot3 应为 cobblestone");
        // 柄竖直：slot5（中中）+ slot8（中下）
        assert!(r.cells.contains(&(5, "stick")), "slot5 应为 stick");
        assert!(r.cells.contains(&(8, "stick")), "slot8 应为 stick");
        // 不应包含 slot7（左下）——柄不在左列
        assert!(!r.cells.contains(&(7, "stick")), "slot7 不应有 stick（柄应在正中竖列）");
    }

    /// P12 回归测试：lookup_shaped_2x2 必须返回 stick/torch 的形状候选。
    /// 历史bug：原 do_craft_2x2 顺序填充把 stick/torch 的 2 个原料横放在 slot1+slot2，
    /// 但 vanilla 是竖直配方 → 服务端配方匹配失败 → 100% 失败。
    #[test]
    fn regression_lookup_shaped_2x2_returns_vertical_recipes() {
        // stick 应有 1 个候选（2 planks 竖直）
        let stick_candidates = lookup_shaped_2x2("stick");
        assert_eq!(stick_candidates.len(), 1, "stick 应有 1 个候选");
        let (cells, out) = &stick_candidates[0];
        assert_eq!(*out, 4, "stick 每次产出 4 个");
        // 必须是 slot1 + slot3（左列竖直），不是 slot1 + slot2（横放）
        assert!(cells.contains(&(1, "oak_planks")), "slot1 应有 oak_planks");
        assert!(cells.contains(&(3, "oak_planks")), "slot3 应有 oak_planks");
        assert!(!cells.contains(&(2, "oak_planks")), "slot2 不应有 planks（会导致横放）");

        // torch 应有 2 个候选（coal + charcoal 变体）
        let torch_candidates = lookup_shaped_2x2("torch");
        assert_eq!(torch_candidates.len(), 2, "torch 应有 2 个候选（coal/charcoal）");
        // coal 变体（第一个）
        let (coal_cells, _) = &torch_candidates[0];
        assert!(coal_cells.contains(&(1, "coal")), "coal 变体 slot1 应为 coal");
        assert!(coal_cells.contains(&(3, "stick")), "coal 变体 slot3 应为 stick");
        // charcoal 变体（第二个）
        let (charcoal_cells, _) = &torch_candidates[1];
        assert!(charcoal_cells.contains(&(1, "charcoal")), "charcoal 变体 slot1 应为 charcoal");
        assert!(charcoal_cells.contains(&(3, "stick")), "charcoal 变体 slot3 应为 stick");

        // 带 minecraft: 前缀
        assert_eq!(lookup_shaped_2x2("minecraft:stick").len(), 1);
        assert_eq!(lookup_shaped_2x2("minecraft:torch").len(), 2);

        // 不在表中的物品应返回空（回退到顺序填充）
        assert!(lookup_shaped_2x2("oak_planks").is_empty(), "oak_planks 无形状配方，应回退顺序填充");
        assert!(lookup_shaped_2x2("crafting_table").is_empty(), "crafting_table 无形状配方");
    }
}
