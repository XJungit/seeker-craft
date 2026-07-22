//! 让 bot 自动造全套铁盔甲（头盔/胸甲/腿甲/靴子），然后转交给玩家。
//!
//! 验证：LLM 只需"造全套铁甲给我"这种高层意图，mod 侧 GoalEngine 自动分解
//! （矿→raw→ingot→stick→各部位），造完再 give_player 交给真实玩家。
//!
//! 运行（MC 已重启并加载新 jar）：
//! ```bash
//! cargo run -p craft-agent-minecraft --example give_armor --features mod-bridge
//! ```

use craft_agent_minecraft::adapter_mod::MinecraftModAdapter;
use craft_agent_minecraft::bridge::{DEFAULT_PORT, ModCommand};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let adapter = Arc::new(Mutex::new(MinecraftModAdapter::connect("127.0.0.1", DEFAULT_PORT)?));
    println!("[give_armor] connected.");

    let send_dbg = |a: &Arc<Mutex<MinecraftModAdapter>>, cmd: ModCommand| {
        let _ = a.lock().unwrap().send_debug(cmd);
    };
    let give = |a: &Arc<Mutex<MinecraftModAdapter>>, item: &str, n: u32| {
        let _ = a.lock().unwrap().send_debug(ModCommand::DebugGive {
            item: item.to_string(),
            num: n,
        });
    };
    let goal_idle = |a: &Arc<Mutex<MinecraftModAdapter>>| -> bool {
        let s = a.lock().unwrap().goal_status().unwrap().detail;
        s.contains("idle") || s.contains("done") || s.contains("failed")
    };
    let wait_goal = |a: &Arc<Mutex<MinecraftModAdapter>>, secs: u64| {
        let start = Instant::now();
        while start.elapsed().as_secs() < secs {
            if goal_idle(a) { return; }
            std::thread::sleep(Duration::from_millis(500));
        }
    };
    let count = |a: &Arc<Mutex<MinecraftModAdapter>>, sub: &str| -> u32 {
        let st = a.lock().unwrap().reload().unwrap();
        st.inventory.iter().filter(|i| i.id.contains(sub)).map(|i| i.count).sum()
    };

    // 干净环境
    send_dbg(&adapter, ModCommand::DebugSetFixture { fixture: "platform".into() });
    std::thread::sleep(Duration::from_millis(400));

    // 给 bot 足够原料（全套铁甲：头盔5+胸甲8+腿甲7+靴子4 = 24 iron_ingot，加 stick 链用的原木）
    give(&adapter, "raw_iron", 30);
    give(&adapter, "coal", 30);
    give(&adapter, "oak_log", 16);
    std::thread::sleep(Duration::from_millis(300));

    // 自动造 4 件铁甲
    let parts = ["iron_helmet", "iron_chestplate", "iron_leggings", "iron_boots"];
    for p in parts {
        let _ = adapter.lock().unwrap().goal_execute("craft", p, 1);
        wait_goal(&adapter, 90);
        let n = count(&adapter, p);
        println!("[give_armor] crafted {p}: x{n}");
    }

    // 取玩家名
    let ack = adapter.lock().unwrap().list_players()?;
    let player_name = ack.players
        .and_then(|v| v.as_array().cloned())
        .and_then(|arr| arr.first().cloned())
        .and_then(|p| p.get("name").cloned())
        .and_then(|n| n.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "player".to_string());
    println!("[give_armor] target player = {player_name}");

    // 逐件交给玩家
    for p in parts {
        let have = count(&adapter, p);
        if have == 0 {
            println!("[give_armor] skip {p}: not in inventory");
            continue;
        }
        let r = adapter.lock().unwrap().give_player(&player_name, p, 1)?;
        println!("[give_armor] give {p} -> {}", r.detail);
        std::thread::sleep(Duration::from_millis(400));
    }

    println!("[give_armor] done. 玩家 {player_name} 请拾取掉落的铁甲。");
    Ok(())
}
