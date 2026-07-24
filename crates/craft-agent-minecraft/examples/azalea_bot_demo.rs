//! Phase 3 集成验证：用正式 `azalea` 模块（AzaleaBot）连入服并执行动作。
//!
//! 运行：`cargo run --example azalea_bot_demo --features azalea-bot 4444`
//! 行为：连入 -> 收 Spawn 事件 -> 挖脚下方块 + 发聊天 -> 打印状态快照。

use craft_agent_minecraft::azalea::AzaleaBot;
use craft_agent_minecraft::azalea::BotEvent;

#[tokio::main]
async fn main() {
    let port = std::env::args().nth(1).unwrap_or_else(|| "4444".to_string());
    let addr = format!("localhost:{port}");
    println!("[demo] 连接 {addr}");

    let bot = AzaleaBot::connect(&addr, "craftbot", None).await.expect("连接失败");
    println!("[demo] 句柄就绪，等待事件...");

    // 驱动：从事件流消费，连入后发指令。
    let bot = std::sync::Arc::new(bot);
    let bot_cmd = bot.clone();

    // 后台任务：收到 Spawn 后触发动作。
    let driver = {
        let bot = bot_cmd.clone();
        tokio::spawn(async move {
            // 给一点时间让 Spawn 到达
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            println!("[demo] 发指令：挖脚下方块 + 聊天");
            bot.mine_below();
            bot.chat("azalea module online");
        })
    };

    // 主循环：消费事件流
    while let Some(ev) = bot.next_event().await {
        match ev {
            BotEvent::Spawn { position } => {
                println!("[demo] ✓ 连入！坐标 = ({:.1},{:.1},{:.1})", position.x, position.y, position.z);
            }
            BotEvent::Chat { content } => {
                println!("[demo] 收到聊天: {content}");
            }
            BotEvent::State { position, inventory, player_count, yaw, pitch, block_under, block_ahead, health, food, saturation: _, held_item, biome, nearby, game_state: _ } => {
                println!(
                    "[demo] 状态: pos=({:.1},{:.1},{:.1}) yaw={:.0} pitch={:.0} hp={:.1}/{} food={}/{} held={} biome={} under={} ahead={} nearby=[{}] inv=[{}] players={}",
                    position.x, position.y, position.z, yaw, pitch,
                    health, "20", food, "20", held_item, biome,
                    block_under, block_ahead, nearby, inventory, player_count
                );
            }
            BotEvent::Disconnect { reason } => {
                println!("[demo] 断开: {reason}");
                break;
            }
        }
    }

    driver.abort();
    println!("[demo] 结束");
}
