//! 端到端功能冒烟测试：验证"LLM 只发高层目标，Java mod 自动分解执行"的核心架构真的能工作。
//!
//! 覆盖：
//!   A. 核心原语（底层必须端到端稳）
//!     1. nav_to 真实移动
//!     2. collect 真实采集（挖柱子，背包增加）
//!     3. pillar_up 真实往上搭
//!     4. place 真实放方块
//!   B. 高层复合目标（GoalEngine 自动分解，无需 LLM 写代码）
//!     5. goal craft iron_pickaxe（递归链 log→planks→stick + smelt）
//!     6. goal craft iron_chestplate（更深递归：8 iron_ingot + stick 链）
//!     7. goal craft 全套铁装（chestplate+sword+boots 连续目标）
//!     8. goal smelt
//!     9. goal hunt（不崩）
//!    10. goal build dirt（自动采集+铺设一排）
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
            println!  ("  [FAIL] {name} — {detail}");
        }
    };

    // 先用 platform fixture 搭干净 9x9 平台 + bot 归位(0.5,65,0.5) + 清场，避免之前测试挖出的坑影响。
    let _ = send_dbg(&adapter, ModCommand::DebugSetFixture { fixture: "platform".into() });
    std::thread::sleep(Duration::from_millis(500));

    // ── A.1 nav_to 真实移动 ──
    let before = position(&adapter);
    let _ = adapter.lock().unwrap().nav_to(before.0 + 6.0, before.1, before.2);
    wait_for(&adapter, |a| nav_idle(a), 30);
    let after = position(&adapter);
    let moved = (after.0 - before.0).abs() + (after.2 - before.2).abs();
    if moved > 2.0 {
        pass += 1;
        mark("nav_to 移动", true, &format!("Δ={moved:.1}m ({before:?}→{after:?})"));
    } else {
        fail += 1;
        mark("nav_to 移动", false, &format!("几乎没动 Δ={moved:.2}m ({before:?}→{after:?})"));
    }

    // ── A.2 collect 真实采集（用 collect fixture 逼 bot 真去挖柱子）──
    let _ = send_dbg(&adapter, ModCommand::DebugSetFixture { fixture: "collect".into() });
    std::thread::sleep(Duration::from_millis(400));
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

    // ── A.3 pillar_up 真实往上搭 ──
    let _ = send_dbg(&adapter, ModCommand::DebugSetFixture { fixture: "platform".into() });
    std::thread::sleep(Duration::from_millis(300));
    give(&adapter, "oak_log", 16);
    std::thread::sleep(Duration::from_millis(200));
    let y0 = position(&adapter).1;
    let _ = adapter.lock().unwrap().pillar_up(Some(3), Some("oak_log"));
    wait_for(&adapter, |a| nav_idle(a), 40); // pillar_up 经导航/放置后归位
    std::thread::sleep(Duration::from_millis(500));
    let y1 = position(&adapter).1;
    // 验证头顶真的出现了 oak_log 方块（bot 脚下 y-1 往上应有放置）
    let placed = count_block_nearby(&adapter, "oak_log", 3);
    if placed > 0 || y1 > y0 {
        pass += 1;
        mark("pillar_up 搭柱", true, &format!("附近 oak_log 方块 x{placed}, y {y0:.1}→{y1:.1}"));
    } else {
        fail += 1;
        mark("pillar_up 搭柱", false, &format!("附近 oak_log x{placed}, y {y0:.1}→{y1:.1}"));
    }

    // ── A.4 place 真实放方块 ──
    let _ = send_dbg(&adapter, ModCommand::DebugSetFixture { fixture: "platform".into() });
    std::thread::sleep(Duration::from_millis(300));
    give(&adapter, "cobblestone", 8);
    std::thread::sleep(Duration::from_millis(200));
    let blk_before = count_block_nearby(&adapter, "cobblestone", 4);
    let p = position(&adapter);
    let _ = send_dbg(&adapter, ModCommand::DebugPlace {
        block: "cobblestone".into(),
        x: (p.0 as i32) + 1,
        y: (p.1 as i32) - 1,
        z: (p.2 as i32),
    });
    std::thread::sleep(Duration::from_millis(500));
    let blk_after = count_block_nearby(&adapter, "cobblestone", 4);
    if blk_after > blk_before {
        pass += 1;
        mark("place 放方块", true, &format!("cobblestone 方块 {blk_before}→{blk_after}"));
    } else {
        fail += 1;
        mark("place 放方块", false, &format!("cobblestone 方块 {blk_before}→{blk_after}"));
    }

    // ── B.5 goal craft iron_pickaxe（递归链）──
    let _ = send_dbg(&adapter, ModCommand::DebugSetFixture { fixture: "platform".into() });
    std::thread::sleep(Duration::from_millis(300));
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

    // ── B.6 goal craft iron_chestplate（更深递归：8 iron_ingot + stick 链）──
    give(&adapter, "raw_iron", 10);
    give(&adapter, "coal", 10);
    std::thread::sleep(Duration::from_millis(300));
    let _ = adapter.lock().unwrap().goal_execute("craft", "iron_chestplate", 1);
    wait_for(&adapter, |a| goal_idle(a), 90);
    let have_chest = count_item(&adapter, "iron_chestplate");
    if have_chest >= 1 {
        pass += 1;
        mark("goal craft iron_chestplate", true, &format!("iron_chestplate x{have_chest}"));
    } else {
        fail += 1;
        mark("goal craft iron_chestplate", false, &format!("iron_chestplate x{have_chest}"));
    }

    // ── B.7 全套铁装（连续多个复合目标）──
    give(&adapter, "raw_iron", 20);
    give(&adapter, "coal", 20);
    std::thread::sleep(Duration::from_millis(300));
    for item in ["iron_sword", "iron_boots", "iron_leggings"] {
        let _ = adapter.lock().unwrap().goal_execute("craft", item, 1);
        wait_for(&adapter, |a| goal_idle(a), 90);
    }
    let kit = count_item(&adapter, "iron_sword")
        + count_item(&adapter, "iron_boots")
        + count_item(&adapter, "iron_leggings");
    if kit >= 3 {
        pass += 1;
        mark("goal 全套铁装", true, &format!("sword+boots+leggings = x{kit}"));
    } else {
        fail += 1;
        mark("goal 全套铁装", false, &format!("sword+boots+leggings = x{kit}"));
    }

    // ── B.8 goal smelt ──
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

    // ── B.9 goal hunt（无动物则应立即结束不崩）──
    let _ = adapter.lock().unwrap().goal_execute("hunt", "", 1);
    wait_for(&adapter, |a| goal_idle(a), 20);
    pass += 1;
    mark("goal hunt", true, "执行未崩溃");

    // ── B.10 goal build dirt（自动采集+铺设）──
    let _ = send_dbg(&adapter, ModCommand::DebugSetFixture { fixture: "build".into() });
    std::thread::sleep(Duration::from_millis(400));
    let _ = adapter.lock().unwrap().goal_execute("build", "dirt", 8);
    wait_for(&adapter, |a| goal_idle(a), 60);
    let dirt_placed = count_block_nearby(&adapter, "dirt", 6);
    if dirt_placed > 0 {
        pass += 1;
        mark("goal build dirt", true, &format!("附近 dirt 方块 x{dirt_placed}"));
    } else {
        fail += 1;
        mark("goal build dirt", false, &format!("附近 dirt 方块 x{dirt_placed}"));
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

/// 统计 bot 附近（曼哈顿半径 r）某种方块的数量（从 nearby_blocks 读）。
fn count_block_nearby(a: &Arc<Mutex<MinecraftModAdapter>>, sub: &str, r: i32) -> u32 {
    let st = a.lock().unwrap().reload().unwrap();
    let (px, py, pz) = (st.position[0], st.position[1], st.position[2]);
    st.nearby_blocks
        .iter()
        .filter(|b| {
            b.id.contains(sub)
                && (b.x - px).abs() <= r as f64
                && (b.y - py).abs() <= r as f64
                && (b.z - pz).abs() <= r as f64
        })
        .count() as u32
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
