//! BotCommand 命令层：动作指令 + 聊天命令解析（probe 驱动）。
//! P2.2（2026-08-03）：从 azalea/mod.rs 纯移动拆出，行为与拆前逐字一致。
//! 解析保持纯函数，无客户端即可测试。

/// 动作指令：由 `AzaleaBot` 发出，handler 内部用 `bot` 执行。
#[derive(Debug, Clone)]
pub enum BotCommand {
    Goto {
        x: i32,
        y: i32,
        z: i32,
    },
    /// P110：按锚点名导航（参考 Mindcraft goToRememberedPlace）。
    /// handler 从 `bot.memory` 查锚点坐标后执行 goto；锚点不存在返回明确错误。
    GotoAnchor {
        name: String,
    },
    /// P110b：probe 侧操作共享 WorldMemory（与 LLM memory 工具同一实例）。
    /// anchor: 设锚点（name,x,y,z,label）；query: 列全部锚点。
    Memory {
        action: String,
        name: Option<String>,
        x: Option<i32>,
        y: Option<i32>,
        z: Option<i32>,
    },
    Mine {
        x: i32,
        y: i32,
        z: i32,
    },
    MineBelow,
    /// 持续向上挖：从 bot 头顶逐格挖到空气，让 bot 跳出竖井/地下脱困。
    /// 用于「被埋在地下/卡在 1x1 竖井」场景——mine_below 是向下挖，
    /// mine_above 反向，向上挖通到地表。
    MineAbove,
    BlockInteract {
        x: i32,
        y: i32,
        z: i32,
    },
    /// P84：犁地+播种（参考 Mindcraft tillAndSow）。目标 (x,y,z) 需为 dirt/grass_block/farmland，
    /// 自动持锄头右键犁地、持种子右键播种并验证。seed 如 "wheat_seeds"。
    TillAndSow {
        x: i32,
        y: i32,
        z: i32,
        seed: String,
    },
    /// P85：睡觉跳夜（参考 Mindcraft goToBed）。找附近床 → 靠近 → 空主手 →
    /// 右键上床 → 验证入睡 → 睡到自然醒。白天/附近有怪物会失败。
    Sleep,
    /// P86：收割成熟作物（参考 Mindcraft collectBlock 作物分支）。自动扫描
    /// 附近成熟的小麦/胡萝卜/土豆/甜菜/下界疣并徒手挖取，掉落物自动拾取。
    Harvest,
    Chat {
        content: String,
    },
    Attack {
        target: String,
    },
    /// 2×2 背包合成（无需工作台）：item 为目标物品 id（如 "oak_planks"），count 为期望数量。
    Craft2x2 {
        item: String,
        count: u32,
    },
    /// 3×3 工作台合成。item 为目标物品 id，count 为期望数量。
    /// table_pos=Some 时使用该坐标的现有工作台；None 时 bot 自动放置+打开+关闭工作台（P1-4）。
    Craft3x3 {
        item: String,
        count: u32,
        table_pos: Option<(i32, i32, i32)>,
    },
    /// 熔炼。output 为目标物品 id（如 "iron_ingot"），fuel 为燃料物品 id（如 "coal"），count 为期望数量。
    /// table_pos=Some 时使用该坐标的现有熔炉；None 时 bot 自动放置+打开+关闭熔炉（P1-4）。
    Smelt {
        output: String,
        fuel: String,
        count: u32,
        table_pos: Option<(i32, i32, i32)>,
    },
    /// 采集：走到最近的指定方块（如 "oak_log" / "stone" / "coal_ore"）并挖掘，直到背包有 count 个。
    Gather {
        item: String,
        count: u32,
    },
    /// P67：自动造黑曜石。bot 需手持 water_bucket，且附近（半径 12）有岩浆源。
    /// 工具会：在岩浆旁放一格水→生成黑曜石→用 diamond_pickaxe 挖下→重复 count 次。
    /// 用于下界传送门框架。若没水/没岩浆/没钻石镐会返回错误。
    MakeObsidian {
        count: u32,
    },
    /// 放置：把手持物品 item 放到世界坐标 (x,y,z) 旁（右键放置）。
    Place {
        item: String,
        x: i32,
        y: i32,
        z: i32,
    },
    /// 打开容器：打开世界坐标 (x,y,z) 处的容器（工作台/熔炉/箱子等）。
    OpenContainer {
        x: i32,
        y: i32,
        z: i32,
    },
    /// 高层自动合成（木链）：采集→2×2→放置工作台→开→3×3，一键造木制品。
    AutoCraft {
        item: String,
        count: u32,
    },
    /// 附魔：在已打开的附魔台中，给 item 附魔（需背包有 item 与青金石 lapis_lazuli）。
    /// level 为 1/2/3，对应附魔台三个选项槽。
    Enchant {
        item: String,
        level: u32,
    },
    /// 村民交易：与最近的村民交易，选第 offer 个报价（0 起）。bot 自动打开村民。
    Trade {
        offer: u32,
    },
    /// 实体右键交互（打开村民/动物/展示框等）：与最近的指定种类实体交互。
    /// kind 为实体种类关键词，如 "villager"。
    InteractEntity {
        kind: String,
    },
    /// 捡起附近掉落物：bot 走 4 个方向扫一圈，让物理引擎自然吸取掉落物。
    /// 无参数。挖矿/战斗后调用一次，避免"挖了 8 个石头但只捡到 3 个"。
    Pickup,
    /// 自动防御：等待 5 秒让 handler 层 self_defense mode 自动攻击附近敌人。
    /// 期间监测血量，若受到严重伤害提前返回建议撤退。
    Defend,
    /// 装备背包中的指定物品到指定槽位。
    /// slot: "hand"/"helmet"/"chestplate"/"leggings"/"boots"
    Equip {
        item: String,
        slot: String,
    },
    /// 丢弃背包中的指定物品。count 为丢弃数量（0 表示全部）。
    Discard {
        item: String,
        count: u32,
    },
    /// 消耗（吃/喝）背包中的指定物品。
    Consume {
        item: String,
    },
    /// 查看容器物品列表（打开→读→关闭）。
    ChestView {
        x: i32,
        y: i32,
        z: i32,
    },
    /// 从容器取出 item（count 个）到 bot 背包。
    ChestWithdraw {
        x: i32,
        y: i32,
        z: i32,
        item: String,
        count: u32,
    },
    /// 把背包中的 item（count 个）存入容器。
    ChestDeposit {
        x: i32,
        y: i32,
        z: i32,
        item: String,
        count: u32,
    },
    /// P68：跟随玩家。target 为玩家名（None 表示跟随最近的其他玩家）。
    /// handler 每 tick 读取该玩家坐标并 goto，实现"跟着我"。
    Follow {
        target: Option<String>,
    },
    /// P111：按玩家名单次导航（对齐 Mindcraft goToPlayer）。name 为玩家名
    ///（None 表示最近的其他玩家）。解析玩家当前坐标后按 Goto 执行一次，
    /// 不持续跟随（持续跟随用 Follow）。
    GotoPlayer {
        name: Option<String>,
    },
    /// P112：搜索指定方块在半径内的全部坐标（对齐 Mindcraft searchForBlock）。
    /// 只返回坐标供规划，不挖掘（要挖用 gather）。
    SearchBlock {
        item: String,
        radius: u32,
    },
    /// P113：向远离指定实体的方向移动（对齐 Mindcraft moveAway）。
    /// target 为实体名（None=最近的非玩家实体）；distance 为反向移动距离（默认 8）。
    MoveAway {
        target: Option<String>,
        distance: u32,
    },
    /// P68：停止跟随（解除 Follow 模式）。
    StopFollow,
    /// P68：把物品丢在指定玩家脚边（玩家拾取）。item 为物品 id，count 为数量（0=全部）。
    /// target 为玩家名（None 表示最近的其他玩家）。基于现有 Discard 能力，但丢在玩家坐标而非 bot 脚边。
    Give {
        item: String,
        count: u32,
        target: Option<String>,
    },
    /// P88：原始状态 dump（调试通道）。不经任何渲染/聚合/翻译，直接以原始格式
    /// 输出 azalea API 数据：精确位置、逐槽背包、全量实体（含玩家 id/坐标/距离）、
    /// 脚下方块、维度、朝向等。供 probe 作为与 LLM 感知通道相互独立的事实来源
    /// （LLM 感知数据出错/死循环时用它对撞出真相）。LLM 工具不暴露此命令。
    RawState,
}

