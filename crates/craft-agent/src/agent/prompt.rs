use crate::core::message::system_chatml;
use crate::core::prompt::PromptBuilder;
use serde_json::Value;

use super::{Agent, Context, MANAGE_KNOWLEDGE_TOOL};

/// Few-shot 示例：场景关键词 + 成功工具调用模式
///
/// **重要约束**（务必保持，否则 LLM 会抄错签名）：
/// - 工具调用以 OpenAI tool_calls 形式呈现（assistant 调用 → system 返回结果）
/// - 工具名必须与 `tools_azalea.rs::create_mc_azalea_tools_full` 注册的 37 个工具 100% 一致
/// - 参数名必须与各工具的 `parameters()` schema 一致（如 gather 用 item/count，
///   goto 用 x/y/z，attack 用 target 字符串，非位置参数）
/// - 不要用假坐标 (10,64,20) — 用占位变量或感知真实坐标
struct Example {
    keywords: &'static [&'static str],
    /// (role, content) 对，role 为 "user"/"system"/"assistant"
    turns: &'static [(&'static str, &'static str)],
}

const FEW_SHOT_EXAMPLES: &[Example] = &[
    // 探索场景：最常用，必须放在最前
    Example {
        keywords: &["explore", "探索", "find", "找", "look", "看看", "附近", "周围", "周围"],
        turns: &[
            ("assistant", "我先 perceive 看看周围，然后选个方向走 20 格探索。"),
            ("system", "[perceive 结果: 群系 dark_forest, 木材 darkoaklog:8, 矿石 coalore:16]"),
            ("assistant", "向南走 20 格探索。调用 goto(x=-489, y=88, z=-144)"),
            ("system", "已到达目标 (-489,88,-144)"),
            ("assistant", "perceive 看看新位置有什么。"),
            ("system", "[perceive 结果: 发现村庄/villager:2]"),
            ("assistant", "memory(action=\"save\", x=-489, y=88, z=-144, kind=\"structure\", label=\"村庄\") 记下来。"),
            ("system", "已记录记忆 @(-489,88,-144) kind=Structure label=村庄"),
        ],
    },
    // 砍树 → 木板 → 工作台 → 棒子（最经典开局）
    Example {
        keywords: &["oak_log", "darkoaklog", "dark_oak", "wood", "tree", "砍树", "木头", "原木", "log"],
        turns: &[
            ("assistant", "gather(item=\"darkoaklog\", count=4) 砍 4 根原木。"),
            ("system", "Action output:\nSuccessfully gathered 4 darkoaklog, you now have 4 darkoaklog."),
            ("assistant", "craft(item=\"oak_planks\", count=4) 把原木合成木板（2×2 背包网格）。"),
            ("system", "Action output:\nSuccessfully crafted oak_planks, you now have 16 oak_planks."),
            ("assistant", "craft(item=\"crafting_table\", count=1) 合成工作台。"),
            ("system", "Action output:\nSuccessfully crafted crafting_table, you now have 1 crafting_table."),
            ("assistant", "place(item=\"crafting_table\", x=-489, y=88, z=-144) 把工作台放在脚边。"),
            ("system", "Action output:\nPlaced crafting_table at (-489, 88, -144)."),
        ],
    },
    // 制作石镐（开局第二阶段）
    Example {
        keywords: &["stone_pickaxe", "石镐", "pickaxe", "镐", "cobblestone", "圆石"],
        turns: &[
            ("assistant", "先确认有工作台（用 perceive 检查 10x10 是否有 crafting_table）。然后 gather(item=\"stone\", count=3) 挖 3 个石头得 cobblestone。"),
            ("system", "Action output:\nSuccessfully gathered 3 cobblestone, you now have 3 cobblestone."),
            ("assistant", "open(x=-489, y=88, z=-144) 打开工作台。"),
            ("system", "Action output:\nOpened crafting_table at (-489, 88, -144)."),
            ("assistant", "先 craft(item=\"stick\", count=2) 用木板做棒子（2×2 即可），再 craft_3x3(item=\"stone_pickaxe\", count=1) 用工作台合石镐。"),
            ("system", "Action output:\nSuccessfully crafted stone_pickaxe, you now have 1 stone_pickaxe."),
        ],
    },
    // 铁矿 → 熔炼铁锭
    Example {
        keywords: &["iron_ore", "铁矿", "iron", "铁", "smelt", "熔炼", "furnace", "熔炉"],
        turns: &[
            ("assistant", "gather(item=\"iron_ore\", count=3) 挖铁矿。需石镐+。"),
            ("system", "Action output:\nSuccessfully gathered 3 iron_ore, you now have 3 iron_ore."),
            ("assistant", "auto_craft(item=\"furnace\", count=1) 一键合成并放置熔炉。"),
            ("system", "Action output:\nPlaced furnace at (-489, 88, -145)."),
            ("assistant", "open(x=-489, y=88, z=-145) 打开熔炉，再 smelt(output=\"iron_ingot\", fuel=\"coal\", count=3) 烧 3 个铁锭。"),
            ("system", "Action output:\nSuccessfully smelted 3 iron_ingot, you now have 3 iron_ingot."),
        ],
    },
    // 战斗：攻击僵尸 / 自卫
    Example {
        keywords: &["zombie", "僵尸", "creeper", "苦力怕", "skeleton", "骷髅", "spider", "蜘蛛", "attack", "攻击", "fight", "打", "combat"],
        turns: &[
            ("assistant", "perceive 看到附近 zombie:3。attack(target=\"zombie\") 攻击最近僵尸。"),
            ("system", "Action output:\nSuccessfully killed zombie, health: 20/20."),
            ("assistant", "继续 attack(target=\"zombie\") 打下一个。"),
            ("system", "Action output:\nCould not find any zombie nearby. Health: 19/20."),
        ],
    },
    // 食物：狩猎 + 烤肉
    Example {
        keywords: &["cow", "pig", "sheep", "chicken", "牛", "猪", "羊", "鸡", "food", "hungry", "饿", "饱食"],
        turns: &[
            ("assistant", "附近 cow:3。goto 走过去后 attack(target=\"cow\") 击杀。"),
            ("system", "Action output:\nSuccessfully killed cow, dropped raw_beef x2."),
            ("assistant", "auto_craft(item=\"furnace\", count=1); open 后 smelt(output=\"cooked_beef\", fuel=\"coal\", count=2) 烤熟。"),
            ("system", "Action output:\nSuccessfully smelted 2 cooked_beef, you now have 2 cooked_beef."),
        ],
    },
    // 火把
    Example {
        keywords: &["torch", "火把", "light", "照明", "暗", "夜晚", "night"],
        turns: &[
            ("assistant", "gather(item=\"coal_ore\", count=1) 挖煤，craft(item=\"stick\", count=1) 做棒子。"),
            ("system", "Action output:\nSuccessfully gathered 1 coal, crafted 1 stick."),
            ("assistant", "craft(item=\"torch\", count=4) 用 1 煤 + 1 棒合 4 火把。"),
            ("system", "Action output:\nSuccessfully crafted 4 torch, you now have 4 torch."),
        ],
    },
    // 箱子存储
    Example {
        keywords: &["chest", "箱子", "存储", "store", "store"],
        turns: &[
            ("assistant", "gather(item=\"oak_log\", count=8); craft(item=\"oak_planks\", count=8) 准备 32 木板。"),
            ("system", "Action output:\nSuccessfully crafted 32 oak_planks."),
            ("assistant", "craft(item=\"chest\", count=1); place(item=\"chest\", x=-489, y=88, z=-146) 放箱子。"),
            ("system", "Action output:\nPlaced chest at (-489, 88, -146)."),
        ],
    },
    // 床
    Example {
        keywords: &["bed", "床", "sleep", "睡觉", "夜晚"],
        turns: &[
            ("assistant", "杀 3 只羊得 white_wool:3，砍 4 原木合木板。"),
            ("system", "Action output:\nGot 3 white_wool, 12 oak_planks."),
            ("assistant", "open 工作台后 craft_3x3(item=\"bed\", count=1) 合床。"),
            ("system", "Action output:\nSuccessfully crafted bed, you now have 1 bed."),
        ],
    },
    // 村民交易
    Example {
        keywords: &["villager", "村民", "trade", "交易"],
        turns: &[
            ("assistant", "interact_entity(kind=\"villager\") 打开附近村民交易界面。"),
            ("system", "Action output:\nOpened villager trade UI."),
            ("assistant", "trade(offer=0) 买第 1 个报价。"),
            ("system", "Action output:\nTrade completed, got 1 bread."),
        ],
    },
    // 下矿：用 mine_below 挖矿井
    Example {
        keywords: &["mine_below", "下矿", "dig", "挖矿", "挖矿井"],
        turns: &[
            ("assistant", "mine_below() 挖脚下方块开始下探。"),
            ("system", "Action output:\nMined block below, now at y=87."),
            ("assistant", "perceive 看看新方块，发现 iron_ore。mine(x=-489, y=85, z=-144) 挖矿。"),
            ("system", "Action output:\nMined block at (-489,85,-144), got iron_ore."),
        ],
    },
    // 建造：用 build 工具执行蓝图
    Example {
        keywords: &["build", "建造", "house", "房子", "shelter", "庇护所", "蓝", "蓝图"],
        turns: &[
            ("assistant", "先 gather(item=\"oak_log\", count=16) 准备材料，craft 成 32 oak_planks。"),
            ("system", "Action output:\nGot 32 oak_planks."),
            ("assistant", "build(blueprint=\"{\\\"blocks\\\":[{\\\"x\\\":-489,\\\"y\\\":88,\\\"z\\\":-144,\\\"block\\\":\\\"oak_planks\\\"},{\\\"x\\\":-489,\\\"y\\\":89,\\\"z\\\":-144,\\\"block\\\":\\\"oak_planks\\\"}]}\") 按蓝图放 2 个方块。"),
            ("system", "Action output:\n第1个: placed oak_planks @(-489,88,-144)\n第2个: placed oak_planks @(-489,89,-144)"),
        ],
    },
    // 附魔
    Example {
        keywords: &["enchant", "附魔", "enchanting", "附魔台"],
        turns: &[
            ("assistant", "需钻石+黑曜石+书。gather(item=\"diamond_ore\", count=2); craft(item=\"enchanting_table\", count=1); place 放置。"),
            ("system", "Action output:\nPlaced enchanting_table at (-489,88,-144)."),
            ("assistant", "open(x=-489, y=88, z=-144) 后 enchant(item=\"iron_sword\", level=2) 附魔 2 级。"),
            ("system", "Action output:\nEnchanted iron_sword with Sharpness II."),
        ],
    },
    // 多步计划：用 run_plan 一次跑多步
    Example {
        keywords: &["plan", "计划", "多步", "step", "run_plan", "依次"],
        turns: &[
            ("assistant", "用 run_plan 一次执行多步：先走到资源点，再挖矿，回工作台合成。"),
            ("assistant", "run_plan(steps=[{\"action\":\"goto\",\"x\":-490,\"y\":80,\"z\":-156},{\"action\":\"mine\",\"x\":-490,\"y\":80,\"z\":-156},{\"action\":\"goto\",\"x\":-489,\"y\":88,\"z\":-144},{\"action\":\"craft\",\"item\":\"torch\",\"count\":4}])"),
            ("system", "Action output:\n步骤1 (goto) 完成: 已到达目标 (-490,80,-156)\n步骤2 (mine) 完成: 已挖掉方块 (-490,80,-156), got coal_ore\n步骤3 (goto) 完成: 已到达目标 (-489,88,-144)\n步骤4 (craft) 完成: Successfully crafted 4 torch"),
        ],
    },
    // 卡住脱困（与 jailbreak 规则呼应）
    Example {
        keywords: &["stuck", "卡住", "未移动", "脱困", "trap"],
        turns: &[
            ("assistant", "perceive 提示「⚠ 卡住! 坐标5轮未移动」。改用 goto 到侧前方 3 格空地脱困。"),
            ("assistant", "goto(x=-486, y=88, z=-161) 走到附近空地。"),
            ("system", "Action output:\n已到达目标 (-486,88,-161)"),
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
    if keywords.is_empty() { 0.0 } else { matched as f64 / keywords.len() as f64 }
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

        // Few-shot 示例注入：词重叠检索最相关场景（离线可用，无需 embedding）
        // 关键：必须明确告诉 LLM 这是「预期 function calling 形态」的示例，
        // 否则 LLM 会模仿成在 assistant 文字里写 `tool(...)` 伪调用（实测反模式）。
        let mut scored: Vec<(f64, &Example)> = FEW_SHOT_EXAMPLES
            .iter()
            .map(|ex| (word_overlap_score(&recent_perception, ex.keywords), ex))
            .filter(|(score, _)| *score > 0.0)
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        if !scored.is_empty() {
            let mut example_text = String::from(
                "【参考示例】（以下展示预期的 function calling 形态：assistant 文字简短说明意图 + 真实 tool_calls JSON。**禁止**在 assistant 文字里写 `tool(...)` 伪调用，必须通过 function calling 输出工具调用。）\n",
            );
            for (i, (_, ex)) in scored.iter().take(2).enumerate() {
                example_text.push_str(&format!("场景 {}:\n", i + 1));
                for (role, content) in ex.turns {
                    let label = match *role {
                        "assistant" => "assistant (含 tool_call)",
                        "system" => "tool result",
                        _ => "user",
                    };
                    example_text.push_str(&format!("  {label}: {content}\n"));
                }
            }
            parts.push(example_text);
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

    pub fn build_context(&mut self) -> Context {
        use crate::core::message::Message;
        let jailbreak = "自主行动。工具失败时调整参数重试——不准假装成功。\n\
            行为准则：收到感知里的「卡住计数」≥3 时，说明坐标连续不变（可能下探被基岩/空气挡住或脚下方块无法破坏）——立即停止当前下探，改用 goto 侧前方 3 格空地或跳跃脱困，再重新 perceive；不要原地反复 perceive 或假装在下探。连续同工具≤3次后应向玩家 chat 汇报进度。工具没回报「实际获得X」就当作没获得，不得虚构成功。";
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
        if self.config.enable_knowledge_tool {
            if let Ok(def) = serde_json::from_str::<Value>(MANAGE_KNOWLEDGE_TOOL) {
                tool_defs.push(def);
            }
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
        assert!((score - 2.0 / 3.0).abs() < 1e-6, "2/3 keywords matched: {score}");
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
        assert!(!sys.contains("观察提醒"), "system prompt 不得含 obs_streak 动态文本");
        assert!(!sys.contains("不需要重新输入"), "system prompt 不得含 bootstrap 动态文本");
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
        assert!(has_manage_knowledge, "knowledge tool should be in tools when enabled");
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
        assert!(!has_manage_knowledge, "knowledge tool should be absent when disabled");
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
}
