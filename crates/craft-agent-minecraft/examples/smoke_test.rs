//! Fixture-driven real-MC smoke test: enumerate every registered tool from
//! `create_mc_mod_tools`, first build a minimal test fixture via the `debug_*`
//! commands (spawn zombie / give oak_log / damage self / drop item), then call
//! the tool with safe default args derived from its JSON schema, and record
//! PASS/FAIL/SKIP. Never abort on a single failure.
//!
//! Run (MC + craft-agent-bridge loaded, player in a world):
//! ```bash
//! cargo run -p craft-agent-minecraft --example smoke_test --features mod-bridge
//! ```

#![allow(
    clippy::let_unit_value,
    clippy::collapsible_if,
    clippy::redundant_guards,
    unused_variables
)]

use craft_agent::core::tool::ToolUpdateFn;
use craft_agent_minecraft::adapter_mod::MinecraftModAdapter;
use craft_agent_minecraft::bridge::{DEFAULT_PORT, ModAck, ModCommand};
use craft_agent_minecraft::tools_mod::create_mc_mod_tools;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let host = std::env::var("MC_BRIDGE_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = std::env::var("MC_BRIDGE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    println!("[full-smoke] connecting to MC bridge @ {host}:{port} ...");
    let adapter = {
        let mut a = None;
        for attempt in 1..=60 {
            match MinecraftModAdapter::connect(&host, port) {
                Ok(conn) => {
                    a = Some(conn);
                    break;
                }
                Err(_) => {
                    if attempt == 1 {
                        println!(
                            "[full-smoke] waiting for MC to come up (retry every 5s, up to 5min)..."
                        );
                    }
                    std::thread::sleep(std::time::Duration::from_secs(5));
                }
            }
        }
        match a {
            Some(conn) => Arc::new(Mutex::new(conn)),
            None => {
                eprintln!(
                    "[full-smoke] connect failed after 5min. start MC + load craft-agent-bridge first."
                );
                std::process::exit(2);
            }
        }
    };
    println!("[full-smoke] connected.");

    let tools = create_mc_mod_tools(adapter.clone(), None, None, false, None);
    println!("[full-smoke] {} tools registered", tools.len());

    let noop: Option<ToolUpdateFn> = None;

    // 一键搭测试平台：9x9 dirt 平台 + bot 归位(0.5,65.0,0.5) + 基线 oak_log x16。
    // 所有坐标类工具在固定平台测，每次设后 bot 在(0.5,65.0,0.5)。
    send_debug(
        &adapter,
        ModCommand::DebugSetFixture {
            fixture: "platform".into(),
        },
    );

    let mut failures = 0usize;
    let mut skipped = 0usize;
    let mut passed = 0usize;

    // SKIP 仅保留「单人世界 + debug fixture 确实无法造出环境」的工具。
    // 已修复（9→2）：collect/build/transfer/eat_item/collect_items/trade_with_villager/
    // go_to_player/attack_player（均已添加 mod 侧 fixture 支持）。
    // build_portal: MC 26.2 survival reach/useItemOn 阻止框架放置，fixture 仍给材料+creative。.
    // 注意：键名必须与工具 name() 完全一致（全为 snake_case）。
    let destructive: HashMap<&str, &str> = [
        (
            "build_portal",
            "placeAt/useItemOn distance check in MC 26.2 survival mode blocks top frame blocks",
        ),
        ("teleport_to", "dimension change is disruptive"),
    ]
    .iter()
    .cloned()
    .collect();

    for t in tools.iter() {
        let name = t.name();
        if let Some(reason) = destructive.get(name) {
            println!("  [SKIP] {:<22} destructive: {}", name, reason);
            skipped += 1;
            continue;
        }

        // 一键搭测试环境：清场 + 平台 + bot归位(0.5,65,0.5) + 基线物品 + 工具特定设置。
        // 替代之前 ~170 次 debug_place/spawn/give/teleport/wait 的 TCP 往返。
        send_debug(
            &adapter,
            ModCommand::DebugSetFixture {
                fixture: name.into(),
            },
        );

        let args = default_args(name, t.parameters(), 0.5, 65.0, 0.5);
        // *_player 工具需要真实玩家（你本人）的名字作为目标。
        let mut args = args;
        if matches!(
            name,
            "go_to_player" | "attack_player" | "give_player" | "follow_player" | "look_at_player"
        ) {
            if let Some(real) = real_player_name(&adapter) {
                args["player_name"] = serde_json::json!(real);
                let _ = send_debug(
                    &adapter,
                    ModCommand::DebugTeleportPlayer {
                        name: real.clone(),
                        dist: Some(2.0),
                    },
                );
            }
        }
        let start = Instant::now();
        let res = t.execute("full-smoke", args, noop.clone());
        let dur = start.elapsed();

        match res {
            Ok(r) => {
                if r.is_error {
                    failures += 1;
                    println!(
                        "  [FAIL] {:<22} ({:>5}ms) {}",
                        name,
                        dur.as_millis(),
                        r.message.lines().next().unwrap_or("")
                    );
                } else {
                    passed += 1;
                    println!(
                        "  [PASS] {:<22} ({:>5}ms) {}",
                        name,
                        dur.as_millis(),
                        r.message.lines().next().unwrap_or("")
                    );
                }
            }
            Err(e) => {
                failures += 1;
                println!("  [FAIL] {:<22} ({:>5}ms) ERR {}", name, dur.as_millis(), e);
            }
        }
        // 无需显式 clean——下次 DebugSetFixture 自动重置环境。
    }

    println!("[full-smoke] === report ===");
    println!(
        "[full-smoke] PASS={passed} FAIL={failures} SKIP={skipped} total={}",
        tools.len()
    );
    if failures > 0 {
        println!("[full-smoke] RESULT: FAIL ({failures} tools errored)");
        std::process::exit(1);
    } else {
        println!("[full-smoke] RESULT: PASS (all executable tools ran without error)");
    }
    Ok(())
}

