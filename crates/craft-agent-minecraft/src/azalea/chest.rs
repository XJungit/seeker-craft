//! 容器（箱子/熔炉等）交互：查看 / 取出 / 存入。
//!
//! 学习自 Mindcraft library/skills.js 的 depositItemIntoChest / withdrawItemFromChest /
//! viewChest。三个操作都遵循同一流程：
//! 1. `open_container_at(pos)` 打开容器（服务端下发 ContainerCloseContent 等）
//! 2. 通过 `menu().player_slots_range()` 区分容器槽位（前段）与玩家槽位（后段）
//! 3. shift_click 在两段槽位间移动物品（服务端自动归并堆叠）
//! 4. `close()` 关闭容器（避免遗留 GUI 状态影响后续 craft/smelt）
//!
//! 设计要点：
//! - 一次工具调用完成「开→操作→关」，LLM 无需先 open 再操作
//! - shift_click 让服务端处理堆叠归并，比 left_click 手动拼堆更稳
//! - count=0 表示全部，count>0 表示按数量（不足时尽力而为）

use azalea::BlockPos;
use azalea::container::ContainerHandleRef;
use azalea::prelude::*;
use azalea_registry::builtin::ItemKind;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;

/// 把 "oak_planks" / "minecraft:oak_planks" 统一为 "minecraft:oak_planks"。
fn normalize_item_id(item: &str) -> String {
    if item.starts_with("minecraft:") {
        item.to_string()
    } else {
        format!("minecraft:{item}")
    }
}

/// 解析物品 id 为 ItemKind，兼容带/不带 minecraft: 前缀。
fn parse_item_kind(item: &str) -> Result<ItemKind, String> {
    ItemKind::from_str(&normalize_item_id(item))
        .or_else(|_| ItemKind::from_str(item))
        .map_err(|_| format!("未知物品 {item}"))
}

/// 等待容器菜单变为可用（open_container_at 异步返回后菜单可能还未同步）。
async fn wait_for_menu(bot: &Client) -> Option<ContainerHandleRef> {
    for _ in 0..20 {
        if let Ok(inv) = bot.get_inventory() {
            // 容器打开后，menu 是非 Player 的变体（Generic9x3 / Furnace / ...）
            if let Ok(Some(menu)) = inv.menu() {
                // Player 菜单表示没打开任何容器（只是自己的背包）
                if !matches!(menu, azalea::inventory::Menu::Player(_)) {
                    return Some(inv);
                }
            }
        }
        sleep(Duration::from_millis(50)).await;
    }
    None
}

/// 返回容器槽位范围（菜单中前段，即非玩家槽位）。
/// 容器槽位 = 0..player_slots_range.start()
fn container_slots_range(inv: &ContainerHandleRef) -> Option<std::ops::Range<usize>> {
    let menu = inv.menu().ok().flatten()?;
    let player_start = *menu.player_slots_range().start();
    Some(0..player_start)
}

