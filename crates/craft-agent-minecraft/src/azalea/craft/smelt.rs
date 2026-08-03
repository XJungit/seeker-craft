//! 共享合成工具函数（被 craft_table/smelt/brew/enchant/smith 各域复用）。
use super::*;

/// 熔炼：配方表 + 熔炉执行（对齐 mindcraft smeltItem）。
/// 熔炼配方：产物 -> (输入物品 id, 每次产出数)。
pub(crate) struct SmeltRecipe {
    pub(crate) input: &'static str,
    pub(crate) output_per_craft: u32,
}

pub(crate) const SMELT_RECIPES: &[(&str, SmeltRecipe)] = &[
    (
        "iron_ingot",
        SmeltRecipe {
            input: "iron_ore",
            output_per_craft: 1,
        },
    ),
    (
        "iron_ingot",
        SmeltRecipe {
            input: "raw_iron",
            output_per_craft: 1,
        },
    ),
    (
        "copper_ingot",
        SmeltRecipe {
            input: "copper_ore",
            output_per_craft: 1,
        },
    ),
    (
        "copper_ingot",
        SmeltRecipe {
            input: "raw_copper",
            output_per_craft: 1,
        },
    ),
    (
        "gold_ingot",
        SmeltRecipe {
            input: "gold_ore",
            output_per_craft: 1,
        },
    ),
    (
        "gold_ingot",
        SmeltRecipe {
            input: "raw_gold",
            output_per_craft: 1,
        },
    ),
    (
        "glass",
        SmeltRecipe {
            input: "sand",
            output_per_craft: 1,
        },
    ),
    (
        "stone",
        SmeltRecipe {
            input: "cobblestone",
            output_per_craft: 1,
        },
    ),
    (
        "smooth_stone",
        SmeltRecipe {
            input: "stone",
            output_per_craft: 1,
        },
    ),
    (
        "charcoal",
        SmeltRecipe {
            input: "oak_log",
            output_per_craft: 1,
        },
    ),
    (
        "baked_potato",
        SmeltRecipe {
            input: "potato",
            output_per_craft: 1,
        },
    ),
    // P78：熟肉熔炼（LLM 杀了羊想烤 cooked_mutton 被拒——SMELT_RECIPES 缺全部食物。
    // 食物保障主线核心：生肉→熟肉 +3 饥饿/+3 饱和度（直接吃生肉回 3 但掉饱食/中毒）。
    (
        "cooked_beef",
        SmeltRecipe {
            input: "beef",
            output_per_craft: 1,
        },
    ),
    (
        "cooked_porkchop",
        SmeltRecipe {
            input: "porkchop",
            output_per_craft: 1,
        },
    ),
    (
        "cooked_chicken",
        SmeltRecipe {
            input: "chicken",
            output_per_craft: 1,
        },
    ),
    (
        "cooked_mutton",
        SmeltRecipe {
            input: "mutton",
            output_per_craft: 1,
        },
    ),
    (
        "cooked_rabbit",
        SmeltRecipe {
            input: "rabbit",
            output_per_craft: 1,
        },
    ),
    (
        "cooked_cod",
        SmeltRecipe {
            input: "cod",
            output_per_craft: 1,
        },
    ),
    (
        "cooked_salmon",
        SmeltRecipe {
            input: "salmon",
            output_per_craft: 1,
        },
    ),
];