/// 队列中的命令包装：携带结果回传通道（None 表示 fire-and-forget，如聊天指令）。
#[derive(Clone)]
pub struct QueuedCommand {
    pub cmd: BotCommand,
    pub result_tx: Option<std::sync::mpsc::Sender<String>>,
}
fn parse_chat_coords(rest: &str) -> Option<(i32, i32, i32)> {
    let values: Vec<i32> = rest
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    (values.len() == 3).then(|| (values[0], values[1], values[2]))
}

/// Parse the small, synchronous chat command surface used for in-game control.
/// Keeping this pure makes malformed commands testable without a live client.
pub fn parse_chat_command(content: &str) -> Option<BotCommand> {
    let content = content.trim();
    if let Some(rest) = content.strip_prefix("autocraft ") {
        let mut parts = rest.split_whitespace();
        return Some(BotCommand::AutoCraft {
            item: parts.next()?.to_string(),
            count: parts
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1),
        });
    }
    if let Some(rest) = content.strip_prefix("open ") {
        let (x, y, z) = parse_chat_coords(rest)?;
        return Some(BotCommand::OpenContainer { x, y, z });
    }
    if let Some(rest) = content.strip_prefix("place ") {
        let mut parts = rest.split_whitespace();
        let item = parts.next()?.to_string();
        let (x, y, z) = parse_chat_coords(&parts.collect::<Vec<_>>().join(" "))?;
        return Some(BotCommand::Place { item, x, y, z });
    }
    if let Some(rest) = content.strip_prefix("gather ") {
        let mut parts = rest.split_whitespace();
        return Some(BotCommand::Gather {
            item: parts.next()?.to_string(),
            count: parts
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1),
        });
    }
    // P112: searchblock <方块> [半径] —— 搜块返回坐标（默认半径 32）。
    if let Some(rest) = content.strip_prefix("searchblock ") {
        let mut parts = rest.split_whitespace();
        let item = parts.next()?.to_string();
        let radius = parts
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(32);
        return Some(BotCommand::SearchBlock { item, radius });
    }
    // P113: moveaway [实体名] [距离] —— 向远离指定实体的方向移动（默认 8m）。
    if let Some(rest) = content.strip_prefix("moveaway") {
        let rest = rest.trim();
        if !rest.is_empty() {
            let mut parts = rest.split_whitespace();
            let first = parts.next()?.to_string();
            let (target, distance) = match first.parse::<u32>() {
                Ok(d) => (None, d),
                Err(_) => (
                    Some(first),
                    parts.next().and_then(|v| v.parse().ok()).unwrap_or(8),
                ),
            };
            return Some(BotCommand::MoveAway { target, distance });
        }
        return Some(BotCommand::MoveAway {
            target: None,
            distance: 8,
        });
    }
    if let Some(rest) = content.strip_prefix("craft3 ") {
        let mut parts = rest.split_whitespace();
        return Some(BotCommand::Craft3x3 {
            item: parts.next()?.to_string(),
            count: parts
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1),
            table_pos: None,
        });
    }
    if let Some(rest) = content.strip_prefix("smelt ") {
        let mut parts = rest.split_whitespace();
        return Some(BotCommand::Smelt {
            output: parts.next()?.to_string(),
            fuel: parts.next().unwrap_or("coal").to_string(),
            count: parts
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1),
            table_pos: None,
        });
    }
    if let Some(rest) = content.strip_prefix("craft ") {
        let mut parts = rest.split_whitespace();
        return Some(BotCommand::Craft2x2 {
            item: parts.next()?.to_string(),
            count: parts
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1),
        });
    }
    if let Some(rest) = content.strip_prefix("goto ") {
        let rest = rest.trim();
        // P110: 单个 token 非坐标格式 → 按锚点名导航（goto home）
        // 坐标格式 `x y z` 三个数才走 parse_chat_coords，其余视为锚点名
        let tokens: Vec<&str> = rest.split_whitespace().collect();
        if tokens.len() == 1 && tokens[0].parse::<i32>().is_err() {
            return Some(BotCommand::GotoAnchor {
                name: tokens[0].to_string(),
            });
        }
        let (x, y, z) = parse_chat_coords(rest)?;
        return Some(BotCommand::Goto { x, y, z });
    }
    // P110b: memory anchor <name> <x> <y> <z> / memory query
    if let Some(rest) = content.strip_prefix("memory ") {
        let mut parts = rest.split_whitespace();
        let action = parts.next()?.to_string();
        return match action.as_str() {
            "anchor" => {
                let name = parts.next()?.to_string();
                let (x, y, z) = parse_chat_coords(&parts.collect::<Vec<_>>().join(" "))?;
                Some(BotCommand::Memory {
                    action,
                    name: Some(name),
                    x: Some(x),
                    y: Some(y),
                    z: Some(z),
                })
            }
            "query" => Some(BotCommand::Memory {
                action,
                name: None,
                x: None,
                y: None,
                z: None,
            }),
            _ => None,
        };
    }
    if let Some(rest) = content.strip_prefix("mine ") {
        let (x, y, z) = parse_chat_coords(rest)?;
        return Some(BotCommand::Mine { x, y, z });
    }
    if let Some(rest) = content.strip_prefix("chat ") {
        let msg = rest.trim();
        if !msg.is_empty() {
            return Some(BotCommand::Chat {
                content: msg.to_string(),
            });
        }
    }
    if content == "minebelow" {
        return Some(BotCommand::MineBelow);
    }
    if content == "mineabove" {
        return Some(BotCommand::MineAbove);
    }
    if content == "sleep" {
        return Some(BotCommand::Sleep);
    }
    if content == "attack" {
        return Some(BotCommand::Attack {
            target: "chat".into(),
        });
    }
    if let Some(rest) = content.strip_prefix("enchant ") {
        let mut parts = rest.split_whitespace();
        return Some(BotCommand::Enchant {
            item: parts.next()?.to_string(),
            level: parts
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1),
        });
    }
    if let Some(rest) = content.strip_prefix("trade ") {
        return rest
            .trim()
            .parse()
            .ok()
            .map(|offer| BotCommand::Trade { offer });
    }
    if let Some(rest) = content.strip_prefix("interact ") {
        let kind = rest.trim();
        if !kind.is_empty() {
            return Some(BotCommand::InteractEntity {
                kind: kind.to_string(),
            });
        }
        return None;
    }
    if let Some(rest) = content.strip_prefix("interactblock ") {
        let mut parts = rest.split_whitespace();
        let (x, y, z) = (
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
        );
        return Some(BotCommand::BlockInteract { x, y, z });
    }
    if let Some(rest) = content.strip_prefix("tillandsow ") {
        let mut parts = rest.split_whitespace();
        let (x, y, z, seed) = (
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
            parts.next()?.to_string(),
        );
        return Some(BotCommand::TillAndSow { x, y, z, seed });
    }
    if content == "harvest" {
        return Some(BotCommand::Harvest);
    }
    if content == "follow" {
        return Some(BotCommand::Follow { target: None });
    }
    if let Some(rest) = content.strip_prefix("follow ") {
        return Some(BotCommand::Follow {
            target: (!rest.trim().is_empty()).then(|| rest.trim().to_string()),
        });
    }
    // P111：按玩家名单次导航（gotoplayer / gotoplayer <名字>）。
    if content == "gotoplayer" {
        return Some(BotCommand::GotoPlayer { name: None });
    }
    if let Some(rest) = content.strip_prefix("gotoplayer ") {
        return Some(BotCommand::GotoPlayer {
            name: (!rest.trim().is_empty()).then(|| rest.trim().to_string()),
        });
    }
    if content == "stopfollow" || content == "stop" {
        return Some(BotCommand::StopFollow);
    }
    if let Some(rest) = content.strip_prefix("give ") {
        let mut parts = rest.split_whitespace();
        let item = parts.next()?.to_string();
        let second = parts.next();
        let (count, target) = match second {
            None => (0, None),
            Some(value) => match value.parse::<u32>() {
                Ok(count) => (count, parts.next().map(str::to_string)),
                Err(_) => (0, Some(value.to_string())),
            },
        };
        return Some(BotCommand::Give {
            item,
            count,
            target,
        });
    }
    if let Some(rest) = content.strip_prefix("equip ") {
        let mut parts = rest.split_whitespace();
        let item = parts.next()?.to_string();
        let slot = parts
            .next()
            .map(str::to_string)
            .unwrap_or_else(|| "hand".to_string());
        return Some(BotCommand::Equip { item, slot });
    }
    if let Some(rest) = content.strip_prefix("discard ") {
        let mut parts = rest.split_whitespace();
        let item = parts.next()?.to_string();
        let count = parts
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        return Some(BotCommand::Discard { item, count });
    }
    if let Some(rest) = content.strip_prefix("consume ") {
        let item = rest.trim();
        if item.is_empty() {
            return None;
        }
        return Some(BotCommand::Consume {
            item: item.to_string(),
        });
    }
    if let Some(rest) = content.strip_prefix("chestview ") {
        let (x, y, z) = parse_chat_coords(rest)?;
        return Some(BotCommand::ChestView { x, y, z });
    }
    if let Some(rest) = content.strip_prefix("chestwithdraw ") {
        let mut parts = rest.split_whitespace();
        let (x, y, z) = parse_chat_coords(&parts.clone().collect::<Vec<_>>()[0..3].join(" "))?;
        let _ = parts.next();
        let _ = parts.next();
        let _ = parts.next();
        let item = parts.next()?.to_string();
        let count = parts
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1);
        return Some(BotCommand::ChestWithdraw {
            x,
            y,
            z,
            item,
            count,
        });
    }
    if let Some(rest) = content.strip_prefix("chestdeposit ") {
        let mut parts = rest.split_whitespace();
        let (x, y, z) = parse_chat_coords(&parts.clone().collect::<Vec<_>>()[0..3].join(" "))?;
        let _ = parts.next();
        let _ = parts.next();
        let _ = parts.next();
        let item = parts.next()?.to_string();
        let count = parts
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1);
        return Some(BotCommand::ChestDeposit {
            x,
            y,
            z,
            item,
            count,
        });
    }
    if let Some(rest) = content.strip_prefix("makeobsidian") {
        let count = rest
            .split_whitespace()
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1);
        return Some(BotCommand::MakeObsidian { count });
    }
    if content == "pickup" {
        return Some(BotCommand::Pickup);
    }
    if content == "defend" {
        return Some(BotCommand::Defend);
    }
    if content == "rawstate" {
        return Some(BotCommand::RawState);
    }
    None
}

