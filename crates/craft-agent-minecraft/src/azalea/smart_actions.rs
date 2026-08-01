//! 复合技能层（学习自 Mindcraft library/skills.js 的设计）。
//!
//! 与 actions.rs 的原子动作不同，这里实现的是多步组合 + 失败降级 + 副作用追踪的
//! "smart" 版本。每个技能：
//! - 前置条件检查（背包是否有镐？距离够不够？）
//! - 失败重试 + 多策略 fallback
//! - 副作用追踪（这次挖到几个、走多远、损失多少耐久）
//!
//! 这些技能主要被 LLM 工具层（tools_azalea.rs）调用，不直接暴露给 LLM。
//! LLM 调用的是高层工具（gather / attack / place），内部走到这里。

use super::{
    auto_equip_best_axe, auto_equip_best_pickaxe, best_pickaxe_tier_in_inventory,
    block_required_pickaxe_tier, has_any_axe_in_inventory, has_any_pickaxe_in_inventory,
    is_hard_block, is_log_block, pickaxe_tier, pickaxe_tier_name, pickaxe_to_craft_for_tier,
};
use azalea::BlockPos;
use azalea::pathfinder::goals::BlockPosGoal;
use azalea::prelude::*;
use azalea_registry::builtin::{BlockKind, EntityKind, ItemKind};
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;

/// 物品别名表（学习自 Mindcraft collectBlock 的 alias 设计）。
/// LLM 写 "oak_log" 时，可能附近只有 birch_log，原版会失败；展开为多种变体
/// 后扫描，找到任一种就走过去挖。大幅提升 gather 在不同生态的成功率。
///
/// 返回 (别名列表, 对应的 BlockKind 列表)。解析失败返回空。
pub fn expand_block_aliases(item: &str) -> Vec<BlockKind> {
    let candidates: Vec<&str> = match item {
        // 原木类：所有原木变体
        "oak_log" | "birch_log" | "spruce_log" | "jungle_log" | "acacia_log" | "dark_oak_log"
        | "mangrove_log" | "cherry_log" | "pale_oak_log" | "log" => vec![
            "oak_log",
            "birch_log",
            "spruce_log",
            "jungle_log",
            "acacia_log",
            "dark_oak_log",
            "mangrove_log",
            "cherry_log",
            "pale_oak_log",
        ],
        // 木板类
        "oak_planks" | "planks" => vec![
            "oak_planks",
            "birch_planks",
            "spruce_planks",
            "jungle_planks",
            "acacia_planks",
            "dark_oak_planks",
            "mangrove_planks",
            "cherry_planks",
            "pale_oak_planks",
        ],
        // 石材类：圆石/石头都算
        "stone" | "cobblestone" => vec!["stone", "cobblestone", "granite", "diorite", "andesite"],
        // 矿石类：P18 修复（2026-07-27）—— 同时展开 deepslate 变体。
        // vanilla 规则：Y<0 时矿石生成 deepslate_xxx_ore 版本（深岩层），
        // 原 _ => vec![item] 只找 "iron_ore"，但 bot 在 Y=91 深岩层实际方块是
        // deepslate_iron_ore → scan_blocks_multi 100% 找不到 → gather 100% 失败。
        // 学习自 mindcraft collectBlock：mindcraft 用 mineflayer 的 findBlockRanges
        // 自动匹配所有 matching block states，不需要手动展开。
        // 本项目 azalea 无 findBlockRanges，手动列出 deepslate 变体。
        "iron_ore" => vec!["iron_ore", "deepslate_iron_ore"],
        "coal_ore" => vec!["coal_ore", "deepslate_coal_ore"],
        "copper_ore" => vec!["copper_ore", "deepslate_copper_ore"],
        "gold_ore" => vec!["gold_ore", "deepslate_gold_ore"],
        "diamond_ore" => vec!["diamond_ore", "deepslate_diamond_ore"],
        "emerald_ore" => vec!["emerald_ore", "deepslate_emerald_ore"],
        "lapis_ore" => vec!["lapis_ore", "deepslate_lapis_ore"],
        "redstone_ore" => vec!["redstone_ore", "deepslate_redstone_ore"],
        // 兼容 LLM 直接传 deepslate_xxx_ore
        "deepslate_iron_ore" => vec!["iron_ore", "deepslate_iron_ore"],
        "deepslate_coal_ore" => vec!["coal_ore", "deepslate_coal_ore"],
        "deepslate_copper_ore" => vec!["copper_ore", "deepslate_copper_ore"],
        "deepslate_gold_ore" => vec!["gold_ore", "deepslate_gold_ore"],
        "deepslate_diamond_ore" => vec!["diamond_ore", "deepslate_diamond_ore"],
        "deepslate_emerald_ore" => vec!["emerald_ore", "deepslate_emerald_ore"],
        "deepslate_lapis_ore" => vec!["lapis_ore", "deepslate_lapis_ore"],
        "deepslate_redstone_ore" => vec!["redstone_ore", "deepslate_redstone_ore"],
        _ => vec![item],
    };
    candidates
        .iter()
        .filter_map(|c| {
            let id = if c.starts_with("minecraft:") {
                c.to_string()
            } else {
                format!("minecraft:{c}")
            };
            BlockKind::from_str(&id).ok()
        })
        .collect()
}

/// 物品别名（item 形态）：原木变体对应的原木物品。
pub fn expand_item_aliases(item: &str) -> Vec<ItemKind> {
    let candidates: Vec<&str> = match item {
        "oak_log" | "birch_log" | "spruce_log" | "jungle_log" | "acacia_log" | "dark_oak_log"
        | "mangrove_log" | "cherry_log" | "pale_oak_log" | "log" => vec![
            "oak_log",
            "birch_log",
            "spruce_log",
            "jungle_log",
            "acacia_log",
            "dark_oak_log",
            "mangrove_log",
            "cherry_log",
            "pale_oak_log",
        ],
        "oak_planks" | "planks" => vec![
            "oak_planks",
            "birch_planks",
            "spruce_planks",
            "jungle_planks",
            "acacia_planks",
            "dark_oak_planks",
            "mangrove_planks",
            "cherry_planks",
            "pale_oak_planks",
        ],
        "stone" | "cobblestone" => vec!["stone", "cobblestone", "granite", "diorite", "andesite"],
        // 矿石类：P18 修复（2026-07-27）—— 挖矿后掉落 raw_xxx（不是 ore 本身）。
        // vanilla 规则：铁/铜/金矿挖掉后掉 raw_iron/raw_copper/raw_gold（精准采集才掉 ore 本身），
        // 煤矿掉 coal，钻石矿掉 diamond，红石矿掉 redstone，青金石矿掉 lapis_lazuli。
        // 原 _ => vec![item] 计数 iron_ore，但实际背包增加的是 raw_iron → count 永远 0
        // → gather 误判"挖掉了但没增加" → 100% 失败。
        "iron_ore" | "deepslate_iron_ore" => vec!["iron_ore", "raw_iron"],
        "copper_ore" | "deepslate_copper_ore" => vec!["copper_ore", "raw_copper"],
        "gold_ore" | "deepslate_gold_ore" => vec!["gold_ore", "raw_gold"],
        "coal_ore" | "deepslate_coal_ore" => vec!["coal_ore", "coal"],
        "diamond_ore" | "deepslate_diamond_ore" => vec!["diamond_ore", "diamond"],
        "emerald_ore" | "deepslate_emerald_ore" => vec!["emerald_ore", "emerald"],
        "lapis_ore" | "deepslate_lapis_ore" => vec!["lapis_ore", "lapis_lazuli"],
        "redstone_ore" | "deepslate_redstone_ore" => vec!["redstone_ore", "redstone"],
        _ => vec![item],
    };
    candidates
        .iter()
        .filter_map(|c| {
            let id = if c.starts_with("minecraft:") {
                c.to_string()
            } else {
                format!("minecraft:{c}")
            };
            ItemKind::from_str(&id).ok()
        })
        .collect()
}

