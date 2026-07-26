use crate::core::message::{Message, Usage, system_chatml};
use crate::core::session::SessionEntry as SessionFileEntry;
use serde_json::Value;

use super::{Agent, CompactionResult, LlmProvider, build_knowledge_string};

impl Agent {
    pub const CHARS_PER_TOKEN: usize = 2;
    pub const IMAGE_TOKENS: u64 = 1200;

    pub fn estimate_tokens(&self) -> u32 {
        let system_chars = self.config.prompt.len()
            + build_knowledge_string(&self.tools, self.config.knowledge_base.as_deref()).len();
        let system_tokens = system_chars as u64 / Self::CHARS_PER_TOKEN as u64;

        let mut measured_total: u64 = 0;
        let mut last_usage_end: usize = 0;
        for (i, m) in self.messages.iter().enumerate() {
            if let Message::Assistant(a) = m
                && a.usage.total_tokens > 0
            {
                measured_total = measured_total.saturating_add(a.usage.total_tokens);
                last_usage_end = i + 1;
            }
        }

        let tail_estimate = self.estimate_tokens_range(last_usage_end, self.messages.len());

        let total = if measured_total > 0 {
            system_tokens
                .saturating_add(measured_total)
                .saturating_add(tail_estimate)
        } else {
            let all_heuristic = self.estimate_tokens_range(0, self.messages.len());
            system_tokens.saturating_add(all_heuristic)
        };

        u32::try_from(total).unwrap_or(u32::MAX)
    }

    pub fn estimate_tokens_range(&self, start: usize, end: usize) -> u64 {
        let mut total: u64 = 0;
        for m in &self.messages[start..end] {
            total = total.saturating_add(Self::msg_tokens(m));
        }
        total
    }

    pub fn msg_tokens(m: &Message) -> u64 {
        match m {
            Message::User(u) => {
                let text_tokens = u.content.len() as u64 / Self::CHARS_PER_TOKEN as u64;
                let image_tokens = u.images.len() as u64 * Self::IMAGE_TOKENS;
                text_tokens.saturating_add(image_tokens)
            }
            Message::Assistant(a) => {
                let mut tokens: u64 = 0;
                if let Some(r) = &a.reasoning {
                    tokens = tokens.saturating_add(r.len() as u64 / Self::CHARS_PER_TOKEN as u64);
                }
                if let Some(c) = &a.content
                    && !c.is_empty()
                {
                    tokens = tokens.saturating_add(c.len() as u64 / Self::CHARS_PER_TOKEN as u64);
                }
                for tc in &a.tool_calls {
                    let json_len = Self::json_byte_len(&tc.arguments) as u64;
                    tokens = tokens.saturating_add((tc.name.len() as u64 + json_len) / 3);
                }
                tokens
            }
            Message::ToolResult(r) => {
                let text_tokens = r.content.len() as u64 / Self::CHARS_PER_TOKEN as u64;
                let image_tokens = r.images.len() as u64 * Self::IMAGE_TOKENS;
                text_tokens.saturating_add(image_tokens)
            }
        }
    }

    pub fn json_byte_len(value: &Value) -> usize {
        struct Counter(usize);
        impl std::io::Write for Counter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0 = self.0.saturating_add(buf.len());
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut c = Counter(0);
        let _ = serde_json::to_writer(&mut c, value);
        c.0
    }