#[cfg(test)]
mod chat_parser_tests {
    use super::*;

    #[test]
    fn chat_parser_handles_give_count_and_target_forms() {
        assert!(matches!(
            parse_chat_command("give diamond 3 Steve"),
            Some(BotCommand::Give {
                item,
                count: 3,
                target: Some(target),
            }) if item == "diamond" && target == "Steve"
        ));
        assert!(matches!(
            parse_chat_command("give diamond Steve"),
            Some(BotCommand::Give {
                item,
                count: 0,
                target: Some(target),
            }) if item == "diamond" && target == "Steve"
        ));
        assert!(matches!(
            parse_chat_command("give diamond"),
            Some(BotCommand::Give { item, count: 0, target: None }) if item == "diamond"
        ));
    }

    #[test]
    fn chat_parser_rejects_malformed_coordinates_and_preserves_follow() {
        assert!(parse_chat_command("goto 1 2").is_none());
        assert!(matches!(
            parse_chat_command("follow Steve"),
            Some(BotCommand::Follow { target: Some(target) }) if target == "Steve"
        ));
        assert!(matches!(
            parse_chat_command("follow"),
            Some(BotCommand::Follow { target: None })
        ));
        assert!(matches!(
            parse_chat_command("stop"),
            Some(BotCommand::StopFollow)
        ));
    }

