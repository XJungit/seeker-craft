//! 放置方块 + 打开容器（补齐 3×3 合成 / 熔炼的自主闭环断点）。
//!
//! - `do_place`：把手持物品 item 放到世界坐标 pos 旁（右键放置，需先把该物品选到手中）。
//! - `do_open_container`：打开 pos 处的容器（工作台/熔炉/箱子等），后续 craft_3x3 / smelt 才能操作。

use azalea::BlockPos;
use azalea::container::ContainerHandleRef;
use azalea::prelude::*;
use azalea_registry::builtin::{BlockKind, ItemKind};
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;

/// 找到背包里持有 item 的 hotbar 槽位（0..=8），无则 None。
fn find_hotbar_slot(inv: &ContainerHandleRef, kind: ItemKind) -> Option<u8> {
    let menu = inv.menu().ok()??;
    // hotbar_slots_range() 返回 hotbar 的绝对槽号范围（最后 9 个 player slot）。
    // hotbar 索引 0 = range.start(), 索引 8 = range.end()。
    // 之前的代码用 (last - s).unsigned_abs() 算 idx，结果完全反了：
    // s=end → idx=0（应该是 8），s=start → idx=8（应该是 0），导致
    // bot.set_selected_hotbar_slot() 选错槽位，block_interact 时手持空手/错物品，
    // 服务端拒绝放置 → place 100% 失败。
    let hotbar_range = menu.hotbar_slots_range();
    let hotbar_start = *hotbar_range.start();
    let slots = inv.slots()?;
    for s in hotbar_range {
        if let Some(stack) = slots.get(s) {
            if !stack.is_empty() && stack.kind() == kind {
                let idx = (s - hotbar_start) as u8;
                debug_assert!(idx <= 8, "hotbar idx out of range: {idx}");
                return Some(idx);
            }
        }
    }
    None
}

/// 把 ItemKind 映射到对应的 BlockKind（用于放置后的方块种类校验）。
/// 大多数放置类物品（工作台/熔炉/箱子/砖块等）的 item id 与 block id 一致。
fn item_to_block_kind(item: &str) -> Option<BlockKind> {
    let id = normalize_item(item);
    BlockKind::from_str(&id).ok()
}

/// 在主背包（非 hotbar）找指定物品的槽位。
/// 用于 place 时：物品刚合成落到主背包，需要先 shift_click 到 hotbar 才能选中放置。
fn find_item_slot_in_main_inventory(inv: &ContainerHandleRef, kind: ItemKind) -> Option<usize> {
    let menu = inv.menu().ok()??;
    let player_range = menu.player_slots_range();
    let hotbar_range = menu.hotbar_slots_range();
    let slots = inv.slots()?;
    for s in player_range {
        if hotbar_range.contains(&s) {
            continue; // 跳过 hotbar
        }
        if let Some(stack) = slots.get(s) {
            if !stack.is_empty() && stack.kind() == kind {
                return Some(s);
            }
        }
    }
    None
}

