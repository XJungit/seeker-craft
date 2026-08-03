//! 共享合成工具函数（被 craft_table/smelt/brew/enchant/smith 各域复用）。
use super::*;

/// 酿造（配方书驱动，blaze_powder 燃料）。
pub async fn do_brew(
    bot: &Client,
    recipe: &crate::azalea::recipe_book::StoredRecipe,
    count: u32,
) -> Result<String, String> {
    use crate::azalea::recipe_book::StoredRecipe;
    let (ingredient, base) = match recipe {
        StoredRecipe::Brewing {
            ingredient, base, ..
        } => (
            ingredient.items.first().copied(),
            base.items.first().copied(),
        ),
        _ => return Err("do_brew 仅支持 Brewing 配方".to_string()),
    };
    let ing_kind = ingredient.ok_or("酿造配方缺少原料".to_string())?;
    let base_kind = base.ok_or("酿造配方缺少基底（如 water_bottle）".to_string())?;
    let fuel_kind =
        ItemKind::from_str("blaze_powder").map_err(|_| "blaze_powder 解析失败".to_string())?;

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
