//! 种植工具（P84，参考 Mindcraft tillAndSow）：
//! 校验目标为 dirt/grass_block/farmland → 靠近 4.5m → 持锄头右键犁地 →
//! 持种子右键播种。全程不依赖 LLM 逐步操作，一次命令完成。

use azalea::BlockPos;
use azalea::Client;
use azalea::pathfinder::goals::BlockPosGoal;
use azalea::prelude::*;
use azalea_registry::builtin::BlockKind;
use azalea_registry::builtin::ItemKind;
use std::time::Duration;

/// 种子物品名 → 种下后上方生成的作物方块（用于播种验证与幂等判断）。
/// 覆盖常见主食作物。返回 None = 不支持的种子。
pub(crate) fn seed_to_crop_kind(seed: &str) -> Option<BlockKind> {
    match seed {
        "wheat_seeds" => Some(BlockKind::Wheat),
        "beetroot_seeds" => Some(BlockKind::Beetroots),
        "carrot" => Some(BlockKind::Carrots),
        "potato" => Some(BlockKind::Potatoes),
        "melon_seeds" => Some(BlockKind::MelonStem),
        "pumpkin_seeds" => Some(BlockKind::PumpkinStem),
        _ => None,
    }
}

/// 可犁方块：草方块/泥土/已耕地。
pub(crate) fn is_tillable(kind: BlockKind) -> bool {
    matches!(
        kind,
        BlockKind::GrassBlock | BlockKind::Dirt | BlockKind::Farmland
    )
}

/// 在背包（含 hotbar）里找任意锄头，返回物品名（如 "iron_hoe"）。
/// 无锄头返回 None——调用方应提示先 craft 锄头。
pub(crate) fn find_hoe_in_inventory(bot: &Client) -> Option<String> {
    let inv = bot.get_inventory().ok()?;
    let menu = inv.menu().ok().flatten()?;
    let slots = inv.slots()?;
    let range = menu.player_slots_range();
    let mut best: Option<(u8, String)> = None;
    // 锄头品质（好→差），与 pickaxe 同思路
    fn hoe_rank(k: ItemKind) -> Option<u8> {
        match k {
            ItemKind::NetheriteHoe => Some(6),
            ItemKind::DiamondHoe => Some(5),
            ItemKind::IronHoe => Some(4),
            ItemKind::GoldenHoe => Some(3),
            ItemKind::StoneHoe => Some(2),
            ItemKind::WoodenHoe => Some(1),
            _ => None,
        }
    }
    for s in range {
        if let Some(st) = slots.get(s)
            && !st.is_empty()
        {
            let kind = st.kind();
            if let Some(rank) = hoe_rank(kind)
                && best.as_ref().map(|(r, _)| rank > *r).unwrap_or(true)
            {
                best = Some((rank, kind.to_string()));
            }
        }
    }
    best.map(|(_, name)| name)
}

/// 在目标附近（半径 4，y±1）找可犁且上方无阻挡的方块。
/// P102 新增：LLM 常传错一格坐标（记忆/感知偏差），目标格是空气/草而非 dirt，
/// 直接报错会让 LLM 反复试错（实机 4 次连续失败）。自动修正到最近合法位置。
fn find_tillable_nearby(bot: &Client, target: BlockPos) -> Option<(BlockPos, BlockKind)> {
    let world = bot.world().ok()?;
    let mut best: Option<(i32, BlockPos, BlockKind)> = None;
    for dy in -1..=1i32 {
        for dx in -4..=4i32 {
            for dz in -4..=4i32 {
                let pos = BlockPos::new(target.x + dx, target.y + dy, target.z + dz);
                let w = world.read();
                let Some(kind): Option<BlockKind> = w.get_block_state(pos).map(Into::into) else {
                    continue;
                };
                if !is_tillable(kind) {
                    continue;
                }
                let above = w
                    .get_block_state(pos.up(1))
                    .map(Into::into)
                    .unwrap_or(BlockKind::Air);
                if above != BlockKind::Air {
                    continue;
                }
                let d = dx.abs() + dy.abs() + dz.abs();
                if best.as_ref().map(|(bd, _, _)| d < *bd).unwrap_or(true) {
                    best = Some((d, pos, kind));
                }
            }
        }
    }
    best.map(|(_, pos, kind)| (pos, kind))
}

