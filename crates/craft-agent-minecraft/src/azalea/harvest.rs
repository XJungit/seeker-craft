//! 自动收割（P86，参考 Mindcraft collectBlock 作物分支）：
//! 扫描附近成熟的农作物（wheat/carrots/potatoes/beetroots/nether_wart）→
//! 徒手挖掘 → 等待掉落物被拾取。未成熟的作物跳过（等 LLM 下次再来）。

use azalea::BlockPos;
use azalea::Client;
use azalea::pathfinder::goals::BlockPosGoal;
use azalea::prelude::*;
use azalea::block::{BlockState, BlockTrait};
use azalea_registry::builtin::BlockKind;
use std::time::Duration;
use tokio::time::sleep;

/// 可收割的作物方块种类（种子作物；甜瓜/南瓜是结果实方块，由 gather 覆盖）。
pub(crate) fn harvestable_crop_kinds() -> Vec<BlockKind> {
    vec![
        BlockKind::Wheat,
        BlockKind::Carrots,
        BlockKind::Potatoes,
        BlockKind::Beetroots,
        BlockKind::NetherWart,
    ]
}

/// 该作物是否成熟（age 达到最大值）。读不到 age 属性视为未成熟。
pub(crate) fn crop_is_mature(state: BlockState) -> bool {
    let kind: BlockKind = state.into();
    let age = state.to_trait().get_property("age");
    match kind {
        BlockKind::Wheat | BlockKind::Carrots | BlockKind::Potatoes => age == Some("7"),
        BlockKind::Beetroots | BlockKind::NetherWart => age == Some("3"),
        _ => false,
    }
}

/// 扫描半径内所有成熟的作物，返回按距离升序的坐标。
fn scan_mature_crops(bot: &Client, radius: i32) -> Vec<BlockPos> {
    let Ok(world) = bot.world() else {
        return Vec::new();
    };
    let w = world.read();
    let Ok(center) = bot.position() else {
        return Vec::new();
    };
    let cx = center.x.floor() as i32;
    let cy = center.y.floor() as i32;
    let cz = center.z.floor() as i32;
    let kinds = harvestable_crop_kinds();
    let mut found = Vec::new();
    for dx in -radius..=radius {
        for dy in -radius..=radius {
            for dz in -radius..=radius {
                let pos = BlockPos::new(cx + dx, cy + dy, cz + dz);
                if let Some(state) = w.get_block_state(pos) {
                    let kind: BlockKind = state.into();
                    if kinds.contains(&kind) && crop_is_mature(state) {
                        found.push(pos);
                    }
                }
            }
        }
    }
    found.sort_by_key(|p| (p.x - cx).pow(2) + (p.y - cy).pow(2) + (p.z - cz).pow(2));
    found
}

/// 执行收割：挖掉附近全部成熟作物（上限 `max_crops` 棵），返回收获统计。
pub(crate) async fn do_harvest(bot: &Client, max_crops: u32) -> Result<String, String> {
    let radius = 32;
    let mut harvested = 0u32;
    let mut last_error: Option<String> = None;

    for _ in 0..max_crops.max(1) {
        // 1) 找最近的成熟作物
        let Some(target) = scan_mature_crops(bot, radius).into_iter().next() else {
            break;
        };

        // 2) 走到作物下方（作物长在 farmland 上，站其脚下贴脸挖）
        let stand = BlockPos::new(target.x, target.y - 1, target.z);
        bot.start_goto(BlockPosGoal(stand));
        let mut reached = false;
        for _ in 0..40 {
            sleep(Duration::from_millis(100)).await;
            if let Ok(p) = bot.position() {
                let d = ((p.x - target.x as f64).powi(2)
                    + (p.y - target.y as f64).powi(2)
                    + (p.z - target.z as f64).powi(2))
                .sqrt();
                if d < 3.0 {
                    reached = true;
                    break;
                }
            }
        }
        bot.stop_pathfinding();
        if !reached {
            last_error = Some(format!(
                "无法走到成熟作物 @ ({},{},{})（可能被阻挡）",
                target.x, target.y, target.z
            ));
            continue;
        }

        // 3) 徒手挖作物（作物不需要工具）
        bot.start_mining(target);

        // 4) 等待方块消失
        let mut gone = false;
        for _ in 0..40 {
            sleep(Duration::from_millis(100)).await;
            let Ok(world) = bot.world() else {
                continue;
            };
            let is_gone = world
                .read()
                .get_block_state(target)
                .map(|s| s.is_air())
                .unwrap_or(true);
            if is_gone {
                gone = true;
                break;
            }
        }
        bot.stop_pathfinding();
        if !gone {
            last_error = Some(format!(
                "挖成熟作物 @ ({},{},{}) 4s 未消失（可能服务端拒绝）",
                target.x, target.y, target.z
            ));
            continue;
        }

        // 5) 等 1.5s 让 bot 拾取掉落物（wheat 掉 wheat+wheat_seeds 等）
        sleep(Duration::from_millis(1500)).await;
        harvested += 1;
    }

    if harvested == 0 {
        let err = last_error.unwrap_or_else(|| {
            format!("附近 {radius} 格内没有成熟作物——未成熟的作物不会掉落小麦，请等待作物成熟（或先用 till_and_sow 补种）")
        });
        Err(err)
    } else {
        Ok(format!(
            "收割完成：共挖掉 {harvested} 棵成熟作物（掉落物已自动拾取，含小麦/种子等）。若有未成熟作物可稍后再来收割"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harvestable_kinds_are_seed_crops() {
        let kinds = harvestable_crop_kinds();
        assert_eq!(kinds.len(), 5);
        assert!(kinds.contains(&BlockKind::Wheat));
        assert!(kinds.contains(&BlockKind::NetherWart));
        assert!(!kinds.contains(&BlockKind::Pumpkin));
    }

    #[test]
    fn mature_judgement_by_age() {
        // 用 blocks 模块的具体方块结构体构造不同 age 的作物状态
        let mut age0 = azalea::block::blocks::Wheat::default();
        let _ = age0.set_property("age", "0");
        assert!(!crop_is_mature(BlockState::from(age0)));

        let mut age7 = azalea::block::blocks::Wheat::default();
        let _ = age7.set_property("age", "7");
        assert!(crop_is_mature(BlockState::from(age7)));

        let mut beet3 = azalea::block::blocks::Beetroots::default();
        let _ = beet3.set_property("age", "3");
        assert!(crop_is_mature(BlockState::from(beet3)));

        let mut beet2 = azalea::block::blocks::Beetroots::default();
        let _ = beet2.set_property("age", "2");
        assert!(!crop_is_mature(BlockState::from(beet2)));

        // 非作物一律不算成熟
        let stone = BlockState::from(azalea::block::blocks::Stone::default());
        assert!(!crop_is_mature(stone));
    }
}

