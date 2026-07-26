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
async fn walk_to_reach(bot: &Client, pos: BlockPos) {
    use azalea::pathfinder::goals::RadiusGoal;
    use azalea::Vec3;
    let p = match bot.position() {
        Ok(p) => p,
        Err(_) => return,
    };
    let dx = p.x - pos.x as f64;
    let dz = p.z - pos.z as f64;
    let dist = (dx * dx + dz * dz).sqrt();
    // 已经在 reach 范围内（<3m），不需要走
    if dist < 3.0 {
        return;
    }
    // 用 RadiusGoal 走到 pos 旁 1.5m 范围
    // P5 修复：bot.goto() 返回 Future 必须await才执行；这里用 tokio::select 加超时
    // 避免goto future阻塞过久（pathfinder 找不到路时 future 不会结束）。
    let target = Vec3::new(pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5);
    let goto_fut = bot.goto(RadiusGoal { pos: target, radius: 1.5 });
    let _ = tokio::time::timeout(Duration::from_secs(5), goto_fut).await;
    // 即使 pathfinder 没完全走到，也尝试打开容器（可能已经足够近）
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

    // 2. hint_pos 给定：走到附近并打开
    if let Some(pos) = hint_pos {
        // P5 修复：实际走到容器旁（之前只检查距离不移动，导致 bot 离桌 3-4m 时
        // 服务端 reach 检查失败，open_container_at 超时）。用 pathfinder 走到
        // 距 pos 1-2m 的位置，确保在 4.5m reach 范围内。
        walk_to_reach(bot, pos).await;
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
        // furnace 类桌不能 2×2 合成，直接返回错误让 LLM 先 craft_3x3
        if table_kind != "crafting_table" && table_kind != "table" && table_kind != "workbench" {
            return Err(format!(
                "背包未持有 {item_id}（{table_kind} 是 3×3 合成物品，无法自动合成）。\
                 建议：先 craft_3x3('{item_id}') 合成一个（furnace 需要 cobblestone×8），\
                 再重新调用本工具。"
            ));
        }
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
    }

    // P5 关键修复：跳过 overhead_slot（bot 自己占据头顶格，服务端拒绝放置），
    // 直接扫描附近 2-3 格半径找一个 bot 不占据的合法位置（air + 下方 solid）。
    // 原代码先试头顶再扫描附近 → 头顶 100% 失败 → 每次都浪费一次 do_place 尝试 +
    // 错误日志污染。改为直接用 find_nearby_placement_spot。
    let placement_pos = match find_nearby_placement_spot(bot) {
        Some(nearby) => {
            // 走到附近位置旁（确保在 reach 范围内）
            walk_to_reach(bot, nearby).await;
            nearby
        }
        None => {
            return Err(format!(
                "当前位置附近 3 格内无空间放置 {item_id}（bot 可能在 1x1 竖井/窄洞里，或周围都站满方块）。\
                 建议：1) goto 回到地面宽敞处再 craft_3x3；或 2) mine 旁边一格挖出 2x1 空间后重试；\
                 3) 用 /tp @s ~ 70 ~ 传送到地表。"
            ));
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