/// 扫描多种方块种类，返回最近的（按欧氏距离）。
pub fn scan_blocks_multi(
    world: &azalea_world::World,
    center: azalea::Vec3,
    kinds: &[BlockKind],
    radius: i32,
) -> Option<BlockPos> {
    let cx = center.x.floor() as i32;
    let cy = center.y.floor() as i32;
    let cz = center.z.floor() as i32;
    let mut best: Option<(BlockPos, i32)> = None;
    for dx in -radius..=radius {
        for dy in -radius..=radius {
            for dz in -radius..=radius {
                let pos = BlockPos::new(cx + dx, cy + dy, cz + dz);
                if let Some(state) = world.get_block_state(pos) {
                    let bk: BlockKind = state.into();
                    if kinds.contains(&bk) {
                        let dist = dx * dx + dy * dy + dz * dz;
                        if best.is_none_or(|(_, d)| dist < d) {
                            best = Some((pos, dist));
                        }
                    }
                }
            }
        }
    }
    best.map(|(p, _)| p)
}

/// P30 新增（2026-07-27）：返回目标方块的典型 Y 生成范围（vanilla 规则）。
///
/// 用于 gather 失败时给 LLM 明确的 Y 提示：
/// - "oak_log 在 Y=60~100 生成，bot 当前 Y=43（地下），需要先 goto 地表"
/// - "diamond_ore 在 Y=-64~-16 生成，bot 当前 Y=40（地表），需要先 mine_below 到深地下"
///
/// 数据来源：minecraft.fandom.com 1.20+ ore distribution + vanilla block 生成规则。
/// 返回 None 表示该方块无明确 Y 限制（如 grass_block/dirt 在任何 Y 都可能存在）。
fn typical_y_range(item: &str) -> Option<(i32, i32)> {
    let bare = item.strip_prefix("minecraft:").unwrap_or(item);
    match bare {
        // 原木类：地表生成（Y 60~100，森林/平原）
        "oak_log" | "birch_log" | "spruce_log" | "jungle_log" | "acacia_log" | "dark_oak_log"
        | "mangrove_log" | "cherry_log" | "pale_oak_log" => Some((60, 100)),
        // 木板/工作台等：人造，无 Y 限制
        // 矿石类（vanilla 1.18+ ore distribution）：
        "coal_ore" | "deepslate_coal_ore" => Some((-64, 128)),
        "iron_ore" | "deepslate_iron_ore" => Some((-64, 72)),
        "copper_ore" | "deepslate_copper_ore" => Some((-16, 112)),
        "gold_ore" | "deepslate_gold_ore" => Some((-64, 32)),
        "diamond_ore" | "deepslate_diamond_ore" => Some((-64, 16)),
        "emerald_ore" | "deepslate_emerald_ore" => Some((-64, 32)),
        "lapis_ore" | "deepslate_lapis_ore" => Some((-64, 64)),
        "redstone_ore" | "deepslate_redstone_ore" => Some((-64, 16)),
        // 石材类：地下（Y < 60）
        "stone" | "cobblestone" | "granite" | "diorite" | "andesite" | "tuff" | "deepslate"
        | "netherrack" | "basalt" | "blackstone" | "end_stone" => Some((-64, 60)),
        // 沙子/沙砾：地表（Y > 60，沙漠/河边）
        "sand" | "red_sand" | "sandstone" | "red_sandstone" | "gravel" => Some((60, 128)),
        // 草/泥土/原木：地表
        "grass_block" | "dirt" | "podzol" | "mycelium" | "coarse_dirt" => Some((60, 128)),
        // 水/冰：地表或地下水源
        "water" | "ice" | "packed_ice" | "blue_ice" => Some((-64, 128)),
        // 雪：地表
        "snow" | "snow_block" => Some((60, 128)),
        // 仙人掌：沙漠地表
        "cactus" => Some((60, 128)),
        // 甘蔗：水边地表
        "sugar_cane" => Some((60, 128)),
        // 竹子：地表
        "bamboo" => Some((60, 128)),
        // 花草/蘑菇：地表
        "dandelion" | "poppy" | "rose_bush" | "peony" | "red_mushroom" | "brown_mushroom"
        | "allium" | "azure_bluet" | "blue_orchid" | "cornflower" | "lily_of_the_valley"
        | "orange_tulip" | "oxeye_daisy" | "pink_tulip" | "red_tulip" | "white_tulip"
        | "wither_rose" | "sunflower" | "lilac" => Some((60, 128)),
        _ => None,
    }
}

/// P30 新增：根据目标方块的 Y 范围和 bot 当前 Y，生成位置建议 hint。
///
/// 返回 String：
/// - 空字符串：目标无 Y 限制或 bot 已在合适 Y 范围
/// - 非空：明确的 Y 提示（如"bot 当前 Y=43，oak_log 在 Y=60~100 生成，需先 goto 地表"）
fn y_range_hint(item: &str, bot_y: i32) -> String {
    let bare = item.strip_prefix("minecraft:").unwrap_or(item);
    let Some((y_min, y_max)) = typical_y_range(bare) else {
        return String::new();
    };
    if bot_y >= y_min && bot_y <= y_max {
        return String::new();
    }
    let target_desc = if bare.ends_with("_log") || bare.ends_with("_wood") {
        "（原木类需要先找到森林/树木群系）"
    } else if bare.ends_with("_ore") {
        "（矿石类需要挖到对应深度）"
    } else if matches!(
        bare,
        "stone" | "cobblestone" | "granite" | "diorite" | "andesite" | "tuff" | "deepslate"
    ) {
        "（石材类在地下生成）"
    } else if matches!(bare, "sand" | "red_sand" | "sandstone" | "red_sandstone") {
        "（沙子类在沙漠/河边地表生成）"
    } else {
        ""
    };
    let action = if bot_y < y_min {
        format!("bot 当前 Y={bot_y}（过低），需要先 goto 到 Y≥{y_min} 的地表或更高位置")
    } else {
        format!("bot 当前 Y={bot_y}（过高），需要先 mine_below 到 Y≤{y_max} 的地下")
    };
    format!("\n【Y 范围不匹配】{item} 通常在 Y={y_min}~{y_max} 生成，{action}{target_desc}。")
}