pub async fn do_place(bot: &Client, item: &str, pos: BlockPos) -> Result<String, String> {
    let kind = ItemKind::from_str(&normalize_item(item))
        .or_else(|_| ItemKind::from_str(item))
        .map_err(|_| format!("未知物品 {item}"))?;

    // P11 修复（2026-07-26）：原代码在最后才检查背包是否有 item，
    // bot 会先走过去、再发现"背包未持有"——浪费一轮往返。
    // 现在：先检查背包（包括 hotbar + 主背包），无则立即报错，避免无谓的 pathfinder 调用。
    // 同时处理"hotbar 已持有但 set_selected_hotbar_slot 失败"的边缘场景。
    let inv_pre = bot
        .get_inventory()
        .map_err(|e| format!("获取背包失败: {e:?}"))?;
    let pre_hotbar = find_hotbar_slot(&inv_pre, kind);
    let pre_main = if pre_hotbar.is_some() {
        None
    } else {
        find_item_slot_in_main_inventory(&inv_pre, kind)
    };
    if pre_hotbar.is_none() && pre_main.is_none() {
        // 背包确实没有此物品——立即报错，避免无谓的走动
        return Err(format!(
            "背包未持有 {item}（无法放置）。\
             建议：1) 先 craft 或 craft_3x3 合成此物品；2) 用 perceive 查看背包确认物品名；\
             3) 若物品已在地面上，用 pickup 拾取后再 place。"
        ));
    }
    drop(inv_pre);

    // P5 关键修复 5：LLM 常给 bot 自己占据的坐标（如 bot 当前位置 +1 高度），
    // 服务端拒绝在 bot bounding box 内放方块 → place 100% 失败。
    // 检测：若 pos 与 bot 自身位置重合（foot 或 head），强制重定位到附近。
    let bot_pos = bot.position().ok();
    let pos_blocked_by_bot = if let Some(bp) = bot_pos {
        let bx = bp.x.floor() as i32;
        let by = bp.y.floor() as i32;
        let bz = bp.z.floor() as i32;
        // bot 占据 foot (bx,by,bz) + head (bx,by+1,bz)
        (pos.x == bx && pos.z == bz && (pos.y == by || pos.y == by + 1))
    } else {
        false
    };

    // P5 关键修复 4：LLM 常给无效坐标（已有方块/下方不实心），导致 place 100% 失败。
    // 策略：先试 LLM 给的 pos；若无效（非空气 或 下方不实心 或 bot 自身占据），扫描附近 3 格半径
    // 找一个合法位置（air + 下方 solid），自动用该位置放置。
    // 这样 LLM 只需"大致指个位置"，bot 自己找精确可放点——大幅降低 place 失败率。
    // P11 增强：扫描半径从 3 扩大到 5，覆盖更多地下场景（地下空间狭窄，3 格常找不到合法点）。
    let mut placement_pos = pos;
    let mut relocated = false;
    let pos_invalid = !is_block_air(bot, pos).await
        || !is_block_solid(bot, pos.down(1)).await
        || pos_blocked_by_bot;
    if pos_invalid {
        // LLM 给的位置无效，扫描附近（先 3 格，再扩到 5 格）
        let nearby = find_valid_placement_nearby(bot, pos)
            .or_else(|| find_valid_placement_nearby_radius(bot, pos, 5));
        if let Some(np) = nearby {
            placement_pos = np;
            relocated = true;
        }
        // 若 nearby 也找不到，继续用原 pos（下面的检查会给出具体错误）
    }

    // P5 关键修复 1：放置前必须确认 placement_pos 是空气。
    let pos_was_air = is_block_air(bot, placement_pos).await;
    if !pos_was_air {
        let existing = block_kind_at(bot, placement_pos)
            .map(|k| format!("{k:?}"))
            .unwrap_or_else(|| "未知方块".to_string());
        return Err(format!(
            "放置 {item} 于 ({},{},{}) 失败：该位置已有方块 {existing}（不是空气），且附近 5 格内未找到合法放置点。\
             block_interact 无法在已有方块的位置放置新方块。\
             建议：换一个空气位置 place，或先 mine 掉该处方块。",
            placement_pos.x, placement_pos.y, placement_pos.z
        ));
    }

    // P5 关键修复 3：bot 距离过远时 block_interact 会因 reach 检查失败而静默无效。
    // 原代码不检查距离，导致 LLM 给远距离坐标时 place 100% 失败。
    // 现在：距离 > 4.0m 时先用 pathfinder 走到目标旁 1.5m，再放置。
    walk_to_reach_for_place(bot, placement_pos).await;

    // P11 修复（2026-07-26）：原 pre_check 缓存了 pre_main 槽位索引，
    // 但 bot 走动（walk_to_reach_for_place）后背包状态可能变化（拾取/同步），
    // shift_click 缓存的槽位会点错位置——表面 bug 表现为"背包未持有 X"但实际有。
    // 修复：pre_check 只用作"早期失败检测"（true negative），不缓存槽位索引；
    // 真正选中 hotbar 时重新读 inventory 拿最新槽位。
    let slot = {
        let inv = bot
            .get_inventory()
            .map_err(|e| format!("获取背包失败: {e:?}"))?;
        // 先看 hotbar
        if let Some(s) = find_hotbar_slot(&inv, kind) {
            s
        } else {
            // 不在 hotbar，从主背包 shift_click 到 hotbar
            let main_slot = find_item_slot_in_main_inventory(&inv, kind)
                .ok_or_else(|| format!("背包未持有 {item}（无法放置）"))?;
            inv.shift_click(main_slot);
            sleep(Duration::from_millis(200)).await;
            drop(inv);
            let inv2 = bot
                .get_inventory()
                .map_err(|e| format!("移动物品后获取背包失败: {e:?}"))?;
            match find_hotbar_slot(&inv2, kind) {
                Some(s) => s,
                None => {
                    // P16 修复（2026-07-26）：shift_click 失败（hotbar 满）时，
                    // 学习 mineflayer 的 LRU round-robin 策略：选一个 hotbar 槽，
                    // 把目标物品与该槽物品交换（原 hotbar 物品回到主背包）。
                    // mineflayer 源码: lib/plugins/simple_inventory.js::equip()
                    //   destSlot = QUICK_BAR_START + nextQuickBarSlot;
                    //   nextQuickBarSlot = (nextQuickBarSlot + 1) % QUICK_BAR_COUNT;
                    // 这里简化为选第一个 hotbar 槽（slot 36），用 left_click 三步交换。
                    let menu = inv2.menu().ok().flatten()
                        .ok_or_else(|| format!("放置 {item} 失败：读取菜单失败"))?;
                    let hotbar_range = menu.hotbar_slots_range();
                    let target_hotbar = *hotbar_range.start();

                    // 找目标物品在主背包的槽位（shift_click 可能已经改变了位置，重新找）
                    let source_slot = find_item_slot_in_main_inventory(&inv2, kind)
                        .ok_or_else(|| format!("放置 {item} 失败：shift_click 后物品既不在 hotbar 也不在主背包"))?;

                    eprintln!(
                        "[place] hotbar 满，LRU 交换：source=slot{source_slot} ↔ target=slot{target_hotbar}"
                    );

                    // 三步交换：
                    // 1. left_click(source) 拿起目标物品
                    inv2.left_click(source_slot);
                    sleep(Duration::from_millis(150)).await;
                    // 2. left_click(target_hotbar) 放下目标物品，拿起原 hotbar 物品
                    drop(inv2);
                    let inv3 = bot.get_inventory()
                        .map_err(|e| format!("交换中获取背包失败: {e:?}"))?;
                    inv3.left_click(target_hotbar);
                    sleep(Duration::from_millis(150)).await;
                    // 3. left_click(source) 把原 hotbar 物品放回主背包
                    drop(inv3);
                    let inv4 = bot.get_inventory()
                        .map_err(|e| format!("交换后获取背包失败: {e:?}"))?;
                    inv4.left_click(source_slot);
                    sleep(Duration::from_millis(200)).await;

                    // 验证目标物品现在在 hotbar
                    drop(inv4);
                    let inv5 = bot.get_inventory()
                        .map_err(|e| format!("验证时获取背包失败: {e:?}"))?;
                    find_hotbar_slot(&inv5, kind).ok_or_else(|| {
                        format!("放置 {item} 失败：LRU 交换后物品仍不在 hotbar（交换可能失败）")
                    })?
                }
            }
        }
    };
    bot.set_selected_hotbar_slot(slot);
    // P5 关键修复：50ms 不够让服务端收到 ServerboundSetCarriedItem 并切换手持物品。
    // ensure_has_sent_carried_item 系统在 GameTick（50ms）里发 packet，服务端处理后
    // 才认为 bot 持有 crafting_table。如果太早 block_interact，服务端按"空手/旧物品"
    // 处理 use_item_on，拒绝放置 → place 100% 失败。
    // 等 200ms（4 tick）确保 packet 已发送并被服务端处理。
    sleep(Duration::from_millis(200)).await;

    // P5 关键修复：block_interact(pos) 是「右键点击 pos 处的方块」，
    // 但 pos 是目标空气格——服务端无法右键空气放置方块。
    // 正确做法：右键 pos 下方的实心方块（force_block 机制固定用 Direction::Up，
    // 即「右键方块顶面」），服务端会把新方块放到该方块上方 = pos。
    // 前提：pos.down(1) 必须是实心方块；否则放不上。
    let below = placement_pos.down(1);
    let below_solid = is_block_solid(bot, below).await;
    if !below_solid {
        return Err(format!(
            "放置 {item} 于 ({},{},{}) 失败：下方 ({},{},{}) 不是实心方块。\
             block_interact 只能右键实心方块的顶面来放置新方块。\
             建议：选一个下方有实心方块的位置 place，或先 place 一个 dirt/stone 做底座。",
            placement_pos.x, placement_pos.y, placement_pos.z, below.x, below.y, below.z
        ));
    }

    // 距离检查：pathfinder 可能找不到路，走不到 reach 范围内。
    // 此时直接返回错误，让 LLM 知道要先 goto。
    if let Ok(p) = bot.position() {
        let dx = p.x - (placement_pos.x as f64 + 0.5);
        let dy = p.y - (placement_pos.y as f64 + 0.5);
        let dz = p.z - (placement_pos.z as f64 + 0.5);
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        if dist > 5.0 {
            return Err(format!(
                "放置 {item} 于 ({},{},{}) 失败：bot 距目标 {dist:.1}m 过远（>5m，pathfinder 未能走到 reach 范围）。\
                 当前 bot 位置 ({:.1},{:.1},{:.1})。建议：先 goto 到目标旁 1-2m 再 place。",
                placement_pos.x, placement_pos.y, placement_pos.z, p.x, p.y, p.z
            ));
        }
    }

    bot.block_interact(below);
    sleep(Duration::from_millis(200)).await;

    // P5 关键修复 2：验证 pos 处的方块是否变成了期望的 BlockKind，
    // 而不是只查"非空气"。这样能区分"实际放上去了"与"位置本来就被占据"。
    let expected_block = item_to_block_kind(item);
    let placed_ok = verify_block_placed(bot, placement_pos, expected_block).await;
    if placed_ok {
        // P5 关键修复：成功消息必须用 placement_pos（实际放置位置）而非 pos（LLM 原始坐标）。
        // 原 bug：成功消息用 pos，但 placement_pos 可能因自动重定位而不同 →
        // LLM 被告知"已放在 X"但实际在 Y，后续 open(X) 必然失败。
        let relocate_note = if relocated {
            format!("（原坐标 ({},{},{}) 无效，已自动移到附近合法位置）", pos.x, pos.y, pos.z)
        } else {
            String::new()
        };
        Ok(format!(
            "Action output:\nPlaced {item} at ({},{},{}).{relocate_note}",
            placement_pos.x, placement_pos.y, placement_pos.z
        ))
    } else {
        // 检查 placement_pos 现在是什么方块，给 LLM 更准确的诊断
        let now_kind = block_kind_at(bot, placement_pos).map(|k| format!("{k:?}")).unwrap_or_else(|| "空气".to_string());
        Err(format!(
            "放置 {item} 于 ({},{},{}) 失败——放置后该处为 {now_kind}（期望 {}）。\
             可能原因：1) bot 距离过远（>4.5m，reach 检查失败）；2) 服务端拒绝放置（保护区/碰撞）；\
             3) 物品未真正选到手中。建议：先 goto 到目标旁 1-2m，确认 line-of-sight，再 place。",
            placement_pos.x, placement_pos.y, placement_pos.z, item
        ))
    }
}

