//! 造全套铁盔甲 + 全套铁工具，然后全部放进箱子。
//!
//! 验证：GoalEngine 自动分解造出 8 件铁装备（甲4+工具4），再经 container
//! transfer(QUICK_MOVE) 一次性存入箱子。
//!
//! 运行（MC 已重启并加载新 jar）：
//! ```bash
//! cargo run -p craft-agent-minecraft --example gear_to_chest --features mod-bridge
//! ```

use craft_agent_minecraft::adapter_mod::MinecraftModAdapter;
use craft_agent_minecraft::bridge::{DEFAULT_PORT, ModCommand};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let adapter = Arc::new(Mutex::new(MinecraftModAdapter::connect("127.0.0.1", DEFAULT_PORT)?));
    println!("[gear_to_chest] connected.");

    let send_dbg = |a: &Arc<Mutex<MinecraftModAdapter>>, cmd: ModCommand| {
        let _ = a.lock().unwrap().send_debug(cmd);
    };
    let sleep = |ms: u64| std::thread::sleep(Duration::from_millis(ms));

    let give = |a: &Arc<Mutex<MinecraftModAdapter>>, item: &str, n: u32| {
        let _ = a.lock().unwrap().send_debug(ModCommand::DebugGive { item: item.to_string(), num: n });
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

    // 干净环境 + 原料（甲:5+8+7+4=24 ingot, 工具:3+3+2+1=9 ingot, stick链用原木）
    send_dbg(&adapter, ModCommand::DebugSetFixture { fixture: "platform".into() });
    sleep(400);
    give(&adapter, "raw_iron", 40);
    give(&adapter, "coal", 40);
    give(&adapter, "oak_log", 16);
    sleep(300);

    // 造 8 件铁装备
    let gear = [
        "iron_helmet", "iron_chestplate", "iron_leggings", "iron_boots",
        "iron_pickaxe", "iron_axe", "iron_sword", "iron_shovel",
    ];
    for g in gear {
        let _ = adapter.lock().unwrap().goal_execute("craft", g, 1);
        wait_goal(&adapter, 90);
        println!("[gear_to_chest] crafted {g}: x{}", count(&adapter, g));
    }
    let made: u32 = gear.iter().map(|g| count(&adapter, g)).sum();
    println!("[gear_to_chest] 共造出铁装备 x{made}（含可能尚未稳定计数的末件）");

    // 放箱子并打开 GUI
    send_dbg(&adapter, ModCommand::DebugPlace { block: "chest".into(), x: 1, y: 64, z: 0 });
    sleep(400);
    let _ = adapter.lock().unwrap().activate_block(1, 64, 0)?;
    sleep(500);
    let gui = adapter.lock().unwrap().inspect_gui()?;
    if !gui.has_gui.unwrap_or(false) {
        println!("[gear_to_chest] FAIL: 箱子 GUI 未打开");
        std::process::exit(1);
    }

    // 收集玩家侧所有铁装备槽位，一次性 shift 存入
    let slots = gui.slots.clone().unwrap_or(serde_json::Value::Null);
    let mut moves = Vec::new();
    if let Some(arr) = slots.as_array() {
        for s in arr {
            let id = s.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let side = s.get("side").and_then(|v| v.as_str()).unwrap_or("");
            if id.contains("iron_") && side == "player" {
                if let Some(idx) = s.get("slot_index").and_then(|v| v.as_u64()) {
                    moves.push(serde_json::json!({ "from": idx as u32, "to": null }));
                }
            }
        }
    }
    println!("[gear_to_chest] 准备存入 {} 个槽位", moves.len());
    let r = adapter.lock().unwrap().transfer(serde_json::Value::Array(moves))?;
    println!("[gear_to_chest] transfer: {}", r.detail);
    sleep(400);

    // 验证箱内铁装备数量
    let gui2 = adapter.lock().unwrap().inspect_gui()?;
    let slots2 = gui2.slots.clone().unwrap_or(serde_json::Value::Null);
    let mut in_chest = 0u32;
    if let Some(arr) = slots2.as_array() {
        for s in arr {
            let id = s.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let side = s.get("side").and_then(|v| v.as_str()).unwrap_or("");
            if id.contains("iron_") && side == "container" {
                in_chest += s.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            }
        }
    }
    let _ = adapter.lock().unwrap().close_gui()?;

    println!("[gear_to_chest] 箱子内铁装备 x{in_chest}（期望 8）");
    if in_chest >= 8 {
        println!("[gear_to_chest] PASS: 全套铁装备已存入箱子 (x{in_chest})");
        Ok(())
    } else {
        println!("[gear_to_chest] FAIL: 箱内 x{in_chest}");
        std::process::exit(1);
    }
}
