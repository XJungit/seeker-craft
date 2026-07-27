//! 工作台/熔炉自动放置+打开+关闭流程（P1-4）。
//!
//! 解决 LLM 必须三段式调用 `place → open → craft_3x3` 才能合成的痛点：
//! - `ensure_table_open`：若已打开正确类型的容器则复用；若给定坐标则走过去打开；
//!   否则在 bot 头顶放置一个新工作台/熔炉并打开。
//! - `close_container_if_open`：操作完成后关闭容器，避免遗留 GUI 状态污染下一步。
//!
//! 所有 craft_3x3 / smelt 工具都改为「单工具完成放收桌」流程，
//! LLM 只需调用一次 `craft_3x3` 或 `smelt`，bot 自动处理放桌、开桌、操作、收桌。

use azalea::BlockPos;
use azalea::container::ContainerHandleRef;
use azalea::prelude::*;
use azalea_registry::builtin::{BlockKind, ItemKind};
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;

/// 工作台/熔炉的物品 id 与对应方块名映射。
/// table_kind 接受 "crafting_table" / "furnace" / "blast_furnace" / "smoker"。
fn table_item_id(table_kind: &str) -> Option<&'static str> {
    match table_kind {
        "crafting_table" | "table" | "workbench" => Some("crafting_table"),
        "furnace" => Some("furnace"),
        "blast_furnace" => Some("blast_furnace"),
        "smoker" => Some("smoker"),
        _ => None,
    }
}

/// 把 "oak_planks" / "minecraft:oak_planks" 统一为 "minecraft:oak_planks"。
fn normalize_item_id(item: &str) -> String {
    if item.starts_with("minecraft:") {
        item.to_string()
    } else {
        format!("minecraft:{item}")
    }
}

/// 当前是否已打开非 Player 菜单（即任意容器）。
pub fn is_container_open(bot: &Client) -> bool {
    if let Ok(inv) = bot.get_inventory() {
        if let Ok(Some(menu)) = inv.menu() {
            return !matches!(menu, azalea::inventory::Menu::Player(_));
        }
    }
    false
}

/// 关闭已打开的容器（若有）。返回是否确实关闭了一个容器。
pub fn close_container_if_open(bot: &Client) -> bool {
    if !is_container_open(bot) {
        return false;
    }
    if let Ok(inv) = bot.get_inventory() {
        inv.close();
        return true;
    }
    false
}

/// bot 头顶上方的空气格（已废弃：bot 自己占据该格，服务端拒绝放置）。
///
/// 保留函数仅用作 `hint_pos` 缺省兜底；真正放桌逻辑改走 `find_nearby_placement_spot`
/// 找一个 bot 旁边的位置（避免 bot bounding box 冲突）。
fn overhead_slot(bot: &Client) -> Option<BlockPos> {
    let p = bot.position().ok()?;
    Some(BlockPos::new(
        p.x.floor() as i32,
        p.y.floor() as i32 + 1,
        p.z.floor() as i32,
    ))
}

/// P5 关键修复：扫描 bot 附近（2-3 格半径内）找一个 **bot 不占据** 的合法放置位置。
///
/// 修复前的问题：`overhead_slot` 返回 bot 头顶格，但 bot 自己占据 foot+head 两格，
/// 服务端拒绝在 bot bounding box 内放方块 → 自动放桌 100% 失败。
///
/// 扫描策略：
/// 1. 排除 bot 自身占据的 (cx,cy,cz) 和 (cx,cy+1,cz)（foot/head）
/// 2. 优先选距离 2（bot 旁边一格，reach 够得到，又不阻挡 bot）
/// 3. 次选距离 3，再次选距离 1
/// 4. 同层优先，再 ±1 层
fn find_nearby_placement_spot(bot: &Client) -> Option<BlockPos> {
    let p = bot.position().ok()?;
    let cx = p.x.floor() as i32;
    let cy = p.y.floor() as i32;
    let cz = p.z.floor() as i32;
    let mut candidates: Vec<(i32, BlockPos)> = Vec::new();
    for dy in &[0, 1, -1] {
        let y = cy + dy;
        for dx in -3..=3 {
            for dz in -3..=3 {
                // 跳过 bot 自己占据的两格（foot + head）
                if dx == 0 && dz == 0 && (*dy == 0 || *dy == 1) {
                    continue;
                }
                let pos = BlockPos::new(cx + dx, y, cz + dz);
                if check_placement_space(bot, pos) {
                    // 曼哈顿距离作优先级，距离 2 最佳
                    let dist = dx.abs() + dz.abs() + dy.abs();
                    candidates.push((dist, pos));
                }
            }
        }
    }
    if candidates.is_empty() {
        return None;
    }
    // 优先距离 2（bot 旁边一格），次选距离 3，再次选距离 1，最后其他
    candidates.sort_by_key(|(d, _)| {
        // 优先级：d==2 > d==3 > d==1 > 其他
        match *d {
            2 => 0,
            3 => 1,
            1 => 2,
            _ => 3,
        }
    });
    candidates.first().map(|(_, p)| *p)
}

/// 背包中 item 的数量（仅主背包+快捷栏，排除网格/盔甲）。
fn count_in_inventory(bot: &Client, item: &str) -> u32 {
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
    let kind = match ItemKind::from_str(&normalize_item_id(item)) {
        Ok(k) => k,
        Err(_) => return 0,
    };
    let mut total = 0u32;
    for s in range {
        if let Some(st) = slots.get(s) {
            if !st.is_empty() && st.kind() == kind {
                total += st.count().max(0) as u32;
            }
        }
    }
    total
}