/// 检查 pos 处方块是否实心（非空气）。
async fn is_block_solid(bot: &Client, pos: BlockPos) -> bool {
    let world = match bot.world() {
        Ok(w) => w,
        Err(_) => return false,
    };
    let world = world.read();
    let state = world.get_block_state(pos).unwrap_or_default();
    !state.is_air()
}

/// 检查 pos 处是否为空气。
async fn is_block_air(bot: &Client, pos: BlockPos) -> bool {
    let world = match bot.world() {
        Ok(w) => w,
        Err(_) => return false,
    };
    let world = world.read();
    let state = world.get_block_state(pos).unwrap_or_default();
    state.is_air()
}

/// 读取 pos 处的 BlockKind（空气或读取失败返回 None）。
fn block_kind_at(bot: &Client, pos: BlockPos) -> Option<BlockKind> {
    let world = bot.world().ok()?;
    let world = world.read();
    let state = world.get_block_state(pos)?;
    if state.is_air() {
        return None;
    }
    Some(state.into())
}

/// 验证 pos 处是否变成了期望的方块。
/// `expected` 为 None 时退化为"非空气"判定（向后兼容）。
///
/// P5 关键修复：原代码只 sleep 150ms 后查一次，但服务端同步可能更慢
/// （block_interact 触发事件→bevy 系统→发包→服务端处理→回包→本地世界更新），
/// 导致 verify 误判为失败（实际已放置但本地缓存还没更新）或误判为成功
/// （本地缓存未更新但 verify 不再重查）。改为轮询最多 1 秒（每 100ms 查一次），
/// 一旦看到期望方块就立即返回 true；超时返回 false。
async fn verify_block_placed(bot: &Client, pos: BlockPos, expected: Option<BlockKind>) -> bool {
    for _ in 0..10 {
        sleep(Duration::from_millis(100)).await;
        let now = block_kind_at(bot, pos);
        let ok = match expected {
            Some(expected_kind) => now == Some(expected_kind),
            None => now.is_some(),
        };
        if ok {
            return true;
        }
    }
    false
}