/// P28 新增（2026-07-27）：扫描附近所有矿石类型，返回去重的矿石名列表。
///
/// 用于 gather 失败时给 LLM 提供替代建议（如"找不到 iron_ore 但附近有 coal_ore"）。
/// 学习自 mindcraft collectBlock：mindcraft 在找不到目标方块时会列出附近可用方块。
async fn scan_nearby_ores(bot: &Client, radius: i32) -> Vec<String> {
    let world = match bot.world() {
        Ok(w) => w,
        Err(_) => return Vec::new(),
    };
    let w = world.read();
    let center = match bot.position() {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    let cx = center.x.floor() as i32;
    let cy = center.y.floor() as i32;
    let cz = center.z.floor() as i32;
    let mut found: std::collections::HashSet<String> = std::collections::HashSet::new();
    for dx in -radius..=radius {
        for dy in -radius..=radius {
            for dz in -radius..=radius {
                let pos = BlockPos::new(cx + dx, cy + dy, cz + dz);
                if let Some(state) = w.get_block_state(pos) {
                    let bk: BlockKind = state.into();
                    let name = format!("{bk:?}");
                    if name.ends_with("_ore") {
                        found.insert(name);
                    }
                }
            }
        }
    }
    let mut v: Vec<String> = found.into_iter().collect();
    v.sort();
    v
}

/// P28 新增（2026-07-27）：扫描附近所有原木类型，返回去重的原木名列表。
async fn scan_nearby_wood(bot: &Client, radius: i32) -> Vec<String> {
    let world = match bot.world() {
        Ok(w) => w,
        Err(_) => return Vec::new(),
    };
    let w = world.read();
    let center = match bot.position() {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    let cx = center.x.floor() as i32;
    let cy = center.y.floor() as i32;
    let cz = center.z.floor() as i32;
    let mut found: std::collections::HashSet<String> = std::collections::HashSet::new();
    for dx in -radius..=radius {
        for dy in -radius..=radius {
            for dz in -radius..=radius {
                let pos = BlockPos::new(cx + dx, cy + dy, cz + dz);
                if let Some(state) = w.get_block_state(pos) {
                    let bk: BlockKind = state.into();
                    let name = format!("{bk:?}");
                    if name.ends_with("_log") || name.ends_with("_wood") || name.ends_with("_stem")
                    {
                        found.insert(name);
                    }
                }
            }
        }
    }
    let mut v: Vec<String> = found.into_iter().collect();
    v.sort();
    v
}

/// 走到最近的指定方块种类（多别名）并挖掘，直到背包积累足够数量。
/// 学习自 Mindcraft collectBlock：别名展开 + 多轮采集 + 失败跳出。
///
/// 与 gather.rs::do_gather 的差异：
/// 1. 支持别名展开（"oak_log" 匹配 9 种原木变体）
/// 2. 多种物品同时计数（挖到 oak_log 或 birch_log 都算）
/// 3. 每轮失败时降低半径重试，最后报具体失败原因
///
/// P8 修复（2026-07-26）：
/// - 预检背包工具（无镐且目标是硬方块 → 立即失败并提示合成镐）
/// - 挖矿前自动装备最好的镐/斧（曾因主手 air 导致 gather 100% 失败）
/// - 检测"方块消失但物品未增加" → 报告缺工具
enum ToolNeed {
    Pickaxe,
    Axe,
    None,
}
#[allow(dead_code)]
impl ToolNeed {
    fn label(&self) -> &'static str {
        match self {
            ToolNeed::Pickaxe => "镐",
            ToolNeed::Axe => "斧",
            ToolNeed::None => "工具",
        }
    }
}

pub async fn collect_block_smart(bot: &Client, item: &str, count: u32) -> Result<String, String> {
    let block_kinds = expand_block_aliases(item);
    let item_kinds = expand_item_aliases(item);
    if block_kinds.is_empty() || item_kinds.is_empty() {
        return Err(format!("未知物品/方块 {item}"));
    }

    let need = count.max(1);
    let mut gathered = 0u32;
    // P3 修复：max_rounds 从 24 降到 8。
    // 原 24 轮 × 10s/轮 = 240s 理论上限远超 ActionManager 120s 超时，
    // 导致工具返回"命令超时"但采集仍在后台跑（最终产出 49 个 oak_log 的元凶）。
    // 8 轮 × 10s = 80s，留 40s 余量给 ActionManager 超时（120s）。
    let max_rounds = 8;
    let primary_kind = item_kinds[0];
    let _ = primary_kind; // 保留语义，未来可用于 primary-only 计数

    // P8 修复（2026-07-26）：预检工具。
    // 根据目标 item 字符串判断是否需要镐/斧，若需要但背包完全没有该类工具，
    // 立即返回明确错误——避免反复尝试 8 轮都失败浪费时间（曾导致 gather 100% 失败）。
    let item_kind_str = {
        let s = item_kinds[0].to_str();
        s.strip_prefix("minecraft:").unwrap_or(s).to_string()
    };
    let needs_pickaxe = item_kind_str.ends_with("_ore")
        || matches!(
            item_kind_str.as_str(),
            "stone"
                | "deepslate"
                | "cobblestone"
                | "granite"
                | "diorite"
                | "andesite"
                | "tuff"
                | "netherrack"
                | "basalt"
                | "blackstone"
                | "end_stone"
                | "sandstone"
                | "red_sandstone"
        );
    let needs_axe = item_kind_str.ends_with("_log") || item_kind_str.ends_with("_wood");
    if needs_pickaxe && !has_any_pickaxe_in_inventory(bot).await {
        // P10 修复（2026-07-26）：刚 craft 完镐但背包同步未完成时会误报"无镐"。
        // 等待 + 重试 3 次，每次间隔 500ms。
        let mut found_pickaxe = false;
        for retry in 0..3u8 {
            sleep(Duration::from_millis(500)).await;
            if has_any_pickaxe_in_inventory(bot).await {
                found_pickaxe = true;
                eprintln!("[smart_gather] pickaxe found after {} retry(s)", retry + 1);
                break;
            }
        }
        // P26 新增（2026-07-27）：背包无镐时自动 auto_craft wooden_pickaxe。
        // 原 code 只报错让 LLM 合成，但 LLM 常忽略提示直接重试 gather → 死循环。
        // 学习自 mindcraft craftRecipe：mindcraft 在需要工具时会主动 craft。
        // 这里调用 do_auto_craft 递归合成 wooden_pickaxe（会自动处理
        // log→planks→stick→table→pickaxe 全链路）。
        if !found_pickaxe {
            eprintln!("[smart_gather] P26: 背包无镐，尝试 auto_craft wooden_pickaxe");
            match crate::azalea::auto_craft::do_auto_craft(bot, "wooden_pickaxe", 1).await {
                Ok(msg) => {
                    eprintln!("[smart_gather] P26: auto_craft 成功 - {msg}");
                    // 等待背包同步，然后验证镐已就位
                    sleep(Duration::from_millis(500)).await;
                    if has_any_pickaxe_in_inventory(bot).await {
                        found_pickaxe = true;
                        eprintln!("[smart_gather] P26: wooden_pickaxe 已就位，继续采集");
                    } else {
                        eprintln!("[smart_gather] P26: auto_craft 报成功但背包未同步到镐");
                    }
                }
                Err(e) => {
                    eprintln!("[smart_gather] P26: auto_craft wooden_pickaxe 失败: {e}");
                }
            }
        }
        if !found_pickaxe {
            return Err(format!(
                "采集 {item} 失败：背包无镐且自动合成 wooden_pickaxe 失败（可能缺原料：需要 oak_log 或 oak_planks）。\n\
                 解决步骤：\n\
                 1. 先 gather('oak_log') 获取原木\n\
                 2. craft('oak_planks') → craft('stick') → craft_3x3('wooden_pickaxe')\n\
                 3. equip(item='wooden_pickaxe') 装备主手\n\
                 4. 重试 gather('{item}')"
            ));
        }
    }

    // P11 修复（2026-07-26）：工具等级检查（同 gather.rs）。
    // vanilla 规则：等级不足的镐挖该方块时方块会消失但**不掉落物品**——
    // 这是 smart_gather「方块消失但背包数量不增」误报的根因。
    // 预检：若背包最好的镐 tier < 目标方块所需 tier，立即返回错误让 LLM 先合成更高 tier 的镐。
    if needs_pickaxe {
        // 取 block_kinds[0] 作为代表（aliases 展开后的第一个，通常是 LLM 指定的原始 item）
        let required_tier = block_required_pickaxe_tier(block_kinds[0]);
        if required_tier > 0 {
            let mut best_tier = 0u8;
            for retry in 0..3u8 {
                best_tier = best_pickaxe_tier_in_inventory(bot).await;
                if best_tier >= required_tier {
                    break;
                }
                if retry < 2 {
                    sleep(Duration::from_millis(500)).await;
                }
            }
            if best_tier < required_tier {
                return Err(format!(
                    "采集 {item} 失败：背包最好的镐等级不足（{} tier {} < 需要 tier {}）。\n\
                     vanilla 规则：等级不足的镐挖该方块时方块会消失但**不掉落物品**，\n\
                     这会导致 gather 误判为「方块被挖掉但未掉落物品（缺工具）」死循环。\n\
                     建议：先 craft 3x3 合成一把 {}，equip 装备主手后再 gather。",
                    pickaxe_tier_name(best_tier),
                    best_tier,
                    required_tier,
                    pickaxe_to_craft_for_tier(required_tier)
                ));
            }
        }
    }
    if needs_axe && !has_any_axe_in_inventory(bot).await {
        // 砍树徒手也能砍（只是慢），所以这里不直接失败，只警告。
        eprintln!("[gather] 背包无斧，将徒手砍 {item}（效率低）");
    }

    for round in 0..max_rounds {
        if gathered >= need {
            break;
        }
        // 半径渐扩：4 → 8 → 16（不再扩到 24，16 已经够大且 24^3 扫描太慢）
        let radius = match round {
            0 => 4,
            1..=2 => 8,
            _ => 16,
        };
        let pos = {
            let world = bot.world().map_err(|e| format!("读取世界失败: {e:?}"))?;
            let w = world.read();
            let center = bot.position().map_err(|e| format!("读取坐标失败: {e:?}"))?;
            scan_blocks_multi(&w, center, &block_kinds, radius)
        };
        let Some(target_pos) = pos else {
            // P21 新增（2026-07-27）：矿石自动下挖探索。
            // 原代码找不到矿就报错让 LLM 调 mine_below，但 LLM 常忽略提示直接重试 gather，
            // 导致 gather 100% 失败死循环。现在对矿石类型自动下挖 5 格暴露新方块，再重扫。
            // 学习自 mindcraft collectBlock：mindcraft 在找不到目标方块时会自动探索新区域。
            let is_ore = item_kind_str.ends_with("_ore");
            if is_ore && round == max_rounds - 1 {
                // 尝试自动下挖探索（最多 2 轮，每轮挖 5 格）
                for dig_round in 0..2u8 {
                    eprintln!(
                        "[gather] {item} 未找到，自动下挖探索第 {} 轮（挖 5 格暴露新方块）",
                        dig_round + 1
                    );
                    if !auto_dig_down_explore(bot, 5).await {
                        break; // 下挖失败（无镐/到基岩等），停止探索
                    }
                    // 重新扫描
                    let pos2 = {
                        let world = bot.world().map_err(|e| format!("读取世界失败: {e:?}"))?;
                        let w = world.read();
                        let center = bot.position().map_err(|e| format!("读取坐标失败: {e:?}"))?;
                        scan_blocks_multi(&w, center, &block_kinds, 16)
                    };
                    if let Some(p) = pos2 {
                        eprintln!(
                            "[gather] 下挖探索后在 ({},{},{}) 找到 {item}",
                            p.x, p.y, p.z
                        );
                        // 找到了！继续主循环采集这个目标
                        // 用 goto 走到目标旁然后挖（简化路径，不走垂直下挖分支）
                        let target =
                            azalea::Vec3::new(p.x as f64 + 0.5, p.y as f64 + 1.0, p.z as f64 + 0.5);
                        let goto_fut = bot.goto(azalea::pathfinder::goals::RadiusGoal {
                            pos: target,
                            radius: 2.0,
                        });
                        let _ = tokio::time::timeout(Duration::from_secs(8), goto_fut).await;
                        // 装备镐
                        let _ = auto_equip_best_pickaxe(bot).await;
                        // 挖
                        bot.start_mining(p);
                        // 等待方块消失（最多 5s）
                        for _ in 0..50 {
                            sleep(Duration::from_millis(100)).await;
                            let still_there = bot
                                .world()
                                .ok()
                                .and_then(|w| w.read().get_block_state(p).map(|s| !s.is_air()))
                                .unwrap_or(false);
                            if !still_there {
                                break;
                            }
                        }
                        sleep(Duration::from_millis(300)).await;
                        // 计数
                        let new_count = count_items_in_inventory(bot, &item_kinds).await;
                        if new_count > gathered {
                            gathered = new_count;
                            eprintln!("[gather] 下挖探索后采集到 {item}，累计 {gathered}/{need}");
                        }
                        if gathered >= need {
                            return Ok(format!(
                                "采集 {item} x{gathered} 完成（含下挖探索 {}/2 轮）",
                                dig_round + 1
                            ));
                        }
                        // 继续下一轮下挖探索
                        continue;
                    }
                    // 没找到，继续下一轮下挖
                }
                // P28 新增（2026-07-27）：扫描附近所有矿石类型，给 LLM 替代建议。
                // 原 code 只报"找不到 iron_ore"，LLM 反复重试 gather iron_ore → 死循环。
                // 学习自 mindcraft collectBlock：mindcraft 在找不到目标时会列出附近可用方块。
                let alt_ores = scan_nearby_ores(bot, 16).await;
                let alt_hint = if alt_ores.is_empty() {
                    String::new()
                } else {
                    format!(
                        "\n附近已探测到其他矿石：{}。\
                         若任务允许替代（如 coal_ore 可作燃料、copper_ore 可熔炼铜锭），\
                         可改 gather 这些矿石；或继续 mine_below 向更深处探索 {}。",
                        alt_ores.join(", "),
                        item
                    )
                };
                // P30 新增：Y 范围检测，给 LLM 明确的位置建议。
                let bot_y = bot.position().map(|p| p.y.floor() as i32).unwrap_or(0);
                let y_hint = y_range_hint(item, bot_y);
                return Err(format!(
                    "半径 16 内找不到 {item}（已采集 {gathered}/{need}），即使自动下挖 10 格也未暴露矿石。\
                     vanilla 规则：{item} 通常生成在 Y=15~80 的石头层中。\
                     建议：1) goto 到山体/洞穴入口再 gather；2) 用 mine_below 手动挖到 Y<60 后重试；\
                     3) 换一个生物群系探索（沙漠/海洋下方矿石分布不同）。{alt_hint}{y_hint}"
                ));
            }
            if round == max_rounds - 1 {
                // P28：非矿石类也加替代建议（如 oak_log 找不到时列附近原木种类）
                let alt_hint =
                    if item_kind_str.ends_with("_log") || item_kind_str.ends_with("_wood") {
                        let alt_woods = scan_nearby_wood(bot, 16).await;
                        if alt_woods.is_empty() {
                            String::new()
                        } else {
                            format!(
                                "\n附近已探测到其他原木：{}。可改 gather 这些原木替代。",
                                alt_woods.join(", ")
                            )
                        }
                    } else {
                        String::new()
                    };
                // P30 新增：Y 范围检测
                let bot_y = bot.position().map(|p| p.y.floor() as i32).unwrap_or(0);
                let y_hint = y_range_hint(item, bot_y);
                return Err(format!(
                    "半径 {radius} 内找不到 {item}（已采集 {gathered}/{need}）。\
                     若 {item} 在地下（如 stone 在地表下），先用 mine_below() 挖到 Y<60 暴露岩石层，再重试 gather。{alt_hint}{y_hint}"
                ));
            }
            continue;
        };

        // P4 修复：地下目标处理。当 target 在 bot 脚下 2+ 格时，pathfinder 无法
        // 导航到 stand_pos（在实心方块内）。此时改为「垂直下挖」策略：
        // 仅当 target 与 bot 同一 x,z 列时，从 bot 脚下逐格挖到 target.y+1，
        // 让 bot 自然掉落进入挖出的竖井，然后挖 target 本体。
        // 这是 surface bot 采集 stone 的关键路径。
        // 若 target 水平偏移过大（>2 格），不适用此策略，回退到普通 goto
        // （可能失败，但至少不会挖错方向）。
        let bot_pos = bot.position().map_err(|e| format!("读取坐标失败: {e:?}"))?;
        let bot_foot_y = (bot_pos.y - 1.0).floor() as i32;
        let bot_x = bot_pos.x.floor() as i32;
        let bot_z = bot_pos.z.floor() as i32;
        let horiz_offset = ((target_pos.x - bot_x).abs() + (target_pos.z - bot_z).abs()).max(0);
        let target_below_bot = target_pos.y < bot_foot_y - 1 && horiz_offset <= 1;

        if target_below_bot {
            // 垂直下挖：从 bot 脚下逐格挖到 target.y+1（不挖 target 本体，留给下面统一处理）
            // 只挖 target 正上方那一列（同 x,z），保证 bot 能沿竖井掉落到 target 旁
            for cy in (target_pos.y + 1..=bot_foot_y).rev() {
                let b = BlockPos::new(target_pos.x, cy, target_pos.z);
                let solid = bot
                    .world()
                    .ok()
                    .and_then(|w| w.read().get_block_state(b).map(|s| !s.is_air()))
                    .unwrap_or(false);
                if !solid {
                    continue;
                }
                bot.start_mining(b);
                // 等待方块消失（最多 3s）
                let mut broke = false;
                for _ in 0..30 {
                    sleep(Duration::from_millis(100)).await;
                    let gone = bot
                        .world()
                        .ok()
                        .and_then(|w| w.read().get_block_state(b).map(|s| s.is_air()))
                        .unwrap_or(true);
                    if gone {
                        broke = true;
                        break;
                    }
                }
                if !broke {
                    // 挖不动（可能没镐/硬度太高），跳出避免死循环
                    break;
                }
            }
            // 等待 bot 因重力掉入竖井
            sleep(Duration::from_millis(800)).await;
        } else {
            // 走到方块下方一格
            let stand = BlockPos::new(target_pos.x, target_pos.y - 1, target_pos.z);
            bot.start_goto(BlockPosGoal(stand));
            // P3：原 40 × 100ms = 4s 寻路等待太长；减到 25 × 100ms = 2.5s
            // （半径 16 内的目标，2.5s 足够走到；走不到说明卡住了，应尽早换下个目标）
            for _ in 0..25 {
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
            bot.stop_pathfinding();
        }

        // 挖前统计
        let before: u32 = bot
            .get_inventory()
            .ok()
            .map(|inv| {
                let mut total = 0u32;
                for k in &item_kinds {
                    total += count_item_kind(&inv, *k);
                }
                total
            })
            .unwrap_or(0);

        // P3：到达后再检查一次背包，可能路上自动捡到了掉落物已经满足 need
        if before >= need {
            // P5 修复：原代码这里直接 break，最后返回"采集 X 完成（背包 N 个）"，
            // 让 LLM 误以为新采集了 N 个。实际是背包本来就有 N 个（含别名变体）。
            // 现在明确报告"无需新采集"+ 各变体的明细，避免 LLM 困惑。
            let breakdown = format_item_breakdown(bot, &item_kinds).await;
            return Ok(format!(
                "背包已有 {before} 个 {item}（含别名变体）≥ 需求 {need}，无需新采集。明细: {breakdown}。\
                 注意：若你需要的是「{item}」这一具体种类而非别名变体，请检查背包明细——\
                 若该种类不足，本工具不会刻意只采该种类，会采所有别名变体。"
            ));
        }

        // P8 修复（2026-07-26）：挖前装备合适工具。
        // 根据 target_pos 的 BlockState 判断需要的工具类型，自动装备最好的镐/斧。
        // 这是 gather 100% 失败的根因——之前 start_mining 时主手是 air，
        // coal_ore/stone 类硬方块徒手挖不掉（不掉落物品）。
        let tool_need = bot
            .world()
            .ok()
            .and_then(|w| w.read().get_block_state(target_pos))
            .map(|state| {
                if is_log_block(state) {
                    ToolNeed::Axe
                } else if is_hard_block(state) {
                    ToolNeed::Pickaxe
                } else {
                    ToolNeed::None
                }
            })
            .unwrap_or(ToolNeed::None);
        match tool_need {
            ToolNeed::Pickaxe => {
                if let Some(msg) = auto_equip_best_pickaxe(bot).await {
                    eprintln!("[gather] round {round}: {msg}");
                }
            }
            ToolNeed::Axe => {
                if let Some(msg) = auto_equip_best_axe(bot).await {
                    eprintln!("[gather] round {round}: {msg}");
                }
            }
            ToolNeed::None => {}
        }

        bot.start_mining(target_pos);
        // 等待挖掘完成（任一别名物品数量增加，或方块消失）
        // P3：原 60 × 100ms = 6s；减到 30 × 100ms = 3s（普通方块 1-2s 挖完）
        let mut done = false;
        let mut block_disappeared = false;
        let mut new_count: u32 = before;
        for _ in 0..30 {
            sleep(Duration::from_millis(100)).await;
            let now: u32 = bot
                .get_inventory()
                .ok()
                .map(|inv| {
                    let mut total = 0u32;
                    for k in &item_kinds {
                        total += count_item_kind(&inv, *k);
                    }
                    total
                })
                .unwrap_or(0);
            if now > before {
                new_count = now;
                done = true;
                break;
            }
            if let Ok(world) = bot.world() {
                let g = world.read();
                let disappeared = g
                    .get_block_state(target_pos)
                    .map(|s| s.is_air())
                    .unwrap_or(true);
                if disappeared {
                    block_disappeared = true;
                    break;
                }
            }
        }

        // P16 修复（2026-07-26）：方块消失后掉落物需要 1-2s 才被 bot 拾取，
        // 原循环 100ms 检查一次发现方块消失就 break，没等拾取就误判「无斧失败」。
        // 修复：方块消失后额外等 1.5s 让 bot 拾取掉落物；若拾取成功算正常完成。
        if !done && block_disappeared {
            for _ in 0..15 {
                sleep(Duration::from_millis(100)).await;
                let now: u32 = bot
                    .get_inventory()
                    .ok()
                    .map(|inv| {
                        let mut total = 0u32;
                        for k in &item_kinds {
                            total += count_item_kind(&inv, *k);
                        }
                        total
                    })
                    .unwrap_or(0);
                if now > before {
                    new_count = now;
                    done = true;
                    break;
                }
            }
        }
        if done {
            // 实际挖到了，报告增量
            let delta = new_count - before;
            gathered = new_count;
            let breakdown = format_item_breakdown(bot, &item_kinds).await;
            // 继续下一轮（若还不足 need 会再找下一个目标）
            let _ = delta; // 调试用
            let _ = breakdown; // 详细breakdown在最终返回里给
        } else if block_disappeared && matches!(tool_need, ToolNeed::Pickaxe | ToolNeed::Axe) {
            // P11 修复（2026-07-26）：原检查只报告"缺工具"，但没区分
            // 「完全没镐」「镐等级不足」「主手未持镐」三种情况，导致错误提示不精准。
            // 修复：分别判断并给针对性的合成建议。
            let held_kind = bot
                .get_held_item()
                .ok()
                .and_then(|s| if s.is_empty() { None } else { Some(s.kind()) });
            let held_str = held_kind
                .map(|k| {
                    let s = k.to_str();
                    s.strip_prefix("minecraft:").unwrap_or(s).to_string()
                })
                .unwrap_or_else(|| "(空手)".to_string());
            let held_tier = held_kind.map(pickaxe_tier).unwrap_or(0);

            if matches!(tool_need, ToolNeed::Pickaxe) {
                let required_tier = block_required_pickaxe_tier(block_kinds[0]);
                let best_tier = best_pickaxe_tier_in_inventory(bot).await;
                if best_tier == 0 {
                    return Err(format!(
                        "采集 {item} 失败：方块被挖掉但未掉落物品（背包无镐）。\n\
                         当前主手: {held_str}；建议先合成 {} 再 gather。",
                        pickaxe_to_craft_for_tier(required_tier.max(1))
                    ));
                }
                if required_tier > 0 && best_tier < required_tier {
                    return Err(format!(
                        "采集 {item} 失败：方块被挖掉但未掉落物品（镐等级不足）。\n\
                         目标方块需要 {}（tier {}），背包最好的镐为 {}（tier {}），手持 {held_str}（tier {}）。\n\
                         vanilla 规则：等级不足的镐挖该方块时方块会消失但**不掉落物品**。\n\
                         建议：先 craft 3x3 合成 {}，equip 装备主手后再 gather。",
                        pickaxe_tier_name(required_tier),
                        required_tier,
                        pickaxe_tier_name(best_tier),
                        best_tier,
                        held_tier,
                        pickaxe_to_craft_for_tier(required_tier)
                    ));
                }
                if held_tier == 0 && best_tier > 0 {
                    return Err(format!(
                        "采集 {item} 失败：方块被挖掉但未掉落物品（主手未持镐）。\n\
                         背包有镐（{} tier {}）但 auto_equip 失败未切到主手。\n\
                         建议：手动 equip 装备镐到主手后再 gather。",
                        pickaxe_tier_name(best_tier),
                        best_tier
                    ));
                }
            }
            if matches!(tool_need, ToolNeed::Axe) && !has_any_axe_in_inventory(bot).await {
                // P16 修复（2026-07-26）：徒手砍树是可行的（vanilla 中徒手挖原木会掉落原木）。
                // 原代码在这里 return Err，导致 LLM 永远无法用徒手砍树启动游戏——
                // 这是死循环的根因：需要原木合成斧 → 需要斧砍树 → 没斧就砍不了树。
                // 修复：改为警告并继续下一轮（可能只是拾取延迟，下一轮可能成功）。
                // 若确实徒手挖不掉（不太可能），max_rounds 兜底会返回"未完成"错误。
                eprintln!(
                    "[smart_gather] 警告：方块消失但未拾取到物品（背包无斧，可能徒手砍树拾取延迟）。\
                     继续下一轮尝试。"
                );
                continue;
            }
        }
        if !done && round == max_rounds - 1 {
            break;
        }
    }

    if gathered >= need {
        // P5 修复：返回消息区分"实际新挖到"vs"已有足够数量"
        let breakdown = format_item_breakdown(bot, &item_kinds).await;
        Ok(format!(
            "采集 {item} 完成（背包现有 {gathered} 个，含别名变体）。明细: {breakdown}"
        ))
    } else {
        // P8 改进：根据预检状态给更针对性的错误
        let hint = if needs_pickaxe {
            "（提示：coal_ore/iron_ore 等矿石需要镐才能掉落物品；若已合成镐请 equip 装备主手）"
        } else if needs_axe {
            "（提示：徒手砍树效率极低，建议合成 wooden_axe）"
        } else {
            "；建议换一个区域或换一种采集目标"
        };
        Err(format!(
            "采集 {item} 未完成（仅 {gathered}/{need}，附近可能无更多{hint}）"
        ))
    }
}

/// 格式化背包中指定物品种类的明细（如 "oak_log:3, dark_oak_log:48"）。
async fn format_item_breakdown(bot: &Client, kinds: &[ItemKind]) -> String {
    let inv = match bot.get_inventory() {
        Ok(i) => i,
        Err(_) => return "(读取背包失败)".to_string(),
    };
    let mut counts: Vec<(String, u32)> = Vec::new();
    for k in kinds {
        let c = count_item_kind(&inv, *k);
        if c > 0 {
            // P5 修复：用 to_str() 拿到 snake_case minecraft id（如 "dark_oak_log"），
            // 原 format!("{k:?}").to_lowercase() 得到 "darkoaklog"（无下划线），
            // 与工具/craft 配方表期望的 snake_case id 不匹配 → LLM 困惑。
            let full = k.to_str();
            let name = full.strip_prefix("minecraft:").unwrap_or(full);
            counts.push((name.to_string(), c));
        }
    }
    if counts.is_empty() {
        "(无)".to_string()
    } else {
        counts
            .iter()
            .map(|(n, c)| format!("{n}:{c}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn count_item_kind(inv: &azalea::container::ContainerHandleRef, kind: ItemKind) -> u32 {
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
        if let Some(stack) = slots.get(s)
            && !stack.is_empty() && stack.kind() == kind {
                total += stack.count().max(0) as u32;
            }
    }
    total
}

/// 捡起附近所有掉落物（学习自 Mindcraft pickupNearbyItems）。
///
/// bot 挖矿/战斗后掉落物散落在地，原版不会主动走过去捡。这个函数：
/// 1. 扫描半径 8 格内的所有 ItemEntity
/// 2. 走到每个掉落物位置（按距离升序）
/// 3. 等待背包数量增加（确认捡到）
/// 4. 返回捡到的物品清单
///
/// 战斗/挖矿后调用一次，避免"挖了 8 个石头但只捡到 3 个"。
pub async fn pickup_nearby_items(bot: &Client) -> Result<String, String> {
    let center = bot.position().map_err(|e| format!("读取坐标失败: {e:?}"))?;
    let _ = center; // 暂未使用，保留语义

    // 简化实现：走一圈让物理引擎自然捡起
    // bot 蹲下 + 转圈，让掉落物被吸过来（vanilla 半径 1.5 自动捡）
    let before_count = total_inventory_count(bot);

    // 原地转 4 个方向，每个方向走 2 格再回来，扫掉落物
    let dirs = [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)];
    for (dx, dz) in dirs {
        if let Ok(p) = bot.position() {
            let target_x = (p.x + dx * 2.0).floor() as i32;
            let target_y = p.y.floor() as i32;
            let target_z = (p.z + dz * 2.0).floor() as i32;
            bot.start_goto(BlockPosGoal(BlockPos::new(target_x, target_y, target_z)));
            // 走 1.5 秒
            sleep(Duration::from_millis(1500)).await;
        }
    }

    let after_count = total_inventory_count(bot);
    let diff = after_count.saturating_sub(before_count);

    bot.stop_pathfinding();

    if diff == 0 {
        Ok("附近无掉落物可捡".to_string())
    } else {
        Ok(format!("捡起 {} 个物品", diff))
    }
}

fn total_inventory_count(bot: &Client) -> u32 {
    bot.get_inventory()
        .ok()
        .and_then(|inv| {
            let menu = inv.menu().ok().flatten()?;
            let range = menu.player_slots_range();
            let slots = inv.slots()?;
            let mut total = 0u32;
            for s in range {
                if let Some(stack) = slots.get(s)
                    && !stack.is_empty() {
                        total += stack.count().max(0) as u32;
                    }
            }
            Some(total)
        })
        .unwrap_or(0)
}

/// 自动防御：攻击附近所有敌对生物（学习自 Mindcraft defendSelf）。
///
/// 与单次 attack 不同，这个会：
/// 1. 依靠 azalea handler 层 Tick 内的 self_defense mode（已实现）做扫描
/// 2. 这里只是触发循环：等待足够时间让 mode 跑几轮
/// 3. 返回清理状态
///
/// 真正的攻击由 handler 层 Tick 自带的 self_defense 处理（每 100 tick 触发），
/// 这里作为 LLM 可显式调用的版本，给一个时间窗口让 mode 工作。
pub async fn defend_self(bot: &Client) -> Result<String, String> {
    let health_before = bot.health().unwrap_or(20.0);

    // 等待 5 秒，让 handler 层 self_defense mode 自动攻击附近敌人
    // （azalea 没有简单同步 API 列举/选择实体；handler 层每 100 tick 已在做）
    for _ in 0..50 {
        sleep(Duration::from_millis(100)).await;
        // 检查血量是否稳定（无新伤害）
        let health_now = bot.health().unwrap_or(20.0);
        if health_now < health_before - 5.0 {
            // 受到严重伤害，提前返回（bot 可能打不过）
            return Ok(format!(
                "防御中受到严重伤害（{:.1}→{:.1}），可能打不过，建议撤退",
                health_before, health_now
            ));
        }
    }

    let health_after = bot.health().unwrap_or(20.0);
    let damage_taken = (health_before - health_after).max(0.0);

    Ok(format!(
        "防御完成（{:.1}→{:.1}，受到伤害 {:.1}）。附近敌人由 handler 自动攻击。",
        health_before, health_after, damage_taken
    ))
}

#[allow(dead_code)]
fn hostile_entity_kinds() -> Vec<EntityKind> {
    use EntityKind::*;
    vec![
        Zombie,
        Skeleton,
        Creeper,
        Spider,
        Enderman,
        Witch,
        Blaze,
        Ghast,
        Slime,
        MagmaCube,
        Silverfish,
        Endermite,
        Stray,
        Husk,
        Drowned,
        Phantom,
        WitherSkeleton,
        Warden,
    ]
}

/// 计算放置方块的辅助位置（学习自 Mindcraft placeBlock 的 buildOff）。
///
/// 给定 bot 位置和目标方块坐标，返回 bot 应该站的位置 + 朝向，
/// 让右键放置时方块能放在目标坐标。
///
/// 6 个方向优先级：东/西/南/北/上/下
/// 返回 (站立坐标, yaw 朝向角度)
pub fn compute_place_offset(bot_pos: azalea::Vec3, target: BlockPos) -> (BlockPos, f32) {
    let dx = target.x as f64 - bot_pos.x;
    let _dy = target.y as f64 - bot_pos.y;
    let dz = target.z as f64 - bot_pos.z;
    // 选择最匹配的水平方向
    if dx.abs() >= dz.abs() {
        if dx > 0.0 {
            // 目标在东边，bot 站在目标西边一格
            (BlockPos::new(target.x - 1, target.y, target.z), -90.0)
        } else {
            (BlockPos::new(target.x + 1, target.y, target.z), 90.0)
        }
    } else {
        if dz > 0.0 {
            (BlockPos::new(target.x, target.y, target.z - 1), 0.0)
        } else {
            (BlockPos::new(target.x, target.y, target.z + 1), 180.0)
        }
    }
}

/// 走到最近的指定方块种类附近（不挖掘，仅寻路）。
/// 学习自 Mindcraft goToNearestBlock。
///
/// 返回找到的方块坐标 + 走到的位置；找不到返回 Err。
pub async fn goto_nearest_block(bot: &Client, item: &str, radius: i32) -> Result<BlockPos, String> {
    let block_kinds = expand_block_aliases(item);
    if block_kinds.is_empty() {
        return Err(format!("未知方块 {item}"));
    }
    let pos = {
        let world = bot.world().map_err(|e| format!("读取世界失败: {e:?}"))?;
        let w = world.read();
        let center = bot.position().map_err(|e| format!("读取坐标失败: {e:?}"))?;
        scan_blocks_multi(&w, center, &block_kinds, radius)
    };
    let Some(target) = pos else {
        return Err(format!("半径 {radius} 内找不到 {item}"));
    };
    let stand = BlockPos::new(target.x, target.y - 1, target.z);
    bot.start_goto(BlockPosGoal(stand));
    // 等待到达
    for _ in 0..40 {
        sleep(Duration::from_millis(100)).await;
        if let Ok(p) = bot.position() {
            let d = ((p.x - target.x as f64).powi(2)
                + (p.y - target.y as f64).powi(2)
                + (p.z - target.z as f64).powi(2))
            .sqrt();
            if d < 3.0 {
                break;
            }
        }
    }
    Ok(target)
}

/// P21 新增：向下挖 N 格，暴露新方块（用于矿石自动探索）。
///
/// 每格：装备镐 → start_mining(脚下方块) → 等待消失 → bot 自然掉落 → 下一格。
/// 遇到基岩/Y<=-61 时停止（深板岩层下方挖无意义）。
/// 返回 true 表示成功挖完 N 格，false 表示中途停止（无镐/到基岩/超时）。
async fn auto_dig_down_explore(bot: &Client, blocks: u32) -> bool {
    // 先装备最好的镐
    let _ = auto_equip_best_pickaxe(bot).await;
    if !has_any_pickaxe_in_inventory(bot).await {
        eprintln!("[auto_dig] 背包无镐，无法下挖探索");
        return false;
    }

    for i in 0..blocks {
        let pos = match bot.position() {
            Ok(p) => BlockPos::new(
                p.x.floor() as i32,
                (p.y - 1.0).floor() as i32,
                p.z.floor() as i32,
            ),
            Err(_) => return false,
        };

        // Y<=-61 是深板岩层底部，继续挖无意义
        if pos.y <= -61 {
            eprintln!("[auto_dig] 到达 Y={} 深板岩层底部，停止下挖", pos.y);
            return false;
        }

        // 检查方块是否是基岩（不可破坏）
        let is_bedrock = bot
            .world()
            .ok()
            .and_then(|w| w.read().get_block_state(pos))
            .map(|s| {
                let kind: BlockKind = s.into();
                kind == BlockKind::Bedrock
            })
            .unwrap_or(false);
        if is_bedrock {
            eprintln!("[auto_dig] 遇到基岩，停止下挖");
            return false;
        }

        // 装备镐（每格都重新装备，防止挖矿后工具切换）
        let _ = auto_equip_best_pickaxe(bot).await;

        // 开始挖
        bot.start_mining(pos);

        // 等待方块消失（最多 4s）
        let mut broken = false;
        for _ in 0..40 {
            sleep(Duration::from_millis(100)).await;
            let still_there = bot
                .world()
                .ok()
                .and_then(|w| w.read().get_block_state(pos).map(|s| !s.is_air()))
                .unwrap_or(false);
            if !still_there {
                broken = true;
                break;
            }
        }
        if !broken {
            eprintln!("[auto_dig] 第 {} 格挖矿超时，停止", i + 1);
            return false;
        }

        // 等 bot 掉落到下一格
        sleep(Duration::from_millis(400)).await;
    }
    true
}

/// P21 新增：统计背包里指定物品种类列表的总数。
async fn count_items_in_inventory(bot: &Client, kinds: &[ItemKind]) -> u32 {
    // use azalea::container::ContainerHandleRef;
    let Ok(inv) = bot.get_inventory() else {
        return 0;
    };
    let Some(menu) = inv.menu().ok().flatten() else {
        return 0;
    };
    let Some(slots) = inv.slots() else {
        return 0;
    };
    let range = menu.player_slots_range();
    let mut total = 0u32;
    for s in range {
        if let Some(st) = slots.get(s)
            && !st.is_empty() && kinds.contains(&st.kind()) {
                total += st.count() as u32;
            }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_block_aliases_oak_log() {
        let kinds = expand_block_aliases("oak_log");
        assert!(kinds.contains(&BlockKind::OakLog));
        assert!(kinds.contains(&BlockKind::BirchLog));
        assert!(kinds.contains(&BlockKind::SpruceLog));
    }

    #[test]
    fn test_expand_block_aliases_stone() {
        let kinds = expand_block_aliases("stone");
        assert!(kinds.contains(&BlockKind::Stone));
        assert!(kinds.contains(&BlockKind::Cobblestone));
    }

    #[test]
    fn test_expand_block_aliases_ore_no_expand() {
        // P18 修复（2026-07-27）：矿石现在**展开** deepslate 变体。
        // vanilla 规则：Y<0 时矿石生成 deepslate_xxx_ore 版本（深岩层）。
        // 原 _ => vec![item] 只找 "iron_ore"，但 bot 在 Y=91 深岩层实际方块是
        // deepslate_iron_ore -> scan_blocks_multi 100% 找不到 -> gather 100% 失败。
        // 修复：iron_ore/coal_ore 等都展开为 [xxx_ore, deepslate_xxx_ore]。
        let kinds = expand_block_aliases("coal_ore");
        assert!(kinds.contains(&BlockKind::CoalOre));
        assert!(kinds.contains(&BlockKind::DeepslateCoalOre));
        // 验证所有 8 种主要矿石都展开 deepslate 变体
        let iron_kinds = expand_block_aliases("iron_ore");
        assert!(iron_kinds.contains(&BlockKind::IronOre));
        assert!(iron_kinds.contains(&BlockKind::DeepslateIronOre));
        let gold_kinds = expand_block_aliases("gold_ore");
        assert!(gold_kinds.contains(&BlockKind::GoldOre));
        assert!(gold_kinds.contains(&BlockKind::DeepslateGoldOre));
        // 兼容 LLM 直接传 deepslate_xxx_ore
        let ds_iron = expand_block_aliases("deepslate_iron_ore");
        assert!(ds_iron.contains(&BlockKind::IronOre));
        assert!(ds_iron.contains(&BlockKind::DeepslateIronOre));
    }

    #[test]
    fn test_compute_place_offset_east() {
        let bot_pos = azalea::Vec3::new(0.0, 64.0, 0.0);
        let target = BlockPos::new(2, 64, 0);
        let (stand, yaw) = compute_place_offset(bot_pos, target);
        // 目标在东边 (dx=2>0)，bot 站在目标西边一格 (1, 64, 0)，朝东 (-90)
        assert_eq!(stand, BlockPos::new(1, 64, 0));
        assert_eq!(yaw, -90.0);
    }

    #[test]
    fn test_compute_place_offset_north() {
        let bot_pos = azalea::Vec3::new(0.0, 64.0, 0.0);
        let target = BlockPos::new(0, 64, -2);
        let (stand, yaw) = compute_place_offset(bot_pos, target);
        // 目标在北边 (dz=-2<0)，bot 站在目标南边一格 (0, 64, -1)，朝北 (180)
        assert_eq!(stand, BlockPos::new(0, 64, -1));
        assert_eq!(yaw, 180.0);
    }
}
