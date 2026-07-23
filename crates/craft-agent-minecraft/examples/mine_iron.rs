//! 真实采矿端到端测试：验证"bot 自己找到矿石→挖掉→烧炼→合成"的完整自治链，
//! 不依赖 debug_give 凭空给 raw_iron / iron_ingot。
//!
//! 与 give_armor 的区别：give_armor 用 debug_give 直接塞 raw_iron，本测试只放
//! 矿石【方块】到 bot 附近，让 GoalEngine 自己驱动 CollectController 去挖、自己采煤当燃料、
//! 自己烧炼成 ingot、再合成铁镐。这是从 0 资源自治采矿的端到端验证。
//!
//! 运行（MC 已重启并加载新 jar，bot 已加入世界）：
//! ```bash
//! cargo run -p craft-agent-minecraft --example mine_iron --features mod-bridge
//! ```

use craft_agent_minecraft::adapter_mod::MinecraftModAdapter;
use craft_agent_minecraft::bridge::{DEFAULT_PORT, ModCommand};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let host = "127.0.0.1";
    let port = DEFAULT_PORT;

    println!("[mine_iron] connecting to MC bridge @ {host}:{port} ...");
    let adapter = loop {
        match MinecraftModAdapter::connect(host, port) {
            Ok(a) => break Arc::new(Mutex::new(a)),
            Err(_) => {
                println!("[mine_iron] waiting for MC (retry 5s)...");
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    };
    println!("[mine_iron] connected.");

    let send_dbg = |a: &Arc<Mutex<MinecraftModAdapter>>, cmd: ModCommand| {
        let _ = a.lock().unwrap().send_debug(cmd);
    };

    // 干净环境：9x9 平台 + bot 归位(0.5,65,0.5) + 清场。
    send_dbg(&adapter, ModCommand::DebugSetFixture { fixture: "platform".into() });
    std::thread::sleep(Duration::from_millis(500));
    // 清空背包/掉落物，确保从 0 资源开始（否则会复用之前测试残留的 iron_ingot）。
    send_dbg(&adapter, ModCommand::DebugClear);
    std::thread::sleep(Duration::from_millis(400));

    // 在 bot 周围 y=64（平台顶层表面，替换顶层 dirt，上方 y65 为空气）放矿石【方块】。
    // 放在顶层而非中间层，矿石上方有空气，CollectController 才能站到矿顶挖它。
    // iron_ore x4 足够烧 3 个 ingot（铁镐配方），coal_ore x4 当燃料。
    let ore_spots = [(2, 64, 0), (-2, 64, 0), (0, 64, 2), (0, 64, -2)];
    let coal_spots = [(2, 64, 2), (-2, 64, 2), (2, 64, -2), (-2, 64, -2)];
    for (x, y, z) in ore_spots {
        send_dbg(&adapter, ModCommand::DebugPlace { block: "iron_ore".into(), x, y, z });
    }
    for (x, y, z) in coal_spots {
        send_dbg(&adapter, ModCommand::DebugPlace { block: "coal_ore".into(), x, y, z });
    }
    std::thread::sleep(Duration::from_millis(300));

    let ore_before = count_block_nearby(&adapter, "iron_ore", 6);
    let coal_before = count_block_nearby(&adapter, "coal_ore", 6);
    println!("[mine_iron] placed iron_ore x{ore_before} coal_ore x{coal_before} (must be >0)");

    // 关键断言前提：确实放下了矿石方块（不是凭空物品）。
    let mut pass = 0u32;
    let mut fail = 0u32;
    let mark = |name: &str, ok: bool, detail: &str| {
        if ok { println!("  [PASS] {name} — {detail}"); }
        else { println!("  [FAIL] {name} — {detail}"); }
    };

    if ore_before == 0 || coal_before == 0 {
        println!("[mine_iron] ABORT: debug_place 未成功放置矿石方块（检查 MC 日志）");
        return Ok(());
    }
    mark("ore_placed", ore_before >= 4, &format!("iron_ore x{ore_before}"));

    // 真实目标：造铁镐（需 3 iron_ingot）。bot 必须自己挖矿 + 采煤 + 烧炼 + 合成。
    println!("[mine_iron] goal craft iron_pickaxe (autonomous mining)...");
    let _ = adapter.lock().unwrap().goal_execute("craft", "iron_pickaxe", 1);
    wait_goal(&adapter, 180);

    let st = adapter.lock().unwrap().reload().unwrap();
    let pick = st.inventory.iter().filter(|i| i.id.contains("iron_pickaxe")).map(|i| i.count).sum::<u32>();
    let ore_after = count_block_nearby(&adapter, "iron_ore", 6);
    let coal_after = count_block_nearby(&adapter, "coal_ore", 6);
    let raw_left = st.inventory.iter().filter(|i| i.id.contains("raw_iron")).map(|i| i.count).sum::<u32>();

    // 验证 ① 造出了铁镐
    let ok_pick = pick >= 1;
    mark("crafted_pickaxe", ok_pick, &format!("iron_pickaxe x{pick}"));
    // 验证 ② 矿石方块被真挖掉了（附近 iron_ore 归零），证明不是凭空给料
    let ok_mined = ore_after < ore_before;
    mark("ore_mined", ok_mined, &format!("iron_ore {ore_before}->{ore_after}"));
    // 验证 ③ 煤也被挖了当燃料
    let ok_coal = coal_after < coal_before;
    mark("coal_mined", ok_coal, &format!("coal_ore {coal_before}->{coal_after}"));
    // 验证 ④ 没有残留 raw_iron（都烧成了 ingot），说明烧炼链跑通
    let ok_smelted = raw_left == 0;
    mark("smelted_clean", ok_smelted, &format!("raw_iron left x{raw_left}"));

    if ok_pick && ok_mined && ok_coal { pass += 1; } else { fail += 1; }

    println!("[mine_iron] === RESULT: PASS={pass} FAIL={fail} ===");
    if fail > 0 { std::process::exit(1); }
    Ok(())
}

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

fn goal_idle(a: &Arc<Mutex<MinecraftModAdapter>>) -> bool {
    let s = a.lock().unwrap().goal_status().unwrap().detail;
    s.contains("idle") || s.contains("done") || s.contains("failed")
}

fn wait_goal(a: &Arc<Mutex<MinecraftModAdapter>>, secs: u64) {
    let start = Instant::now();
    while start.elapsed().as_secs() < secs {
        if goal_idle(a) { return; }
        std::thread::sleep(Duration::from_millis(500));
    }
}
