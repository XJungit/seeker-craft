use crate::core::message::system_chatml;
use crate::core::prompt::PromptBuilder;
use serde_json::Value;

use super::{Agent, Context, MANAGE_KNOWLEDGE_TOOL};

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

    pub fn build_context(&mut self) -> Context {
        use crate::core::message::Message;
        let jailbreak = "自主行动。工具失败时调整参数重试——不准假装成功。";
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