/// 从 list_players 里找真实玩家（排除 bot 自己 CraftAgent），返回其名字。
/// 这样 *_player 工具就能以你（真实玩家）为目标，而 bot 仍操控 fakePlayer。
fn real_player_name(adapter: &Arc<Mutex<MinecraftModAdapter>>) -> Option<String> {
    let ack = adapter
        .lock()
        .unwrap()
        .send_debug(ModCommand::ListPlayers)
        .ok()?;
    // ack.players 是 JSON 数组：[{name,uuid,position,dist}, ...]
    let players = ack.players.as_ref()?;
    let arr = players.as_array()?;
    for p in arr {
        let nm = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if !nm.is_empty() && nm != "CraftAgent" {
            return Some(nm.to_string());
        }
    }
    None
}

/// Send a debug command, log failures but don't abort the run.
fn send_debug(adapter: &Arc<Mutex<MinecraftModAdapter>>, cmd: ModCommand) {
    let r: anyhow::Result<ModAck> = adapter.lock().unwrap().send_debug(cmd);
    match r {
        Ok(ack) => {
            let detail = ack.detail.as_str();
            println!("  [DBG ] ack: {detail}");
        }
        Err(e) => println!("  [DBG ] fixture setup failed: {e}"),
    }
}

/// Derive safe default args from a tool_args schema object. `name` is the tool
/// name, used to special-case tools whose default item/coordinate differs.
///
/// The schema is `{ "type": "object", "properties": { key: {type, ...} } }`,
/// so we iterate `properties` and pick a default based on the declared type
/// plus the key name.
fn default_args(name: &str, schema: Value, px: f64, py: f64, pz: f64) -> Value {
    let mut out = serde_json::Map::new();
    let props = schema.get("properties").and_then(|p| p.as_object());
    if let Some(props) = props {
        for (key, spec) in props {
            // Per-tool overrides: placeholder "test" or wrong defaults that would
            // otherwise make a tool fail even with a correct fixture.
            let override_val: Option<Value> = match (name, key.as_str()) {
                ("searchForEntity", "type")
                | ("searchForBlock", "type")
                | ("activate_nearest_block", "block_type") => {
                    if name == "searchForEntity" {
                        Some(json!("zombie"))
                    } else if name == "activate_nearest_block" {
                        Some(json!("crafting_table"))
                    } else {
                        Some(json!("oak_log"))
                    }
                }
                ("useOn", "target") | ("use_on_entity", "entity_type") => Some(json!("cow")),
                ("useOn", "tool_name") => Some(json!("hand")),
                ("chest", "action") => Some(json!("view")),
                ("ride", "action") => Some(json!("mount")),
                ("getCraftingPlan", "targetItem") => Some(json!("oak_planks")),
                ("getBlueprintLevel", "blueprint") | ("build", "blueprint") => {
                    Some(json!("dirt_shelter"))
                }
                ("collect_items", "item_ids") => Some(json!(["oak_log"])),
                ("transfer", "moves") => Some(json!([{"from": 54, "to": 0}])),
                _ => None,
            };
            if let Some(ov) = override_val {
                out.insert(key.clone(), ov);
                continue;
            }
            let ty = spec
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("string");
            match (key.as_str(), ty) {
                ("x", _) => {
                    out.insert(key.clone(), serde_json::json!((px + 1.0) as i64));
                }
                ("y", _) => {
                    out.insert(key.clone(), serde_json::json!((py - 1.0) as i64));
                }
                ("z", _) => {
                    out.insert(key.clone(), serde_json::json!(pz as i64));
                }
                (k, "string")
                    if matches!(
                        k,
                        "target" | "block" | "item" | "entity" | "material" | "mat"
                    ) =>
                {
                    let v = if k == "item" && name == "place" {
                        "dirt"
                    } else if k == "item" && name == "craft" {
                        "oak_planks"
                    } else if k == "item" && (name == "eat_item" || name == "consume") {
                        "apple"
                    } else {
                        "oak_log"
                    };
                    out.insert(key.clone(), serde_json::json!(v));
                }
                (k, "string")
                    if matches!(
                        k,
                        "name" | "place_name" | "blueprint" | "message" | "prompt" | "query"
                    ) =>
                {
                    out.insert(key.clone(), serde_json::json!("test"));
                }
                (k, "string") if k == "mode" => {
                    out.insert(key.clone(), serde_json::json!("retreat"));
                }
                (k, "integer" | "number")
                    if matches!(k, "count" | "num" | "ticks" | "max_count" | "amount") =>
                {
                    // ticks 用于吃东西/使用时长，32 tick≈1.6s 是合理默认。
                    let v = if k == "ticks" { 32 } else { 1 };
                    out.insert(key.clone(), serde_json::json!(v));
                }
                (k, "integer" | "number") if k == "radius" => {
                    out.insert(key.clone(), serde_json::json!(8));
                }
                (k, "integer" | "number") if k == "search_range" => {
                    out.insert(key.clone(), serde_json::json!(64));
                }
                (k, "integer" | "number") if matches!(k, "distance" | "max_distance") => {
                    out.insert(key.clone(), serde_json::json!(16));
                }
                ("slot", "integer") => {
                    out.insert(key.clone(), serde_json::json!(0));
                }
                (k, "integer") if matches!(k, "from_slot" | "from" | "source") => {
                    out.insert(key.clone(), serde_json::json!(0));
                }
                (k, "integer") if matches!(k, "to_slot" | "to" | "dst") => {
                    out.insert(key.clone(), serde_json::json!(1));
                }
                (_, "string") => {
                    out.insert(key.clone(), serde_json::json!("test"));
                }
                (_, "integer" | "number") => {
                    out.insert(key.clone(), serde_json::json!(1));
                }
                (_, "array") => {
                    out.insert(key.clone(), serde_json::json!([]));
                }
                (_, "boolean") => {
                    out.insert(key.clone(), serde_json::json!(true));
                }
                _ => {
                    out.insert(key.clone(), serde_json::json!("test"));
                }
            }
        }
    }
    Value::Object(out)
}