    #[test]
    fn chat_parser_sleep_and_tillandsow() {
        assert!(matches!(
            parse_chat_command("sleep"),
            Some(BotCommand::Sleep)
        ));
        assert!(matches!(
            parse_chat_command("tillandsow 10 64 20 wheat_seeds"),
            Some(BotCommand::TillAndSow {
                x: 10,
                y: 64,
                z: 20,
                seed,
            }) if seed == "wheat_seeds"
        ));
        assert!(matches!(
            parse_chat_command("chat hello"),
            Some(BotCommand::Chat { content }) if content == "hello"
        ));
        assert!(matches!(
            parse_chat_command("harvest"),
            Some(BotCommand::Harvest)
        ));
        assert!(parse_chat_command("harvest 3").is_none());
        assert!(parse_chat_command("tillandsow 1 2").is_none());
    }

    /// P111：gotoplayer 按玩家名单次导航解析。
    #[test]
    fn chat_parser_goto_player() {
        assert!(matches!(
            parse_chat_command("gotoplayer"),
            Some(BotCommand::GotoPlayer { name: None })
        ));
        assert!(matches!(
            parse_chat_command("gotoplayer Jun"),
            Some(BotCommand::GotoPlayer {
                name: Some(name),
            }) if name == "Jun"
        ));
        // goto 单 token 仍走锚点（P110），gotoplayer 是独立命令不冲突
        assert!(matches!(
            parse_chat_command("goto Jun"),
            Some(BotCommand::GotoAnchor { name }) if name == "Jun"
        ));
    }