    pub fn compact(&mut self) -> anyhow::Result<CompactionResult> {
        let keep_tokens = self.config.compaction.keep_recent;
        let mut kept: u32 = 0;
        let mut cut = self.messages.len();
        for (i, msg) in self.messages.iter().enumerate().rev() {
            let t = Self::msg_tokens(msg) as u32;
            if kept + t > keep_tokens {
                cut = i + 1;
                break;
            }
            kept += t;
        }
        if cut == 0 || cut >= self.messages.len() {
            self.usage = Usage::default();
            return Ok(CompactionResult::default());
        }

        while cut < self.messages.len() {
            let prev_is_assistant = self
                .messages
                .get(cut.wrapping_sub(1))
                .map(|m| matches!(m, Message::Assistant(a) if !a.tool_calls.is_empty()))
                .unwrap_or(false);
            let cur_is_tool_result = self
                .messages
                .get(cut)
                .map(|m| matches!(m, Message::ToolResult(_)))
                .unwrap_or(false);
            if cur_is_tool_result && !prev_is_assistant {
                cut -= 1;
            } else {
                break;
            }
        }
        if cut == 0 {
            return Ok(CompactionResult::default());
        }

        // 用统一的 token 估算（实测优先 / 消息累加 / 启发式），避免重复计入：
        // 之前 `msg_tokens 求和 + self.usage.total_tokens` 会把每条消息算两遍（usage 已含同等内容）。
        let tokens_before = self.estimate_tokens_range(0, cut);
        let recent_count = self.messages.len() - cut;
        let first_kept_entry_id = self
            .session
            .as_ref()
            .map(|s| {
                let mut count = 0usize;
                for entry in s.entries.iter().rev() {
                    if let SessionFileEntry::Message(_) = entry {
                        count += 1;
                        if count == recent_count {
                            return entry.id().to_string();
                        }
                    }
                }
                String::new()
            })
            .unwrap_or_default();
        // 序列化旧历史时剔除"易变瞬时注入"（perceive 状态、邻近世界记忆、
        // [当前目标] 重注、nudge 提示词），它们每轮重生且易过期，
        // 进入摘要会污染压缩结果（如矛盾坐标、过时目标）。
        let old: Vec<String> = self.messages[..cut]
            .iter()
            .filter(|m| match m {
                Message::User(u) => {
                    !(u.content.starts_with("【当前游戏状态（自动注入）】")
                        || u.content.starts_with("【邻近世界记忆】")
                        || u.content.starts_with("[当前目标]")
                        // P1 改进5: 过滤 nudge 提示词 — 它们是瞬时纠正，不应进入摘要
                        || u.content.starts_with("【纠正】")
                        || u.content.starts_with("【继续】")
                        || u.content.starts_with("【强制行动】")
                        || u.content.starts_with("【死循环警告】")
                        || u.content.starts_with("【连续失败警告】")
                        || u.content.starts_with("【探索建议】")
                        || u.content.starts_with("【系统提示】"))
                }
                _ => true,
            })
            .map(Self::serialize_msg)
            .collect();
        let mut prompt = format!("<conversation>\n{}\n</conversation>\n\n", old.join("\n\n"));
        let system = if let Some(prev) = &self.previous_summary {
            prompt.push_str(&format!(
                "<previous-summary>\n{prev}\n</previous-summary>\n\n"
            ));
            prompt.push_str(super::UPDATE_SUMMARIZATION_PROMPT);
            super::COMPACTION_SYSTEM
        } else {
            prompt.push_str(super::SUMMARIZATION_PROMPT);
            super::COMPACTION_SYSTEM
        };

        let cm = vec![system_chatml(system), Message::user(prompt).to_chatml()];

        // 压缩调用：先专用模型，失败再回退主模型。返回 (摘要, 是否成功, 失败原因)。
        fn try_summarize(messages: &[Value], provider: &dyn LlmProvider) -> Result<String, String> {
            let mut result: Option<String> = None;
            let mut last_err: Option<String> = None;
            for attempt in 1..=3 {
                match provider.complete(messages, &[]) {
                    Ok(resp) => {
                        if let Some(t) = resp.content.as_ref().filter(|t| !t.trim().is_empty()) {
                            result = Some(t.clone());
                        } else if let Some(t) =
                            resp.reasoning.as_ref().filter(|t| !t.trim().is_empty())
                        {
                            result = Some(t.clone());
                        } else {
                            last_err = Some("empty response".into());
                        }
                        if result.is_some() {
                            break;
                        }
                    }
                    Err(e) => {
                        last_err = Some(format!("{e}"));
                        if attempt < 3 {
                            std::thread::sleep(std::time::Duration::from_millis(
                                500 * attempt as u64,
                            ));
                        }
                    }
                }
            }
            result.ok_or_else(|| last_err.unwrap_or_else(|| "unknown".into()))
        }

        // 1) 优先专用压缩模型（隔离主模型/换小模型）
        let summary = if let Some(comp) = self.compaction_provider.as_ref() {
            match try_summarize(&cm, comp.as_ref()) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[compaction] 专用压缩模型失败，回退主模型: {e}");
                    // 2) 回退主决策模型再试一次
                    match try_summarize(&cm, self.provider.as_ref()) {
                        Ok(s) => s,
                        Err(e2) => {
                            return Err(anyhow::anyhow!(
                                "compaction failed: 专用模型({e}) 与主模型({e2}) 均失败"
                            ));
                        }
                    }
                }
            }
        } else {
            // 未配专用模型：直接用主模型
            match try_summarize(&cm, self.provider.as_ref()) {
                Ok(s) => s,
                Err(e) => {
                    return Err(anyhow::anyhow!("compaction failed: 主模型 {e}"));
                }
            }
        };

        let recent: Vec<_> = self.messages.drain(cut..).collect();
        let summary_msg = Message::user(format!(
            "The conversation history before this point was compacted into the following summary:\n\n<summary>\n{}\n</summary>",
            summary
        ));
        self.messages = vec![summary_msg];
        self.messages.extend(recent);
        self.previous_summary = Some(summary.clone());
        self.pending_checkpoint = true;
        self.usage = Usage::default();

        let comp_result = CompactionResult {
            summary,
            first_kept_entry_id,
            tokens_before,
        };
        self.last_compaction = Some(comp_result.clone());
        Ok(comp_result)
    }

    /// 硬截断：不调 LLM，直接丢弃最旧的消息，保留系统提示 + 最近 N 条。
    /// 用于 compaction 失败或禁用时兜底，避免小模型因上下文过长而卡死。
    pub fn hard_truncate(&mut self) {
        let keep_tokens = self.config.compaction.keep_recent;
        // 从最新往前累加，找到保留边界
        let mut kept: u32 = 0;
        let mut cut = self.messages.len();
        for (i, msg) in self.messages.iter().enumerate().rev() {
            let t = Self::msg_tokens(msg) as u32;
            if kept + t > keep_tokens {
                cut = i + 1;
                break;
            }
            kept += t;
        }
        if cut == 0 || cut >= self.messages.len() {
            return; // 没有可丢弃的旧消息
        }
        // 修正边界：避免把 tool_result 单独留下（需要前面的 assistant 配对）
        while cut < self.messages.len() {
            let prev_is_assistant = self
                .messages
                .get(cut.wrapping_sub(1))
                .map(|m| matches!(m, Message::Assistant(a) if !a.tool_calls.is_empty()))
                .unwrap_or(false);
            let cur_is_tool_result = self
                .messages
                .get(cut)
                .map(|m| matches!(m, Message::ToolResult(_)))
                .unwrap_or(false);
            if cur_is_tool_result && !prev_is_assistant {
                cut -= 1;
            } else {
                break;
            }
        }
        if cut == 0 {
            return;
        }
        let dropped = self.messages.len() - cut;
        self.messages.drain(0..cut);
        self.usage = Usage::default();
        self.pending_checkpoint = true;
        eprintln!(
            "[compaction] 硬截断 {} 条旧消息（保留最近 {}），未调用 LLM",
            dropped,
            self.messages.len()
        );
    }

    pub fn serialize_msg(m: &Message) -> String {
        match m {
            Message::User(u) => {
                let mut s = format!("user: {}", u.content);
                if !u.images.is_empty() {
                    s.push_str(&format!(" [{} images]", u.images.len()));
                }
                s
            }
            Message::Assistant(a) => {
                let mut s = String::new();
                if let Some(r) = &a.reasoning {
                    s.push_str(&format!("[Think] {r}\n"));
                }
                if let Some(c) = &a.content
                    && !c.is_empty()
                {
                    s.push_str(&format!("{c}\n"));
                }
                for tc in &a.tool_calls {
                    s.push_str(&format!("-> {}({})\n", tc.name, tc.arguments));
                }
                s.trim().to_string()
            }
            Message::ToolResult(r) => format!("result({}): {}", r.tool_name, r.content),
        }
    }
}

