//! 实机校准（localhost:4444）：验证零侵入扩展层——配方书捕获 + 通用合成 + 村民交易。
//!
//! 运行：
//! ```bash
//! cargo run -p craft-agent-minecraft --example calibrate_4444 --features azalea-bot
//! ```

#[cfg(feature = "azalea-bot")]
fn main() -> anyhow::Result<()> {
    use craft_agent_minecraft::azalea::AzaleaBot;
    use std::time::Duration;
    use std::sync::Arc;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let bot = Arc::new(AzaleaBot::connect("localhost:4444", "CraftBot").await?);
        println!("[calibrate] 已连接，等待配方书下发...");

        // 等待配方书填充（服务端登录后不久下发）
        for i in 0..40 {
            let n = bot.ext.lock().unwrap().recipes.len();
            if n > 0 {
                println!("[calibrate] 配方书已捕获 {n} 条（{i} 轮）");
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let n = bot.ext.lock().unwrap().recipes.len();
        println!("[calibrate] 当前配方书大小: {n}");
        if n > 0 {
            // 抽样列出前几个产物
            let sample: Vec<String> = bot
                .ext
                .lock()
                .unwrap()
                .recipes
                .keys()
                .into_iter()
                .take(8)
                .collect();
            println!("[calibrate] 样例产物: {sample:?}");
        }

        // 测通用合成：先用 auto_craft 造 oak_planks（确保有木板）
        println!("[calibrate] 测试 auto_craft(\"oak_planks\", 8) ...");
        bot.auto_craft("oak_planks".to_string(), 8);
        tokio::time::sleep(Duration::from_millis(2500)).await;

        println!("[calibrate] 测试 auto_craft(\"chest\", 1)（应走配方书 3x3）...");
        bot.auto_craft("chest".to_string(), 1);
        tokio::time::sleep(Duration::from_millis(4000)).await;

        println!("[calibrate] 测试 auto_craft(\"crafting_table\", 1) ...");
        bot.auto_craft("crafting_table".to_string(), 1);
        tokio::time::sleep(Duration::from_millis(3000)).await;

        println!("[calibrate] 测试 auto_craft(\"smithing_table\", 1)（验证锻造路径解析）...");
        bot.auto_craft("smithing_table".to_string(), 1);
        tokio::time::sleep(Duration::from_millis(3000)).await;

        // 监听 bot 回显的事件（含 [自动合成]/[交易] 结果）
        println!("[calibrate] 监听 bot 事件 20s ...");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(500), bot.next_event()).await {
                Ok(Some(evt)) => println!("[bot-event] {evt:?}"),
                Ok(None) => break,
                Err(_) => continue,
            }
        }
        println!("[calibrate] 校准完成");
        Ok(())
    })
}