pub(crate) fn lookup_smelt_all(output: &str) -> Vec<SmeltRecipe> {
    // P17 修复（2026-07-27）：原代码用 normalize_item(output) 把 id 变成
    // "minecraft:iron_ingot"，但 SMELT_RECIPES 存的是裸 id "iron_ingot"，
    // 比较永远 false → lookup_smelt 100% 返回 None → smelt 报"不支持 iron_ingot"
    // 但错误消息里又列出 iron_ingot 作为支持项，自相矛盾。
    // 修复：用 bare() 去前缀（与 P12 lookup_shaped 修复同理）。
    //
    // P18 修复（2026-07-27）：返回**所有**候选配方（不再 .find().first()）。
    // SMELT_RECIPES 中 iron_ingot 有两条候选：iron_ore 和 raw_iron。
    // vanilla 中挖 iron_ore 掉 raw_iron（不是 iron_ore 本身），所以 bot
    // 背包里通常是 raw_iron。原 lookup_smelt 只返回第一个（iron_ore），
    // do_smelt 报"背包缺少输入 iron_ore"——但 bot 实际有 raw_iron×7！
    // 修复：返回所有候选，do_smelt 优先选背包里有的原料。
    let norm = bare(output);
    SMELT_RECIPES
        .iter()
        .filter(|(id, _)| *id == norm)
        .map(|(_, r)| SmeltRecipe {
            input: r.input,
            output_per_craft: r.output_per_craft,
        })
        .collect()
}
pub async fn do_smelt(
    bot: &Client,
    output: &str,
    fuel: &str,
    count: u32,
) -> Result<String, String> {
    // ============================================================
    // P47 系统性重构（2026-07-27）：对齐 mindcraft smeltItem。
    //
    // 学习自 mindcraft src/agent/library/skills.js smeltItem (line 142-273)：
    //   1. isSmeltable 闸门 → 我方 lookup_smelt_all 判空
    //   2. 仅在背包有 furnace 时放炉（不自动合成）→ P45 已对齐
    //   3. 检查炉子是否在炼别的东西 → P47 新增
    //   4. 检查原料数量 → P43 已对齐
    //   5. 一次性放足燃料（Math.ceil(num/output)）→ P47 新增（原每次放 1 个）
    //   6. takeOutput 循环（每秒轮询，11s 无产物才 break）→ P47 重构（原固定等 30s）
    //   7. 回收 input/fuel 槽剩余物 → P47 新增
    //
    // 核心改动：从"每次放 1 原料+1 燃料 → 等 30s → 取 1 产物"的串行循环，
    // 改为"一次性放足原料+燃料 → takeOutput 循环动态收集 → 回收剩余"的并行流水线。
    // 这与 mindcraft 行为一致，且更高效（不浪费燃料预热时间）。
    // ============================================================

    let candidates = lookup_smelt_all(output);
    if candidates.is_empty() {
        return Err(format!(
            "不支持的熔炼产物 {output}（当前支持 iron_ingot/copper_ingot/gold_ingot/glass/stone/charcoal 等，需先打开熔炉）"
        ));
    }

    let inv = bot
        .get_inventory()
        .map_err(|e| format!("获取容器失败（确认已打开熔炉）: {e:?}"))?;

    // 对齐 mindcraft line 186-194：检查炉子是否在炼别的东西。
    // mindcraft: if (input_item && input_item.type !== mc.getItemId(itemName) && input_item.count > 0)
    // 我方：若炉子 input 槽已有别种物品，不抢占，直接报错让 LLM 处理。
    let inv_slots = inv.slots();
    let existing_input = inv_slots
        .as_ref()
        .and_then(|s| s.first())
        .filter(|st| !st.is_empty());
    if let Some(existing) = existing_input {
        let input_kind_check = ItemKind::from_str(&normalize_item(
            candidates.iter().map(|c| c.input).next().unwrap_or(""),
        ))
        .ok();
        if let Some(expected) = input_kind_check
            && existing.kind() != expected
        {
            return Err(format!(
                "熔炉正在炼别的东西（input 槽有 {}x{}，期望 {}）。\
                     不抢占炉子。建议：1) 等当前熔炼完成；2) 打开另一个炉子；3) 关闭炉子取回原料后重试。",
                existing.kind().to_str(),
                existing.count(),
                expected.to_str()
            ));
        }
    }

    // 优先选背包里有的原料；若都没有，选第一个候选并报错（列出所有候选原料）
    let recipe = {
        let mut chosen: Option<SmeltRecipe> = None;
        for c in &candidates {
            let kind = match ItemKind::from_str(&normalize_item(c.input)) {
                Ok(k) => k,
                Err(_) => continue,
            };
            if find_source_slot(&inv, kind).is_some() {
                chosen = Some(SmeltRecipe {
                    input: c.input,
                    output_per_craft: c.output_per_craft,
                });
                break;
            }
        }
        match chosen {
            Some(r) => r,
            None => {
                let inputs: Vec<&str> = candidates.iter().map(|c| c.input).collect();
                return Err(format!(
                    "背包缺少熔炼 {output} 的原料（候选: {}）。\
                     vanilla 规则：挖 {} 矿掉的是 raw_xxx（不是 ore 本身）\
                     ——请用 perceive 查看背包实际拥有的原料 id。",
                    inputs.join(" / "),
                    output
                ));
            }
        }
    };

    let input_kind = ItemKind::from_str(&normalize_item(recipe.input))
        .map_err(|_| format!("未知输入 {}", recipe.input))?;
    let fuel_kind =
        ItemKind::from_str(&normalize_item(fuel)).map_err(|_| format!("未知燃料 {fuel}"))?;

    // P22: 燃料 fallback 列表（保留，与 mindcraft getSmeltingFuel 等价）
    let fuel_candidates: Vec<ItemKind> = {
        let mut v = vec![fuel_kind];
        let fallbacks = [
            "coal",
            "charcoal",
            "oak_log",
            "birch_log",
            "spruce_log",
            "jungle_log",
            "acacia_log",
            "dark_oak_log",
            "mangrove_log",
            "cherry_log",
            "pale_oak_log",
            "oak_planks",
            "birch_planks",
            "spruce_planks",
            "jungle_planks",
            "acacia_planks",
            "dark_oak_planks",
            "mangrove_planks",
            "cherry_planks",
            "pale_oak_planks",
            "stick",
            "coal_block",
        ];
        for f in fallbacks {
            if let Ok(k) = ItemKind::from_str(&normalize_item(f))
                && !v.contains(&k)
            {
                v.push(k);
            }
        }
        v
    };

    // P43: 按背包实际原料数量调整熔炼数（保留）
    let inv_for_count = bot
        .get_inventory()
        .map_err(|e| format!("获取容器失败（P43 计数）: {e:?}"))?;
    let actual_input = count_item_in_player_slots(&inv_for_count, input_kind);
    let requested_count = count.max(1);
    if actual_input == 0 {
        return Err(format!(
            "背包无 {}（熔炼 {} 需要 {}）。请先 gather 采集 {} 后再 smelt。",
            recipe.input, output, recipe.input, recipe.input
        ));
    }
    let actual_smelt_count = actual_input.min(requested_count);
    if actual_input < requested_count {
        eprintln!(
            "[smelt] P43: 背包只有 {} 个 {}，少于请求的 {} 个，按实际数量熔炼 {} 个",
            actual_input, recipe.input, requested_count, actual_smelt_count
        );
    }

    // P57 分批熔炼（2026-07-27）：避免工具调用 120s 超时。
    // 实测 scan_20260727_212144.md 显示 smelt(count=15) 超时 120s——
    // 15 个 × 10s/个 = 150s > 120s 工具调用超时。
    // 修复：单次最多熔炼 8 个（80s + 11s 无产物超时 + 放料/回收 ≈ 95s < 120s）。
    // 剩余的让 LLM 看到返回结果后再次调用 smelt 继续。
    // 这对齐 mindcraft 哲学：工具做能做的部分，LLM 决策下一步。
    const MAX_SMELT_PER_BATCH: u32 = 8;
    let original_count = actual_smelt_count;
    let actual_smelt_count = actual_smelt_count.min(MAX_SMELT_PER_BATCH);
    if actual_smelt_count < original_count {
        eprintln!(
            "[smelt] P57: 请求熔炼 {} 个，但单批上限 {} 个（避免 120s 超时），本次熔炼 {} 个，剩余 {} 个下次继续",
            original_count,
            MAX_SMELT_PER_BATCH,
            actual_smelt_count,
            original_count - actual_smelt_count
        );
    }

    // P47 对齐 mindcraft line 205-226：一次性放足燃料。
    // mindcraft: const put_fuel = Math.ceil(num / mc.getFuelSmeltOutput(fuel.name));
    // 我方：根据燃料类型算每单位能炼几个，再算需要的燃料数。
    // 燃烧时间（vanilla ticks → 秒 → 每个物品 10s）：
    //   coal/charcoal = 80s → 8 个物品 / coal_block = 800s → 80 个
    //   log = 15s → 1.5 个（取 1）/ planks = 15s → 1.5 个（取 1）/ stick = 5s → 0.5 个（取 1，2 个 stick 炼 1 个）
    let (fuel_per_item, _fuel_burn_seconds) = fuel_candidates
        .iter()
        .map(|&fk| {
            let name = fk.to_str();
            let per_item: u32 = if name.contains("coal") && !name.contains("block") {
                8 // coal/charcoal: 80s / 10s = 8
            } else {
                1
            };
            (per_item, 0)
        })
        .next()
        .unwrap_or((1, 0));

    // 清理熔炉槽位 + 光标（P32 保留）
    clear_grid(&inv, 0..=1).await;
    clear_cursor(&inv).await;

    // P47 对齐 mindcraft line 228-229：一次性放足原料 + 燃料。
    // 找到背包里的原料 slot，按 actual_smelt_count 一次性放入 input 槽（slot 0）
    let inv_put = bot
        .get_inventory()
        .map_err(|e| format!("获取容器失败（放料前）: {e:?}"))?;
    let src_in = find_source_slot(&inv_put, input_kind)
        .ok_or_else(|| format!("背包缺少输入 {}", recipe.input))?;

    // 找到燃料 slot
    let src_fuel = fuel_candidates
        .iter()
        .find_map(|&fk| find_source_slot(&inv_put, fk))
        .ok_or_else(|| {
            format!(
                "背包缺少燃料 {fuel}（也无可用的替代燃料 coal/charcoal/log/planks/stick）。\
                 vanilla 燃料燃烧时间：coal/charcoal=80s（8 个物品）、log=15s（1.5 个）、\
                 planks=15s（1.5 个）、stick=5s（0.5 个）。\
                 建议：1) gather oak_log/spruce_log 等原木；2) craft planks 后做燃料；\
                 3) mine coal_ore 获得 coal（最佳燃料）。"
            )
        })?;
    let actual_fuel_kind = {
        let inv_put_slots = inv_put.slots();
        inv_put_slots
            .as_ref()
            .and_then(|s| s.get(src_fuel))
            .filter(|st| !st.is_empty())
            .map(|st| st.kind())
            .unwrap_or(fuel_kind)
    };
    let actual_fuel_name = actual_fuel_kind.to_str();
    // 计算实际需要的燃料数
    let fuel_needed = if fuel_per_item == 0 {
        actual_smelt_count
    } else {
        actual_smelt_count.div_ceil(fuel_per_item)
    };
    eprintln!(
        "[smelt] P47: 准备熔炼 {} x{}，燃料 {} 每个炼 {} 个，需要燃料 {} 个",
        output, actual_smelt_count, actual_fuel_name, fuel_per_item, fuel_needed
    );

    // 放原料（用 move_stack 放 actual_smelt_count 个到 slot 0）
    // mindcraft: await furnace.putInput(mc.getItemId(itemName), null, num);
    clear_cursor(&inv_put).await;
    move_stack_count(&inv_put, src_in, 0, actual_smelt_count).await;
    clear_cursor(&inv_put).await;

    // 放燃料（用 move_stack_count 放 fuel_needed 个到 slot 1）
    // mindcraft: await furnace.putFuel(fuel.type, null, put_fuel);
    let inv_fuel = bot
        .get_inventory()
        .map_err(|e| format!("获取容器失败（放燃料前）: {e:?}"))?;
    let src_fuel_now = fuel_candidates
        .iter()
        .find_map(|&fk| find_source_slot(&inv_fuel, fk))
        .ok_or_else(|| format!("放燃料时背包找不到燃料（{actual_fuel_name}）"))?;
    clear_cursor(&inv_fuel).await;
    move_stack_count(&inv_fuel, src_fuel_now, 1, fuel_needed).await;
    clear_cursor(&inv_fuel).await;

    // ============================================================
    // P47 对齐 mindcraft line 234-249：takeOutput 循环。
    // mindcraft:
    //   let total = 0;
    //   while (total < num) {
    //     await sleep(1000);
    //     if (furnace.outputItem()) {
    //       smelted_item = await furnace.takeOutput();
    //       if (smelted_item) { total += smelted_item.count; last_collected = Date.now(); }
    //     }
    //     if (Date.now() - last_collected > 11000) break; // 11s 无新产物才超时
    //   }
    //
    // 我方原代码：循环 actual_smelt_count 次，每次放 1 原料+1 燃料，等 30s 取 1 产物。
    // 问题：1) 浪费燃料预热时间；2) 30s 太长，常超时；3) 每次放料状态污染风险高。
    //
    // 重构为：一次性放足料 → 轮询结果槽 → 一有产物立刻取 → 11s 无新产物才 break。
    // ============================================================
    let mut total_smelted = 0u32;
    let mut last_collected_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let target_total = actual_smelt_count; // 每次产出 1 个，目标数 = 原料数

    // 轮询间隔 1s（与 mindcraft 一致），超时 11s 无新产物（与 mindcraft 一致）
    let poll_interval = Duration::from_millis(1000);
    let no_progress_timeout = Duration::from_millis(11000);

    // 等待 200ms 让服务端处理放料（与 mindcraft line 233 一致）
    sleep(Duration::from_millis(200)).await;

    loop {
        if total_smelted >= target_total {
            break;
        }
        sleep(poll_interval).await;

        let inv_now = match bot.get_inventory() {
            Ok(i) => i,
            Err(_) => continue,
        };
        let inv_now_slots = inv_now.slots();
        let result_slot = inv_now_slots
            .as_ref()
            .and_then(|s| s.get(2))
            .filter(|st| !st.is_empty());

        if let Some(result) = result_slot {
            // 有产物，立刻取（shift_click(2) 收回背包）
            // P49 改进（2026-07-27）：对齐 do_craft_3x3 的 P20 逻辑。
            // 原 P47 直接 shift_click(2) + total_smelted += count，没有验证产物是否真进入背包。
            // 问题：背包满时 shift_click 静默失败，产物仍在结果槽，但 total_smelted 已虚增。
            // 下次轮询再次"取到"同一产物，total_smelted 再次虚增，最终报"成功"但实际没拿到。
            // 修复：shift_click 后验证结果槽是否空；不空则 left_click 兜底；仍不空则不计数。
            let expected_kind = result.kind();
            let expected_count = result.count().max(1) as u32;
            inv_now.shift_click(2usize);
            sleep(Duration::from_millis(200)).await;

            // 验证结果槽是否空（产物是否真的被收集）
            let inv_after = match bot.get_inventory() {
                Ok(i) => i,
                Err(_) => continue,
            };
            let inv_after_slots = inv_after.slots();
            let still_has_result = inv_after_slots
                .as_ref()
                .and_then(|s| s.get(2))
                .filter(|st| !st.is_empty())
                .is_some();

            if still_has_result {
                // shift_click 失败（背包满），用 left_click 兜底
                // left_click(2) 把产物拿到光标，再 left_click(empty_slot) 放到背包
                eprintln!("[smelt] P49: shift_click(2) 失败（背包可能满），尝试 left_click 兜底");
                inv_after.left_click(2usize);
                sleep(Duration::from_millis(150)).await;
                let inv_after2 = match bot.get_inventory() {
                    Ok(i) => i,
                    Err(_) => continue,
                };
                if let Some(empty) = find_empty_player_slot(&inv_after2) {
                    inv_after2.left_click(empty);
                    sleep(Duration::from_millis(150)).await;
                } else {
                    // 背包完全满，无法收集，不计数
                    eprintln!(
                        "[smelt] P49: 背包完全满，无法收集产物 {}x{}",
                        expected_kind.to_str(),
                        expected_count
                    );
                    // 不增加 total_smelted，让 11s 超时 break
                    continue;
                }
            }

            // 验证产物真的进入背包（结果槽空了）
            let inv_final_check = match bot.get_inventory() {
                Ok(i) => i,
                Err(_) => continue,
            };
            let inv_final_slots = inv_final_check.slots();
            let result_still_there = inv_final_slots
                .as_ref()
                .and_then(|s| s.get(2))
                .filter(|st| !st.is_empty())
                .is_some();

            if !result_still_there {
                // 产物成功收集
                total_smelted = total_smelted.saturating_add(expected_count);
                last_collected_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(last_collected_ms);
                eprintln!(
                    "[smelt] P49: takeOutput 成功取到 {}x{}（累计 {}/{})",
                    expected_kind.to_str(),
                    expected_count,
                    total_smelted,
                    target_total
                );
            } else {
                eprintln!("[smelt] P49: takeOutput 失败，产物仍在结果槽（不计数）");
            }
        }

        // 11s 无新产物才 break（mindcraft line 244-246）
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(last_collected_ms);
        if now_ms.saturating_sub(last_collected_ms) > no_progress_timeout.as_millis() as u64 {
            eprintln!(
                "[smelt] P47: 11s 无新产物，break 循环（已熔炼 {total_smelted}/{target_total}）"
            );
            break;
        }
    }

    // ============================================================
    // P47 对齐 mindcraft line 251-256：回收 input/fuel 槽剩余物。
    // mindcraft:
    //   if (furnace.inputItem()) await furnace.takeInput();
    //   if (furnace.fuelItem()) await furnace.takeFuel();
    // 我方：shift_click(0) 和 shift_click(1) 把剩余物收回背包。
    // ============================================================
    let inv_final = bot
        .get_inventory()
        .map_err(|e| format!("获取容器失败（回收前）: {e:?}"))?;
    let inv_final_slots = inv_final.slots();
    let input_remaining = inv_final_slots
        .as_ref()
        .and_then(|s| s.first())
        .filter(|st| !st.is_empty())
        .map(|st| (st.kind(), st.count()))
        .map(|(k, c)| format!("{}x{}", k.to_str(), c))
        .unwrap_or_default();
    let fuel_remaining = inv_final_slots
        .as_ref()
        .and_then(|s| s.get(1))
        .filter(|st| !st.is_empty())
        .map(|st| (st.kind(), st.count()))
        .map(|(k, c)| format!("{}x{}", k.to_str(), c))
        .unwrap_or_default();

    // 回收 input 槽剩余
    if !input_remaining.is_empty() {
        eprintln!("[smelt] P47: 回收 input 槽剩余 {input_remaining}");
        inv_final.shift_click(0usize);
        sleep(Duration::from_millis(150)).await;
    }
    // 回收 fuel 槽剩余
    if !fuel_remaining.is_empty() {
        eprintln!("[smelt] P47: 回收 fuel 槽剩余 {fuel_remaining}");
        let inv_fuel_back = bot
            .get_inventory()
            .map_err(|e| format!("获取容器失败（回收燃料）: {e:?}"))?;
        inv_fuel_back.shift_click(1usize);
        sleep(Duration::from_millis(150)).await;
    }

    // 返回结果（对齐 mindcraft line 263-272 的三种返回路径）
    if total_smelted == 0 {
        Err(format!(
            "熔炼 {output} 失败：11s 内结果槽无任何产物。\
             可能原因：1) 燃料不足（{actual_fuel_name} x{fuel_needed} 不够炼 {actual_smelt_count} 个）；\
             2) 输入物品不可熔炼；3) 打开的不是熔炉；4) 服务端 BlockEntity 同步延迟。"
        ))
    } else if total_smelted < target_total {
        Ok(format!(
            "熔炼 {output} 部分完成：实际熔炼 {total_smelted} 个（目标 {target_total}，\
             背包原料 {actual_input} 个 {input}，燃料 {actual_fuel_name} x{fuel_needed}）。\
             原因：11s 内无新产物（燃料可能耗尽）。\
             若需更多 {output}，请补充 {input} 和燃料后重试 smelt。",
            input = recipe.input
        ))
    } else {
        Ok(if actual_smelt_count < requested_count {
            format!(
                "熔炼 {output} 完成：实际熔炼 {total_smelted} 个（请求 {requested_count}，\
                 但背包只有 {actual_input} 个 {input}，已全部熔炼）。\
                 若需更多 {output}，请先 gather 采集 {input} 后再 smelt。",
                input = recipe.input
            )
        } else if original_count > actual_smelt_count {
            // P57 分批熔炼：本次只熔炼了 MAX_SMELT_PER_BATCH 个，还有剩余
            format!(
                "熔炼 {output} 完成：本次熔炼 {total_smelted} 个（请求 {requested_count}，\
                 但 P57 单批上限 {MAX_SMELT_PER_BATCH} 个以避免 120s 工具超时）。\
                 背包还剩 {remaining_raw} 个 {input} 未熔炼，请再次调用 smelt(output=\"{output}\", fuel=\"{fuel}\", count={remaining_raw}) 继续。",
                input = recipe.input,
                fuel = fuel,
                remaining_raw = original_count - actual_smelt_count,
                MAX_SMELT_PER_BATCH = MAX_SMELT_PER_BATCH
            )
        } else {
            format!("熔炼 {output} x{actual_smelt_count} 完成（共 {total_smelted} 个）")
        })
    }
}
