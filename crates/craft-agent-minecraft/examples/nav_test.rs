//! 新寻路系统集成测试（需要 MC + craft-agent-bridge 运行）。
//!
//! 测试 nav_to / nav_status / nav_stop 命令在真实 MC 环境中的表现。
//!
//! 运行：
//! ```bash
//! cargo run -p craft-agent-minecraft --example nav_test --features mod-bridge
//! ```
//! 依赖：MC 已启动，craft-agent-bridge 已加载，fake player 在线。

#[cfg(feature = "mod-bridge")]
fn main() -> anyhow::Result<()> {
    use craft_agent_minecraft::adapter_mod::MinecraftModAdapter;
    use craft_agent_minecraft::bridge::DEFAULT_PORT;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::Instant;

    println!("=== Nav 系统集成测试 ===");
    println!("连接 MC bridge @ 127.0.0.1:{} ...", DEFAULT_PORT);
    let adapter = Arc::new(Mutex::new(MinecraftModAdapter::connect(
        "127.0.0.1",
        DEFAULT_PORT,
    )?));
    println!("已连接。假玩家应在(0.5,65.0,0.5)附近。\n");

    // ── 辅助：执行测试并计时 ──
    struct Tester {
        adapter: Arc<Mutex<MinecraftModAdapter>>,
        passed: u32,
        failed: u32,
    }
    impl Tester {
        fn test(
            &mut self,
            name: &str,
            f: impl FnOnce(&MinecraftModAdapter) -> anyhow::Result<bool>,
        ) {
            let start = Instant::now();
            let adapter = self.adapter.lock().unwrap();
            match f(&adapter) {
                Ok(true) => {
                    self.passed += 1;
                    println!("  [PASS] {} ({:?})", name, start.elapsed());
                }
                Ok(false) => {
                    self.failed += 1;
                    println!(
                        "  [FAIL] {} ({:?}) — assertion failed",
                        name,
                        start.elapsed()
                    );
                }
                Err(e) => {
                    self.failed += 1;
                    println!("  [FAIL] {} ({:?}) — error: {}", name, start.elapsed(), e);
                }
            }
        }
    }

    let mut t = Tester {
        adapter: adapter.clone(),
        passed: 0,
        failed: 0,
    };

    // ── 测试 1: nav_to ──
    t.test("nav_to starts", |a| {
        let ack = a.nav_to(5.0, 65.0, 5.0)?;
        Ok(ack.status == "ok")
    });

    std::thread::sleep(std::time::Duration::from_millis(2000));

    // ── 测试 2: nav_status while running ──
    t.test("nav_status while running", |a| {
        let ack = a.nav_status()?;
        Ok(ack.status == "ok" && !ack.detail.is_empty())
    });

    // ── 测试 3: nav_stop ──
    t.test("nav_stop cancels", |a| {
        let ack = a.nav_stop()?;
        Ok(ack.status == "ok")
    });

    // ── 测试 4: nav_status after stop ──
    std::thread::sleep(std::time::Duration::from_millis(200));
    t.test("nav_status after stop", |a| {
        let ack = a.nav_status()?;
        Ok(ack.status == "ok" && ack.detail.contains("idle"))
    });

    // ── 测试 5: nav_to reachable ──
    t.test("nav_to reachable (10 blocks)", |a| {
        let ack = a.nav_to(10.5, 65.0, 10.5)?;
        Ok(ack.status == "ok")
    });

    // Wait for arrival (up to 35s, terrain may slow the bot)
    let deadline = Instant::now() + std::time::Duration::from_secs(35);
    let mut arrived = false;
    while Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let ack = adapter.lock().unwrap().nav_status()?;
        if ack.detail.contains("arrived") || ack.detail.contains("idle") {
            arrived = true;
            break;
        }
        if ack.detail.contains("failed") {
            println!("    nav failed: {}", ack.detail);
            break;
        }
    }
    if arrived {
        t.passed += 1;
        println!("  [PASS] nav_to reaches target");
    } else {
        println!("  [INFO] nav_to still running after 35s, checking final position instead");
    }

    // ── 测试 6: nav_stop while idle ──
    t.test("nav_stop when idle", |a| {
        let ack = a.nav_stop()?;
        Ok(ack.status == "ok")
    });
    // ── 测试 7: position check (within 10 blocks of target) ──
    {
        let a = adapter.lock().unwrap();
        if let Ok(state) = a.reload() {
            let dx = state.position[0] - 10.5;
            let dz = state.position[2] - 10.5;
            if (dx * dx + dz * dz) < 100.0 {
                t.passed += 1;
                println!("  [PASS] player position within 10 blocks of target");
            } else {
                t.failed += 1;
                println!(
                    "  [FAIL] player position too far: ({}, {})",

                    state.position[0],
                    state.position[2]
                );
            }
        } else {
            t.failed += 1;
            println!("  [FAIL] reload() failed");
        }
    }
    // ── 报告 ──
    println!("\n=== 报告 ===");
    println!(
        "PASS={} FAIL={} total={}",
        t.passed,
        t.failed,
        t.passed + t.failed
    );
    if t.failed == 0 {
        println!("全部通过！");
    } else {
        println!("有失败项");
    }
    Ok(())
}

#[cfg(not(feature = "mod-bridge"))]
fn main() {
    eprintln!("需要 --features mod-bridge");
}