/// 执行犁地+播种。返回 Err(msg) 时 msg 面向 LLM，可直接展示。
/// 幂等：目标已是 farmland 且上方有作物 → Ok（不重复种）。
/// P102：目标不可犁时自动修正到附近（半径 4，y±1）合法位置并通知 LLM。
pub(crate) async fn do_till_and_sow(
    bot: &Client,
    x: i32,
    y: i32,
    z: i32,
    seed: &str,
) -> Result<String, String> {
    // 0. 种子支持校验
    let crop = seed_to_crop_kind(seed)
        .ok_or_else(|| format!("不支持的种子 {seed}（支持 wheat_seeds/beetroot_seeds/carrot/potato/melon_seeds/pumpkin_seeds）"))?;

    let (target_pos, target_kind, corrected) = {
        let world = bot.world().map_err(|e| format!("读取世界失败: {e:?}"))?;
        let w = world.read();
        let orig_pos = BlockPos::new(x, y, z);
        match w.get_block_state(orig_pos).map(Into::into) {
            Some(kind) if is_tillable(kind) => (orig_pos, kind, None),
            Some(other) => {
                // P102：目标不可犁（空气/其他方块）→ 自动修正到附近可犁位置并继续
                drop(w);
                let (fixed, k) = find_tillable_nearby(bot, orig_pos).ok_or_else(|| {
                    format!(
                        "({x},{y},{z}) 是 {:?}，且附近 4 格内无可用草方块/泥土/已耕地——\
                         只能犁草方块/泥土/已耕地，请换位置或用 place dirt 先铺泥土",
                        other
                    )
                })?;
                (fixed, k, Some((orig_pos, other)))
            }
            None => {
                // 未加载：交给下方统一处理（会报"目标方块未加载"）
                drop(w);
                (orig_pos, BlockKind::Air, None)
            }
        }
    };
    let above_pos = target_pos.up(1);

    // 1. 目标方块校验（修正后位置）
    if !is_tillable(target_kind) {
        let world = bot.world().map_err(|e| format!("读取世界失败: {e:?}"))?;
        let w = world.read();
        let cur = w
            .get_block_state(target_pos)
            .map(Into::into)
            .unwrap_or(BlockKind::Air);
        return Err(format!(
            "({},{},{}) 是 {:?}，只能犁草方块/泥土/已耕地",
            target_pos.x, target_pos.y, target_pos.z, cur
        ));
    }
    if let Some((orig, other)) = corrected {
        eprintln!(
            "[till] P102 修正：原目标 ({},{},{}) 是 {:?}，改犁 ({},{},{}) 是 {:?}",
            orig.x, orig.y, orig.z, other, target_pos.x, target_pos.y, target_pos.z, target_kind
        );
    }

    // 2. 上方方块校验
    let above_kind: BlockKind = {
        let world = bot.world().map_err(|e| format!("读取世界失败: {e:?}"))?;
        let w = world.read();
        w.get_block_state(above_pos)
            .map(Into::into)
            .unwrap_or(BlockKind::Air)
    };
    if target_kind == BlockKind::Farmland {
        if above_kind != BlockKind::Air {
            // 幂等：已是耕地且上方有作物
            return Ok(format!(
                "({x},{y},{z}) 已是耕地且上方有 {:?}，无需重种",
                above_kind
            ));
        }
    } else if above_kind != BlockKind::Air {
        return Err(format!(
            "({x},{y},{z}) 上方有 {:?} 阻挡，无法犁地——请先移除该方块",
            above_kind
        ));
    }

    // 3. 距离检查 + 自动靠近（P100：force_block 交互需贴近，2.9m 外播种静默失败）
    //    距离以修正后的 target_pos 为准（P102 修正后坐标可能偏移）。
    let p = bot.position().map_err(|e| format!("读取位置失败: {e:?}"))?;
    let dist = ((p.x - target_pos.x as f64).powi(2)
        + (p.y - target_pos.y as f64).powi(2)
        + (p.z - target_pos.z as f64).powi(2))
    .sqrt();
    if dist > 8.0 {
        return Err(format!(
            "({},{},{}) 距离 {dist:.1}m 过远（交互距离 4.5m）——请先 goto 到目标旁再 till_and_sow",
            target_pos.x, target_pos.y, target_pos.z
        ));
    }
    if dist > 2.0 {
        // 自动走到目标旁（站在目标格，pathfinder 会在其相邻格停下）
        bot.start_goto(BlockPosGoal(target_pos));
        let mut reached = false;
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if let Ok(p) = bot.position() {
                let d = ((p.x - target_pos.x as f64).powi(2)
                    + (p.y - target_pos.y as f64).powi(2)
                    + (p.z - target_pos.z as f64).powi(2))
                .sqrt();
                if d <= 2.0 {
                    reached = true;
                    break;
                }
            }
        }
        bot.stop_pathfinding();
        if !reached {
            return Err(format!(
                "无法走近目标 ({},{},{})（6s 内未到达 2m 内，当前 {dist:.1}m）——路径可能被阻挡，请换位置",
                target_pos.x, target_pos.y, target_pos.z
            ));
        }
    }

    // 4. 犁地（非 farmland 时）
    if target_kind != BlockKind::Farmland {
        let hoe = find_hoe_in_inventory(bot).ok_or_else(|| {
            "背包里没有锄头——请先 craft 锄头（如 craft_3x3(\"wooden_hoe\")）".to_string()
        })?;
        let eq_msg = super::do_equip(bot, &hoe, "hand").await;
        if !eq_msg.starts_with("已装备") {
            return Err(format!("装备锄头失败: {eq_msg}"));
        }
        bot.block_interact(target_pos);
        tokio::time::sleep(Duration::from_millis(400)).await;
        let now_kind: BlockKind = {
            let w = bot.world().map_err(|e| format!("读取世界失败: {e:?}"))?;
            let w = w.read();
            w.get_block_state(target_pos)
                .map(Into::into)
                .unwrap_or(BlockKind::Air)
        };
        if now_kind != BlockKind::Farmland {
            return Err(format!(
                "犁地 ({x},{y},{z}) 后目标仍是 {:?}，交互未生效——请确认手持锄头后重试",
                now_kind
            ));
        }
    }

    // 5. 播种
    let eq_msg = super::do_equip(bot, seed, "hand").await;
    if !eq_msg.starts_with("已装备") {
        return Err(format!("装备种子失败: {eq_msg}"));
    }
    bot.block_interact(target_pos);
    tokio::time::sleep(Duration::from_millis(400)).await;
    let now_above: BlockKind = {
        let w = bot.world().map_err(|e| format!("读取世界失败: {e:?}"))?;
        let w = w.read();
        w.get_block_state(above_pos)
            .map(Into::into)
            .unwrap_or(BlockKind::Air)
    };
    if now_above != crop {
        return Err(format!(
            "播种后 ({x},{}, {z}) 是 {:?}，期望 {:?}——交互未生效",
            y + 1,
            now_above,
            crop
        ));
    }

    let mut msg = format!(
        "已犁地并种下 {seed} @ ({},{},{})（上方已长 {:?}，等待成熟后收割）",
        target_pos.x, target_pos.y, target_pos.z, crop
    );
    if let Some((orig, other)) = corrected {
        msg = format!(
            "原目标 ({},{},{}) 是 {:?}（非可犁方块），已自动修正犁最近可犁方块 ({},{},{}) 并完成。{}",
            orig.x, orig.y, orig.z, other, target_pos.x, target_pos.y, target_pos.z, msg
        );
    }
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_to_crop_mapping() {
        assert_eq!(seed_to_crop_kind("wheat_seeds"), Some(BlockKind::Wheat));
        assert_eq!(
            seed_to_crop_kind("beetroot_seeds"),
            Some(BlockKind::Beetroots)
        );
        assert_eq!(seed_to_crop_kind("carrot"), Some(BlockKind::Carrots));
        assert_eq!(seed_to_crop_kind("potato"), Some(BlockKind::Potatoes));
        assert_eq!(seed_to_crop_kind("melon_seeds"), Some(BlockKind::MelonStem));
        assert_eq!(
            seed_to_crop_kind("pumpkin_seeds"),
            Some(BlockKind::PumpkinStem)
        );
        assert_eq!(seed_to_crop_kind("dirt"), None);
        assert_eq!(seed_to_crop_kind("wheat"), None);
    }

    #[test]
    fn tillable_kinds() {
        assert!(is_tillable(BlockKind::GrassBlock));
        assert!(is_tillable(BlockKind::Dirt));
        assert!(is_tillable(BlockKind::Farmland));
        assert!(!is_tillable(BlockKind::Stone));
        assert!(!is_tillable(BlockKind::Water));
        assert!(!is_tillable(BlockKind::Air));
    }
}