pub async fn do_open_container(bot: &Client, pos: BlockPos) -> Result<String, String> {
    // P5 修复：先验证 pos 处确实是容器方块，避免无意义地调 open_container_at
    // （服务端对非容器方块调用会返回 Ok(None)，原代码报"无容器或无法打开"，
    // 但 LLM 不知道是位置错了还是 bot 太远）。
    let block_kind = block_kind_at(bot, pos);
    let is_container = match block_kind {
        Some(k) => is_container_block(k),
        None => false,
    };
    // P11 修复（2026-07-26）：bot 经常缓存过期的工作台坐标（如自己挖掉了、
    // 或上次 place 的实际位置与 LLM 记忆不同），导致连续多次 open 失败陷入死循环。
    // 修复：若目标坐标不是容器，扫描附近 5 格半径找任意容器方块（crafting_table/furnace/chest等），
    // 找到则自动用该坐标继续 open，并提示 LLM "已自动重定位到 X"。
    // 这与 do_place 的自动重定位机制对称，避免 LLM 因坐标漂移陷入死循环。
    if !is_container {
        if let Some(nearby) = find_nearby_container_block(bot, pos, 5) {
            let actual_kind = block_kind_at(bot, nearby)
                .map(|k| format!("{k:?}"))
                .unwrap_or_else(|| "容器".to_string());
            let relocated_note = format!(
                "（原坐标 ({},{},{}) 不是容器，已自动在附近 5 格找到 {actual_kind} 于 ({},{},{})）",
                pos.x, pos.y, pos.z, nearby.x, nearby.y, nearby.z
            );
            // 用新坐标重新走 open_container 流程
            return do_open_container_inner(bot, nearby, Some(relocated_note)).await;
        }
        let actual = block_kind.map(|k| format!("{k:?}")).unwrap_or_else(|| "空气".to_string());
        return Err(format!(
            "({},{},{}) 处不是容器方块（当前为 {actual}），且附近 5 格内未找到任何容器。容器方块包括：crafting_table / furnace / chest / barrel / shulker_box / blast_furnace / smoker / brewing_stand 等。\
             建议：1) 检查坐标是否正确；2) 重新 place 一个容器后立即 open。",
            pos.x, pos.y, pos.z
        ));
    }
    do_open_container_inner(bot, pos, None).await
}

