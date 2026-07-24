use crate::core::message::system_chatml;
use crate::core::prompt::PromptBuilder;
use serde_json::Value;

use super::{Agent, Context, MANAGE_KNOWLEDGE_TOOL};

/// Few-shot 示例：场景关键词 + 成功工具调用模式
struct Example {
    keywords: &'static [&'static str],
    /// (role, content) 对，role 为 "user"/"system"/"assistant"
    turns: &'static [(&'static str, &'static str)],
}

const FEW_SHOT_EXAMPLES: &[Example] = &[
    Example {
        keywords: &["oak_log", "wood", "tree", "砍树", "木头", "原木"],
        turns: &[
            ("assistant", "gather(\"oak_log\", 4)"),
            ("system", "成功采集 4 个 oak_log"),
            ("assistant", "craft(\"oak_planks\", 4)"),
            ("system", "成功合成 16 个 oak_planks"),
        ],
    },
    Example {
        keywords: &["crafting_table", "工作台", "craft"],
        turns: &[
            ("assistant", "craft(\"crafting_table\", 1)"),
            ("system", "成功合成 crafting_table"),
            ("assistant", "place(\"crafting_table\", 10, 64, 20)"),
            ("system", "已放置 crafting_table"),
        ],
    },
    Example {
        keywords: &["stone_pickaxe", "石镐", "pickaxe", "镐"],
        turns: &[
            ("assistant", "gather(\"oak_log\", 4)"),
            ("system", "成功采集 4 个 oak_log"),
            ("assistant", "craft(\"crafting_table\", 1); place(\"crafting_table\", 10, 64, 20)"),
            ("system", "已放置 crafting_table"),
            ("assistant", "open(10, 64, 20); craft(\"stick\", 2)"),
            ("system", "成功合成 stick"),
            ("assistant", "gather(\"cobblestone\", 3)"),
            ("system", "成功采集 3 个 cobblestone"),
            ("assistant", "craft(\"stone_pickaxe\", 1)"),
            ("system", "成功合成 stone_pickaxe"),
        ],
    },
    Example {
        keywords: &["iron_ore", "铁矿", "iron", "铁", "smelt", "熔炼"],
        turns: &[
            ("assistant", "goto(15, 30, -10); mine(15, 30, -10)"),
            ("system", "已挖掉 iron_ore"),
            ("assistant", "gather(\"coal\", 4); craft(\"furnace\", 1); place(\"furnace\", 10, 64, 20)"),
            ("system", "已放置 furnace"),
            ("assistant", "open(10, 64, 20); smelt(\"iron_ingot\", \"coal\", 3)"),
            ("system", "成功熔炼 3 个 iron_ingot"),
        ],
    },
    Example {
        keywords: &["torch", "火把", "light", "照明", "暗"],
        turns: &[
            ("assistant", "craft(\"stick\", 2); craft(\"torch\", 4)"),
            ("system", "成功合成 4 个 torch"),
            ("assistant", "place(\"torch\", 10, 64, 20)"),
            ("system", "已放置 torch"),
        ],
    },
    Example {
        keywords: &["chest", "箱子", "存储", "store"],
        turns: &[
            ("assistant", "gather(\"oak_log\", 8); craft(\"oak_planks\", 8)"),
            ("system", "成功合成 32 个 oak_planks"),
            ("assistant", "craft(\"chest\", 1); place(\"chest\", 10, 64, 20)"),
            ("system", "已放置 chest"),
        ],
    },
    Example {
        keywords: &["bed", "床", "sleep", "睡觉", "夜晚", "night"],
        turns: &[
            ("assistant", "gather(\"white_wool\", 3); gather(\"oak_log\", 4)"),
            ("system", "成功采集材料"),
            ("assistant", "craft(\"oak_planks\", 4); craft(\"bed\", 1)"),
            ("system", "成功合成 bed"),
            ("assistant", "place(\"bed\", 10, 64, 20)"),
            ("system", "已放置 bed"),
        ],
    },
    Example {
        keywords: &["furnace", "熔炉", "smelt", "熔炼", "cook", "烧"],
        turns: &[
            ("assistant", "gather(\"cobblestone\", 8); craft(\"furnace\", 1); place(\"furnace\", 10, 64, 20)"),
            ("system", "已放置 furnace"),
            ("assistant", "open(10, 64, 20); smelt(\"iron_ingot\", \"coal\", 3)"),
            ("system", "成功熔炼 3 个 iron_ingot"),
        ],
    },
    Example {
        keywords: &["enchant", "附魔", "enchanting", "附魔台"],
        turns: &[
            ("assistant", "gather(\"diamond\", 2); craft(\"enchanting_table\", 1); place(\"enchanting_table\", 10, 64, 20)"),
            ("system", "已放置 enchanting_table"),
            ("assistant", "open(10, 64, 20); enchant(\"iron_sword\", 2)"),
            ("system", "附魔完成"),
        ],
    },
    Example {
        keywords: &["villager", "村民", "trade", "交易"],
        turns: &[
            ("assistant", "interact_entity(\"villager\")"),
            ("system", "已打开村民交易界面"),
            ("assistant", "trade(0)"),
            ("system", "交易完成"),
        ],
    },
    Example {
        keywords: &["zombie", "僵尸", "creeper", "苦力怕", "skeleton", "骷髅", "spider", "蜘蛛", "attack", "攻击", "fight", "打"],
        turns: &[
            ("assistant", "attack(\"nearest\")"),
            ("system", "攻击完成"),
        ],
    },
    Example {
        keywords: &["explore", "探索", "find", "找", "look", "看看", "附近", "周围"],
        turns: &[
            ("assistant", "goto(0, 64, 0)"),
            ("system", "已到达 (0, 64, 0)"),
            ("assistant", "memory(\"query\", 32)"),
            ("system", "附近记忆查询结果"),
        ],
    },
    Example {
        keywords: &["build", "建造", "house", "房子", "shelter", "庇护所"],
        turns: &[
            ("assistant", "gather(\"oak_log\", 16); craft(\"oak_planks\", 16); gather(\"cobblestone\", 32)"),
            ("system", "材料已备齐"),
            ("assistant", "craft(\"crafting_table\", 1); place(\"crafting_table\", 10, 64, 20)"),
            ("system", "已放置 crafting_table"),
            ("assistant", "open(10, 64, 20); craft(\"oak_planks\", 8)"),
            ("system", "合成完成"),
        ],
    },
    Example {
        keywords: &["mine_below", "下矿", "dig", "挖矿", "mine", "mine"],
        turns: &[
            ("assistant", "mine_below()"),
            ("system", "已挖掉脚下方块"),
            ("assistant", "mine_below()"),
            ("system", "已挖掉脚下方块"),
        ],
    },
    Example {
        keywords: &["coal", "煤", "charcoal", "木炭", "fuel", "燃料"],
        turns: &[
            ("assistant", "gather(\"coal_ore\", 4)"),
            ("system", "成功采集 4 个 coal"),
            ("assistant", "craft(\"torch\", 4)"),
            ("system", "成功合成 4 个 torch"),
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
        let mut scored: Vec<(f64, &Example)> = FEW_SHOT_EXAMPLES
            .iter()
            .map(|ex| (word_overlap_score(&recent_perception, ex.keywords), ex))
            .filter(|(score, _)| *score > 0.0)
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        if !scored.is_empty() {
            let mut example_text = String::from("【参考示例】\n");
            for (i, (_, ex)) in scored.iter().take(2).enumerate() {
                example_text.push_str(&format!("场景 {}:\n", i + 1));
                for (role, content) in ex.turns {
                    let label = match *role {
                        "assistant" => "行动",
                        "system" => "结果",
                        _ => "输入",
                    };
                    example_text.push_str(&format!("  {label}: {content}\n"));
                }
            }
            parts.push(example_text);
        }

        if self.obs_streak >= 5 {
            if self.obs_streak >= 10 {
                parts.push("【循环警告】你已经连续观察 10+ 步没有实际行动！STOP repeating the same action. Pick a COMPLETELY DIFFERENT tool RIGHT NOW — nav_to, collect, craft, build, combat — anything but what you've been doing.".to_string());
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
