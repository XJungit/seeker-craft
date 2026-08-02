use crate::core::message::Message;
use crate::core::message::{AssistantMsg, ToolCall, ToolResultMsg, Usage, system_chatml};
use crate::core::prompt::PromptBuilder;
use serde_json::Value;

use super::{Agent, Context, MANAGE_KNOWLEDGE_TOOL};

/// Few-shot 示例：场景关键词 + 成功工具调用模式
///
/// **重要约束**（务必保持，否则 LLM 会抄错签名）：
/// - 工具调用以 OpenAI tool_calls 形式呈现（assistant 调用 → tool 返回结果）
/// - 工具名必须与 `tools_azalea.rs::create_mc_azalea_tools_full` 注册的 44 个工具 100% 一致
/// - 参数名必须与各工具的 `parameters()` schema 一致（如 gather 用 item/count，
///   goto 用 x/y/z，attack 用 target 字符串，非位置参数）
/// - 不要用假坐标 (10,64,20) — 用占位变量或感知真实坐标
/// - A1（2026-08-02）：turns 从文本改为结构化 ShotTurn，注入时转换为**真实
///   Message 序列**（assistant 带 tool_calls JSON + role:tool 结果配对），让 LLM
///   直接模仿 function calling 的真实形态，替代旧文本描述（文本会被模仿成伪调用）。
struct Example {
    keywords: &'static [&'static str],
    turns: &'static [ShotTurn],
}

