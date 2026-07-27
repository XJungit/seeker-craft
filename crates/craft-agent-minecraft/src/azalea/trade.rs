//! 村民交易（零侵入）：利用 azalea 已有的 `entity_interact` 打开村民，
//! 通过事件总线拿到 `MerchantOffers`，再用容器 click 完成交易。

use std::time::Duration;

use azalea::Client;
use azalea::entity::metadata::Villager;
use azalea::prelude::*;
use azalea_protocol::packets::game::s_select_trade::ServerboundSelectTrade;
use bevy_ecs::entity::Entity;
use bevy_ecs::query::With;

use crate::azalea::ext_state::SharedExt;

/// 找到最近的村民，返回其 bevy Entity（用于 `entity_interact`）。
pub fn find_nearest_villager(bot: &Client) -> Option<Entity> {
    let entities = bot.nearest_entities::<With<Villager>>().ok()?;
    entities.first().map(|e| e.id())
}

/// 打开最近的村民并等待交易报价下发。返回是否成功拿到报价。
async fn open_nearest_villager(bot: &Client, ext: &SharedExt) -> anyhow::Result<()> {
    // 先清掉旧报价，便于判断"新报价已到达"
    ext.lock().unwrap().merchant = None;
    let villager = find_nearest_villager(bot)
        .ok_or_else(|| anyhow::anyhow!("附近没有村民（需走到村民身边再交易）"))?;
    bot.entity_interact(villager);
    // 轮询等待服务端下发 MerchantOffers（通常很快）
    for _ in 0..20 {
        if ext.lock().unwrap().merchant.is_some() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(anyhow::anyhow!(
        "打开村民后未收到交易报价（村民可能不存在/距离过远/未加载）"
    ))
}

/// 执行一次村民交易：选第 `offer_index` 个报价并确认购买。
/// 需先靠近村民；本函数会自动打开村民（若尚未打开）。
pub async fn do_trade(bot: &Client, ext: &SharedExt, offer_index: u32) -> anyhow::Result<String> {
    // 确保已打开村民且报价就绪
    {
        let has = ext.lock().unwrap().merchant.is_some();
        if !has {
            open_nearest_villager(bot, ext).await?;
        }
    }
    let (result_item, result_count) = {
        let g = ext.lock().unwrap();
        let m = g
            .merchant
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("无村民交易报价"))?;
        let idx = offer_index as usize;
        let offer = m.offers.get(idx).ok_or_else(|| {
            anyhow::anyhow!("报价索引越界：{offer_index}，共 {} 个", m.offers.len())
        })?;
        if offer.out_of_stock {
            return Err(anyhow::anyhow!(
                "该报价已售罄（out_of_stock），需等待村民补货"
            ));
        }
        (offer.result.0, offer.result.1)
    };
    // 选交易（通知服务端准备该交易）
    bot.write_packet(ServerboundSelectTrade { item: offer_index });
    tokio::time::sleep(Duration::from_millis(150)).await;

    // 交易界面是 Merchant 容器菜单：payments=0..1, result=2
    let inv = bot
        .get_inventory()
        .map_err(|e| anyhow::anyhow!("获取交易容器失败（确认村民界面已打开）: {e:?}"))?;
    // 点击结果槽（slot 2）执行交易
    inv.click(azalea::inventory::operations::PickupClick::Left { slot: Some(2) });
    tokio::time::sleep(Duration::from_millis(200)).await;
    // 收回到背包
    inv.shift_click(2usize);
    tokio::time::sleep(Duration::from_millis(100)).await;

    Ok(format!(
        "村民交易完成：获得 {} x{}（报价 #{offer_index}）",
        result_item, result_count
    ))
}
