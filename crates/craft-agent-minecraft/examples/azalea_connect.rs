//! Phase 2 POC：Azalea 客户端连入局域网 MC，验证「移动 + 挖矿」链路。
//!
//! 运行：`cargo run --example azalea_connect --features azalea-bot 4444`
//! 行为：连入 -> 走到目标 -> 到达后挖掉脚下方块 -> 打印挖矿状态。

use azalea::BlockPos;
use azalea::pathfinder::goals::BlockPosGoal;
use azalea::prelude::*;

/// 寻路目标：出生点附近偏移，验证 azalea pathfinder 在 26.2 能用。
const TARGET: BlockPos = BlockPos::new(10, 83, 10);

#[tokio::main]
async fn main() {
    let account = Account::offline("craftbot");
    let port = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "4444".to_string());
    let addr = format!("localhost:{port}");
    println!("[azalea_connect] 连接 {addr}...");

    let handler = |bot: Client, event: Event, _state: azalea::NoState| async move {
        match event {
            Event::Spawn => {
                println!("[azalea_connect] ✓ 已连入！bot 坐标 = {:?}", bot.position());
                // 聊天回传验证：发出一条测试消息（验证发送链路）。
                bot.chat("craftbot online - azalea POC ready");
            }
            // 聊天回传：收到游戏聊天时打印（验证接收链路，LLM 指令入口）。
            Event::Chat(packet) => {
                let content = match &packet {
                    azalea_client::client_chat::ChatPacket::System(p) => format!("{:?}", p.content),
                    _ => format!("{packet:?}"),
                };
                println!("[azalea_connect] 收到聊天: {content}");
            }
            Event::Tick => {
                let t = bot.ticks_connected();
                if t % 20 != 0 {
                    return bot;
                }
                if let Ok(p) = bot.position() {
                    // 挖脚下方块验证 mining 链路（仅在尚未在挖时触发一次）。
                    let mining = bot.is_mining();
                    if !mining {
                        let foot = BlockPos::new(
                            p.x.floor() as i32,
                            (p.y - 1.0).floor() as i32,
                            p.z.floor() as i32,
                        );
                        println!("[azalea_connect] t={t} 开始挖脚下方块 {foot:?}");
                        bot.start_mining(foot);
                    } else {
                        let foot = BlockPos::new(
                            p.x.floor() as i32,
                            (p.y - 1.0).floor() as i32,
                            p.z.floor() as i32,
                        );
                        println!("[azalea_connect] t={t} 挖矿进行中... 脚下方块 {foot:?}");
                    }
                }
                // 每 100 tick 打印一次状态感知（背包 + 附近实体，LLM 决策所需）。
                if t % 100 == 0 && t > 0 {
                    // 背包前 5 格
                    if let Ok(inv) = bot.get_inventory() {
                        if let Some(slots) = inv.slots() {
                            let summary: Vec<String> = slots
                                .iter()
                                .take(5)
                                .map(|s| {
                                    if s.is_empty() {
                                        "空".to_string()
                                    } else {
                                        format!("{:?}", s)
                                    }
                                })
                                .collect();
                            println!("[azalea_connect] t={t} 背包前5格: {:?}", summary);
                        } else {
                            println!("[azalea_connect] t={t} 背包 slots 为 None");
                        }
                    } else {
                        println!("[azalea_connect] t={t} 获取背包失败");
                    }
                    // 附近玩家实体（验证实体感知链路）
                    match bot.nearby_players() {
                        Ok(players) => {
                            let count = players.len();
                            let kinds: Vec<String> = players
                                .iter()
                                .map(|p| match p.kind() {
                                    Ok(k) => format!("{:?}", k),
                                    Err(_) => "?".to_string(),
                                })
                                .collect();
                            println!(
                                "[azalea_connect] t={t} 附近玩家实体数={count} kinds={kinds:?}"
                            );
                        }
                        Err(e) => println!("[azalea_connect] t={t} nearby_players 失败: {e:?}"),
                    }
                    // 放置验证：每 200 tick 对着脚下方块交互（block_interact，验证 place API 链路）。
                    if t % 200 == 0 && t > 0 {
                        if let Ok(p) = bot.position() {
                            let foot = BlockPos::new(
                                p.x.floor() as i32,
                                (p.y - 1.0).floor() as i32,
                                p.z.floor() as i32,
                            );
                            println!(
                                "[azalea_connect] t={t} 尝试放置(block_interact) 目标={foot:?}"
                            );
                            bot.block_interact(foot);
                        }
                    }
                }
            }
            Event::Disconnect(reason) => {
                println!("[azalea_connect] 断开：{reason:?}");
            }
            _ => {}
        }
        bot
    };

    let result = ClientBuilder::new()
        .set_handler(handler)
        .start(account, addr.as_str())
        .await;

    println!("[azalea_connect] bot 退出: {result:?}");
}