pub fn is_obs_tool(name: &str) -> bool {
    matches!(name, "perceive" | "visual_perceive" | "look" | "look_at")
}

#[cfg(test)]
mod tests {
    use crate::core::message::{AssistantMsg, Message, ToolCall, Usage};
    use serde_json::json;

    use super::*;

    // ── is_obs_tool ──

    #[test]
    fn is_obs_tool_recognizes_perceive() {
        assert!(is_obs_tool("perceive"));
        assert!(is_obs_tool("visual_perceive"));
        assert!(is_obs_tool("look"));
        assert!(is_obs_tool("look_at"));
    }

    #[test]
    fn is_obs_tool_rejects_non_obs() {
        assert!(!is_obs_tool("goto"));
        assert!(!is_obs_tool("mine"));
        assert!(!is_obs_tool("craft"));
        assert!(!is_obs_tool("gather"));
        assert!(!is_obs_tool(""));
    }

    // ── json_byte_len ──

    #[test]
    fn json_byte_len_matches_serde_for_empty() {
        let v = json!({});
        assert_eq!(Agent::json_byte_len(&v), serde_json::to_string(&v).unwrap().len());
    }

    #[test]
    fn json_byte_len_matches_serde_for_nested() {
        let v = json!({
            "name": "gather",
            "args": {"item": "oak_log", "count": 5},
            "nested": {"a": [1, 2, 3], "b": "text"}
        });
        assert_eq!(Agent::json_byte_len(&v), serde_json::to_string(&v).unwrap().len());
    }

