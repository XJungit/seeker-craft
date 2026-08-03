//! 共享合成工具函数（被 craft_table/smelt/brew/enchant/smith 各域复用）。
use super::*;

/// 附魔（附魔台菜单，level 1/2/3 → slot 2/3/4）。
pub async fn do_enchant(bot: &Client, item: &str, level: u32) -> Result<String, String> {
    let opt_slot = match level.clamp(1, 3) {
        1 => 2usize,
        2 => 3usize,
        _ => 4usize,
    };
    let item_kind =
        ItemKind::from_str(&normalize_item(item)).map_err(|_| format!("未知物品 {item}"))?;
    let lapis_kind =
        ItemKind::from_str("lapis_lazuli").map_err(|_| "青金石 id 解析失败".to_string())?;

    let inv = bot
        .get_inventory()
        .map_err(|e| format!("获取容器失败（确认已打开附魔台）: {e:?}"))?;

    // 把待附魔物品放进 item 槽(0)
    let src_item =
        find_source_slot(&inv, item_kind).ok_or_else(|| format!("背包缺少待附魔物品 {item}"))?;
    move_stack(&inv, src_item, 0).await;
    // 把青金石放进 lapis 槽(1)
    let src_lapis = find_source_slot(&inv, lapis_kind)
        .ok_or_else(|| "背包缺少青金石 lapis_lazuli".to_string())?;
    move_stack(&inv, src_lapis, 1).await;

    // 等待服务端下发可用附魔选项
    sleep(Duration::from_millis(300)).await;

    // 点击所选附魔选项槽（普通左键），触发附魔（物品仍在 item 槽并带附魔）
    inv.click(PickupClick::Left {
        slot: Some(opt_slot as u16),
    });
    sleep(Duration::from_millis(200)).await;

    let enchanted = {
        let slots = inv.slots();
        slots
            .as_ref()
            .and_then(|s| s.first())
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    };
    if !enchanted {
        return Err(format!(
            "附魔 {item} 失败：物品槽为空（可能等级不足或青金不够）"
        ));
    }
    // 收回到背包
    inv.shift_click(0usize);
    sleep(Duration::from_millis(40)).await;

    Ok(format!("附魔 {item}（等级 {level}）完成"))
}