/// 把容器槽位里的物品聚合为 "item_id:count, ..." 字符串（按数量降序）。
fn summarize_container(inv: &ContainerHandleRef) -> Option<String> {
    let slots = inv.slots()?;
    let range = container_slots_range(inv)?;
    let mut agg: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for s in range {
        if let Some(stack) = slots.get(s) {
            if stack.is_empty() {
                continue;
            }
            let kind = format!("{:?}", stack.kind()).to_lowercase();
            *agg.entry(kind).or_insert(0) += stack.count() as u32;
        }
    }
    if agg.is_empty() {
        return Some("空容器".to_string());
    }
    let mut items: Vec<(String, u32)> = agg.into_iter().collect();
    items.sort_by_key(|x| std::cmp::Reverse(x.1));
    Some(
        items
            .iter()
            .map(|(k, c)| format!("{k}:{c}"))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

/// 查看世界坐标 pos 处容器的物品列表。
/// 打开 → 读取容器槽位 → 关闭，返回 "iron_ingot:32, coal:16" 格式。
pub async fn do_chest_view(bot: &Client, pos: BlockPos) -> Result<String, String> {
    let handle = bot
        .open_container_at(pos)
        .await
        .map_err(|e| format!("打开容器失败: {e:?}"))?;
    if handle.is_none() {
        return Err(format!(
            "({},{},{}) 处无容器或无法打开",
            pos.x, pos.y, pos.z
        ));
    }
    sleep(Duration::from_millis(150)).await;
    let summary = match wait_for_menu(bot).await {
        Some(inv) => summarize_container(&inv).unwrap_or_else(|| "读取失败".into()),
        None => "容器菜单未就绪".to_string(),
    };
    // 关闭容器（ContainerHandle::close 消费 self，但这里只有引用；用 menu close）
    if let Ok(inv) = bot.get_inventory() {
        inv.close();
    }
    sleep(Duration::from_millis(100)).await;
    Ok(format!(
        "容器 ({},{},{}) 内容: {}",
        pos.x, pos.y, pos.z, summary
    ))
}

/// 从世界坐标 pos 处容器取出 item（count 个）到 bot 背包。
/// count=0 表示取全部。用 shift_click 让服务端归并堆叠。
pub async fn do_chest_withdraw(
    bot: &Client,
    pos: BlockPos,
    item: &str,
    count: u32,
) -> Result<String, String> {
    let kind = parse_item_kind(item)?;
    let handle = bot
        .open_container_at(pos)
        .await
        .map_err(|e| format!("打开容器失败: {e:?}"))?;
    if handle.is_none() {
        return Err(format!(
            "({},{},{}) 处无容器或无法打开",
            pos.x, pos.y, pos.z
        ));
    }
    let inv = wait_for_menu(bot)
        .await
        .ok_or_else(|| "容器菜单未就绪".to_string())?;
    let slots = inv.slots().ok_or_else(|| "读取容器槽位失败".to_string())?;
    let range = container_slots_range(&inv).ok_or_else(|| "无法确定容器槽位范围".to_string())?;

    // 收集所有匹配 item 的容器槽位
    let matching: Vec<usize> = range
        .filter(|&s| {
            slots
                .get(s)
                .map(|st| !st.is_empty() && st.kind() == kind)
                .unwrap_or(false)
        })
        .collect();
    if matching.is_empty() {
        if let Ok(inv) = bot.get_inventory() {
            inv.close();
        }
        return Ok(format!("容器内无 {item}（无需取出）"));
    }

    // 取出前的背包数量
    let before = count_in_player_slots(&inv, kind);
    let mut taken: u32 = 0;
    let mut remaining = count; // 0 表示全取
    for &s in &matching {
        if count != 0 && remaining == 0 {
            break;
        }
        let stack_count = slots.get(s).map(|st| st.count() as u32).unwrap_or(0);
        if stack_count == 0 {
            continue;
        }
        // shift_click 该容器槽位 → 服务端移动到玩家背包
        inv.shift_click(s);
        sleep(Duration::from_millis(80)).await;
        taken += stack_count;
        remaining = remaining.saturating_sub(stack_count);
    }
    // 等同步并统计实际拿到多少
    sleep(Duration::from_millis(150)).await;
    let after = bot
        .get_inventory()
        .ok()
        .map(|i| count_in_player_slots(&i, kind))
        .unwrap_or(before);
    let actual = after.saturating_sub(before);
    if let Ok(inv) = bot.get_inventory() {
        inv.close();
    }
    sleep(Duration::from_millis(100)).await;
    Ok(format!(
        "从容器 ({},{},{}) 取出 {item}：尝试 {taken}，实际 +{actual}（背包 {before} → {after}）",
        pos.x, pos.y, pos.z
    ))
}

/// 把背包中的 item（count 个）存入世界坐标 pos 处容器。
/// count=0 表示存全部。用 shift_click 让服务端归并堆叠到容器。
pub async fn do_chest_deposit(
    bot: &Client,
    pos: BlockPos,
    item: &str,
    count: u32,
) -> Result<String, String> {
    let kind = parse_item_kind(item)?;
    let handle = bot
        .open_container_at(pos)
        .await
        .map_err(|e| format!("打开容器失败: {e:?}"))?;
    if handle.is_none() {
        return Err(format!(
            "({},{},{}) 处无容器或无法打开",
            pos.x, pos.y, pos.z
        ));
    }
    let inv = wait_for_menu(bot)
        .await
        .ok_or_else(|| "容器菜单未就绪".to_string())?;
    let slots = inv.slots().ok_or_else(|| "读取槽位失败".to_string())?;
    let menu = inv
        .menu()
        .ok()
        .flatten()
        .ok_or_else(|| "菜单不可用".to_string())?;
    let player_range = menu.player_slots_range();

    // 收集所有匹配 item 的玩家槽位
    let matching: Vec<usize> = player_range
        .filter(|&s| {
            slots
                .get(s)
                .map(|st| !st.is_empty() && st.kind() == kind)
                .unwrap_or(false)
        })
        .collect();
    if matching.is_empty() {
        if let Ok(inv) = bot.get_inventory() {
            inv.close();
        }
        return Ok(format!("背包无 {item}（无需存入）"));
    }

    let before = count_in_player_slots(&inv, kind);
    let mut deposited: u32 = 0;
    let mut remaining = count; // 0 表示全存
    for &s in &matching {
        if count != 0 && remaining == 0 {
            break;
        }
        let stack_count = slots.get(s).map(|st| st.count() as u32).unwrap_or(0);
        if stack_count == 0 {
            continue;
        }
        inv.shift_click(s);
        sleep(Duration::from_millis(80)).await;
        deposited += stack_count;
        remaining = remaining.saturating_sub(stack_count);
    }
    sleep(Duration::from_millis(150)).await;
    let after = bot
        .get_inventory()
        .ok()
        .map(|i| count_in_player_slots(&i, kind))
        .unwrap_or(before);
    let actual = before.saturating_sub(after);
    if let Ok(inv) = bot.get_inventory() {
        inv.close();
    }
    sleep(Duration::from_millis(100)).await;
    Ok(format!(
        "存入容器 ({},{},{}) {item}：尝试 {deposited}，实际 -{actual}（背包 {before} → {after}）",
        pos.x, pos.y, pos.z
    ))
}

/// 统计玩家槽位（含 hotbar）里指定物品的总数。
fn count_in_player_slots(inv: &ContainerHandleRef, kind: ItemKind) -> u32 {
    let Some(slots) = inv.slots() else {
        return 0;
    };
    let Some(menu) = inv.menu().ok().flatten() else {
        return 0;
    };
    let range = menu.player_slots_range();
    slots
        .iter()
        .enumerate()
        .filter(|(i, _)| range.contains(i))
        .filter(|(_, s)| !s.is_empty() && s.kind() == kind)
        .map(|(_, s)| s.count() as u32)
        .sum()
}