/// 内部 open 实现：effective_pos 已确认是容器方块。
/// `relocate_note` 用于在成功/失败消息中标注"自动重定位"。
async fn do_open_container_inner(
    bot: &Client,
    effective_pos: BlockPos,
    relocate_note: Option<String>,
) -> Result<String, String> {
    let note = relocate_note.unwrap_or_default();

    // P5 修复：bot 太远时 open_container_at 会卡住或失败。
    // 先用 pathfinder 走到容器旁 1.5m，再检查距离。pathfinder 找不到路时
    // 距离检查仍会报错，让 LLM 知道要先 goto。
    walk_to_reach_for_place(bot, effective_pos).await;
    if let Ok(p) = bot.position() {
        let dx = p.x - (effective_pos.x as f64 + 0.5);
        let dy = p.y - (effective_pos.y as f64 + 0.5);
        let dz = p.z - (effective_pos.z as f64 + 0.5);
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        if dist > 5.0 {
            return Err(format!(
                "bot 距容器 ({},{},{}) {dist:.1}m 过远（>5m，pathfinder 未能走到 reach 范围）。当前 bot 位置 ({:.1},{:.1},{:.1})。{note}\
                 建议：先 goto 到容器旁 1-2m 再 open。",
                effective_pos.x, effective_pos.y, effective_pos.z, p.x, p.y, p.z
            ));
        }
    }

    // P8 关键修复：防止 ContainerHandle 被 drop 导致容器自动关闭。
    // ContainerHandle 的 Drop 会调用 close() 发送关闭包，使得后续工具调用
    // （如 craft_3x3）无法找到已打开的容器。用 std::mem::forget 阻止 Drop，
    // 容器由调用方（close_container_if_open 或新的 ensure_table_open）负责关闭。
    let handle = bot
        .open_container_at(effective_pos)
        .await
        .map_err(|e| format!("打开容器失败: {e:?}"))?;
    match handle {
        Some(h) => {
            std::mem::forget(h);
            Ok(format!("已打开容器 ({},{},{}){note}", effective_pos.x, effective_pos.y, effective_pos.z))
        }
        None => Err(format!(
            "({},{},{}) 处虽有容器方块但 open_container_at 返回 None——可能 bot 视线被阻挡或服务端拒绝。{note}\
             建议：goto 到容器旁换个角度再 open。",
            effective_pos.x, effective_pos.y, effective_pos.z
        )),
    }
}

