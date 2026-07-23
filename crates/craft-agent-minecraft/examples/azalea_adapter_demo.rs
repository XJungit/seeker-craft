//! Phase 4 集成验证：通过 `GameAdapter` 抽象驱动 azalea bot。
//!
//! 运行：`cargo run --example azalea_adapter_demo --features azalea-bot 4444`
//! 行为：connect adapter -> perceive（结构化 WorldState）-> execute（mine_below + chat）
//! 验证 harness 抽象层与 azalea 执行层打通，无需关心底层协议。

use craft_agent::core::adapter::GameAdapter;
use craft_agent::core::types::{Action, MinecraftAction};
use craft_agent_minecraft::adapter_azalea::ArcAzaleaAdapter;

#[tokio::main]
async fn main() {
    let port = std::env::args().nth(1).unwrap_or_else(|| "4444".to_string());
    let addr = format!("localhost:{port}");
    println!("[adapter_demo] connect {addr}");

    let mut adapter = ArcAzaleaAdapter::connect(&addr, "craftbot")
        .await
        .expect("adapter 连接失败");
    println!("[adapter_demo] adapter 就绪");

    // 给一点时间让首次 State 快照到达。
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // perceive：结构化 WorldState（无截图/VLM）。
    let st = adapter.perceive().expect("perceive 失败");
    println!(
        "[adapter_demo] perceive: scene_desc={} self_hint={}",
        st.scene_desc, st.self_hint
    );

    // execute：通过 Action 抽象下发动作。
    let r1 = adapter
        .execute(Action::Minecraft(MinecraftAction::MineBelow))
        .expect("execute mine_below 失败");
    println!("[adapter_demo] execute mine_below: ok={} detail={}", r1.ok, r1.detail);

    let r2 = adapter
        .execute(Action::Minecraft(MinecraftAction::Chat {
            content: "harness-driven action".to_string(),
        }))
        .expect("execute chat 失败");
    println!("[adapter_demo] execute chat: ok={} detail={}", r2.ok, r2.detail);

    println!("[adapter_demo] 完成（bot 仍在后台运行，ctrl-c 退出）");
    // 保持进程，观察 bot 行为。
    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    println!("[adapter_demo] 结束");
}
