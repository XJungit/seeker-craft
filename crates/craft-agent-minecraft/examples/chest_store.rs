//! 测试：bot 把背包里的物品放进箱子（container deposit）。
//!
//! 链路：place chest → activate_block 打开 GUI → inspect_gui 找玩家槽位
//!      → transfer(QUICK_MOVE) 存入箱子 → inspect_gui 验证箱内出现物品。
//!
//! 运行（MC 已重启并加载新 jar）：
//! ```bash
//! cargo run -p craft-agent-minecraft --example chest_store --features mod-bridge
//! ```

use craft_agent_minecraft::adapter_mod::MinecraftModAdapter;
use craft_agent_minecraft::bridge::{DEFAULT_PORT, ModCommand};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let adapter = Arc::new(Mutex::new(MinecraftModAdapter::connect("127.0.0.1", DEFAULT_PORT)?));
    println!("[chest_store] connected.");

    let send_dbg = |a: &Arc<Mutex<MinecraftModAdapter>>, cmd: ModCommand| {
        let _ = a.lock().unwrap().send_debug(cmd);
    };
    let sleep = |ms: u64| std::thread::sleep(Duration::from_millis(ms));

    // 干净环境 + 给 bot 一些要存的原木
    send_dbg(&adapter, ModCommand::DebugSetFixture { fixture: "platform".into() });
    sleep(400);
    let _ = a_dbg(&adapter, ModCommand::DebugGive { item: "oak_log".into(), num: 16 });
    sleep(300);

    // 1. 放箱子
    send_dbg(&adapter, ModCommand::DebugPlace { block: "chest".into(), x: 1, y: 64, z: 0 });
    sleep(400);

    // 2. 打开箱子 GUI（必须在 5m 内）
    let _ = adapter.lock().unwrap().activate_block(1, 64, 0)?;
    sleep(500);

    // 3. 检查 GUI 是否打开
    let gui = adapter.lock().unwrap().inspect_gui()?;
    let has_gui = gui.has_gui.unwrap_or(false);
    println!("[chest_store] has_gui={has_gui}");
    if !has_gui {
        println!("[chest_store] FAIL: 箱子 GUI 未打开");
        std::process::exit(1);
    }

    // 4. 找到玩家侧含 oak_log 的槽位
    let slots = gui.slots.clone().unwrap_or(serde_json::Value::Null);
    let player_slot = find_slot(&slots, "oak_log", "player");
    let player_slot = match player_slot {
        Some(s) => s,
        None => {
            println!("[chest_store] FAIL: 玩家背包里没找到 oak_log（slots={slots}）");
            std::process::exit(1);
        }
    };
    println!("[chest_store] 玩家槽位 {player_slot} 含 oak_log");

    // 5. 存入箱子（QUICK_MOVE: to=null）
    let moves = serde_json::json!([{ "from": player_slot, "to": null }]);
    let r = adapter.lock().unwrap().transfer(moves)?;
    println!("[chest_store] transfer: {}", r.detail);

    // 6. 再检查 GUI，验证箱内（container 侧）出现 oak_log
    sleep(300);
    let gui2 = adapter.lock().unwrap().inspect_gui()?;
    let slots2 = gui2.slots.clone().unwrap_or(serde_json::Value::Null);
    let chest_count = count_slot(&slots2, "oak_log", "container");
    println!("[chest_store] 箱子内 oak_log x{chest_count}");

    let _ = adapter.lock().unwrap().close_gui()?;

    if chest_count > 0 {
        println!("[chest_store] PASS: 物品已存入箱子 (x{chest_count})");
        Ok(())
    } else {
        println!("[chest_store] FAIL: 箱子内没有 oak_log");
        std::process::exit(1);
    }
}

fn a_dbg(a: &Arc<Mutex<MinecraftModAdapter>>, cmd: ModCommand) -> anyhow::Result<()> {
    a.lock().unwrap().send_debug(cmd)?;
    Ok(())
}

/// 在 slots JSON 数组里找 side==side_filter 且 id 含 sub 的第一个槽位 index。
fn find_slot(slots: &serde_json::Value, sub: &str, side_filter: &str) -> Option<u32> {
    let arr = slots.as_array()?;
    for s in arr {
        let id = s.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let side = s.get("side").and_then(|v| v.as_str()).unwrap_or("");
        if id.contains(sub) && side == side_filter {
            return s.get("slot_index").and_then(|v| v.as_u64()).map(|n| n as u32);
        }
    }
    None
}

fn count_slot(slots: &serde_json::Value, sub: &str, side_filter: &str) -> u32 {
    let arr = match slots.as_array() { Some(a) => a, None => return 0 };
    let mut total = 0u32;
    for s in arr {
        let id = s.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let side = s.get("side").and_then(|v| v.as_str()).unwrap_or("");
        if id.contains(sub) && side == side_filter {
            total += s.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        }
    }
    total
}
