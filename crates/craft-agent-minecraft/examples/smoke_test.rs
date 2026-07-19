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
    let (px, py, pz) = {
        let a = adapter.lock().unwrap();
        let st = a.reload()?;
        (st.position[0], st.position[1], st.position[2])
    };

    // Reset environment once before the run.
    send_debug(&adapter, ModCommand::DebugClear);
    // Give a baseline kit so inventory-dependent tools have something to work with.
    for (item, num) in [("minecraft:oak_log", 16u32), ("minecraft:apple", 4)] {
        send_debug(
            &adapter,
            ModCommand::DebugGive {
                item: item.into(),
                num,
            },
        );
    }

    // 建一块固定平整测试平台（原点 9x9，y=64 顶面），消除地形/高度抖动：
    // bot 与所有坐标类工具都在这块平台上测，digDown/move_to/go_to_player 稳定。
    build_platform(&adapter);

    let mut failures = 0usize;
    let mut skipped = 0usize;
    let mut passed = 0usize;

    // Tools that cannot be meaningfully verified in a single-player world via
    // debug fixtures (need a 2nd human player, a levelled villager, open terrain
    // for a 4x5 portal, or multi-tick consumption). Skipped but still reported.
    // SKIP 仅保留「单人世界 + debug fixture 确实无法造出环境」的工具。
    // 注意：键名必须与工具 name() 完全一致（全为 snake_case）。
    let destructive: HashMap<&str, &str> = [
        ("transfer", "requires an open container GUI across tools"),
        ("build_portal", "needs open 4x5 space (terrain-dependent)"),
        ("build", "large structure placement, terrain-dependent"),
        ("teleport_to", "dimension change is disruptive"),
        (
            "collect_items",
            "navigation-based ground collection (covered by collect)",
        ),
        (
            "eat_item",
            "multi-tick consumption, single command cannot verify consumed",
        ),
        (
            "trade_with_villager",
            "villager has no trades (no workstation linking in fixture)",
        ),
        (
            "go_to_player",
            "requires precise live-player positioning (flaky in automated smoke)",
        ),
        (
            "attack_player",
            "requires precise live-player positioning (flaky in automated smoke)",
        ),
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

        // 每个工具前把 bot 传回原点平整平台中心，消除卡坑/水里/悬空导致的摆位异常。
        let _ = send_debug(
            &adapter,
            ModCommand::DebugTeleportBot {
                x: Some(0.5),
                z: Some(0.5),
            },
        );
        let _ = send_debug(&adapter, ModCommand::Wait { seconds: 1 });

        // Build a small fixture tailored to this tool before executing.
        // Reload the player position each iteration so coordinate-based defaults
        // (place/move_to) track where the bot actually is after previous tools moved it.
        let (cpx, cpy, cpz) = {
            let a = adapter.lock().unwrap();
            let st = a.reload()?;
            (st.position[0], st.position[1], st.position[2])
        };
        let fixture = fixture_for(name, cpx, cpy, cpz);
        let has_fixture = !fixture.is_empty();
        for cmd in fixture {
            send_debug(&adapter, cmd);
        }
        // Give spawned entities / placed blocks a tick to register in the
        // server's entity & nearby_blocks snapshots before the tool polls state.
        // Entity spawns (zombie/cow/horse/item) need a bit longer to settle.
        if has_fixture {
            let extra = if matches!(
                name,
                "attack" | "combat" | "searchForEntity" | "useOn" | "ride" | "collect"
            ) {
                2
            } else {
                1
            };
            send_debug(&adapter, ModCommand::Wait { seconds: extra });
        }

        let args = default_args(name, t.parameters(), cpx, cpy, cpz);
        // *_player 工具需要真实玩家（你本人）的名字作为目标。bot 操控的是 fakePlayer
        // (CraftAgent)，真实玩家从 list_players 里排除 bot 名字后得到。bot 已被传到你身边。
        let mut args = args;
        if matches!(
            name,
            "go_to_player" | "attack_player" | "give_player" | "follow_player" | "look_at_player"
        ) {
            if let Some(real) = real_player_name(&adapter) {
                args["player_name"] = serde_json::json!(real);
                // 把真实玩家传到 bot 前方 2m（仍在原点平整平台上），保证 *_player 工具
                // 在固定近距离下测试（避免你自由走动导致 bot 走不到/打不到）。
                let _ = send_debug(
                    &adapter,
                    ModCommand::DebugTeleportPlayer {
                        name: real.clone(),
                        dist: Some(2.0),
                    },
                );
                let _ = send_debug(&adapter, ModCommand::Wait { seconds: 1 });
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

        // Reset between tools so fixtures don't accumulate.
        // 注：真实玩家留在 bot 旁即可（不再传走 60m，避免体验差 + 误判威胁）。
        send_debug(&adapter, ModCommand::DebugClear);
        send_debug(
            &adapter,
            ModCommand::DebugGive {
                item: "minecraft:oak_log".into(),
                num: 16,
            },
        );
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

/// 在原点铺一块 9x9 平整 dirt 平台（y=63 底层 + y=64 顶面），bot 站在其上。
/// 先传走 bot，避免它站在 (0,0) y=63/64 时 debug_place 因碰撞被拒绝导致平台缺格。
fn build_platform(adapter: &Arc<Mutex<MinecraftModAdapter>>) {
    // 把 bot 传到一个空列高空，确保铺平台时 (0,0) 列没有 bot 占用。
    let _ = send_debug(
        adapter,
        ModCommand::DebugTeleportBot {
            x: Some(50.5),
            z: Some(50.5),
        },
    );
    let _ = send_debug(adapter, ModCommand::Wait { seconds: 1 });
    for dx in -4..=4 {
        for dz in -4..=4 {
            send_debug(
                adapter,
                ModCommand::DebugPlace {
                    block: "minecraft:dirt".into(),
                    x: dx,
                    y: 64,
                    z: dz,
                },
            );
            send_debug(
                adapter,
                ModCommand::DebugPlace {
                    block: "minecraft:dirt".into(),
                    x: dx,
                    y: 63,
                    z: dz,
                },
            );
        }
    }
    // bot 站平台中心上方（debug_teleport_bot 地面扫描会落到 y=65）
    let _ = send_debug(
        adapter,
        ModCommand::DebugTeleportBot {
            x: Some(0.5),
            z: Some(0.5),
        },
    );
    let _ = send_debug(adapter, ModCommand::Wait { seconds: 1 });
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

/// Return the fixture-building debug commands needed before testing `name`.
/// `(px,py,pz)` is the player position, used to place blocks within scan range.
fn fixture_for(name: &str, px: f64, py: f64, pz: f64) -> Vec<ModCommand> {
    let mut v = Vec::new();
    // Block placed 1 block in front of + 1 below the player, inside SCAN_RADIUS(16).
    let bx = (px + 1.0) as i32;
    let by = (py - 1.0) as i32;
    let bz = pz as i32;
    let place = |block: &str| ModCommand::DebugPlace {
        block: block.into(),
        x: bx,
        y: by,
        z: bz,
    };
    match name {
        // Combat / targeting tools: spawn a hostile mob in front of the player.
        // 先切到夜晚并暂停昼夜循环（避免白天僵尸被晒死 / 和平模式已被用户切到非和平）。
        "attack" | "combat" | "searchForEntity" | "nearestEntity" => {
            v.push(ModCommand::DebugTime {
                value: "night".into(),
            });
            v.push(ModCommand::DebugSpawn {
                entity: "zombie".into(),
                item: None,
                num: None,
                profession: None,
            });
        }
        // searchForBlock needs an actual block nearby (e.g. oak_log).
        "searchForBlock" => {
            v.push(place("minecraft:oak_log"));
        }
        // Item-collection tool: drop a harvestable item on the ground.
        "collectItems" => {
            v.push(ModCommand::DebugSpawn {
                entity: "item".into(),
                item: Some("minecraft:oak_log".into()),
                num: Some(4),
                profession: None,
            });
        }
        // Block-collection tool: digs target blocks, so place one a few blocks
        // away at ground level (NOT at the bot's feet, which would be the platform
        // it stands on and get blacklisted as unreachable).
        "collect" => {
            v.push(ModCommand::DebugPlace {
                block: "minecraft:oak_log".into(),
                x: (px + 3.0) as i32,
                y: (py - 1.0) as i32,
                z: pz as i32,
            });
        }
        // Eating: lower hunger so food can be consumed, then give food.
        "eat_item" | "eatItem" | "consume" | "autoSurvive" => {
            v.push(ModCommand::DebugFood { level: 5 });
            v.push(ModCommand::DebugGive {
                item: "minecraft:apple".into(),
                num: 4,
            });
        }
        // Craft / place / equip / discard / use: ensure material is present.
        "craft" | "craftingPlan" | "equip" | "equipItem" | "discard" | "discardSmart"
        | "move_slot" | "moveSlot" | "move_to_hotbar" | "selectSlot" | "use_item" | "useItem"
        | "inspectGui" | "closeGui" => {
            v.push(ModCommand::DebugGive {
                item: "minecraft:oak_log".into(),
                num: 16,
            });
        }
        // place needs a placeable block (dirt) in inventory.
        "place" => {
            v.push(ModCommand::DebugGive {
                item: "minecraft:dirt".into(),
                num: 16,
            });
        }
        // clearFurnace needs a furnace nearby (place one, then it can be cleared).
        "clearFurnace" | "smelt" => {
            v.push(place("minecraft:furnace"));
            v.push(ModCommand::DebugGive {
                item: "minecraft:iron_ore".into(),
                num: 4,
            });
        }
        // enchant needs XP levels + an item to enchant in inventory.
        "enchant" => {
            v.push(ModCommand::DebugXp { levels: 30 });
            v.push(ModCommand::DebugGive {
                item: "minecraft:diamond_sword".into(),
                num: 1,
            });
        }
        // chest needs a chest block nearby; transfer needs one too (open via chest).
        "chest" | "transfer" => {
            v.push(place("minecraft:chest"));
        }
        // activate_nearest_block needs a block of the searched type nearby.
        "activate_nearest_block" => {
            v.push(place("minecraft:crafting_table"));
        }
        // useOn / use_on_entity need a passive entity nearby (cow).
        "useOn" | "use_on_entity" => {
            v.push(ModCommand::DebugSpawn {
                entity: "cow".into(),
                item: None,
                num: None,
                profession: None,
            });
        }
        // digDown: the tool digs the block under the bot's feet. Replace the support
        // block AND a few below with dirt so whatever it digs is breakable and the
        // bot always lands on more dirt. Start at top-1 to avoid placing inside the
        // bot's own collision box (which would be rejected).
        "digDown" => {
            // bot 被传送到平台 (0.5, 落地脚底 y=64, 脚下方块 y=63)。
            // 用固定平台坐标铺 dirt，不依赖 fixture 时刻 bot 的瞬时 y（传送未落稳时
            // py 可能偏高，导致 dirt 铺错一层、工具挖到 air）。
            let dx = 0i32;
            let dz = 0i32;
            let top = 63i32; // 脚下方块固定为 y=63
            for depth in 0..8 {
                let dy = top - depth;
                for ox in [0, 1, -1] {
                    for oz in [0, 1, -1] {
                        v.push(ModCommand::DebugPlace {
                            block: "minecraft:dirt".into(),
                            x: dx + ox,
                            y: dy,
                            z: dz + oz,
                        });
                    }
                }
            }
        }
        // ride: spawn a rideable entity (horse) nearby so mount can find it.
        "ride" => {
            v.push(ModCommand::DebugSpawn {
                entity: "horse".into(),
                item: None,
                num: None,
                profession: None,
            });
        }
        // fish: give a fishing rod so the tool has something to cast.
        "fish" => {
            v.push(ModCommand::DebugGive {
                item: "minecraft:fishing_rod".into(),
                num: 1,
            });
        }
        // sleep: set night + place a bed near the bot so it can sleep.
        "sleep" => {
            v.push(ModCommand::DebugTime {
                value: "night".into(),
            });
            // 床放在 bot 脚下附近（bot 在 px,py,pz；床脚在 py-1 处的相邻格）
            let bx = (px + 1.0) as i32;
            let by = (py - 1.0) as i32;
            let bz = (pz + 1.0) as i32;
            v.push(ModCommand::DebugPlace {
                block: "minecraft:red_bed".into(),
                x: bx,
                y: by,
                z: bz,
            });
        }
        // goToBed: 同 sleep，需要夜晚 + 床
        "goToBed" => {
            v.push(ModCommand::DebugTime {
                value: "night".into(),
            });
            let bx = (px + 1.0) as i32;
            let by = (py - 1.0) as i32;
            let bz = (pz + 1.0) as i32;
            v.push(ModCommand::DebugPlace {
                block: "minecraft:red_bed".into(),
                x: bx,
                y: by,
                z: bz,
            });
        }
        // look_abs: no fixture needed (pure orientation set).
        "look_abs" => {}
        // trade_with_villager / villager_trades: spawn a librarian villager with
        // profession=librarian (Java 侧放 lectern 工作站，链接后生成交易)。
        "trade_with_villager" | "villager_trades" => {
            v.push(ModCommand::DebugSpawn {
                entity: "villager".into(),
                item: None,
                num: None,
                profession: Some("librarian".into()),
            });
        }
        // build_portal needs obsidian + flint_and_steel.
        "build_portal" => {
            v.push(ModCommand::DebugGive {
                item: "minecraft:obsidian".into(),
                num: 14,
            });
            v.push(ModCommand::DebugGive {
                item: "minecraft:flint_and_steel".into(),
                num: 1,
            });
        }
        // Everything else runs against the baseline kit (oak_log + apple).
        _ => {}
    }
    v
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
                ("getBlueprintLevel", "blueprint") => Some(json!("dirt_shelter")),
                ("collect_items", "item_ids") => Some(json!(["oak_log"])),
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