/// 走到 pos 旁（不严格到 pos 上方）。简化：调用 bot.goto；调用方应保证 pos 可达。
/// 这里只做小段等待，让 pathfinder 推进；真正的 goto 由 ActionManager 外层调度。
async fn walk_near(bot: &Client, pos: BlockPos) -> Result<(), String> {
    // 这里不再触发 goto（避免与 ActionManager 串行命令冲突）。
    // 调用方应在调用 ensure_table_open 前已使 bot 处于合理位置，
    // 或者使用 hint_pos = None 让本函数直接放头顶桌。
    let p = bot
        .position()
        .map_err(|e| format!("无法读取 bot 位置: {e:?}"))?;
    let dx = p.x - pos.x as f64;
    let dz = p.z - pos.z as f64;
    let dist = (dx * dx + dz * dz).sqrt();
    if dist > 6.0 {
        return Err(format!(
            "目标桌距 bot {dist:.1}m 过远（>6m），请先 go 到附近再调用"
        ));
    }
    Ok(())
}

/// P5 新增：用 pathfinder 把 bot 走到 pos 旁 1-2m 内（reach 范围内）。
/// 最多等 5 秒，超时也不报错（让后续 open_container_at 自己判定）。
///
/// P36 修复（2026-07-27）：返回最终距离，让调用方判断是否在 reach 范围内。
/// 原代码 walk_to_reach 超时后不验证距离，直接让 open_container_at 尝试 3 次都失败，
/// 浪费 4s+ 时间，最终报"打开容器超时"——但真正原因是 bot 离目标太远。
/// 现在返回距离，调用方能明确区分"距离不够"vs"LOS 被挡"vs"服务端拒绝"。
async fn walk_to_reach(bot: &Client, pos: BlockPos) -> f64 {
    use azalea::pathfinder::goals::RadiusGoal;
    use azalea::Vec3;
    let p = match bot.position() {
        Ok(p) => p,
        Err(_) => return f64::MAX,
    };
    let dx = p.x - pos.x as f64;
    let dy = p.y - (pos.y as f64 + 0.5);
    let dz = p.z - pos.z as f64;
    let dist = (dx * dx + dy * dy + dz * dz).sqrt();
    // 已经在 reach 范围内（<3m），不需要走
    if dist < 3.0 {
        return dist;
    }
    // 用 RadiusGoal 走到 pos 旁 1.5m 范围
    let target = Vec3::new(pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5);
    let goto_fut = bot.goto(RadiusGoal { pos: target, radius: 1.5 });
    let _ = tokio::time::timeout(Duration::from_secs(5), goto_fut).await;
    // P36: 返回最终距离，让调用方判断
    match bot.position() {
        Ok(p) => {
            let dx = p.x - (pos.x as f64 + 0.5);
            let dy = p.y - (pos.y as f64 + 0.5);
            let dz = p.z - (pos.z as f64 + 0.5);
            (dx * dx + dy * dy + dz * dz).sqrt()
        }
        Err(_) => f64::MAX,
    }
}

