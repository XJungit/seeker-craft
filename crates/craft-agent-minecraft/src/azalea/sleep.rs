//! 睡觉跳夜（P85，参考 Mindcraft goToBed）：
//! 夜晚找床 → 走到床旁 → 空主手 → 右键床 → 轮询 Sleeping 组件验证入睡 →
//! 睡到自然醒（跳过夜晚）。白天睡觉服务端会拒绝并返回错误。

use azalea::Client;
use azalea::BlockPos;
use azalea::pathfinder::goals::BlockPosGoal;
use azalea::prelude::*;
use azalea_registry::builtin::BlockKind;
use std::time::Duration;

use crate::azalea::smart_actions::scan_blocks_multi;

/// 全部 16 种颜色的床方块。
pub(crate) fn bed_block_kinds() -> Vec<BlockKind> {
    vec![
        BlockKind::WhiteBed,
        BlockKind::OrangeBed,
        BlockKind::MagentaBed,
        BlockKind::LightBlueBed,
        BlockKind::YellowBed,
        BlockKind::LimeBed,
        BlockKind::PinkBed,
        BlockKind::GrayBed,
        BlockKind::LightGrayBed,
        BlockKind::CyanBed,
        BlockKind::PurpleBed,
        BlockKind::BlueBed,
        BlockKind::BrownBed,
        BlockKind::GreenBed,
        BlockKind::RedBed,
        BlockKind::BlackBed,
    ]
}

/// 找半径内最近的床。返回床方块坐标。
pub(crate) fn find_bed(bot: &Client, radius: i32) -> Option<BlockPos> {
    let world = bot.world().ok()?;
    let w = world.read();
    let center = bot.position().ok()?;
    scan_blocks_multi(&w, center, &bed_block_kinds(), radius)
}

/// 若主手持有物品（右键床会放置方块而非睡觉），切到空 hotbar 槽。
/// 无空槽时返回 false——调用方应报错提示先腾空主手。
fn empty_main_hand(bot: &Client) -> bool {
    if let Ok(st) = bot.get_held_item()
        && st.is_empty()
    {
        return true;
    }
    let inv = match bot.get_inventory() {
        Ok(i) => i,
        Err(_) => return false,
    };
    let menu = match inv.menu().ok().flatten() {
        Some(m) => m,
        None => return false,
    };
    let Some(slots) = inv.slots() else {
        return false;
    };
    // hotbar_slots_range 是绝对槽位（如 36..=44），set_selected_hotbar_slot 需要 0..=8
    let hotbar_start = *menu.hotbar_slots_range().start();
    for s in menu.hotbar_slots_range() {
        if slots.get(s).map(|st| st.is_empty()).unwrap_or(false) {
            bot.set_selected_hotbar_slot((s - hotbar_start) as u8);
            return true;
        }
    }
    false
}

/// 是否正在睡觉（轮询 SleepingPos 组件；玩家睡眠时该组件为 Some(床坐标)）。
fn is_sleeping(bot: &Client) -> bool {
    bot.query_self::<&azalea::entity::metadata::SleepingPos, _>(|s| s.0.is_some())
        .unwrap_or(false)
}

/// 执行睡觉：找床 → 靠近 → 空主手 → 右键 → 验证入睡 → 等自然醒。
pub(crate) async fn do_sleep(bot: &Client) -> Result<String, String> {
    // 1. 找床
    let Some(bed) = find_bed(bot, 32) else {
        return Err(
            "附近 32m 内没有床——请先造一张床（如 craft_3x3(\"red_bed\")，需 3 羊毛+3 木板）或找到村庄的床"
                .to_string(),
        );
    };

    // 2. 靠近床（≤2m）
    let p = bot.position().map_err(|e| format!("读取位置失败: {e:?}"))?;
    let dist = ((p.x - bed.x as f64).powi(2)
        + (p.y - bed.y as f64).powi(2)
        + (p.z - bed.z as f64).powi(2))
        .sqrt();
    if dist > 2.0 {
        let stand = BlockPos::new(bed.x, bed.y, bed.z);
        bot.start_goto(BlockPosGoal(stand));
        let mut arrived = false;
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if let Ok(p) = bot.position() {
                let d = ((p.x - bed.x as f64).powi(2)
                    + (p.y - bed.y as f64).powi(2)
                    + (p.z - bed.z as f64).powi(2))
                    .sqrt();
                if d < 2.0 {
                    arrived = true;
                    break;
                }
            }
        }
        if !arrived {
            return Err(format!("走到床 ({},{},{}) 失败（5s 超时）——可能有障碍", bed.x, bed.y, bed.z));
        }
    }

    // 3. 空主手（避免右键放置方块盖住床）
    if !empty_main_hand(bot) {
        return Err("主手有物品且 hotbar 没有空槽——请先 discard 或移动物品腾出空槽再睡觉".to_string());
    }

    // 4. 右键床
    bot.block_interact(bed);

    // 5. 验证入睡（最长 3s；白天/附近有怪物会被服务端拒绝）
    let mut asleep = false;
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if is_sleeping(bot) {
            asleep = true;
            break;
        }
    }
    if !asleep {
        return Err(format!(
            "右键床 ({},{},{}) 后 3s 内未入睡——可能不是夜晚（服务端会拒绝），或附近有怪物",
            bed.x, bed.y, bed.z
        ));
    }

    // 6. 等自然醒（睡过夜自动醒来；最长 15s 兜底）
    for _ in 0..150 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if !is_sleeping(bot) {
            break;
        }
    }
    Ok("已睡觉跳过夜晚（醒来）".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bed_kinds_has_all_16_colors() {
        let kinds = bed_block_kinds();
        assert_eq!(kinds.len(), 16);
        assert!(kinds.contains(&BlockKind::RedBed));
        assert!(kinds.contains(&BlockKind::WhiteBed));
        assert!(kinds.contains(&BlockKind::BlackBed));
        assert!(!kinds.contains(&BlockKind::CraftingTable));
    }
}