    /// P112：searchblock 搜块返回坐标解析。
    #[test]
    fn chat_parser_search_block() {
        assert!(matches!(
            parse_chat_command("searchblock oak_log"),
            Some(BotCommand::SearchBlock { item, radius }) if item == "oak_log" && radius == 32
        ));
        assert!(matches!(
            parse_chat_command("searchblock diamond_ore 64"),
            Some(BotCommand::SearchBlock { item, radius }) if item == "diamond_ore" && radius == 64
        ));
        assert!(parse_chat_command("searchblock").is_none());
    }

    /// P113：moveaway 远离实体解析。
    #[test]
    fn chat_parser_move_away() {
        assert!(matches!(
            parse_chat_command("moveaway"),
            Some(BotCommand::MoveAway {
                target: None,
                distance: 8
            })
        ));
        assert!(matches!(
            parse_chat_command("moveaway zombie"),
            Some(BotCommand::MoveAway { target: Some(t), distance: 8 }) if t == "zombie"
        ));
        assert!(matches!(
            parse_chat_command("moveaway zombie 20"),
            Some(BotCommand::MoveAway { target: Some(t), distance: 20 }) if t == "zombie"
        ));
        assert!(matches!(
            parse_chat_command("moveaway 15"),
            Some(BotCommand::MoveAway {
                target: None,
                distance: 15
            })
        ));
    }

