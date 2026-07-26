//! 自动采集：走到最近的指定方块并挖掘，直到背包积累足够数量。
//!
//! 用途：让 bot 能自主完成"早期游戏"第一步（砍树/挖石/挖矿），从而把
//! 采集与合成串成端到端任务，无需玩家手动给物品。
//!
//! P8 修复（2026-07-26）：
//! - 挖硬方块前自动装备背包中最好的镐；砍原木前自动装备最好的斧。
//! - 没有合适工具时立即返回明确错误，提示 LLM 先合成工具。
//! - 检测"开始挖后方块长时间不消失" → 视为缺工具，避免空等。

use super::{
    auto_equip_best_axe, auto_equip_best_pickaxe, best_pickaxe_tier_in_inventory,
    block_required_pickaxe_tier, has_any_axe_in_inventory, has_any_pickaxe_in_inventory,
    is_hard_block, is_log_block, pickaxe_tier_name, pickaxe_to_craft_for_tier,
};
use azalea::BlockPos;
use azalea::container::ContainerHandleRef;
use azalea::pathfinder::goals::BlockPosGoal;
use azalea::prelude::*;
use azalea_registry::builtin::{BlockKind, ItemKind};
use std::str::FromStr;
use std::time::{Duration, Instant};
use tokio::time::sleep;

/// 在半径 `radius` 内扫描给定种类的方块，返回按到中心距离升序排序的世界坐标。
fn scan_blocks(
    world: &azalea_world::World,
    center: azalea::Vec3,
    kind: BlockKind,
    radius: i32,
) -> Vec<BlockPos> {
    let cx = center.x.floor() as i32;
    let cy = center.y.floor() as i32;
    let cz = center.z.floor() as i32;
    let mut found = Vec::new();
    for dx in -radius..=radius {
        for dy in -radius..=radius {
            for dz in -radius..=radius {
                let pos = BlockPos::new(cx + dx, cy + dy, cz + dz);
                if let Some(state) = world.get_block_state(pos) {
                    let bk: BlockKind = state.into();
                    if bk == kind {
                        found.push(pos);
                    }
                }
            }
        }
    }
    // 按到中心距离排序
    found.sort_by_key(|p| (p.x - cx).pow(2) + (p.y - cy).pow(2) + (p.z - cz).pow(2));
    found
}

/// 统计背包中指定物品数量。
fn count_item(inv: &ContainerHandleRef, kind: ItemKind) -> u32 {
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
        if let Some(stack) = slots.get(s) {
            if !stack.is_empty() && stack.kind() == kind {
                total += stack.count().max(0) as u32;
            }
        }
    }
    total
}

/// 根据目标方块种类决定应该装备的工具类型。
enum ToolNeed {
    /// 需要镐（硬方块：石/矿/砖等）
    Pickaxe,
    /// 需要斧（原木/木头类）
    Axe,
    /// 徒手可挖（软方块：泥土/沙/雪等）
    None,
}

/// 根据 BlockKind 推断需要的工具类型。读不到状态时返回 None。
fn tool_need_for_block(world: &azalea_world::World, pos: BlockPos) -> ToolNeed {
    if let Some(state) = world.get_block_state(pos) {
        if is_log_block(state) {
            return ToolNeed::Axe;
        }
        if is_hard_block(state) {
            return ToolNeed::Pickaxe;
        }
    }
    ToolNeed::None
}