/// 判断方块是否为容器（可被 open_container_at 打开）。
fn is_container_block(k: BlockKind) -> bool {
    use azalea_registry::builtin::BlockKind as B;
    matches!(
        k,
        B::CraftingTable
            | B::Furnace
            | B::BlastFurnace
            | B::Smoker
            | B::Chest
            | B::EnderChest
            | B::Barrel
            | B::ShulkerBox
            | B::WhiteShulkerBox
            | B::OrangeShulkerBox
            | B::MagentaShulkerBox
            | B::LightBlueShulkerBox
            | B::YellowShulkerBox
            | B::LimeShulkerBox
            | B::PinkShulkerBox
            | B::GrayShulkerBox
            | B::LightGrayShulkerBox
            | B::CyanShulkerBox
            | B::PurpleShulkerBox
            | B::BlueShulkerBox
            | B::BrownShulkerBox
            | B::GreenShulkerBox
            | B::RedShulkerBox
            | B::BlackShulkerBox
            | B::BrewingStand
            | B::Dispenser
            | B::Dropper
            | B::Hopper
            | B::Lectern
            | B::SmithingTable
            | B::Stonecutter
            | B::CartographyTable
            | B::Loom
            | B::Grindstone
            | B::Anvil
            | B::ChippedAnvil
            | B::DamagedAnvil
    )
}