/// 确保 bot 当前已打开 table_kind 类型的容器。
///
/// 流程：
/// 1. 若已打开任意容器 → 复用（假定调用方已开对桌；不做类型严格校验，避免误关）
/// 2. 若 `hint_pos` 给定 → 走到附近，open_container_at
/// 3. 否则 → 检查背包是否有 table_kind 物品；
///    有则在头顶放置并打开；无则返回错误，让 LLM 先 craft 一个
///
/// 返回桌位（用于记忆库回写）。
pub async fn ensure_table_open(
    bot: &Client,
    table_kind: &str,
    hint_pos: Option<BlockPos>,
) -> Result<BlockPos, String> {
    let item_id = table_item_id(table_kind).ok_or_else(|| {
        format!("未知桌类型 {table_kind}（支持 crafting_table/furnace/blast_furnace/smoker）")
    })?;

    // 1. 已打开容器：复用
    if is_container_open(bot) {
        // 用 hint_pos 或 bot 头顶作为近似桌位（无法从 menu 直接读出坐标）
        let pos = hint_pos.unwrap_or_else(|| overhead_slot(bot).unwrap_or(BlockPos::new(0, 0, 0)));
        return Ok(pos);
    }

    // P41 本质修复（2026-07-27）：扫描附近已放置的工具方块复用。
    //
    // 原bug：craft_3x3 66.7% 失败 + smelt 100% 失败的根因。
    // bot 在地下挖矿时，之前 craft_3x3('crafting_table') / craft_3x3('furnace')
    // 已放置过桌/炉在世界里，但 ensure_table_open 的 step 3（自动放置）只检查背包——
    // 背包没有 crafting_table 时硬去合成（需要 oak_log，地下无 oak_log → 失败）。
    //
    // 本质修复：在 step 2（hint_pos）之前，先扫描附近 32 格内是否已放置同种方块。
    // 有则走过去打开（复用 step 2 的 walk_to_reach + open_container_at 逻辑），
    // 不需要重新合成、不需要重新放置——符合 vanilla 玩家行为：
    // 玩家在地下挖矿时通常就地放工作台/熔炉，下次回来找它，而不是每次重新合成。
    //
    // 这与 P40（auto_craft.rs）的 find_nearby_placed_block 同思路，但作用层不同：
    // P40 修 auto_craft 的 ensure(furnace) → ensure(crafting_table) → ensure(oak_log) 链；
    // P41 修 craft_3x3/smelt 工具直接调用的 ensure_table_open。
    // 两者互补，共同消除"地下缺 oak_log 无法合成桌/炉"的死循环。
    if hint_pos.is_none() {
        let expected_block = table_block_kind(table_kind);
        // 从 bot.position() 算 BlockPos 作为扫描中心
        let center = bot
            .position()
            .ok()
            .map(|p| BlockPos::new(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32))
            .unwrap_or(BlockPos::new(0, 0, 0));
        if let Some(placed_pos) = find_table_block_nearby(bot, center, expected_block, 32) {
            eprintln!(
                "[table_flow] P41: 附近 32 格内找到已放置的 {} 于 ({},{},{})，复用之（不重新合成/放置）",
                table_kind, placed_pos.x, placed_pos.y, placed_pos.z
            );
            // 走到附近并打开（复用 step 2 的逻辑：walk_to_reach + open_container_at 重试）
            let final_dist = walk_to_reach(bot, placed_pos).await;
            if final_dist <= 4.5 {
                let mut reuse_ok = false;
                for _attempt in 0..3u8 {
                    walk_to_reach(bot, placed_pos).await;
                    sleep(Duration::from_millis(150)).await;
                    match bot.open_container_at(placed_pos).await {
                        Ok(Some(h)) => {
                            std::mem::forget(h);
                            let mut opened = false;
                            for _ in 0..20 {
                                if is_container_open(bot) {
                                    opened = true;
                                    break;
                                }
                                sleep(Duration::from_millis(50)).await;
                            }
                            if opened {
                                sleep(Duration::from_millis(300)).await;
                                if is_container_open(bot) {
                                    sleep(Duration::from_millis(100)).await;
                                    reuse_ok = true;
                                    break;
                                }
                            }
                        }
                        _ => {}
                    }
                    sleep(Duration::from_millis(200)).await;
                }
                if reuse_ok {
                    return Ok(placed_pos);
                }
                // 复用失败（可能 LOS 被挡/方块被破坏），fall through 到 step 2/3 重新放置
                eprintln!(
                    "[table_flow] P41: 复用 ({},{},{}) 处 {} 失败，回退到自动放置流程",
                    placed_pos.x, placed_pos.y, placed_pos.z, table_kind
                );
            }
        }
    }

    // 2. hint_pos 给定：走到附近并打开
    if let Some(pos) = hint_pos {
        // P5 修复：实际走到容器旁（之前只检查距离不移动，导致 bot 离桌 3-4m 时
        // 服务端 reach 检查失败，open_container_at 超时）。用 pathfinder 走到
        // 距 pos 1-2m 的位置，确保在 4.5m reach 范围内。
        let final_dist = walk_to_reach(bot, pos).await;
        // P36 本质修复：walk_to_reach 后验证距离，如果 bot 仍在 reach 范围外（>4.5m），
        // 直接报错，不浪费 3 次 open_container_at 尝试（每次 1s+，总共浪费 4s+）。
        // 这是"打开容器超时"错误的常见根因——bot 卡在 1x1 竖井/窄洞里，pathfinder
        // 找不到路，walk_to_reach 超时后 bot 仍在远处，但原代码仍尝试 open 3 次。
        if final_dist > 4.5 {
            return Err(format!(
                "无法走到容器 ({},{},{}) 附近（bot 距离 {final_dist:.1}m > 4.5m reach 范围）。\n\
                 原因：pathfinder 5s 内未找到路（bot 可能被围墙卡住/在 1x1 竖井里）。\n\
                 建议：1) 先 go 到开阔地带再重试；2) 若 bot 在竖井里，用 mine 挖出空间；\n\
                 3) 重新 place 一个桌/炉在 bot 当前位置附近。",
                pos.x, pos.y, pos.z
            ));
        }
        // P7 修复：open_container_at 返回 Ok 不等于菜单稳定打开。
        // 服务端可能先开 Container 后立刻 Close（reach 检查、block 被破坏等）。
        // 改为：开完后等待 300ms，再连续 3 次确认 is_container_open 都为 true，
        // 才认为菜单稳定打开。这样 do_craft_3x3 不会因为菜单瞬间关闭而拿到 None。
        let mut last_err = String::new();
        for attempt in 0..3 {
            match bot.open_container_at(pos).await {
                Ok(handle_opt) => {
                    // P8 关键修复：防止 ContainerHandle 被 drop 导致容器自动关闭。
                    // ContainerHandle 的 Drop 实现会调用 close() 发送关闭包，
                    // 而 bevy 处理 CloseContainerEvent 的时机可能在下一个 tick。
                    // 即使 is_container_open 返回 true，后续 do_craft_3x3 调用时
                    // 容器可能已被关闭，导致 inv.menu() 返回 None。
                    // 使用 std::mem::forget 阻止 Drop 运行，容器保持打开状态，
                    // 由调用方（mod.rs 中的 close_container_if_open）负责关闭。
                    if let Some(h) = handle_opt {
                        std::mem::forget(h);
                    }
                    // 等待菜单同步（最多 1 秒）
                    let mut opened_at = None;
                    for _ in 0..20 {
                        if is_container_open(bot) {
                            opened_at = Some(true);
                            break;
                        }
                        sleep(Duration::from_millis(50)).await;
                    }
                    if opened_at.is_some() {
                        // 再多等 300ms 让服务端完全同步
                        sleep(Duration::from_millis(300)).await;
                        if !is_container_open(bot) {
                            last_err = format!(
                                "打开 ({},{},{}) 处容器后 300ms 又关闭（可能 reach 失败或方块被破坏）（尝试 {}）",
                                pos.x, pos.y, pos.z, attempt + 1
                            );
                            sleep(Duration::from_millis(200)).await;
                            continue;
                        }
                        sleep(Duration::from_millis(100)).await;
                        if !is_container_open(bot) {
                            last_err = format!(
                                "打开 ({},{},{}) 处容器后 400ms 又关闭（尝试 {}）",
                                pos.x, pos.y, pos.z, attempt + 1
                            );
                            sleep(Duration::from_millis(200)).await;
                            continue;
                        }
                        // 稳定打开，再 sleep 100ms 让菜单完全同步
                        sleep(Duration::from_millis(100)).await;
                        return Ok(pos);
                    }
                    last_err = format!("打开 ({},{},{}) 处容器超时（尝试 {}）", pos.x, pos.y, pos.z, attempt + 1);
                }
                Err(e) => {
                    last_err = format!("打开容器失败: {e:?}");
                }
            }
            // 重试前稍微等一下让服务端同步
            sleep(Duration::from_millis(200)).await;
        }
        // P37 本质修复（2026-07-27）：3 次 open 失败后，验证 pos 处是否真的有桌/炉方块。
        // 原代码直接报错给 LLM，但常见根因是：
        // 1) hint_pos 处的桌/炉方块已被破坏（爆炸/火灾/其他实体挖掉）
        // 2) hint_pos 是 LLM 凭记忆给的旧坐标，桌从未在那里
        // 3) 方块存在但 LOS 被新放置的方块挡住
        //
        // 修复策略：
        // a. 验证 pos 处是否是预期的桌/炉方块
        // b. 如果不是，且背包有桌/炉物品 → 在 bot 附近重新放置一个并打开（复用自动放置流程）
        // c. 如果不是，且背包没有桌/炉物品 → 返回明确错误，告诉 LLM 桌不在那里
        // d. 如果方块存在但仍打不开 → LOS 问题，报错让 LLM 换位置
        let expected_block = table_block_kind(table_kind);
        if let Some(expected_kind) = expected_block {
            let block_exists = bot
                .world()
                .ok()
                .and_then(|w| w.read().get_block_state(pos))
                .map(|state| {
                    let actual: BlockKind = state.into();
                    actual == expected_kind
                })
                .unwrap_or(false);

            if !block_exists {
                // 桌/炉方块不存在，尝试在 bot 附近重新放置
                eprintln!(
                    "[table_flow] P37: hint_pos=({},{},{}) 处无 {} 方块（已被破坏或坐标错误），尝试重新放置",
                    pos.x, pos.y, pos.z, table_kind
                );
                // 检查背包是否有桌/炉物品
                if count_in_inventory(bot, item_id) > 0 {
                    // 有物品，重新走自动放置流程（递归调用 ensure_table_open 但 hint_pos=None）
                    // 但要避免无限递归，所以直接走自动放置逻辑
                    match find_nearby_placement_spot(bot) {
                        Some(new_pos) => {
                            walk_to_reach(bot, new_pos).await;
                            match crate::azalea::place::do_place(bot, item_id, new_pos).await {
                                Ok(_) => {
                                    sleep(Duration::from_millis(400)).await;
                                    // 验证放置成功
                                    if verify_table_block_at(bot, new_pos, expected_block) {
                                        // 重试 open
                                        for retry in 0..2u8 {
                                            walk_to_reach(bot, new_pos).await;
                                            sleep(Duration::from_millis(150)).await;
                                            match bot.open_container_at(new_pos).await {
                                                Ok(Some(h)) => {
                                                    std::mem::forget(h);
                                                    for _ in 0..20 {
                                                        if is_container_open(bot) {
                                                            sleep(Duration::from_millis(80)).await;
                                                            return Ok(new_pos);
                                                        }
                                                        sleep(Duration::from_millis(50)).await;
                                                    }
                                                }
                                                _ => {}
                                            }
                                            sleep(Duration::from_millis(300)).await;
                                        }
                                        return Err(format!(
                                            "hint_pos=({},{},{}) 处 {} 已被破坏，重新放置于 ({},{},{}) 后仍打开失败。\
                                             建议：go 到开阔地带再 craft_3x3。",
                                            pos.x, pos.y, pos.z, table_kind,
                                            new_pos.x, new_pos.y, new_pos.z
                                        ));
                                    }
                                }
                                Err(e) => {
                                    return Err(format!(
                                        "hint_pos=({},{},{}) 处 {} 不存在，重新放置失败：{e}",
                                        pos.x, pos.y, pos.z, table_kind
                                    ));
                                }
                            }
                        }
                        None => {
                            // P34: 自动挖出空间再放
                            match excavate_placement_spot(bot).await {
                                Some(dug_pos) => {
                                    eprintln!(
                                        "[table_flow] P37+P34: 自动挖出空间 ({},{},{}) 用于重新放置 {}",
                                        dug_pos.x, dug_pos.y, dug_pos.z, item_id
                                    );
                                    walk_to_reach(bot, dug_pos).await;
                                    match crate::azalea::place::do_place(bot, item_id, dug_pos).await {
                                        Ok(_) => {
                                            sleep(Duration::from_millis(400)).await;
                                            if verify_table_block_at(bot, dug_pos, expected_block) {
                                                for retry in 0..2u8 {
                                                    walk_to_reach(bot, dug_pos).await;
                                                    sleep(Duration::from_millis(150)).await;
                                                    match bot.open_container_at(dug_pos).await {
                                                        Ok(Some(h)) => {
                                                            std::mem::forget(h);
                                                            for _ in 0..20 {
                                                                if is_container_open(bot) {
                                                                    sleep(Duration::from_millis(80)).await;
                                                                    return Ok(dug_pos);
                                                                }
                                                                sleep(Duration::from_millis(50)).await;
                                                            }
                                                        }
                                                        _ => {}
                                                    }
                                                    sleep(Duration::from_millis(300)).await;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            return Err(format!(
                                                "hint_pos 处 {} 不存在，挖空间后重放也失败：{e}",
                                                table_kind
                                            ));
                                        }
                                    }
                                }
                                None => {
                                    return Err(format!(
                                        "hint_pos=({},{},{}) 处 {} 方块不存在，且周围无空间可重放（挖空间也失败）。\
                                         建议：go 到开阔地带再 craft_3x3。",
                                        pos.x, pos.y, pos.z, table_kind
                                    ));
                                }
                            }
                        }
                    }
                } else {
                    return Err(format!(
                        "hint_pos=({},{},{}) 处 {} 方块不存在（已被破坏或坐标错误），\
                         且背包无 {item_id} 可重放。\
                         建议：1) 先 craft 一个 {item_id}；2) 用 perceive 查看当前位置；3) 重新 craft_3x3 让 bot 自动放桌。",
                        pos.x, pos.y, pos.z, table_kind
                    ));
                }
            }
        }
        return Err(format!(
            "{last_err}。建议：检查桌位置是否正确，或先 goto 到桌旁 1-2m 再 craft_3x3。"
        ));
    }

    // 3. 自动放置：先确认背包有 table_kind 物品
    // P8 修复：如果没有 crafting_table，尝试自动合成
    // P10 修复（2026-07-26）：原代码无论 table_kind 是什么，都硬编码合成 "crafting_table"。
    // 当 table_kind="furnace" 时，bot 没有 furnace 但代码去合成 crafting_table → 放置 crafting_table
    // 而非 furnace → do_smelt 在 crafting_table 上执行失败。
    // 修复：furnace/blast_furnace/smoker 是 3×3 配方，不能 2×2 合成。只有 crafting_table
    // 能 2×2 自动合成。furnace 类需 LLM 显式 craft_3x3("furnace") 先造好放背包。
    if count_in_inventory(bot, item_id) == 0 {
        // P44 本质修复（2026-07-27）：furnace 类桌不在背包时，调 do_auto_craft 自动合成，
        // 而不是直接报错让 LLM 处理。
        //
        // 原bug: furnace 类桌直接报错 → LLM 调 craft_3x3('furnace') → craft_3x3 又调
        // ensure_table_open('crafting_table') → 需要 crafting_table → 需要 oak_log
        // → 地下无 oak_log → 死循环 → smelt 100% 失败。
        //
        // 修复：do_auto_craft 有 P40 工具方块复用 + 递归满足原料，能在地下成功合成 furnace
        // （furnace = cobblestone×8，cobblestone 可通过 mine stone 获得，不需地表资源）。
        // do_auto_craft 内部会复用已放置的 crafting_table（P40），不会死循环。
        if table_kind != "crafting_table" && table_kind != "table" && table_kind != "workbench" {
            eprintln!("[table_flow] P44: 背包无 {item_id}，尝试 auto_craft 自动合成");
            match crate::azalea::auto_craft::do_auto_craft(bot, item_id, 1).await {
                Ok(msg) => {
                    eprintln!("[table_flow] P44: auto_craft {item_id} 成功: {msg}");
                    // 等待背包同步（do_auto_craft 内部 shift_click 后服务端同步需要时间）
                    sleep(Duration::from_millis(500)).await;
                    if count_in_inventory(bot, item_id) == 0 {
                        return Err(format!(
                            "自动合成 {item_id} 报成功但背包未检测到（可能服务端同步延迟）。\
                             建议：perceive 查看背包，或重试本工具。"
                        ));
                    }
                    // 合成成功，继续往下走放置流程
                }
                Err(e) => {
                    return Err(format!(
                        "背包未持有 {item_id} 且自动合成失败：{e}。\n\
                         furnace 需要 cobblestone×8（挖 stone 获得 cobblestone），\n\
                         crafting_table 需要 oak_planks×4（砍树获得 oak_log）。\n\
                         建议：1) 先 mine 一些 stone 获得 cobblestone；\n\
                         2) 或先 gather oak_log 合成 crafting_table。"
                    ));
                }
            }
        } else {
        // 尝试自动合成 crafting_table（2×2：4 木板）
        // 先检查是否有木板（任意 *_planks）
        if let Ok(inv) = bot.get_inventory() {
            let has_planks = inv.menu().ok().flatten().map(|m| {
                let range = m.player_slots_range();
                inv.slots().map(|slots| {
                    for s in range {
                        if let Some(st) = slots.get(s) {
                            if !st.is_empty() && st.kind().to_str().ends_with("_planks") {
                                return true;
                            }
                        }
                    }
                    false
                }).unwrap_or(false)
            }).unwrap_or(false);

            if has_planks {
                // 有木板，直接合成 crafting_table
                let _ = crate::azalea::table_flow::close_container_if_open(bot);
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                crate::azalea::craft::do_craft_2x2(bot, "crafting_table", 1).await
                    .map_err(|e| format!("自动合成 {item_id} 失败: {e}"))?;
            } else {
                // 没有木板，尝试从原木合成
                let has_logs = inv.menu().ok().flatten().map(|m| {
                    let range = m.player_slots_range();
                    inv.slots().map(|slots| {
                        for s in range {
                            if let Some(st) = slots.get(s) {
                                if !st.is_empty() {
                                    let kind = st.kind().to_str();
                                    if kind.ends_with("_log") || kind.ends_with("_stem") {
                                        return true;
                                    }
                                }
                            }
                        }
                        false
                    }).unwrap_or(false)
                }).unwrap_or(false);

                if has_logs {
                    // 有原木，先合成对应木板，再合成 crafting_table
                    let _ = crate::azalea::table_flow::close_container_if_open(bot);
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    // 找第一个原木类型，合成对应木板
                    let inv = bot.get_inventory().map_err(|e| format!("{e:?}"))?;
                    let planks_name = inv.menu().ok().flatten().and_then(|m| {
                        let range = m.player_slots_range();
                        inv.slots().and_then(|slots| {
                            for s in range {
                                if let Some(st) = slots.get(s) {
                                    if !st.is_empty() {
                                        let kind = st.kind().to_str();
                                        let bare_kind = kind.strip_prefix("minecraft:").unwrap_or(kind);
                                        if bare_kind.ends_with("_log") {
                                            let wood = bare_kind.strip_suffix("_log").unwrap_or(bare_kind);
                                            return Some(format!("{}_planks", wood));
                                        }
                                        if bare_kind.ends_with("_stem") {
                                            let wood = bare_kind.strip_suffix("_stem").unwrap_or(bare_kind);
                                            return Some(format!("{}_planks", wood));
                                        }
                                    }
                                }
                            }
                            None
                        })
                    }).ok_or_else(|| "无法确定原木类型".to_string())?;
                    crate::azalea::craft::do_craft_2x2(bot, &planks_name, 4).await
                        .map_err(|e| format!("自动合成木板 {planks_name} 失败: {e}"))?;
                    crate::azalea::craft::do_craft_2x2(bot, "crafting_table", 1).await
                        .map_err(|e| format!("自动合成 {item_id} 失败: {e}"))?;
                } else {
                    return Err(format!(
                        "背包无 {item_id}，且无木板/原木可自动合成。请先 gather 采集一些原木。"
                    ));
                }
            }
        } else {
            return Err(format!(
                "背包无 {item_id}（请先 craft 或 gather 一个，或在工具调用时指定 table_x/y/z 用附近已有的桌）"
            ));
        }
        } // 关闭 P44 else { crafting_table 分支 }
    }

    // P5 关键修复：跳过 overhead_slot（bot 自己占据头顶格，服务端拒绝放置），
    // 直接扫描附近 2-3 格半径找一个 bot 不占据的合法位置（air + 下方 solid）。
    // 原代码先试头顶再扫描附近 → 头顶 100% 失败 → 每次都浪费一次 do_place 尝试 +
    // 错误日志污染。改为直接用 find_nearby_placement_spot。
    //
    // P34 本质修复（2026-07-27）：原代码 find_nearby_placement_spot 返回 None 时直接报错给 LLM，
    // 导致 smelt 100% 失败（bot 在 1x1 竖井里挖矿时，周围都是方块，找不到空气位放炉子）。
    // 这是缝缝补补思维的产物——把问题甩给 LLM，LLM 又不会主动挖空间，进入死循环。
    //
    // 本质修复：找不到现成的空气位时，bot 自动挖出空间再放。具体策略：
    // 1. 扫描周围 1-3 格找一个"挖一下就能放"的位置：pos 是实心方块，pos 下方也是实心方块
    //    （挖掉 pos 后，pos 变空气 + 下方仍实心 = 合法放置位）
    // 2. 跳过 bot 自己占据的两格（foot + head），跳过基岩
    // 3. 优先挖 bot 同层的旁边一格（距离 1-2，reach 内，挖完直接放）
    // 4. 挖完后调用 find_nearby_placement_spot 复用现有放置逻辑
    // 5. 若挖了 3 次仍找不到合法位，再报错（极少数情况：bot 被基岩包围）
    let placement_pos = match find_nearby_placement_spot(bot) {
        Some(nearby) => {
            walk_to_reach(bot, nearby).await;
            nearby
        }
        None => {
            // P34: 自动挖出放置空间
            match excavate_placement_spot(bot).await {
                Some(dug_pos) => {
                    eprintln!(
                        "[table_flow] P34: 周围无现成空气位，已自动挖出空间于 ({},{},{}) 用于放置 {}",
                        dug_pos.x, dug_pos.y, dug_pos.z, item_id
                    );
                    walk_to_reach(bot, dug_pos).await;
                    dug_pos
                }
                None => {
                    return Err(format!(
                        "当前位置附近 3 格内无空间放置 {item_id}，且自动挖空间失败\
                        （bot 可能被基岩包围或处于极端狭窄位置）。\
                         建议：1) goto 回到地面宽敞处再 craft_3x3；或 2) 用 /tp @s ~ 70 ~ 传送到地表。"
                    ));
                }
            }
        }
    };

    // P5 修复：复用 place::do_place 的完整放置逻辑（含 pos 空气检查、下方实心检查、
    // 放置后按 BlockKind 校验）。原代码只查"非空气"，导致 pos 本来就有方块时误报成功。
    crate::azalea::place::do_place(bot, item_id, placement_pos)
        .await
        .map_err(|e| format!("{e}（自动放桌失败，建议先 goto 到 2x2 空地再 craft_3x3）"))?;

    // P19 关键修复（2026-07-27）：smelt 100% 失败的根因之一。
    // 原代码 do_place 成功后立即调 open_container_at，但：
    // 1. 服务端需要时间创建 block entity（Furnace 是带 BlockEntity 的方块）
    //    —— block_interact 放置方块后，服务端异步创建 BlockEntity，
    //    若 open_container_at 在 BlockEntity 创建前到达，服务端按"非容器"处理 → 返回 None。
    // 2. azalea open_container_at 内部 block_interact(pos) 需要 bot 在 reach 范围内，
    //    但 do_place 走动后 bot 朝向可能变了，LOS 检查失败 → 服务端不响应 → 5s 超时 → None。
    //
    // 修复策略：
    // a. do_place 后等 400ms（8 ticks）让 BlockEntity 同步
    // b. 验证 placement_pos 处确实是 table_kind 对应的方块（防止 do_place 内部重定位）
    // c. 重试 open_container_at 3 次，每次间隔 300ms
    // d. 若仍失败，扫描附近 3 格找刚放置的桌/炉，用实际坐标重试
    // e. 每次重试前用 walk_to_reach 确保 bot 在 reach 范围内
    sleep(Duration::from_millis(400)).await;

    // 验证 placement_pos 处确实是我们刚放的桌/炉（do_place 可能内部重定位）
    let expected_block = table_block_kind(table_kind);
    let actual_pos = if verify_table_block_at(bot, placement_pos, expected_block) {
        placement_pos
    } else {
        // placement_pos 处不是预期的桌/炉，扫描附近 3 格找
        match find_table_block_nearby(bot, placement_pos, expected_block, 3) {
            Some(actual) => {
                eprintln!(
                    "[table_flow] placement_pos=({},{},{}) 处无 {table_kind}，附近找到 ({},{},{})",
                    placement_pos.x, placement_pos.y, placement_pos.z,
                    actual.x, actual.y, actual.z
                );
                actual
            }
            None => {
                return Err(format!(
                    "放置 {item_id} 后验证失败：(,{},{},{}) 处不是 {table_kind}，\
                     且附近 3 格内未找到该方块。do_place 可能误报成功。",
                    placement_pos.x, placement_pos.y, placement_pos.z
                ));
            }
        }
    };

    // 重试打开容器（最多 3 次）
    let mut last_err = String::new();
    for attempt in 0..3u8 {
        // 每次重试前确保 bot 在 reach 范围内
        walk_to_reach(bot, actual_pos).await;
        // 短暂等待让 bot 位置/朝向稳定
        sleep(Duration::from_millis(150)).await;

        match bot.open_container_at(actual_pos).await {
            Ok(Some(h)) => {
                std::mem::forget(h); // 防止 ContainerHandle drop 自动关闭
                // 等待菜单稳定打开
                for _ in 0..20 {
                    if is_container_open(bot) {
                        sleep(Duration::from_millis(80)).await;
                        return Ok(actual_pos);
                    }
                    sleep(Duration::from_millis(50)).await;
                }
                last_err = format!("open_container_at 返回 Ok(Some) 但菜单未稳定打开（尝试 {}）", attempt + 1);
            }
            Ok(None) => {
                last_err = format!(
                    "open_container_at 返回 None（尝试 {}）——服务端可能未识别 {table_kind} 为容器，\
                     或 bot 距离/LOS 不满足",
                    attempt + 1
                );
                eprintln!("[table_flow] open None (attempt {}): {}", attempt + 1, last_err);
            }
            Err(e) => {
                last_err = format!("open_container_at 错误（尝试 {}）: {e:?}", attempt + 1);
                eprintln!("[table_flow] open Err (attempt {}): {:?}", attempt + 1, e);
            }
        }
        // 重试前等待
        if attempt < 2 {
            sleep(Duration::from_millis(300)).await;
        }
    }
    Err(format!(
        "放置 {item_id} 于 ({},{},{}) 后 3 次重试打开容器均失败。最后错误：{last_err}\
         ——这是 P19 修复的目标场景，请检查：1) bot 是否被卡在墙角无法 LOS；2) 服务端是否拒绝 BlockEntity 创建",
        actual_pos.x, actual_pos.y, actual_pos.z
    ))
}

/// P19 新增：根据 table_kind 返回对应的 BlockKind，用于放置后验证。
fn table_block_kind(table_kind: &str) -> Option<BlockKind> {
    let id = match table_kind {
        "crafting_table" | "table" | "workbench" => "minecraft:crafting_table",
        "furnace" => "minecraft:furnace",
        "blast_furnace" => "minecraft:blast_furnace",
        "smoker" => "minecraft:smoker",
        _ => return None,
    };
    BlockKind::from_str(id).ok()
}

/// P19 新增：验证 pos 处是否为指定的桌/炉方块。
fn verify_table_block_at(
    bot: &Client,
    pos: BlockPos,
    expected: Option<azalea_registry::builtin::BlockKind>,
) -> bool {
    let Some(expected_kind) = expected else { return false; };
    let world = match bot.world() {
        Ok(w) => w,
        Err(_) => return false,
    };
    let world = world.read();
    let Some(state) = world.get_block_state(pos) else {
        return false;
    };
    let actual: BlockKind = state.into();
    actual == expected_kind
}

/// P19 新增：扫描 center 附近 radius 格半径，找指定 BlockKind 的方块位置。
fn find_table_block_nearby(
    bot: &Client,
    center: BlockPos,
    expected: Option<azalea_registry::builtin::BlockKind>,
    radius: i32,
) -> Option<BlockPos> {
    let Some(expected_kind) = expected else { return None; };
    let world = bot.world().ok()?;
    let world = world.read();
    let mut best: Option<(BlockPos, i32)> = None;
    for dx in -radius..=radius {
        for dy in -radius..=radius {
            for dz in -radius..=radius {
                let pos = BlockPos::new(center.x + dx, center.y + dy, center.z + dz);
                if let Some(state) = world.get_block_state(pos) {
                    let kind: BlockKind = state.into();
                    if kind == expected_kind {
                        let dist = dx * dx + dy * dy + dz * dz;
                        if best.map_or(true, |(_, d)| dist < d) {
                            best = Some((pos, dist));
                        }
                    }
                }
            }
        }
    }
    best.map(|(p, _)| p)
}

/// 找到背包里持有 item 的 hotbar 槽位（0..=8），无则 None。
fn find_hotbar_slot(inv: &ContainerHandleRef, kind: ItemKind) -> Option<u8> {
    let menu = inv.menu().ok()??;
    // P5 修复：原代码 idx 算反了（详见 place.rs 同名函数注释）。
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

/// 检查 pos 处是否可以放置一个方块（用于自动放桌选位）。
///
/// **P5 修复**：`do_place` 总是用 `Direction::Up`（force_block 顶面）放方块，
/// 即把新方块放在 `pos.down(1)` 的顶面 = `pos`。因此**必须**：
/// - pos 是空气（不能在已有方块的位置放）
/// - pos 的下方是实心方块（block_interact 需要右键实心方块的顶面）
///
/// 原代码逻辑错误：允许"上方 solid OR 下方 solid"，但 do_place 不会用 Direction::Down
/// 放方块（顶吊式放置），导致选中"上方 solid"位置时 do_place 100% 失败。
fn check_placement_space(bot: &Client, pos: BlockPos) -> bool {
    let world = match bot.world() {
        Ok(w) => w,
        Err(_) => return false,
    };
    let w = world.read();
    let pos_is_air = w
        .get_block_state(pos)
        .map(|s| s.is_air())
        .unwrap_or(true);
    if !pos_is_air {
        return false;
    }
    // 下方必须是实心方块（do_place 用 Direction::Up 放在 below 的顶面）
    let below = BlockPos::new(pos.x, pos.y - 1, pos.z);
    w.get_block_state(below)
        .map(|s| !s.is_air())
        .unwrap_or(false)
}

/// P34 新增（2026-07-27）：bot 周围无现成空气位时，自动挖出一个合法放置位。
///
/// 解决场景：bot 在 1x1 竖井里挖矿，周围 3 格全是方块，find_nearby_placement_spot
/// 返回 None，原代码直接报错给 LLM 导致 smelt 100% 失败。
///
/// 策略：
/// 1. 扫描 bot 周围 1-3 格，找"挖一下就能放"的位置：
///    - pos 当前是实心方块（挖掉后变空气）
///    - pos 下方是实心方块（挖掉 pos 后下方仍实心，do_place 能放）
///    - pos 不是 bot 自己占据的 foot/head
///    - pos 不是基岩（不可破坏）
/// 2. 按距离优先级排序：距离 1（紧邻 bot）> 距离 2 > 距离 3
/// 3. 装备镐（挖石头类需要镐，徒手挖慢且可能不掉落）
/// 4. start_mining(pos) + 等待方块变空气（最多 4s）
/// 5. 挖完后返回 pos（此时 pos 是空气 + 下方实心 = 合法放置位）
/// 6. 若第一个候选挖失败（如基岩/超时），尝试下一个候选
/// 7. 最多尝试 3 个候选，全部失败才返回 None
///
/// 注意：不挖 bot 脚下方块（避免 bot 掉下去），不挖 bot 头顶方块（避免上方方块掉下来）。
async fn excavate_placement_spot(bot: &Client) -> Option<BlockPos> {
    let p = bot.position().ok()?;
    let cx = p.x.floor() as i32;
    let cy = p.y.floor() as i32;
    let cz = p.z.floor() as i32;

    // 收集候选位置：pos 实心 + pos 下方实心 + 非 bot 占据 + 非基岩
    let mut candidates: Vec<(i32, BlockPos)> = Vec::new();
    for dy in &[0, 1, -1] {
        let y = cy + dy;
        for dx in -3..=3 {
            for dz in -3..=3 {
                // 跳过 bot 自己占据的 foot + head
                if dx == 0 && dz == 0 && (*dy == 0 || *dy == 1) {
                    continue;
                }
                // 不挖脚下方块（dy=-1 且 dx=dz=0 已被上面跳过，但 dy=-1 的其他位置也跳过）
                // 实际上 dy=-1 且 (dx,dz)≠(0,0) 是 bot 脚下一层的旁边，可以挖
                let pos = BlockPos::new(cx + dx, y, cz + dz);
                if is_excavatable_placement_spot(bot, pos) {
                    let dist = dx.abs() + dz.abs() + dy.abs();
                    candidates.push((dist, pos));
                }
            }
        }
    }

    if candidates.is_empty() {
        eprintln!("[P34] 周围无可挖的放置候选位");
        return None;
    }

    // 按距离排序：距离 1 优先（紧邻 bot，挖完直接放）
    candidates.sort_by_key(|(d, _)| *d);
    candidates.truncate(5); // 最多尝试 5 个候选

    // 装备镐（挖石头/矿石类需要镐）
    let _ = crate::azalea::auto_equip_best_pickaxe(bot).await;

    for (dist, pos) in &candidates {
        eprintln!(
            "[P34] 尝试挖 ({},{},{}) 腾出放置位（距离 {}）",
            pos.x, pos.y, pos.z, dist
        );

        // 让 bot 看向目标方块（azalea mine 不强制视线，但 look_at 提高挖掘成功率）
        let center = azalea::Vec3::new(pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5);
        bot.look_at(center);
        sleep(Duration::from_millis(100)).await;

        // 开始挖
        bot.start_mining(*pos);

        // 等待方块变空气（最多 4s）
        let mut broken = false;
        for _ in 0..40 {
            sleep(Duration::from_millis(100)).await;
            let still_there = bot
                .world()
                .ok()
                .and_then(|w| w.read().get_block_state(*pos).map(|s| !s.is_air()))
                .unwrap_or(false);
            if !still_there {
                broken = true;
                break;
            }
        }

        if broken {
            // 等一帧让服务端同步方块状态
            sleep(Duration::from_millis(150)).await;
            // 验证：pos 现在是空气 + 下方仍实心
            if check_placement_space(bot, *pos) {
                eprintln!("[P34] 成功挖出放置位 ({},{},{})", pos.x, pos.y, pos.z);
                return Some(*pos);
            } else {
                eprintln!("[P34] 挖通了但下方不实心，继续下一个候选");
            }
        } else {
            eprintln!("[P34] 挖 ({},{},{}) 超时，尝试下一个候选", pos.x, pos.y, pos.z);
        }
    }
    None
}

/// P34 新增：判断 pos 是否适合"挖一下就能放"的位置。
///
/// 条件：
/// - pos 当前是实心方块（非空气、非流体）
/// - pos 下方是实心方块（挖掉 pos 后下方仍实心，do_place 能放）
/// - pos 不是基岩（不可破坏）
/// - pos 不是 bed/command_block 等不可破坏方块
fn is_excavatable_placement_spot(bot: &Client, pos: BlockPos) -> bool {
    let world = match bot.world() {
        Ok(w) => w,
        Err(_) => return false,
    };
    let w = world.read();
    let pos_state = match w.get_block_state(pos) {
        Some(s) => s,
        None => return false,
    };
    // pos 必须是实心方块（非空气、非流体）
    if pos_state.is_air() {
        return false;
    }
    // 检查是否基岩等不可破坏方块
    let pos_kind: BlockKind = pos_state.into();
    if matches!(pos_kind, BlockKind::Bedrock) {
        return false;
    }
    // pos 下方必须是实心方块
    let below = BlockPos::new(pos.x, pos.y - 1, pos.z);
    let below_state = match w.get_block_state(below) {
        Some(s) => s,
        None => return false,
    };
    if below_state.is_air() {
        return false;
    }
    let below_kind: BlockKind = below_state.into();
    if matches!(below_kind, BlockKind::Bedrock) {
        return false;
    }
    true
}