    // ── msg_tokens ──

    #[test]
    fn msg_tokens_user_text_only() {
        let m = Message::user("hello world");
        let tokens = Agent::msg_tokens(&m);
        // "hello world" = 11 chars / 2 = 5 tokens
        assert!(tokens >= 5 && tokens <= 10, "expected ~5 tokens, got {tokens}");
    }

    #[test]
    fn msg_tokens_user_with_images() {
        let m = Message::user_with_images("hello", vec!["data:image/png;base64,A".into()]);
        let tokens = Agent::msg_tokens(&m);
        // text: 5/2 = 2, image: 1 * 1200 = 1200
        assert!(tokens >= 1200, "should include image tokens: {tokens}");
    }

    #[test]
    fn msg_tokens_assistant_plain_text() {
        let m = Message::assistant_text("test message");
        let tokens = Agent::msg_tokens(&m);
        assert!(tokens > 0, "assistant text should have tokens");
    }

    #[test]
    fn msg_tokens_assistant_with_tool_calls() {
        let m = Message::Assistant(AssistantMsg {
            content: Some("let me gather".into()),
            reasoning: None,
            tool_calls: vec![ToolCall {
                id: "c1".into(),
                name: "gather".into(),
                arguments: json!({"item": "oak_log", "count": 4}),
            }],
            timestamp: 0,
            usage: Usage::default(),
        });
        let tokens = Agent::msg_tokens(&m);
        assert!(tokens > 0, "assistant with tool calls should have tokens");
    }

    #[test]
    fn msg_tokens_assistant_reasoning() {
        let m = Message::assistant_with_reasoning("thought process", "final answer");
        let tokens = Agent::msg_tokens(&m);
        assert!(tokens > 0, "assistant with reasoning should have tokens");
    }

    #[test]
    fn msg_tokens_tool_result() {
        let m = Message::tool_result("c1", "gather", "got 4 oak_log");
        let tokens = Agent::msg_tokens(&m);
        assert!(tokens > 0, "tool result should have tokens");
    }

    #[test]
    fn msg_tokens_empty_content() {
        let m = Message::Assistant(AssistantMsg {
            content: None,
            reasoning: None,
            tool_calls: vec![],
            timestamp: 0,
            usage: Usage::default(),
        });
        let tokens = Agent::msg_tokens(&m);
        assert_eq!(tokens, 0, "empty assistant should have 0 tokens");
    }

    // ── serialize_msg ──

    #[test]
    fn serialize_msg_user() {
        let m = Message::user("hello");
        let s = Agent::serialize_msg(&m);
        assert_eq!(s, "user: hello");
    }

    #[test]
    fn serialize_msg_user_with_images() {
        let m = Message::user_with_images("look", vec!["base64data".into()]);
        let s = Agent::serialize_msg(&m);
        assert!(s.contains("[1 images]"), "should include image count: {s}");
    }

    #[test]
    fn serialize_msg_assistant_text() {
        let m = Message::assistant_text("ok");
        let s = Agent::serialize_msg(&m);
        assert_eq!(s, "ok");
    }

    #[test]
    fn serialize_msg_assistant_tool_call() {
        let m = Message::assistant_tool_call("c1", "gather", json!({"item":"oak_log"}));
        let s = Agent::serialize_msg(&m);
        assert!(s.contains("-> gather("), "should show tool call: {s}");
    }

    #[test]
    fn serialize_msg_assistant_reasoning() {
        let m = Message::assistant_with_reasoning("think step", "do action");
        let s = Agent::serialize_msg(&m);
        assert!(s.contains("[Think]"), "should include reasoning: {s}");
        assert!(s.contains("do action"), "should include content: {s}");
    }

    #[test]
    fn serialize_msg_tool_result() {
        let m = Message::tool_result("c1", "gather", "got 4 oak_log");
        let s = Agent::serialize_msg(&m);
        assert!(s.starts_with("result(gather)"), "should start with result(tool_name): {s}");
    }

    #[test]
    fn serialize_msg_tool_error() {
        let m = Message::tool_error("c1", "gather", "block not found");
        let s = Agent::serialize_msg(&m);
        assert!(s.starts_with("result(gather)"), "error should still format as result: {s}");
    }
}