fn normalize_item(item: &str) -> String {
    if item.starts_with("minecraft:") {
        item.to_string()
    } else {
        format!("minecraft:{item}")
    }
}

/// P11 新增：扫描 origin 附近 radius 格半径，找最近的容器方块（crafting_table/furnace/chest等）。
///
/// 用于 do_open_container 的 fallback：当 LLM 给的目标坐标不是容器时，
/// 自动在附近找最近的可打开容器，避免 LLM 因坐标漂移/记忆错误陷入 open→fail→open→fail 死循环。
///
/// 返回最近的容器 BlockPos；找不到返回 None。
fn find_nearby_container_block(bot: &Client, origin: BlockPos, radius: i32) -> Option<BlockPos> {
    let world = bot.world().ok()?;
    let world = world.read();
    let mut candidates: Vec<(i32, BlockPos)> = Vec::new();
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                if dx == 0 && dy == 0 && dz == 0 {
                    continue; // 跳过 origin 本身（已确认不是容器）
                }
                let pos = BlockPos::new(origin.x + dx, origin.y + dy, origin.z + dz);
                let kind = world
                    .get_block_state(pos)
                    .map(|s| s.into());
                if let Some(bk) = kind {
                    if is_container_block(bk) {
                        // 曼哈顿距离作优先级，距离近的优先
                        let dist = dx.abs() + dy.abs() + dz.abs();
                        candidates.push((dist, pos));
                    }
                }
            }
        }
    }
    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by_key(|(d, _)| *d);
    candidates.first().map(|(_, p)| *p)
}

/// P11 新增：在 origin 附近 radius 格半径内扫描一个合法放置点（air + 下方 solid）。
///
/// 与 `find_valid_placement_nearby` 相同，但允许自定义扫描半径。
/// 用于 3 格半径找不到时的扩大搜索（地下空间狭窄，常需要 5 格）。
fn find_valid_placement_nearby_radius(bot: &Client, origin: BlockPos, radius: i32) -> Option<BlockPos> {
    let world = bot.world().ok()?;
    let world = world.read();
    let bot_pos = bot.position().ok();
    let (bot_x, bot_y, bot_z) = if let Some(bp) = bot_pos {
        (bp.x.floor() as i32, bp.y.floor() as i32, bp.z.floor() as i32)
    } else {
        (i32::MIN, i32::MIN, i32::MIN)
    };
    let mut candidates: Vec<(i32, BlockPos)> = Vec::new();
    for dy in -radius..=radius {
        let y = origin.y + dy;
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                if dx == 0 && dz == 0 && dy == 0 {
                    continue;
                }
                let pos = BlockPos::new(origin.x + dx, y, origin.z + dz);
                // 排除 bot 自身占据的两格
                if pos.x == bot_x && pos.z == bot_z && (pos.y == bot_y || pos.y == bot_y + 1) {
                    continue;
                }
                let is_air = world
                    .get_block_state(pos)
                    .map(|s| s.is_air())
                    .unwrap_or(false);
                if !is_air {
                    continue;
                }
                let below = pos.down(1);
                let below_solid = world
                    .get_block_state(below)
                    .map(|s| !s.is_air())
                    .unwrap_or(false);
                if below_solid {
                    let dist = dx.abs() + dz.abs() + dy.abs();
                    candidates.push((dist, pos));
                }
            }
        }
    }
    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by_key(|(d, _)| match *d {
        2 => 0,
        3 => 1,
        1 => 2,
        4 => 3,
        5 => 4,
        _ => 5,
    });
    candidates.first().map(|(_, p)| *p)
}

