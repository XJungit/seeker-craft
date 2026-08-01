//! Phase 5 验证：真实放置（place）闭环。
//!
//! 运行：`cargo run --example azalea_place_demo --features azalea-bot 4444`
//! 行为：连入 -> 挖脚下方块（mine_below）若干次 -> 检测背包是否进方块
//!       -> 若手里有方块，在 (x+1,y,z) 放置 -> 验证 place 真实发生。
//!
//! 说明：生存模式无 give，必须靠 bot 自己挖到方块并自动拾取。
//! 若背包始终空，说明掉落物未被捡（矿井场景常见），则 place 需先到开阔地。

use craft_agent::core::adapter::GameAdapter;
use craft_agent::core::types::{Action, MinecraftAction};
use craft_agent_minecraft::adapter_azalea::ArcAzaleaAdapter;

#[tokio::main]
async fn main() {
    let port = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "4444".to_string());
    let addr = format!("localhost:{port}");
    println!("[place_demo] connect {addr}");

    let mut adapter = ArcAzaleaAdapter::connect(&addr, "craftbot")
        .await
        .expect("adapter 连接失败");
    println!("[place_demo] adapter 就绪");

    // 等首次状态快照
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // 挖脚下方块 5 次（间隔等掉落物拾取）
    for i in 1..=5 {
        adapter
            .execute(Action::Minecraft(MinecraftAction::MineBelow))
            .expect("mine_below 失败");
        println!("[place_demo] 第{i}次 mine_below 已下发");
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // 读背包
        let st = adapter.perceive().expect("perceive 失败");
        println!("[place_demo] 背包={:?}", st.scene_desc);

        // 若背包前5格有非空前，尝试放置
        if !st.scene_desc.contains("空") {
            println!("[place_demo] 检测到背包有物品，尝试放置...");
            // 需要知道 bot 坐标来算放置点；scene_desc 里含坐标，简单解析
            if let Some(pos) = parse_pos(&st.scene_desc) {
                let (x, y, z) = pos;
                adapter
                    .execute(Action::Minecraft(MinecraftAction::InteractBlock {
                        x: x + 1,
                        y,
                        z,
                    }))
                    .expect("place 失败");
                println!("[place_demo] ✅ 已在 ({},{},{}) 旁下发放置", x + 1, y, z);
                break;
            }
        }
    }

    println!("[place_demo] 验证结束（bot 仍在后台，ctrl-c 退出）");
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    println!("[place_demo] done");
}

/// 从 scene_desc "坐标=(x,y,z) ..." 解析整数坐标。
fn parse_pos(s: &str) -> Option<(i32, i32, i32)> {
    let start = s.find("坐标=(")?;
    let rest = &s[start + "坐标=(".len()..];
    let end = rest.find(')')?;
    let nums: Vec<i32> = rest[..end]
        .split(',')
        .filter_map(|p| p.trim().parse::<f64>().ok())
        .map(|v| v as i32)
        .collect();
    if nums.len() == 3 {
        Some((nums[0], nums[1], nums[2]))
    } else {
        None
    }
}