/// 示例轮次：转换后为真实消息对。
/// `Assistant` 的第二个字段是工具调用数组 `(name, args_json)`，
/// `Tool` 按出现顺序对应当前 assistant 的调用。
enum ShotTurn {
    /// 用户输入（steering/玩家指令）
    User(&'static str),
    /// assistant 文字 + 0..n 个工具调用
    Assistant(&'static str, &'static [(&'static str, &'static str)]),
    /// 工具结果
    Tool(&'static str),
}

/// A1：把示例 turns 转换为真实消息序列（assistant 带 tool_calls + tool 结果配对，
/// id 用 `fewshot{base}_{i}_{j}` 前缀避免与真实调用冲突）。
fn example_to_messages(ex: &Example, base: usize) -> Vec<Message> {
    let mut msgs = Vec::new();
    let mut pending: Vec<(String, String)> = Vec::new();
    for (i, turn) in ex.turns.iter().enumerate() {
        match turn {
            ShotTurn::User(t) => msgs.push(Message::user(format!("【示例】{t}"))),
            ShotTurn::Assistant(text, calls) => {
                let tool_calls: Vec<ToolCall> = calls
                    .iter()
                    .enumerate()
                    .map(|(j, (name, args))| {
                        let id = format!("fewshot{base}_{i}_{j}");
                        pending.push((id.clone(), name.to_string()));
                        ToolCall {
                            id,
                            name: name.to_string(),
                            arguments: serde_json::from_str(args).unwrap_or(Value::Null),
                        }
                    })
                    .collect();
                msgs.push(Message::Assistant(AssistantMsg {
                    content: Some(format!("【示例】{text}")),
                    reasoning: None,
                    tool_calls,
                    timestamp: 0,
                    usage: Usage::default(),
                }));
            }
            ShotTurn::Tool(content) => {
                let (tool_call_id, tool_name) = pending.remove(0);
                msgs.push(Message::ToolResult(ToolResultMsg {
                    tool_call_id,
                    tool_name,
                    content: format!("【示例】{content}"),
                    is_error: false,
                    timestamp: 0,
                    details: None,
                    images: vec![],
                }));
            }
        }
    }
    msgs
}

const FEW_SHOT_EXAMPLES: &[Example] = &[
    // 探索场景：最常用，必须放在最前
    Example {
        keywords: &[
            "explore", "探索", "find", "找", "look", "看看", "附近", "周围", "周围",
        ],
        turns: &[
            ShotTurn::Assistant(
                "我先 perceive 看看周围，然后选个方向走 20 格探索。",
                &[("perceive", "{}")],
            ),
            ShotTurn::Tool("[perceive 结果: 群系 dark_forest, 木材 darkoaklog:8, 矿石 coalore:16]"),
            ShotTurn::Assistant(
                "向南走 20 格探索。",
                &[("goto", r#"{"x":-489,"y":88,"z":-144}"#)],
            ),
            ShotTurn::Tool("已到达目标 (-489,88,-144)"),
            ShotTurn::Assistant("perceive 看看新位置有什么。", &[("perceive", "{}")]),
            ShotTurn::Tool("[perceive 结果: 发现村庄/villager:2]"),
            ShotTurn::Assistant(
                "把村庄记到空间记忆。",
                &[(
                    "memory",
                    r#"{"action":"save","x":-489,"y":88,"z":-144,"kind":"structure","label":"村庄"}"#,
                )],
            ),
            ShotTurn::Tool("已记录记忆 @(-489,88,-144) kind=Structure label=村庄"),
        ],
    },
    // 砍树 → 木板 → 工作台 → 棒子（最经典开局）
    Example {
        keywords: &[
            "oak_log",
            "darkoaklog",
            "dark_oak",
            "wood",
            "tree",
            "砍树",
            "木头",
            "原木",
            "log",
        ],
        turns: &[
            ShotTurn::Assistant(
                "砍 4 根原木。",
                &[("gather", r#"{"item":"darkoaklog","count":4}"#)],
            ),
            ShotTurn::Tool(
                "Action output:\nSuccessfully gathered 4 darkoaklog, you now have 4 darkoaklog.",
            ),
            ShotTurn::Assistant(
                "把原木合成木板（2×2 背包网格）。",
                &[("craft", r#"{"item":"oak_planks","count":4}"#)],
            ),
            ShotTurn::Tool(
                "Action output:\nSuccessfully crafted oak_planks, you now have 16 oak_planks.",
            ),
            ShotTurn::Assistant(
                "合成工作台。",
                &[("craft", r#"{"item":"crafting_table","count":1}"#)],
            ),
            ShotTurn::Tool(
                "Action output:\nSuccessfully crafted crafting_table, you now have 1 crafting_table.",
            ),
            ShotTurn::Assistant(
                "把工作台放在脚边。",
                &[(
                    "place",
                    r#"{"item":"crafting_table","x":-489,"y":88,"z":-144}"#,
                )],
            ),
            ShotTurn::Tool("Action output:\nPlaced crafting_table at (-489, 88, -144)."),
        ],
    },
    // 制作石镐（开局第二阶段）
    Example {
        keywords: &[
            "stone_pickaxe",
            "石镐",
            "pickaxe",
            "镐",
            "cobblestone",
            "圆石",
        ],
        turns: &[
            ShotTurn::Assistant(
                "先确认有工作台（用 perceive 检查附近是否有 crafting_table），再挖 3 个石头。",
                &[("gather", r#"{"item":"stone","count":3}"#)],
            ),
            ShotTurn::Tool(
                "Action output:\nSuccessfully gathered 3 cobblestone, you now have 3 cobblestone.",
            ),
            ShotTurn::Assistant("打开工作台。", &[("open", r#"{"x":-489,"y":88,"z":-144}"#)]),
            ShotTurn::Tool("Action output:\nOpened crafting_table at (-489, 88, -144)."),
            ShotTurn::Assistant(
                "用木板做 2 个棒子，再用工作台 3x3 合石镐。",
                &[
                    ("craft", r#"{"item":"stick","count":2}"#),
                    ("craft_3x3", r#"{"item":"stone_pickaxe","count":1}"#),
                ],
            ),
            ShotTurn::Tool(
                "Action output:\nSuccessfully crafted stone_pickaxe, you now have 1 stone_pickaxe.",
            ),
        ],
    },
    // 铁矿 → 熔炼铁锭（P52 修正：对齐 9.2 铁律，furnace 必须用 craft_3x3 + place，
    // 不再示范 auto_craft(furnace)——该调用会被工具拒绝返回 Err）
    Example {
        keywords: &[
            "iron_ore", "铁矿", "iron", "铁", "smelt", "熔炼", "furnace", "熔炉",
        ],
        turns: &[
            ShotTurn::Assistant(
                "挖 3 个铁矿（需石镐+）。",
                &[("gather", r#"{"item":"iron_ore","count":3}"#)],
            ),
            ShotTurn::Tool(
                "Action output:\nSuccessfully gathered 3 iron_ore, you now have 3 iron_ore.",
            ),
            ShotTurn::Assistant(
                "熔炼前必须有 furnace：先确认背包有无（perceive 看背包），没有就 gather 8 圆石 → open 工作台 → 3x3 合成。",
                &[
                    ("gather", r#"{"item":"cobblestone","count":8}"#),
                    ("craft_3x3", r#"{"item":"furnace","count":1}"#),
                ],
            ),
            ShotTurn::Tool("Action output:\nSuccessfully crafted furnace, you now have 1 furnace."),
            ShotTurn::Assistant(
                "放熔炉并打开。",
                &[
                    ("place", r#"{"item":"furnace","x":-489,"y":88,"z":-145}"#),
                    ("open", r#"{"x":-489,"y":88,"z":-145}"#),
                ],
            ),
            ShotTurn::Tool("Action output:\nOpened furnace at (-489, 88, -145)."),
            ShotTurn::Assistant(
                "还需燃料，挖 1 个煤（或拿原木代替）。",
                &[("gather", r#"{"item":"coal_ore","count":1}"#)],
            ),
            ShotTurn::Tool("Action output:\nSuccessfully gathered 1 coal."),
            ShotTurn::Assistant(
                "烧 3 个铁锭。",
                &[(
                    "smelt",
                    r#"{"output":"iron_ingot","fuel":"coal","count":3}"#,
                )],
            ),
            ShotTurn::Tool(
                "Action output:\nSuccessfully smelted 3 iron_ingot, you now have 3 iron_ingot.",
            ),
        ],
    },
    // 战斗：攻击僵尸 / 自卫
    Example {
        keywords: &[
            "zombie",
            "僵尸",
            "creeper",
            "苦力怕",
            "skeleton",
            "骷髅",
            "spider",
            "蜘蛛",
            "attack",
            "攻击",
            "fight",
            "打",
            "combat",
        ],
        turns: &[
            ShotTurn::Assistant(
                "perceive 看到附近 zombie:3，攻击最近僵尸。",
                &[("perceive", "{}"), ("attack", r#"{"target":"zombie"}"#)],
            ),
            ShotTurn::Tool("Action output:\nSuccessfully killed zombie, health: 20/20."),
            ShotTurn::Assistant("继续打下一个。", &[("attack", r#"{"target":"zombie"}"#)]),
            ShotTurn::Tool("Action output:\nCould not find any zombie nearby. Health: 19/20."),
        ],
    },
    // 食物：狩猎 + 烤肉（P52 修正：对齐 9.2 铁律，furnace 必须用 craft_3x3 + place，
    // 不再示范 auto_craft(furnace)）
    Example {
        keywords: &[
            "cow", "pig", "sheep", "chicken", "牛", "猪", "羊", "鸡", "food", "hungry", "饿",
            "饱食",
        ],
        turns: &[
            ShotTurn::Assistant(
                "附近 cow:3。走过去后击杀。",
                &[
                    ("goto", r#"{"x":-487,"y":88,"z":-146}"#),
                    ("attack", r#"{"target":"cow"}"#),
                ],
            ),
            ShotTurn::Tool("Action output:\nSuccessfully killed cow, dropped raw_beef x2."),
            ShotTurn::Assistant(
                "烤肉需要 furnace。若背包无 furnace：gather 8 圆石 → open 工作台 → 3x3 合成 → place → open。",
                &[
                    ("gather", r#"{"item":"cobblestone","count":8}"#),
                    ("craft_3x3", r#"{"item":"furnace","count":1}"#),
                    ("place", r#"{"item":"furnace","x":-489,"y":88,"z":-145}"#),
                    ("open", r#"{"x":-489,"y":88,"z":-145}"#),
                ],
            ),
            ShotTurn::Tool("Action output:\nOpened furnace at (-489, 88, -145)."),
            ShotTurn::Assistant(
                "烤熟 2 块牛肉（coal 也可换 planks/log）。",
                &[(
                    "smelt",
                    r#"{"output":"cooked_beef","fuel":"coal","count":2}"#,
                )],
            ),
            ShotTurn::Tool(
                "Action output:\nSuccessfully smelted 2 cooked_beef, you now have 2 cooked_beef.",
            ),
        ],
    },
    // 火把
    Example {
        keywords: &["torch", "火把", "light", "照明", "暗", "夜晚", "night"],
        turns: &[
            ShotTurn::Assistant(
                "挖煤做棒子。",
                &[
                    ("gather", r#"{"item":"coal_ore","count":1}"#),
                    ("craft", r#"{"item":"stick","count":1}"#),
                ],
            ),
            ShotTurn::Tool("Action output:\nSuccessfully gathered 1 coal, crafted 1 stick."),
            ShotTurn::Assistant(
                "用 1 煤 + 1 棒合 4 火把。",
                &[("craft", r#"{"item":"torch","count":4}"#)],
            ),
            ShotTurn::Tool("Action output:\nSuccessfully crafted 4 torch, you now have 4 torch."),
        ],
    },
    // 箱子存储
    Example {
        keywords: &["chest", "箱子", "存储", "store", "store"],
        turns: &[
            ShotTurn::Assistant(
                "砍 8 原木并合 32 木板。",
                &[
                    ("gather", r#"{"item":"oak_log","count":8}"#),
                    ("craft", r#"{"item":"oak_planks","count":8}"#),
                ],
            ),
            ShotTurn::Tool("Action output:\nSuccessfully crafted 32 oak_planks."),
            ShotTurn::Assistant(
                "合 1 箱子并放下。",
                &[
                    ("craft", r#"{"item":"chest","count":1}"#),
                    ("place", r#"{"item":"chest","x":-489,"y":88,"z":-146}"#),
                ],
            ),
            ShotTurn::Tool("Action output:\nPlaced chest at (-489, 88, -146)."),
        ],
    },
    // 床
    Example {
        keywords: &["bed", "床", "sleep", "睡觉", "夜晚"],
        turns: &[
            ShotTurn::Assistant(
                "杀 3 只羊得 white_wool，砍 4 原木合木板。",
                &[
                    ("attack", r#"{"target":"sheep"}"#),
                    ("gather", r#"{"item":"oak_log","count":4}"#),
                ],
            ),
            ShotTurn::Tool("Action output:\nGot 3 white_wool, 12 oak_planks."),
            ShotTurn::Assistant(
                "open 工作台后 3x3 合床。",
                &[("craft_3x3", r#"{"item":"bed","count":1}"#)],
            ),
            ShotTurn::Tool("Action output:\nSuccessfully crafted bed, you now have 1 bed."),
        ],
    },
    // 村民交易
    Example {
        keywords: &["villager", "村民", "trade", "交易"],
        turns: &[
            ShotTurn::Assistant(
                "打开附近村民交易界面。",
                &[("interact_entity", r#"{"kind":"villager"}"#)],
            ),
            ShotTurn::Tool("Action output:\nOpened villager trade UI."),
            ShotTurn::Assistant("买第 1 个报价。", &[("trade", r#"{"offer":0}"#)]),
            ShotTurn::Tool("Action output:\nTrade completed, got 1 bread."),
        ],
    },
    // 下矿：用 mine_below 挖矿井
    Example {
        keywords: &["mine_below", "下矿", "dig", "挖矿", "挖矿井"],
        turns: &[
            ShotTurn::Assistant("挖脚下方块开始下探。", &[("mine_below", "{}")]),
            ShotTurn::Tool("Action output:\nMined block below, now at y=87."),
            ShotTurn::Assistant(
                "perceive 看到 iron_ore，挖它。",
                &[
                    ("perceive", "{}"),
                    ("mine", r#"{"x":-489,"y":85,"z":-144}"#),
                ],
            ),
            ShotTurn::Tool("Action output:\nMined block at (-489,85,-144), got iron_ore."),
        ],
    },
    // 建造：用 build 工具执行蓝图
    Example {
        keywords: &[
            "build",
            "建造",
            "house",
            "房子",
            "shelter",
            "庇护所",
            "蓝",
            "蓝图",
        ],
        turns: &[
            ShotTurn::Assistant(
                "先准备材料：砍 16 原木合 32 木板。",
                &[
                    ("gather", r#"{"item":"oak_log","count":16}"#),
                    ("craft", r#"{"item":"oak_planks","count":8}"#),
                ],
            ),
            ShotTurn::Tool("Action output:\nGot 32 oak_planks."),
            ShotTurn::Assistant(
                "按蓝图放 2 个方块。",
                &[(
                    "build",
                    r#"{"blueprint":"{\"blocks\":[{\"x\":-489,\"y\":88,\"z\":-144,\"block\":\"oak_planks\"},{\"x\":-489,\"y\":89,\"z\":-144,\"block\":\"oak_planks\"}]}"}"#,
                )],
            ),
            ShotTurn::Tool(
                "Action output:\n第1个: placed oak_planks @(-489,88,-144)\n第2个: placed oak_planks @(-489,89,-144)",
            ),
        ],
    },
    // 附魔
    Example {
        keywords: &["enchant", "附魔", "enchanting", "附魔台"],
        turns: &[
            ShotTurn::Assistant(
                "需钻石+黑曜石+书。挖 2 钻石，合附魔台并放置。",
                &[
                    ("gather", r#"{"item":"diamond_ore","count":2}"#),
                    ("craft", r#"{"item":"enchanting_table","count":1}"#),
                    (
                        "place",
                        r#"{"item":"enchanting_table","x":-489,"y":88,"z":-144}"#,
                    ),
                ],
            ),
            ShotTurn::Tool("Action output:\nPlaced enchanting_table at (-489,88,-144)."),
            ShotTurn::Assistant(
                "打开后给铁剑附魔 2 级。",
                &[
                    ("open", r#"{"x":-489,"y":88,"z":-144}"#),
                    ("enchant", r#"{"item":"iron_sword","level":2}"#),
                ],
            ),
            ShotTurn::Tool("Action output:\nEnchanted iron_sword with Sharpness II."),
        ],
    },
    // 多步计划：用 run_plan 一次跑多步
    Example {
        keywords: &["plan", "计划", "多步", "step", "run_plan", "依次"],
        turns: &[
            ShotTurn::Assistant(
                "用 run_plan 一次执行多步：先走到资源点，再挖矿，回工作台合成。",
                &[],
            ),
            ShotTurn::Assistant(
                "执行多步计划。",
                &[(
                    "run_plan",
                    r#"{"steps":[{"action":"goto","x":-490,"y":80,"z":-156},{"action":"mine","x":-490,"y":80,"z":-156},{"action":"goto","x":-489,"y":88,"z":-144},{"action":"craft","item":"torch","count":4}]}"#,
                )],
            ),
            ShotTurn::Tool(
                "Action output:\n步骤1 (goto) 完成: 已到达目标 (-490,80,-156)\n步骤2 (mine) 完成: 已挖掉方块 (-490,80,-156), got coal_ore\n步骤3 (goto) 完成: 已到达目标 (-489,88,-144)\n步骤4 (craft) 完成: Successfully crafted 4 torch",
            ),
        ],
    },
    // 卡住脱困（与 jailbreak 规则呼应）
    Example {
        keywords: &["stuck", "卡住", "未移动", "脱困", "trap"],
        turns: &[
            ShotTurn::Assistant(
                "perceive 提示「⚠ 卡住! 坐标5轮未移动」。改用 goto 到侧前方 3 格空地脱困。",
                &[("perceive", "{}")],
            ),
            ShotTurn::Tool("[perceive 结果: ⚠ 卡住! 坐标 5 轮未移动]"),
            ShotTurn::Assistant(
                "走到附近空地。",
                &[("goto", r#"{"x":-486,"y":88,"z":-161}"#)],
            ),
            ShotTurn::Tool("Action output:\n已到达目标 (-486,88,-161)"),
        ],
    },
    // 响应玩家指令（带 User turn 的完整闭环：玩家说话 → 确认 → 执行 → 回报）
    Example {
        keywords: &["玩家", "指令", "steering", "follow_up", "听从"],
        turns: &[
            ShotTurn::User("玩家: 天快黑了，去砍点木头存起来"),
            ShotTurn::Assistant(
                "收到，我去收集原木。",
                &[("gather", r#"{"item":"darkoaklog","count":4}"#)],
            ),
            ShotTurn::Tool("gather 完成: 实际获得 darkoaklog x4"),
            ShotTurn::Assistant("已收集 4 根原木。还需要我做什么吗？", &[]),
        ],
    },
];

/// 词重叠评分：两个文本共享的单词数 / 总单词数
fn word_overlap_score(text: &str, keywords: &[&str]) -> f64 {
    let text_lower = text.to_lowercase();
    let mut matched = 0;
    for kw in keywords {
        if text_lower.contains(kw) {
            matched += 1;
        }
    }
    if keywords.is_empty() {
        0.0
    } else {
        matched as f64 / keywords.len() as f64
    }
}

impl Agent {
    pub fn recent_perception_text(&self) -> &str {
        use crate::core::message::Message;
        self.messages
            .iter()
            .rev()
            .find_map(|message| match message {
                Message::ToolResult(result) if result.tool_name == "perceive" => {
                    Some(result.content.as_str())
                }
                Message::User(u) if u.content.starts_with("【当前游戏状态") => {
                    Some(u.content.as_str())
                }
                _ => None,
            })
            .unwrap_or("")
    }

    pub fn build_dynamic_context_msg(&mut self) -> Option<String> {
        let recent_perception: String = self.recent_perception_text().to_string();
        if recent_perception.is_empty() {
            return None;
        }

        let mut parts: Vec<String> = Vec::new();

        if self.config.enable_world_info {
            let dynamic_hints = self.world_info.scan_text(&recent_perception, 4_000);
            if !dynamic_hints.is_empty() {
                parts.push(format!("【场景提示】\n{}", dynamic_hints.join("\n")));
            }
        }

        if self.config.enable_skill {
            let now_ms = crate::core::message::now_ms();
            let skill_examples =
                self.skill_lib
                    .to_examples(&recent_perception, &self.config.prompt, 3, now_ms);
            if !skill_examples.is_empty() {
                let examples = skill_examples
                    .iter()
                    .map(|e| format!("- {}", e))
                    .collect::<Vec<_>>()
                    .join("\n");
                parts.push(format!("【经验参考】\n{}", examples));
            }
        }

        if self.obs_streak >= 5 {
            if self.obs_streak >= 10 {
                parts.push("【循环警告】你已经连续观察 10+ 步没有实际行动！STOP repeating perceive. Pick a COMPLETELY DIFFERENT tool RIGHT NOW — goto / gather / mine / craft / attack / build — anything but perceive.".to_string());
            } else {
                parts.push(format!(
                    "【观察提醒】已连续 {} 步纯观察。选一个工具立即行动（goto/gather/mine/craft/attack）。",
                    self.obs_streak
                ));
            }
        }

        if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n\n"))
        }
    }

    /// A1：构建 few-shot 真实消息对（assistant 带 tool_calls JSON + tool 结果）。
    /// 按最近感知词重叠选 top-2 示例。首轮注入一次后永不剔除（内容/位置固定，
    /// 与后续真实交互 append 天然形成稳定前缀，DeepSeek 前缀缓存最优）。
    pub fn build_few_shot_messages(&self) -> Vec<Message> {
        let recent = self.recent_perception_text();
        let mut scored: Vec<(f64, usize, &Example)> = FEW_SHOT_EXAMPLES
            .iter()
            .enumerate()
            .map(|(i, ex)| (word_overlap_score(recent, ex.keywords), i, ex))
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut out = Vec::new();
        for (_, base, ex) in scored.iter().take(2) {
            out.extend(example_to_messages(ex, *base));
        }
        out
    }

    pub fn build_dynamic_instructions_msg(&mut self) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();

        if !self.knowledge_bootstrapped {
            parts.push("【指令】直接开始执行任务，不需要重新输入游戏知识。".to_string());
        }
        if self.obs_streak >= 5 {
            if self.obs_streak >= 10 {
                parts.push("【关键警告】你已经循环了 10+ 步！STOP repeating the same action. Pick a COMPLETELY DIFFERENT tool RIGHT NOW — nav_to, collect, craft, build, combat — anything but what you've been doing.".to_string());
            } else {
                parts.push(format!(
                    "【观察提醒】已连续 {} 步纯观察。选一个工具立即行动。",
                    self.obs_streak
                ));
            }
        }

        if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n\n"))
        }
    }

    /// 渲染 WorldMemory 邻近记忆（以 `__self__` 锚点为中心，半径 64 格）。
    /// 无记忆或可定位锚点时返回 None（不污染上下文）。
    pub fn build_memory_context_msg(&self) -> Option<String> {
        if self.world_memory.is_empty() {
            return None;
        }
        let around = self
            .world_memory
            .find_anchor("__self__")
            .and_then(|a| a.pos)
            .unwrap_or(crate::core::memory::MemoryPos::new(0, 64, 0));
        let rendered = self.world_memory.render_nearby(around, 64);
        if rendered.is_empty() {
            None
        } else {
            Some(rendered)
        }
    }

    /// P97：渲染语义记忆注入（知识/策略/教训，跨会话持久化）。
    /// 查询词 = 当前目标 + 最近 3 个工具调用签名，按相关性 top-N 浮现；
    /// scope 过滤见 AgentConfig::memory_scope（防跨图污染）。无相关记忆
    /// 返回 None。消费后 touch 更新频率统计（last_used/uses）。
    pub fn build_semantic_memory_msg(&mut self) -> Option<String> {
        let mut query = String::new();
        if let super::PromptState::Active { goal, .. } = &self.prompt_state {
            query.push_str(goal);
        }
        for call in self.recent_calls.iter().rev().take(3) {
            query.push(' ');
            query.push_str(call);
        }
        let scope = self.config.memory_scope.clone();
        let mut mem = self.semantic_memory.lock().ok()?;
        let (text, touched) = mem.injection_text(query.trim(), scope.as_deref(), self.turn as i64);
        if touched.is_empty() {
            return None;
        }
        mem.touch(&touched, self.turn as i64);
        Some(text)
    }

    /// B5（2026-08-02）：紧凑渲染——待办最多列 8 条（任务链 23 个全列会占
    /// 大量上下文且 90% 与当前工作无关），超出显示省略行；已完成只计数不列。
    pub fn build_task_progress_msg(&self) -> String {
        if !self.config.enable_task_chain {
            return String::new();
        }
        let tm = &self.task_manager;
        if tm.tasks.is_empty() {
            return String::new();
        }
        const MAX_SHOWN: u32 = 8;
        let mut lines = Vec::new();
        let mut completed = 0u32;
        let mut pending = 0u32;
        let mut pending_shown = 0u32;
        let mut hidden = 0u32;
        for t in &tm.tasks {
            let is_current = tm.current.as_ref().is_some_and(|c| c.task.id == t.id);
            let status = if is_current {
                match tm.current_status() {
                    Some(crate::task::TaskStatus::Failed { .. }) => {
                        pending += 1;
                        Some("✖ 失败")
                    }
                    _ => Some("▶ 进行中"),
                }
            } else if matches!(
                tm.status_for(&t.id),
                Some(crate::task::TaskStatus::Completed { .. })
            ) {
                completed += 1;
                None // 已完成的低级任务，只计数不显示
            } else {
                pending += 1;
                Some("⏳ 待完成")
            };
            let Some(status) = status else { continue };
            if pending_shown >= MAX_SHOWN {
                hidden += 1;
                continue;
            }
            pending_shown += 1;
            let desc = if t.description.len() > 60 {
                let cutoff = t
                    .description
                    .char_indices()
                    .nth(57)
                    .map(|(i, _)| i)
                    .unwrap_or(t.description.len());
                format!("{}...", &t.description[..cutoff])
            } else {
                t.description.clone()
            };
            lines.push(format!("  [{status}] {} ({})", t.name, desc));
        }
        if lines.is_empty() {
            return String::new();
        }
        let mut out = format!("已完成 {} 个低级任务，剩余 {} 个待完成", completed, pending);
        if hidden > 0 {
            out.push_str(&format!("（已省略 {} 个更远的任务）", hidden));
        }
        out.push_str(":\n");
        out.push_str(&lines.join("\n"));
        out
    }

    /// A2（2026-08-02）：分阶段知识注入。按当前任务 tier 聚合所有
    /// tier ≤ 当前等级的 StageKnowledge 文本（早期少、后期累积），
    /// 经【阶段知识】user 消息注入（瞬态，轮间剔除）。
    /// 当前 tier 取 running 任务；无 running 取最低 Pending 任务；全完成 → 6。
    fn current_knowledge_tier(&self) -> u8 {
        let tm = &self.task_manager;
        if let Some(cur) = &tm.current
            && let crate::task::TaskStatus::Running { .. } = cur.status
        {
            return cur.task.tier.clamp(1, 6) as u8;
        }
        tm.tasks
            .iter()
            .filter(|t| tm.status_for(&t.id) == Some(&crate::task::TaskStatus::Pending))
            .map(|t| t.tier)
            .min()
            .map(|t| t.clamp(1, 6) as u8)
            .unwrap_or(6)
    }

    /// 按当前 tier 聚合阶段知识文本；空库或无可注入块时返回 None。
    pub fn build_stage_knowledge_msg(&self) -> Option<String> {
        if self.config.stage_knowledge.is_empty() {
            return None;
        }
        let tier = self.current_knowledge_tier();
        let blocks: Vec<&str> = self
            .config
            .stage_knowledge
            .iter()
            .filter(|sk| sk.tier == 0 || sk.tier <= tier)
            .filter(|sk| !sk.text.is_empty())
            .map(|sk| sk.text.as_str())
            .collect();
        if blocks.is_empty() {
            return None;
        }
        let mut out = String::new();
        for (i, b) in blocks.iter().enumerate() {
            if i > 0 {
                out.push_str("\n\n");
            }
            out.push_str(b);
        }
        Some(out)
    }

    pub fn build_context(&mut self) -> Context {
        use crate::core::message::Message;
        // C7：jailbreak 可经 profile 覆盖（_default.json 的 "jailbreak" 字段，
        // 三层合并可被模式/个体 profile 覆盖）；None 回退内置默认。
        let jailbreak = self
            .config
            .jailbreak
            .clone()
            .unwrap_or_else(|| {
                "自主行动。工具失败时调整参数重试——不准假装成功。\n\
                 行为准则：收到感知里的「卡住计数」≥3 时，说明坐标连续不变（可能下探被基岩/空气挡住或脚下方块无法破坏）——立即停止当前下探，改用 goto 侧前方 3 格空地或跳跃脱困，再重新 perceive；不要原地反复 perceive 或假装在下探。连续同工具≤3次后应向玩家 chat 汇报进度。工具没回报「实际获得X」就当作没获得，不得虚构成功。"
                    .to_string()
            });
        let knowledge = self.knowledge_string();

        let builder = PromptBuilder::new()
            .identity(&self.config.prompt)
            .role_desc(&knowledge)
            .jailbreak(jailbreak);
        let full_prompt = builder.build();
        let system = system_chatml(&full_prompt);
        let mut chatml = vec![system];
        chatml.extend(self.messages.iter().map(Message::to_chatml));
        let mut tool_defs = self.tools.to_openai_defs();
        // 仅当启用知识工具时才追加 manage_knowledge（azalea 等无世界
        // 知识库的路线设 false，避免向 LLM 暴露无用/触发上游 400 的工具）。
        if self.config.enable_knowledge_tool
            && let Ok(def) = serde_json::from_str::<Value>(MANAGE_KNOWLEDGE_TOOL)
        {
            tool_defs.push(def);
        }
        Context {
            system_prompt: full_prompt,
            messages: chatml,
            tools: tool_defs,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::agent::Agent;
    use crate::core::tool::ToolRegistry;

    use super::*;

    // ── word_overlap_score ──

    #[test]
    fn word_overlap_full_match() {
        let score = word_overlap_score("砍树 木头 原木", &["砍树", "木头", "原木"]);
        assert!((score - 1.0).abs() < 1e-6, "all keywords matched: {score}");
    }

    #[test]
    fn word_overlap_partial_match() {
        let score = word_overlap_score("砍树 木头 石头", &["砍树", "木头", "原木"]);
        assert!(
            (score - 2.0 / 3.0).abs() < 1e-6,
            "2/3 keywords matched: {score}"
        );
    }

    #[test]
    fn word_overlap_no_match() {
        let score = word_overlap_score("石头 铁矿 钻石", &["砍树", "木头", "原木"]);
        assert!((score - 0.0).abs() < 1e-6, "no keywords matched: {score}");
    }

    #[test]
    fn word_overlap_empty_keywords() {
        let score = word_overlap_score("anything", &[]);
        assert!((score - 0.0).abs() < 1e-6, "empty keywords: {score}");
    }

    #[test]
    fn word_overlap_case_insensitive() {
        let score = word_overlap_score("OAK_LOG DARK_OAK", &["oak_log", "dark_oak"]);
        assert!((score - 1.0).abs() < 1e-6, "case insensitive: {score}");
    }

    #[test]
    fn word_overlap_substring_match() {
        let score = word_overlap_score("explore the world", &["explore"]);
        assert!((score - 1.0).abs() < 1e-6, "substring match: {score}");
    }

    // ── build_context 字节稳定性 ──

    /// 回归测试：system prompt 不能包含随轮变化的动态变量。
    /// 这是 DeepSeek prefix cache 的前提条件。
    #[test]
    fn regression_build_context_system_prompt_byte_stable() {
        let tools = ToolRegistry::new();
        let mut agent = Agent::new(
            Box::new(StopProvider),
            tools,
            crate::agent::AgentConfig::new("test agent".into(), 5),
        );

        let ctx1 = agent.build_context();
        let ctx2 = agent.build_context();

        assert_eq!(
            ctx1.system_prompt, ctx2.system_prompt,
            "system prompt 两次调用必须字节一致（prefix cache 前提）"
        );
    }

    /// 回归测试：system prompt 不得包含 obs_streak 等动态变量
    #[test]
    fn regression_build_context_no_dynamic_markers_in_system_prompt() {
        let tools = ToolRegistry::new();
        let mut agent = Agent::new(
            Box::new(StopProvider),
            tools,
            crate::agent::AgentConfig::new("sys".into(), 5),
        );

        let ctx = agent.build_context();
        let sys = &ctx.system_prompt;

        // 这些动态标记已移出 system prompt，改为 user message 注入
        assert!(
            !sys.contains("观察提醒"),
            "system prompt 不得含 obs_streak 动态文本"
        );
        assert!(
            !sys.contains("不需要重新输入"),
            "system prompt 不得含 bootstrap 动态文本"
        );
    }

    // ── build_context 工具定义完整性 ──

    #[test]
    fn build_context_includes_knowledge_tool_when_enabled() {
        let tools = ToolRegistry::new();
        let mut config = crate::agent::AgentConfig::new("sys".into(), 5);
        config.enable_knowledge_tool = true;
        let mut agent = Agent::new(Box::new(StopProvider), tools, config);

        let ctx = agent.build_context();
        let has_manage_knowledge = ctx.tools.iter().any(|t| {
            t.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                == Some("manage_knowledge")
        });
        assert!(
            has_manage_knowledge,
            "knowledge tool should be in tools when enabled"
        );
    }

    #[test]
    fn build_context_skips_knowledge_tool_when_disabled() {
        let tools = ToolRegistry::new();
        let mut config = crate::agent::AgentConfig::new("sys".into(), 5);
        config.enable_knowledge_tool = false;
        let mut agent = Agent::new(Box::new(StopProvider), tools, config);

        let ctx = agent.build_context();
        let has_manage_knowledge = ctx.tools.iter().any(|t| {
            t.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                == Some("manage_knowledge")
        });
        assert!(
            !has_manage_knowledge,
            "knowledge tool should be absent when disabled"
        );
    }

    // ── 辅助 mock ──

    struct StopProvider;
    impl crate::agent::LlmProvider for StopProvider {
        fn complete(
            &self,
            _messages: &[serde_json::Value],
            _tools: &[serde_json::Value],
        ) -> anyhow::Result<crate::core::message::AssistantResponse> {
            Ok(crate::core::message::AssistantResponse {
                content: Some("ok".into()),
                reasoning: None,
                tool_calls: vec![],
                usage: crate::core::message::Usage::default(),
                stop_reason: crate::core::message::StopReason::Stop,
            })
        }
    }

    // ── A1: few-shot 真实消息对 ──

    /// A1 回归：few-shot 必须转换为真实消息对——assistant 带 tool_calls JSON
    /// （id 用 fewshot 前缀防冲突、arguments 为可解析 JSON），tool 结果与调用
    /// 按顺序配对，且内容带【示例】标记。旧实现是文本拼接（LLM 会模仿成伪调用）。
    #[test]
    fn a1_few_shot_messages_are_real_message_pairs() {
        let tools = crate::core::tool::ToolRegistry::new();
        let agent = Agent::new(
            Box::new(StopProvider),
            tools,
            crate::agent::AgentConfig::new("a1".into(), 5),
        );
        let msgs = agent.build_few_shot_messages();
        assert!(!msgs.is_empty(), "应注入至少一个示例");

        use crate::core::message::Message;
        let mut assistant_calls = 0usize;
        let mut tool_results = 0usize;
        let mut tool_result_ids: Vec<String> = Vec::new();
        for m in &msgs {
            match m {
                Message::Assistant(a) => {
                    assistant_calls += a.tool_calls.len();
                    for tc in &a.tool_calls {
                        assert!(
                            tc.id.starts_with("fewshot"),
                            "调用 id 必须带 fewshot 前缀防冲突: {}",
                            tc.id
                        );
                        assert!(
                            serde_json::from_str::<serde_json::Value>(&tc.arguments.to_string())
                                .is_ok(),
                            "arguments 必须是合法 JSON: {}",
                            tc.arguments
                        );
                    }
                    if let Some(text) = &a.content {
                        assert!(text.starts_with("【示例】"), "assistant 文本需带示例标记");
                    }
                }
                Message::ToolResult(t) => {
                    tool_results += 1;
                    tool_result_ids.push(t.tool_call_id.clone());
                    assert!(t.content.starts_with("【示例】"), "tool 结果需带示例标记");
                }
                _ => {}
            }
        }
        assert!(assistant_calls >= 1, "示例中必须含真实 tool_calls");
        assert_eq!(
            tool_results, assistant_calls,
            "tool 结果数量必须等于调用数量"
        );
        let _ = tool_result_ids;
    }

    /// A1 回归：示例中的 tool 结果按顺序与调用配对（pending 队列消费），
    /// 一个 assistant 多调用 → 多个连续 tool 结果。
    #[test]
    fn a1_few_shot_tool_results_pair_in_order() {
        let tools = crate::core::tool::ToolRegistry::new();
        let agent = Agent::new(
            Box::new(StopProvider),
            tools,
            crate::agent::AgentConfig::new("a1b".into(), 5),
        );
        let msgs = agent.build_few_shot_messages();
        use crate::core::message::Message;
        let mut i = 0usize;
        while i < msgs.len() {
            if let Message::Assistant(a) = &msgs[i] {
                let n = a.tool_calls.len();
                if n > 0 {
                    let ids: Vec<&str> = a.tool_calls.iter().map(|c| c.id.as_str()).collect();
                    for (j, expect_id) in ids.iter().enumerate() {
                        let next = msgs.get(i + 1 + j);
                        match next {
                            Some(Message::ToolResult(t)) => {
                                assert_eq!(
                                    &t.tool_call_id, expect_id,
                                    "第 {} 个 tool 结果必须配对第 {} 个调用",
                                    j, j
                                );
                                assert_eq!(
                                    t.tool_name, a.tool_calls[j].name,
                                    "tool_name 必须与调用名一致"
                                );
                            }
                            other => {
                                panic!("调用后第 {} 个消息必须是配对 tool 结果，实际: {other:?}", j)
                            }
                        }
                    }
                    i += 1 + n;
                    continue;
                }
            }
            i += 1;
        }
    }

    // ── A2: 分阶段知识注入 ──

    /// A2 回归：stage_knowledge 按任务 tier 过滤——早期只注入低 tier 块，
    /// tier 推进后累积注入；无任务（全完成）注入全部块。
    #[test]
    fn a2_stage_knowledge_filters_by_tier() {
        use crate::profile::StageKnowledge;
        use crate::task::{Task, TaskStatus};

        let config = crate::agent::AgentConfig::new("a2".into(), 5).with_stage_knowledge(vec![
            StageKnowledge {
                tier: 1,
                text: "DAY1".into(),
            },
            StageKnowledge {
                tier: 3,
                text: "DIAMOND".into(),
            },
            StageKnowledge {
                tier: 6,
                text: "DRAGON".into(),
            },
            StageKnowledge {
                tier: 0,
                text: "ALWAYS".into(),
            },
        ]);
        let mut agent = Agent::new(
            Box::new(StopProvider),
            crate::core::tool::ToolRegistry::new(),
            config,
        );

        // 无任务 → 全完成 → tier 6：全部块注入
        let msg = agent.build_stage_knowledge_msg().unwrap();
        assert!(
            msg.contains("DAY1")
                && msg.contains("DIAMOND")
                && msg.contains("DRAGON")
                && msg.contains("ALWAYS")
        );

        // running tier3 任务 → 只注入 tier ≤ 3（含 tier0 常驻块）
        let t = Task {
            id: "tier3_test".into(),
            name: "t".into(),
            description: "d".into(),
            goal: "g".into(),
            tier: 3,
            order: 1,
            success: crate::task::SuccessCondition::InventoryHas {
                item: "x".into(),
                count: 1,
            },
            failure: None,
            timeout_secs: None,
            reward: None,
        };
        agent.task_manager.tasks.push(t.clone());
        agent.task_manager.current = Some(crate::task::TaskInstance {
            task: t,
            status: TaskStatus::Running { started_at: 1 },
        });
        let msg = agent.build_stage_knowledge_msg().unwrap();
        assert!(msg.contains("DAY1"), "tier3 应含 tier1 块");
        assert!(msg.contains("DIAMOND"), "tier3 应含 tier3 块");
        assert!(!msg.contains("DRAGON"), "tier3 不应含 tier6 块");
        assert!(msg.contains("ALWAYS"), "tier0 常驻块始终注入");
        assert!(
            msg.find("DAY1").unwrap() < msg.find("DIAMOND").unwrap(),
            "注入顺序按声明顺序，保持可读性"
        );
    }

    /// A2 回归：空 stage_knowledge 不注入任何内容（默认配置零开销）。
    #[test]
    fn a2_stage_knowledge_empty_is_noop() {
        let agent = Agent::new(
            Box::new(StopProvider),
            crate::core::tool::ToolRegistry::new(),
            crate::agent::AgentConfig::new("a2b".into(), 5),
        );
        assert!(agent.build_stage_knowledge_msg().is_none());
    }
}