pub async fn do_gather(bot: &Client, item: &str, count: u32) -> Result<String, String> {
    let target = ItemKind::from_str(&normalize_item(item))
        .or_else(|_| ItemKind::from_str(item))
        .map_err(|_| format!("未知物品 {item}"))?;
    // 方块种类与物品同 id（oak_log <-> OakLog），直接复用归一化 id 解析 BlockKind。
    let block_kind = BlockKind::from_str(&normalize_item(item))
        .or_else(|_| BlockKind::from_str(item))
        .map_err(|_| {
            format!("无法解析方块种类 {item}（采集需方块形态，如 oak_log / stone / coal_ore）")
        })?;

    let need = count.max(1);
    let mut gathered = 0u32;
    let max_rounds = 24;

    // 一次性预检：根据目标方块类型决定是否需要工具，若需要但背包没有任何工具
    // 就立即失败并明确告知 LLM——避免反复尝试 24 轮都失败浪费时间。
    // 注意：这里只看 BlockKind 字符串，因为方块还没扫到。
    let kind_str = {
        let s = block_kind.to_str();
        s.strip_prefix("minecraft:").unwrap_or(s).to_string()
    };
    let is_log_kind = kind_str.ends_with("_log") || kind_str.ends_with("_wood");
    let is_ore_or_stone_like = kind_str.ends_with("_ore")
        || kind_str == "stone"
        || kind_str == "deepslate"
        || kind_str == "cobblestone"
        || kind_str == "granite"
        || kind_str == "diorite"
        || kind_str == "andesite"
        || kind_str == "tuff"
        || kind_str == "netherrack"
        || kind_str == "basalt"
        || kind_str == "blackstone"
        || kind_str == "end_stone"
        || kind_str == "sandstone"
        || kind_str == "red_sandstone";
    if is_log_kind && !has_any_axe_in_inventory(bot).await {
        // P16 修复（2026-07-26）：徒手砍树是可行的（只是慢），不应阻止 gather。
        // 原 P10 逻辑在这里直接 return Err，导致 LLM 永远无法用徒手砍树启动游戏——
        // 这是死循环的根因：需要原木合成斧 → 需要斧砍树 → 没斧就砍不了树。
        // 修复：改为警告（eprintln），让 gather 继续执行徒手砍树。
        // 徒手砍树验证：vanilla 中徒手挖原木会掉落原木（只是速度慢，约 3s/块）。
        // P10 的「等 500ms 再检查」逻辑保留——若刚 craft 完斧头但同步未完成，
        // 等一下就能检测到斧头，正常装备斧头砍树。
        sleep(Duration::from_millis(500)).await;
        if !has_any_axe_in_inventory(bot).await {
            eprintln!(
                "[gather] 警告：背包无斧，将徒手砍树（速度慢但可行）。\
                 建议后续 craft wooden_axe 提升效率。"
            );
        }
    }
    if is_ore_or_stone_like && !has_any_pickaxe_in_inventory(bot).await {
        // P10 修复（2026-07-26）：刚 craft 完镐但背包同步未完成时会误报"无镐"。
        // 现象：session 中 craft_3x3 stone_pickaxe 成功后立即 gather deepslate_iron_ore
        // 报"背包无镐"——但实际上 stone_pickaxe 已在背包（shift_click(0) 后服务端
        // ContainerSetContent 包可能还在路上）。
        // 修复：等待 500ms（10 server ticks）让服务端同步背包，再检查一次。
        // 同时做 3 次重试（每次间隔 500ms），覆盖网络抖动场景。
        let mut found_pickaxe = false;
        for retry in 0..3u8 {
            sleep(Duration::from_millis(500)).await;
            if has_any_pickaxe_in_inventory(bot).await {
                found_pickaxe = true;
                eprintln!("[gather] pickaxe found after {} retry(s)", retry + 1);
                break;
            }
        }
        if !found_pickaxe {
            return Err(format!(
                "采集 {item} 失败：背包无镐，矿石/石头类方块徒手挖不掉（不掉落物品）。\n\
                 解决步骤：\n\
                 1. 先 perceive 查看背包，确认是否已有镐（搜 *_pickaxe）\n\
                 2a. 若已有镐：用 equip(item='xxx_pickaxe') 装备主手后重试 gather\n\
                 2b. 若无镐：根据背包原料合成——\n\
                     - wooden_pickaxe = oak_planks×3 + stick×2（craft 2×2 即可）\n\
                     - stone_pickaxe = cobblestone×3 + stick×2（需 craft_3x3 工作台）\n\
                     - iron_pickaxe = iron_ingot×3 + stick×2（需 craft_3x3 工作台）\n\
                     stick 由 2 个 planks 合成 4 个\n\
                 3. 合成后 equip 装备，再重试 gather"
            ));
        }
    }

    // P11 修复（2026-07-26）：工具等级检查。
    // vanilla 规则：等级不足的镐挖该方块时方块会消失但**不掉落物品**——
    // 这是 gather「方块消失但背包数量不增」误报的根因。
    // 例如：wooden_pickaxe 挖 iron_ore → 方块消失但无 iron_ore 掉落；
    //       stone_pickaxe 挖 diamond_ore → 方块消失但无 diamond_ore 掉落。
    // 预检：若背包最好的镐 tier < 目标方块所需 tier，立即返回错误让 LLM 先合成更高 tier 的镐。
    if is_ore_or_stone_like {
        let required_tier = block_required_pickaxe_tier(block_kind);
        if required_tier > 0 {
            // 等待背包同步后取最好的镐 tier
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

    let mut last_skip_reason: Option<String> = None;

    for round in 0..max_rounds {
        if gathered >= need {
            break;
        }

        // 1) 找最近的方块
        let (target_pos, tool_need) = {
            let world = bot
                .world()
                .map_err(|e| format!("读取世界失败: {e:?}"))?;
            let w = world.read();
            let center = bot.position().map_err(|e| format!("读取坐标失败: {e:?}"))?;
            let pos = scan_blocks(&w, center, block_kind, 32).into_iter().next();
            let Some(p) = pos else {
                return Err(format!(
                    "附近 32 格内找不到 {item}（已采集 {gathered}/{need}）。\
                     建议：用 go 走到其它区域再 gather，或换一个采集目标。"
                ));
            };
            let need = tool_need_for_block(&w, p);
            (p, need)
        };

        // 2) 装备合适工具
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

        // 3) 走到方块下方一格（贴脸挖）
        let stand = BlockPos::new(target_pos.x, target_pos.y - 1, target_pos.z);
        bot.start_goto(BlockPosGoal(stand));
        // 等待靠近（4s 上限，与 32m 寻路范围匹配；不违反 go 3s 上限，因为 gather 是复合动作）
        let mut reached = false;
        for _ in 0..40 {
            sleep(Duration::from_millis(100)).await;
            if let Ok(p) = bot.position() {
                let d = ((p.x - target_pos.x as f64).powi(2)
                    + (p.y - target_pos.y as f64).powi(2)
                    + (p.z - target_pos.z as f64).powi(2))
                .sqrt();
                if d < 3.0 {
                    reached = true;
                    break;
                }
            }
        }
        bot.stop_pathfinding(); // 停止导航，准备挖掘
        if !reached {
            last_skip_reason = Some(format!(
                "无法到达 {item} @ ({},{},{})（可能被阻挡或距离过远）",
                target_pos.x, target_pos.y, target_pos.z
            ));
            continue;
        }

        // 4) 挖掘目标方块
        let before = {
            let inv = bot.get_inventory().map_err(|e| format!("{e:?}"))?;
            count_item(&inv, target)
        };
        let mine_start = Instant::now();
        bot.start_mining(target_pos);

        // 等待方块被挖掉（背包数量增加）或超时
        let mut done = false;
        let mut block_disappeared = false;
        for _ in 0..60 {
            sleep(Duration::from_millis(100)).await;
            let inv = match bot.get_inventory() {
                Ok(i) => i,
                Err(_) => continue,
            };
            let now = count_item(&inv, target);
            if now > before {
                gathered = now;
                done = true;
                break;
            }
            // 方块已消失但数量未变（被他人捡走/没工具挖不掉掉落物）
            if let Ok(world) = bot.world() {
                let gone = world
                    .read()
                    .get_block_state(target_pos)
                    .map(|s| s.is_air())
                    .unwrap_or(true);
                if gone {
                    block_disappeared = true;
                    break;
                }
            }
        }

        // 5) 若方块消失但背包没增加 → 多半是缺工具（徒手挖石/矿不掉落）
        if !done && block_disappeared {
            // P11 修复（2026-07-26）：原检查只判断「是否有任意镐」，但没判断「镐等级是否足够」。
            // 例如：wooden_pickaxe 挖 iron_ore → 方块消失但无 iron_ore 掉落，
            // 原检查 has_any_pickaxe_in_inventory=true → 误判为「有镐，继续下一轮」→ 死循环。
            // 修复：判断手持物（或背包最好的镐）的 tier 是否 >= 目标方块所需 tier。
            let held_kind = bot.get_held_item().ok().and_then(|s| {
                if s.is_empty() {
                    None
                } else {
                    Some(s.kind())
                }
            });
            let required_tier = block_required_pickaxe_tier(block_kind);
            let held_tier = held_kind
                .map(|k| crate::azalea::pickaxe_tier(k))
                .unwrap_or(0);
            let best_tier = best_pickaxe_tier_in_inventory(bot).await;

            // 完全没镐 → 缺工具
            if best_tier == 0 && matches!(tool_need, ToolNeed::Pickaxe) {
                return Err(format!(
                    "采集 {item} 失败：方块被挖掉但未掉落物品（背包无镐）。\n\
                     手持物：{:?}；建议先合成 {} 再 gather。",
                    held_kind,
                    pickaxe_to_craft_for_tier(required_tier.max(1))
                ));
            }
            // 有镐但等级不足 → 提示合成更高 tier 的镐
            if matches!(tool_need, ToolNeed::Pickaxe)
                && required_tier > 0
                && best_tier < required_tier
            {
                return Err(format!(
                    "采集 {item} 失败：方块被挖掉但未掉落物品（镐等级不足）。\n\
                     目标方块需要 {}（tier {}），背包最好的镐为 {}（tier {}），手持 {:?}（tier {}）。\n\
                     vanilla 规则：等级不足的镐挖该方块时方块会消失但**不掉落物品**。\n\
                     建议：先 craft 3x3 合成 {}，equip 装备主手后再 gather。",
                    pickaxe_tier_name(required_tier),
                    required_tier,
                    pickaxe_tier_name(best_tier),
                    best_tier,
                    held_kind,
                    held_tier,
                    pickaxe_to_craft_for_tier(required_tier)
                ));
            }
            // 有足够等级的镐但仍未掉落 → 可能是徒手挖（auto_equip 失败）
            if matches!(tool_need, ToolNeed::Pickaxe) && held_tier == 0 && best_tier > 0 {
                return Err(format!(
                    "采集 {item} 失败：方块被挖掉但未掉落物品（主手未持镐）。\n\
                     背包有镐（{} tier {}）但 auto_equip 失败未切到主手。\n\
                     建议：手动 equip 装备镐到主手后再 gather。",
                    pickaxe_tier_name(best_tier),
                    best_tier
                ));
            }
            // 斧类检查（徒手砍树只是慢，不掉物品几乎不会发生）
            // P15 修复（2026-07-26）：原代码在「方块消失但背包未增」时直接报失败，
            // 但徒手砍树是会掉落的（只是慢）。方块消失后掉落物需要 1-2s 才被 bot 拾取，
            // 原循环 100ms 检查一次发现方块消失就 break，没等拾取就误判「无斧失败」。
            // 修复：方块消失后额外等 1.5s 让 bot 拾取掉落物；若拾取成功算正常完成；
            // 若仍失败再判断是否真没斧（无斧时给警告但继续，因为徒手能砍树）。
            if matches!(tool_need, ToolNeed::Axe) {
                // 等待 1.5s 让 bot 拾取掉落物
                for _ in 0..15 {
                    sleep(Duration::from_millis(100)).await;
                    let inv = match bot.get_inventory() {
                        Ok(i) => i,
                        Err(_) => continue,
                    };
                    let now = count_item(&inv, target);
                    if now > before {
                        gathered = now;
                        done = true;
                        break;
                    }
                }
                if done {
                    break;
                }
                // 仍没拾取到：如果背包无斧，给警告但继续下一轮（徒手能砍树，只是慢）
                if !has_any_axe_in_inventory(bot).await {
                    last_skip_reason = Some(
                        "徒手砍树效率极低（无斧），建议合成 wooden_axe 后再 gather".to_string()
                    );
                    continue;
                }
                // 有斧但仍未拾取：可能是同步延迟，继续下一轮
                last_skip_reason = Some(
                    "方块消失但掉落物未拾取（可能服务端同步延迟）".to_string()
                );
                continue;
            }
        }

        if !done {
            // 挖了 6s 还没结束且方块还在 → 多半是挖不动（服务端拒绝/工具不对）
            let elapsed = mine_start.elapsed();
            if elapsed >= Duration::from_secs(6) {
                last_skip_reason = Some(format!(
                    "挖 {item} @ ({},{},{}) 超时 ({:?})，可能工具不对或服务端拒绝",
                    target_pos.x, target_pos.y, target_pos.z, elapsed
                ));
                // 主动停止挖掘，避免卡住
                bot.stop_pathfinding();
            }
            continue;
        }
    }

    if gathered >= need {
        Ok(format!("采集 {item} 完成（背包 {gathered} 个）"))
    } else {
        let reason = last_skip_reason
            .map(|r| format!("；最后原因: {r}"))
            .unwrap_or_default();
        Err(format!(
            "采集 {item} 未完成（仅 {gathered}/{need}，附近可能无更多{reason}）"
        ))
    }
}

fn normalize_item(item: &str) -> String {
    if item.starts_with("minecraft:") {
        item.to_string()
    } else {
        format!("minecraft:{item}")
    }
}
