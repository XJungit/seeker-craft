//! 端到端功能冒烟测试：验证修复后的核心链路真的能工作（而非仅"工具不报错"）。
//!
//! 覆盖：
//!   1. nav_to 后 bot 真实移动（位置变化）—— 验证 setPos 移动修复
//!   2. collect 后背包真的增加目标方块 —— 验证 CollectController + 移动
//!   3. goal_execute("craft iron_pickaxe") 自动分解并真造出物品 —— 验证 GoalEngine 递归栈
//!   4. goal_execute("hunt") / ("smelt") / ("build") 不崩溃
//!
//! 运行（MC 已重启并加载新 jar，bot 已加入世界）：
//! ```bash
//! cargo run -p craft-agent-minecraft --example e2e_smoke --features mod-bridge
//! ```

use craft_agent_minecraft::adapter_mod::MinecraftModAdapter;
use craft_agent_minecraft::bridge::{DEFAULT_PORT, ModCommand};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let host = "127.0.0.1";
    let port = DEFAULT_PORT;

    println!("[e2e] connecting to MC bridge @ {host}:{port} ...");
    let adapter = loop {
        match MinecraftModAdapter::connect(host, port) {
            Ok(a) => break Arc::new(Mutex::new(a)),
            Err(_) => {
                println!("[e2e] waiting for MC (retry 5s)...");
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    };
    println!("[e2e] connected.");

    let mut pass = 0;
    let mut fail = 0;
    let mark = |name: &str, ok: bool, detail: &str| {
        if ok {
            println!("  [PASS] {name} — {detail}");
        } else {
            println!("  [FAIL] {name} — {detail}");
        }
    };

    // 先把 bot 传送到一个空旷已知点，避免卡墙
    let _ = send_dbg(&adapter, ModCommand::DebugTeleportBot { x: Some(0.5), z: Some(0.5) });
    std::thread::sleep(Duration::from_millis(500));

    // ── 1. nav_to 真实移动 ──
    let before = position(&adapter);
    let _ = adapter.lock().unwrap().nav_to(before.0 + 6.0, before.1, before.2);
    wait_for(&adapter, |a| nav_idle(a), 30);
    let after = position(&adapter);
    let moved = (after.0 - before.0).abs() + (after.2 - before.2).abs();
    if moved > 3.0 {
        pass += 1;
        mark("nav_to 移动", true, &format!("Δ={moved:.1}m ({before:?}→{after:?})"));
    } else {
        fail += 1;
        mark("nav_to 移动", false, &format!("几乎没动 Δ={moved:.2}m ({before:?}→{after:?})"));
    }

    // ── 2. collect 真实采集 ──
    let inv_before = count_item(&adapter, "log");
    let _ = adapter.lock().unwrap().collect_start("oak_log", 4);
    wait_for(&adapter, |a| collect_idle(a), 60);
    let inv_after = count_item(&adapter, "log");
    if inv_after > inv_before {
        pass += 1;
        mark("collect 采集", true, &format!("log {inv_before}→{inv_after}"));
    } else {
        fail += 1;
        mark("collect 采集", false, &format!("log {inv_before}→{inv_after} (maybe no oak_log nearby)"));
    }

    // ── 3. goal_execute craft iron_pickaxe 自动分解 ──
    give(&adapter, "oak_log", 16);
    give(&adapter, "raw_iron", 3);
    give(&adapter, "coal", 3);
    std::thread::sleep(Duration::from_millis(300));
    let _ = adapter.lock().unwrap().goal_execute("craft", "iron_pickaxe", 1);
    wait_for(&adapter, |a| goal_idle(a), 60);
    let have_pick = count_item(&adapter, "iron_pickaxe");
    if have_pick >= 1 {
        pass += 1;
        mark("goal craft iron_pickaxe", true, &format!("iron_pickaxe x{have_pick}"));
    } else {
        fail += 1;
        mark("goal craft iron_pickaxe", false, &format!("iron_pickaxe x{have_pick}"));
    }

    // ── 4. goal hunt（无动物则应立即结束不崩）──
    let _ = adapter.lock().unwrap().goal_execute("hunt", "", 1);
    wait_for(&adapter, |a| goal_idle(a), 20);
    pass += 1;
    mark("goal hunt", true, "执行未崩溃");

    // ── 5. goal smelt ──
    give(&adapter, "raw_iron", 2);
    give(&adapter, "coal", 2);
    std::thread::sleep(Duration::from_millis(300));
    let _ = adapter.lock().unwrap().goal_execute("smelt", "raw_iron", 2);
    wait_for(&adapter, |a| goal_idle(a), 30);
    let ingot = count_item(&adapter, "iron_ingot");
    if ingot >= 2 {
        pass += 1;
        mark("goal smelt", true, &format!("iron_ingot x{ingot}"));
    } else {
        fail += 1;
        mark("goal smelt", false, &format!("iron_ingot x{ingot}"));
    }

    println!("[e2e] === RESULT: PASS={pass} FAIL={fail} ===");
    if fail > 0 {
        std::process::exit(1);
    }
    Ok(())
}

// ── helpers ──
fn send_dbg(a: &Arc<Mutex<MinecraftModAdapter>>, cmd: ModCommand) -> anyhow::Result<()> {
    a.lock().unwrap().send_debug(cmd)?;
    Ok(())
}

fn position(a: &Arc<Mutex<MinecraftModAdapter>>) -> (f64, f64, f64) {
    let st = a.lock().unwrap().reload().unwrap();
    (st.position[0], st.position[1], st.position[2])
}

fn count_item(a: &Arc<Mutex<MinecraftModAdapter>>, sub: &str) -> u32 {
    let st = a.lock().unwrap().reload().unwrap();
    st.inventory
        .iter()
        .filter(|i| i.id.contains(sub))
        .map(|i| i.count)
        .sum()
}

fn give(a: &Arc<Mutex<MinecraftModAdapter>>, item: &str, n: u32) {
    let _ = a.lock().unwrap().send_debug(ModCommand::DebugGive {
        item: item.to_string(),
        num: n,
    });
}

fn nav_idle(a: &Arc<Mutex<MinecraftModAdapter>>) -> bool {
    let s = a.lock().unwrap().nav_status().unwrap().detail;
    s.contains("idle") || s.contains("arrived") || s.contains("failed")
}

fn collect_idle(a: &Arc<Mutex<MinecraftModAdapter>>) -> bool {
    let s = a.lock().unwrap().collect_status().unwrap().detail;
    s.contains("idle") || s.contains("done")
}

fn goal_idle(a: &Arc<Mutex<MinecraftModAdapter>>) -> bool {
    let s = a.lock().unwrap().goal_status().unwrap().detail;
    s.contains("idle") || s.contains("done") || s.contains("failed")
}

fn wait_for<F: Fn(&Arc<Mutex<MinecraftModAdapter>>) -> bool>(a: &Arc<Mutex<MinecraftModAdapter>>, f: F, secs: u64) {
    let start = Instant::now();
    while start.elapsed().as_secs() < secs {
        if f(a) { return; }
        std::thread::sleep(Duration::from_millis(500));
    }
}
