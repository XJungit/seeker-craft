//! 临时验证 B：行动回写闭环（砍树/挖矿 → 记忆更新）。
//!
//! 不依赖 LLM。连入 localhost:4444（共享 world_mem），扫描后取记忆里一个
//! 资源坐标（优先 dark_oak_log，否则任意矿石），用 MineTool 真正挖掉它，
//! 等超过扫描 TTL(30s) 后再次扫描重验，确认该坐标变为 depleted / 被遗忘。
//!
//! 运行（先确保 bot 出生在有资源方块的地面）：
//! ```bash
//! cargo run -p craft-agent-minecraft --example verify_b --features azalea-bot
//! ```

#[cfg(feature = "azalea-bot")]
fn main() -> anyhow::Result<()> {
    use craft_agent::core::memory::{MemoryKind, MemoryPos, WorldMemory};
    use craft_agent::core::tool::GameTool;
    use craft_agent_minecraft::adapter_azalea::ArcAzaleaAdapter;
    use craft_agent_minecraft::tools_azalea::{AzaleaToolCtx, MineTool};
    use serde_json::json;

    let world_mem = WorldMemory::new();
    let adapter = {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        rt.block_on(ArcAzaleaAdapter::connect_with_memory(
            "localhost:4444",
            "craftbot",
            world_mem.clone(),
        ))
        .expect("azalea adapter 连接失败（确认服为纯 vanilla 26.2 且端口 4444 开放）")
    };

    // 等扫描回填
    std::thread::sleep(std::time::Duration::from_secs(8));
    println!("[verify_b] 扫描到 {} 条记忆", world_mem.len());

    // 优先找 dark_oak_log，否则任意 Resource
    let target = world_mem
        .query(Some(MemoryKind::Resource), Some("dark_oak_log"))
        .into_iter()
        .find(|c| !c.depleted)
        .or_else(|| {
            world_mem
                .query(Some(MemoryKind::Resource), None)
                .into_iter()
                .find(|c| !c.depleted)
        });

    let target = match target {
        Some(c) => {
            println!(
                "[verify_b] 选定目标资源 @({:?}) item={:?} label={}",
                c.pos, c.item, c.label
            );
            c.pos
        }
        None => {
            eprintln!("[verify_b] 记忆里没有任何未耗尽的资源点（bot 可能不在有资源的地面）。退出。");
            return Ok(());
        }
    };

    // 用 GotoTool 先走到目标树旁（地面层），确保 bot 在方块附近能挖到
    let adapter_for_perceive = adapter.clone();
    let ctx = std::sync::Arc::new(AzaleaToolCtx::new(adapter, world_mem.clone()));
    let goto_tool = craft_agent_minecraft::tools_azalea::GotoTool::new(ctx.clone());
    let gargs = json!({ "x": target.x, "y": target.y - 1, "z": target.z });
    println!("[verify_b] 调用 goto({:?}) 让 bot 走到树旁", target);
    let _ = goto_tool.execute("verify_b", gargs, None);
    std::thread::sleep(std::time::Duration::from_secs(8));

    // 打印 bot 当前坐标（确认已靠近）
    if let Ok(st) = adapter_for_perceive.perceive_shared() {
        println!("[verify_b] bot 当前坐标: {}", st.self_hint);
    }

    // 用 MineTool 真正挖掉该坐标（树块）
    let mine_tool = MineTool::new(ctx);
    let args = json!({
        "x": target.x,
        "y": target.y,
        "z": target.z,
    });
    println!("[verify_b] 调用 mine({:?})", target);
    match mine_tool.execute("verify_b", args, None) {
        Ok(r) => println!("[verify_b] mine 结果: {}", r.message),
        Err(e) => println!("[verify_b] mine 失败: {e}"),
    }
    std::thread::sleep(std::time::Duration::from_secs(3));

    // 等超过扫描 TTL(30s)，让 record_surroundings 重验：方块消失 → depleted / forget
    println!("[verify_b] 等待 35s 让 TTL 重验...");
    std::thread::sleep(std::time::Duration::from_secs(35));

    // dump 该坐标状态
    let after = world_mem.get(target);
    match after {
        None => println!(
            "[verify_b] 坐标 {:?} 记忆已移除（forget_pos 生效，B 闭环✓）",
            target
        ),
        Some(c) if c.depleted => println!(
            "[verify_b] 坐标 {:?} 已标记 depleted（挖光保留，B 闭环✓）",
            target
        ),
        Some(c) => println!(
            "[verify_b] 坐标 {:?} 仍存在且未耗尽 item={:?} —— B 未触发（可能方块还在/未挖到）",
            target, c.item
        ),
    }
    println!(
        "[verify_b] 最终记忆 {} 条；depleted 资源点 {} 个",
        world_mem.len(),
        world_mem
            .query(Some(MemoryKind::Resource), None)
            .into_iter()
            .filter(|c| c.depleted)
            .count()
    );
    Ok(())
}

#[cfg(not(feature = "azalea-bot"))]
fn main() {
    eprintln!("需要 --features azalea-bot");
}