    /// P110：goto 单 token 非数字 → GotoAnchor；数字坐标仍走 Goto。
    #[test]
    fn chat_parser_goto_anchor_vs_coords() {
        assert!(matches!(
            parse_chat_command("goto home"),
            Some(BotCommand::GotoAnchor { name }) if name == "home"
        ));
        assert!(matches!(
            parse_chat_command("goto nether_portal"),
            Some(BotCommand::GotoAnchor { name }) if name == "nether_portal"
        ));
        // 单数字不是锚点（坐标格式不完整 → None）
        assert!(parse_chat_command("goto 10").is_none());
        assert!(matches!(
            parse_chat_command("goto 10 64 20"),
            Some(BotCommand::Goto {
                x: 10,
                y: 64,
                z: 20
            })
        ));
    }

    /// P110b：memory 命令解析（anchor/query）。
    #[test]
    fn chat_parser_memory_anchor_and_query() {
        assert!(matches!(
            parse_chat_command("memory anchor home 10 64 20"),
            Some(BotCommand::Memory {
                action,
                name: Some(name),
                x: Some(10),
                y: Some(64),
                z: Some(20),
            }) if action == "anchor" && name == "home"
        ));
        assert!(matches!(
            parse_chat_command("memory query"),
            Some(BotCommand::Memory { action, name: None, x: None, y: None, z: None }) if action == "query"
        ));
        assert!(parse_chat_command("memory foo").is_none());
        assert!(parse_chat_command("memory anchor home 1 2").is_none());
    }
}
