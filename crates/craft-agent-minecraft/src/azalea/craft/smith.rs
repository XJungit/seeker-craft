//! 共享合成工具函数（被 craft_table/smelt/brew/enchant/smith 各域复用）。
use super::*;

/// 锻造台与切石机合成（配方书驱动）。
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
        } => (
            template.items.first().copied(),
            base.items.first().copied(),
            addition.items.first().copied(),
        ),
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
        // P50 改进：对齐 do_craft_3x3 的 P20 验证逻辑。
        // 原 bug：shift_click(3) 后直接 made += 1，背包满时 shift_click 静默失败，made 虚增。
        let result_kind = match recipe {
            crate::azalea::recipe_book::StoredRecipe::Smithing { result, .. } => *result,
            _ => return Err("do_craft_smithing 仅支持 Smithing 配方".to_string()),
        };
        let before = count_item_in_player_slots(&inv, result_kind);
        inv.shift_click(3usize); // 取结果
        sleep(Duration::from_millis(150)).await;
        let after = count_item_in_player_slots(&inv, result_kind);
        if after > before {
            made += 1;
        } else {
            // shift_click 失败，用 left_click 兜底
            inv.left_click(3usize);
            sleep(Duration::from_millis(150)).await;
            let inv2 = match bot.get_inventory() {
                Ok(i) => i,
                Err(e) => return Err(format!("锻造 left_click 后读取背包失败: {e:?}")),
            };
            if let Some(empty) = find_empty_player_slot(&inv2) {
                inv2.left_click(empty);
                sleep(Duration::from_millis(150)).await;
            } else {
                return Err(
                    "锻造失败：背包完全满，产物无法收集。建议：先 discard 腾出空位".to_string(),
                );
            }
            let inv3 = match bot.get_inventory() {
                Ok(i) => i,
                Err(e) => return Err(format!("锻造验证时读取背包失败: {e:?}")),
            };
            let after2 = count_item_in_player_slots(&inv3, result_kind);
            if after2 > before {
                made += 1;
            } else {
                return Err(
                    "锻造失败：产物无法从结果槽移入背包（shift_click + left_click 均失败）"
                        .to_string(),
                );
            }
        }
    }
    Ok(format!("锻造合成 x{count} 完成（约 {made} 次）"))
}

/// 切石机合成。
///
/// 槽位布局（azalea `Menu::Stonecutter` 宏生成）：
/// - slot 0 = input（输入槽）
/// - slot 1 = result（结果槽）
/// - slot 2..=37 = 玩家背包
///
/// P51 修复（2026-07-27）：
/// 1. 原 code 把 input 放入 slot 1（结果槽），导致服务端永远算不出结果。
///    正确槽位是 slot 0（input slot）。
/// 2. 原 code 直接 `made += 1` 不验证 shift_click 是否真的收集到产物，
///    背包满时 made 虚增。对齐 P50 验证模式：count_item_in_player_slots
///    前后对比 + left_click 兜底 + 背包满明确报错。
/// 3. 原 code 在循环外只 fetch 一次 inv，循环内 find_source_slot 用 stale
///    数据 → 第二次找不到原料槽。改为每轮重新 fetch。
pub async fn do_craft_stonecutter(
    bot: &Client,
    recipe: &crate::azalea::recipe_book::StoredRecipe,
    count: u32,
) -> Result<String, String> {
    use crate::azalea::recipe_book::StoredRecipe;
    let (input_kind, result_kind) = match recipe {
        StoredRecipe::Stonecutter { input, result, .. } => (input.items.first().copied(), *result),
        _ => return Err("do_craft_stonecutter 仅支持 Stonecutter 配方".to_string()),
    };

    const INPUT_SLOT: usize = 0;
    const RESULT_SLOT: usize = 1;

    let mut made = 0u32;
    for round in 0..count.max(1) {
        // 每轮重新 fetch inventory，避免 stale state
        let inv = bot.get_inventory().map_err(|e| {
            format!(
                "获取容器失败（第 {} 轮，确认已打开切石机）: {e:?}",
                round + 1
            )
        })?;

        // 放原料到 input 槽（slot 0）
        if let Some(k) = input_kind {
            let src = find_source_slot(&inv, k)
                .ok_or_else(|| format!("背包缺少切石机原料 {k}（第 {} 轮）", round + 1))?;
            move_stack(&inv, src, INPUT_SLOT).await;
        }
        sleep(Duration::from_millis(80)).await;

        // 等服务端算结果（最多 2s）
        let mut has_result = false;
        for _ in 0..20 {
            sleep(Duration::from_millis(100)).await;
            let r = inv
                .slots()
                .as_ref()
                .and_then(|s| s.get(RESULT_SLOT))
                .map(|s| !s.is_empty())
                .unwrap_or(false);
            if r {
                has_result = true;
                break;
            }
        }
        if !has_result {
            return Err(format!(
                "切石失败：结果槽无产物（第 {} 轮，等待 2s slot {} 仍空，原料可能不足）",
                round + 1,
                RESULT_SLOT
            ));
        }

        // P51 验证：对齐 P50 模式，count_item_in_player_slots 前后对比
        let before_count = count_item_in_player_slots(&inv, result_kind);

        // 先 shift_click(RESULT_SLOT)
        inv.shift_click(RESULT_SLOT);
        sleep(Duration::from_millis(200)).await;
        let after_count = count_item_in_player_slots(&inv, result_kind);
        if after_count > before_count {
            made += 1;
            continue;
        }

        // shift_click 失败，用 left_click 兜底
        inv.left_click(RESULT_SLOT);
        sleep(Duration::from_millis(150)).await;
        let inv2 = match bot.get_inventory() {
            Ok(i) => i,
            Err(e) => return Err(format!("切石 left_click 后读取背包失败: {e:?}")),
        };
        match find_empty_player_slot(&inv2) {
            Some(empty) => {
                inv2.left_click(empty);
                sleep(Duration::from_millis(150)).await;
            }
            None => {
                return Err("切石失败：背包完全满，产物无法收集。\
                     建议：1) 先 discard 丢弃垃圾物品腾出空位；2) 关闭切石机再重新打开后重试。"
                    .to_string());
            }
        }
        let inv3 = match bot.get_inventory() {
            Ok(i) => i,
            Err(e) => return Err(format!("切石验证时读取背包失败: {e:?}")),
        };
        let after_count2 = count_item_in_player_slots(&inv3, result_kind);
        if after_count2 > before_count {
            made += 1;
            continue;
        }

        // 都失败了
        return Err(
            "切石失败：产物无法从结果槽移入背包（shift_click + left_click 均失败）。\
             建议：1) 先 discard 腾出空位；2) 关闭切石机再重新打开后重试。"
                .to_string(),
        );
    }
    Ok(format!("切石合成 x{count} 完成（约 {made} 次）"))
}