/// P5 新增：在 origin 附近（3 格半径内）扫描一个合法放置点（air + 下方 solid）。
///
/// 当 LLM 给的 pos 无效（非空气 / 下方不实心 / bot 自身占据）时调用本函数自动重定位。
/// 扫描顺序：先同层周围 3 格，再上下层。返回 None 表示附近找不到合法点。
///
/// **P5 修复**：排除 bot 自身占据的 foot+head 两格，避免选到 bot 自己位置。
fn find_valid_placement_nearby(bot: &Client, origin: BlockPos) -> Option<BlockPos> {
    // 注意：这里用同步世界读取，不能 .await；放到 spawn_blocking 或直接同步读。
    // azalea 的 world() 返回 RwLockReadGuard，可以同步读。
    let world = bot.world().ok()?;
    let world = world.read();
    // bot 当前占据的格（foot + head）
    let bot_pos = bot.position().ok();
    let (bot_x, bot_y, bot_z) = if let Some(bp) = bot_pos {
        (bp.x.floor() as i32, bp.y.floor() as i32, bp.z.floor() as i32)
    } else {
        (i32::MIN, i32::MIN, i32::MIN) // 无法读取时不会匹配任何位置
    };
    let mut candidates: Vec<(i32, BlockPos)> = Vec::new();
    for dy in &[0, 1, -1] {
        let y = origin.y + dy;
        for dx in -3..=3 {
            for dz in -3..=3 {
                if dx == 0 && dz == 0 && *dy == 0 {
                    continue; // 跳过 origin 本身（已知无效）
                }
                let pos = BlockPos::new(origin.x + dx, y, origin.z + dz);
                // 排除 bot 自身占据的两格（foot + head）
                if pos.x == bot_x && pos.z == bot_z && (pos.y == bot_y || pos.y == bot_y + 1) {
                    continue;
                }
                let is_air = world
                    .get_block_state(pos)
                    .map(|s| s.is_air())
                    .unwrap_or(false);
                if !is_air {
                    continue;
                }
                let below = pos.down(1);
                let below_solid = world
                    .get_block_state(below)
                    .map(|s| !s.is_air())
                    .unwrap_or(false);
                if below_solid {
                    // 距离 origin 曼哈顿距离作优先级，距离 2 最佳（不太近也不太远）
                    let dist = dx.abs() + dz.abs() + dy.abs();
                    candidates.push((dist, pos));
                }
            }
        }
    }
    if candidates.is_empty() {
        return None;
    }
    // 优先距离 2（origin 旁边一格），次选距离 3，再次选距离 1
    candidates.sort_by_key(|(d, _)| match *d {
        2 => 0,
        3 => 1,
        1 => 2,
        _ => 3,
    });
    candidates.first().map(|(_, p)| *p)
}

/// P5 新增：用 pathfinder 把 bot 走到 pos 旁 1.5m 内（reach 范围内）。
///
/// 用于 do_place 前置：LLM 常给远距离坐标 place，原代码不走路，
/// block_interact 因 reach 检查失败而静默无效，导致 place 100% 失败。
/// 最多等 5 秒，超时也不报错（让后续距离检查自己判定）。
async fn walk_to_reach_for_place(bot: &Client, pos: BlockPos) {
    use azalea::pathfinder::goals::RadiusGoal;
    use azalea::Vec3;
    let p = match bot.position() {
        Ok(p) => p,
        Err(_) => return,
    };
    let dx = p.x - pos.x as f64;
    let dy = p.y - pos.y as f64;
    let dz = p.z - pos.z as f64;
    let dist = (dx * dx + dy * dy + dz * dz).sqrt();
    // 已经在 reach 范围内（<4m），不需要走
    if dist < 4.0 {
        return;
    }
    // 用 RadiusGoal 走到 pos 旁 1.5m 范围
    let target = Vec3::new(pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5);
    let goto_fut = bot.goto(RadiusGoal { pos: target, radius: 1.5 });
    let _ = tokio::time::timeout(Duration::from_secs(5), goto_fut).await;
}
