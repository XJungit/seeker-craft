//! azalea 扩展层（零侵入）：通过 bevy Plugin + 游戏包事件总线，
//! 在不修改 azalea 源码的前提下，补上村民交易报价、配方书、实体缓存等
//! mineflayer / Mindcraft 已有而 azalea 高层未封装的能力。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use azalea_client::packet::game::ReceiveGamePacketEvent;
use azalea_protocol::packets::game::{ClientboundGamePacket, ClientboundMerchantOffers};
use azalea_registry::builtin::EntityKind;
use bevy_app::{App, Plugin, Update};
use bevy_ecs::message::MessageReader;
use bevy_ecs::prelude::*;

use crate::azalea::recipe_book::{RecipeBook, store_recipe_book_entry};

/// 跨系统/跨 handler 共享的扩展状态（挂在 ecs 资源上，handler 也能读）。
#[derive(Default, Clone)]
pub struct BotExtState {
    /// 最近一次村民交易报价（打开村民后由服务端下发）。
    pub merchant: Option<MerchantSnapshot>,
    /// 服务端下发的配方书（按产物 id 聚合，最后下发优先）。
    pub recipes: RecipeBook,
    /// 附近实体缓存（kind 便于交互时筛选村民/动物）。
    pub entities: HashMap<Entity, EntityKind>,
}

/// 村民交易报价快照（去掉了 Derive 负担，仅保留交易所需）。
#[derive(Clone, Debug)]
pub struct MerchantSnapshot {
    pub container_id: i32,
    pub offers: Vec<MerchantOfferSnapshot>,
    pub villager_level: u32,
    pub can_restock: bool,
}

#[derive(Clone, Debug)]
pub struct MerchantOfferSnapshot {
    pub input_a: (azalea_registry::builtin::ItemKind, i32),
    pub input_b: Option<(azalea_registry::builtin::ItemKind, i32)>,
    pub result: (azalea_registry::builtin::ItemKind, i32),
    pub out_of_stock: bool,
    pub max_uses: i32,
}

/// 共享扩展状态句柄。
pub type SharedExt = Arc<Mutex<BotExtState>>;

/// 包装成 bevy Resource，供插件 system 通过 `Res` 读取/写入。
#[derive(Resource, Clone, Default)]
pub struct BotExtResource(pub SharedExt);

/// 在 ecs 中注册的资源（handler 通过 `bot.ecs()` 取 `BotExtResource`）。
pub fn insert_ext_resource(app: &mut App, ext: SharedExt) {
    app.insert_resource(BotExtResource(ext));
}

/// 我们自己的 azalea 插件：注册读包 system，把村民报价/配方书写入共享状态。
pub struct CraftAgentPlugin {
    pub ext: SharedExt,
}

impl Plugin for CraftAgentPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(BotExtResource(self.ext.clone()));
        app.add_systems(Update, read_game_packets);
    }
}

/// 从 `ClientboundMerchantOffers` 抽取精简快照。
fn snapshot_merchant(p: &ClientboundMerchantOffers) -> MerchantSnapshot {
    let offers = p
        .offers
        .iter()
        .map(|o| MerchantOfferSnapshot {
            input_a: (o.base_cost_a.item, o.base_cost_a.count),
            input_b: o.cost_b.as_ref().map(|c| (c.item, c.count)),
            result: {
                let s = &o.result;
                (s.kind(), s.count())
            },
            out_of_stock: o.out_of_stock,
            max_uses: o.max_uses,
        })
        .collect();
    MerchantSnapshot {
        container_id: p.container_id,
        offers,
        villager_level: p.villager_level,
        can_restock: p.can_restock,
    }
}

/// 读包 system：监听所有收到的游戏包，挑出村民报价与配方书。
fn read_game_packets(mut events: MessageReader<ReceiveGamePacketEvent>, ext: Res<BotExtResource>) {
    for ev in events.read() {
        match ev.packet.as_ref() {
            ClientboundGamePacket::MerchantOffers(p) => {
                let snap = snapshot_merchant(p);
                ext.0.lock().unwrap().merchant = Some(snap);
            }
            ClientboundGamePacket::RecipeBookAdd(p) => {
                let mut g = ext.0.lock().unwrap();
                for e in &p.entries {
                    store_recipe_book_entry(&mut g.recipes, e);
                }
            }
            _ => {}
        }
    }
}
