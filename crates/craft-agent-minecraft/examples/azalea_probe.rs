//! azalea_probe：不开 LLM 直接驱动 azalea bot 工具层做行为验证。
//!
//! 用途：LLM 实机测试慢（每回合 30-60s+），工具层行为验证（equip/gather/craft/
//! smelt/chest 等）用本工具秒级完成，无需 viewer/agent/LLM。
//!
//! 用法：
//! ```bash
//! # 单条命令
//! cargo run -p craft-agent-minecraft --example azalea_probe --features azalea-bot -- 4444 --cmd "equip iron_helmet helmet"
//! # 脚本（多条步骤）
//! cargo run -p craft-agent-minecraft --example azalea_probe --features azalea-bot -- 4444 --script probe.json
//! ```
//!
//! script.json 格式：
//! ```json
//! {
//!   "between_delay_ms": 1500,
//!   "timeout_ms": 60000,
//!   "steps": [
//!     {"cmd": "equip iron_helmet helmet"},
//!     {"wait_ms": 3000},
//!     {"state": true},
//!     {"cmd": "gather iron_ore 3"}
//!   ]
//! }
//! ```
//! 支持命令文本见 `parse_chat_command`（equip/discard/consume/goto/gather/craft/
//! craft3/smelt/mine/minebelow/place/open/chestview/chestwithdraw/chestdeposit/
//! makeobsidian/pickup/defend/attack/...）。

use std::sync::{Arc, Mutex};

use craft_agent_minecraft::azalea::{parse_chat_command, AzaleaBot, BotEvent};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let port = args
        .first()
        .cloned()
        .unwrap_or_else(|| "4444".to_string());
    let addr = format!("localhost:{port}");

    let (script_path, cmd_text) = if let Some(i) = args.iter().position(|a| a == "--script") {
        (Some(args[i + 1].clone()), None)
    } else if let Some(i) = args.iter().position(|a| a == "--cmd") {
        (None, Some(args[i + 1].clone()))
    } else {
        (None, Some("state".to_string()))
    };

    println!("[probe] 连接 {addr} ...");
    let bot = AzaleaBot::connect(&addr, "craftbot_probe", None)
        .await
        .expect("连接失败");
    let bot = Arc::new(bot);
    println!("[probe] 已连接");

    // 后台：消费事件流，维护最新状态快照。
    let latest_state: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let latest_state_task = latest_state.clone();
    let bot_ev = bot.clone();
    let event_task = tokio::spawn(async move {
        while let Some(ev) = bot_ev.next_event().await {
            match ev {
                BotEvent::Spawn { position } => {
                    println!("[probe] 出生: ({:.1},{:.1},{:.1})", position.x, position.y, position.z);
                }
                BotEvent::Chat { content } => {
                    println!("[probe] chat: {content}");
                }
                BotEvent::State {
                    position,
                    inventory,
                    player_count,
                    health,
                    food,
                    held_item,
                    biome,
                    nearby,
                    ..
                } => {
                    let snapshot = format!(
                        "pos=({:.1},{:.1},{:.1}) hp={:.1}/20 food={}/20 held={} biome={} nearby=[{}] inv=[{}] players={}",
                        position.x, position.y, position.z, health, food, held_item, biome, nearby, inventory, player_count
                    );
                    if let Ok(mut g) = latest_state_task.lock() {
                        *g = Some(snapshot.clone());
                    }
                }
                BotEvent::Disconnect { reason } => {
                    println!("[probe] 断开: {reason}");
                    break;
                }
                _ => {}
            }
        }
    });

    // 等 Spawn + 初始同步
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;

    // 构建步骤
    enum Step {
        Cmd(String),
        Wait(u64),
        State,
    }
    let mut steps: Vec<Step> = Vec::new();
    let mut between_delay_ms = 1500u64;
    let mut timeout_ms = 60000u64;

    if let Some(script_path) = script_path {
        let raw = std::fs::read_to_string(&script_path)
            .unwrap_or_else(|e| panic!("读取脚本失败 {script_path}: {e}"));
        let json: serde_json::Value = serde_json::from_str(&raw).expect("脚本 JSON 解析失败");
        between_delay_ms = json
            .get("between_delay_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(between_delay_ms);
        timeout_ms = json.get("timeout_ms").and_then(|v| v.as_u64()).unwrap_or(timeout_ms);
        let arr = json
            .get("steps")
            .and_then(|v| v.as_array())
            .expect("steps 必须是数组");
        for step in arr {
            if let Some(c) = step.get("cmd").and_then(|v| v.as_str()) {
                steps.push(Step::Cmd(c.to_string()));
            } else if let Some(w) = step.get("wait_ms").and_then(|v| v.as_u64()) {
                steps.push(Step::Wait(w));
            } else if step.get("state").and_then(|v| v.as_bool()).unwrap_or(false) {
                steps.push(Step::State);
            } else {
                println!("[probe] 跳过未知步骤: {step}");
            }
        }
    } else if let Some(cmd_text) = cmd_text {
        steps.push(Step::Cmd(cmd_text));
    } else {
        steps.push(Step::State);
    }

    let mut failed = 0usize;
    for (i, step) in steps.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(between_delay_ms)).await;
        }
        match step {
            Step::Cmd(text) => {
                println!("\n=== step {}: {text} ===", i + 1);
                let cmd = match parse_chat_command(text) {
                    Some(c) => c,
                    None => {
                        println!("[probe] 无法解析命令: {text}");
                        failed += 1;
                        continue;
                    }
                };
                match bot.push_cmd_and_wait(cmd, timeout_ms) {
                    Ok(msg) => println!("[probe] 结果: {msg}"),
                    Err(e) => {
                        println!("[probe] 失败: {e}");
                        failed += 1;
                    }
                }
            }
            Step::Wait(ms) => {
                println!("[probe] 等待 {ms}ms ...");
                tokio::time::sleep(std::time::Duration::from_millis(*ms)).await;
            }
            Step::State => {
                let snap = latest_state
                    .lock()
                    .map(|g| g.clone())
                    .unwrap_or(None)
                    .unwrap_or_else(|| "（暂无状态快照）".to_string());
                println!("[probe] 状态: {snap}");
            }
        }
    }

    println!("\n[probe] 完成，失败 {failed}/{}", steps.len());
    event_task.abort();
}
