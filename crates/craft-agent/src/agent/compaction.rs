use crate::core::message::{Message, Usage, system_chatml};
use crate::core::session::SessionEntry as SessionFileEntry;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::Ordering;

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

        // P96：后台预压缩已产出摘要 → 直接应用，本轮不阻塞调用 LLM。
        let prefetched = self.prefetch_summary.lock().unwrap().take();
        if let Some(summary) = prefetched {
            let comp_result = self.apply_summary(cut, summary);
            self.last_compaction = Some(comp_result.clone());
            return Ok(comp_result);
        }

        // 序列化旧历史时剔除"易变瞬时注入"（perceive 状态、邻近世界记忆、
        // [当前目标] 重注、nudge 提示词），它们每轮重生且易过期，
        // 进入摘要会污染压缩结果（如矛盾坐标、过时目标）。
        let cm = build_cm(&self.messages[..cut], self.previous_summary.as_deref());

        // 1) 优先专用压缩模型（隔离主模型/换小模型）
        // 2) 失败回退主决策模型再试一次
        let summary = request_summary(
            self.compaction_provider.as_deref(),
            self.provider.as_ref(),
            &cm,
        )
        .map_err(|e| anyhow::anyhow!("compaction failed: {e}"))?;

        let comp_result = self.apply_summary(cut, summary);
        self.last_compaction = Some(comp_result.clone());
        Ok(comp_result)
    }

    /// 用摘要替换 `[..cut]` 的历史，保留最近 `cut..` 消息。
    /// 同步压缩与 P96 后台预压缩共用此路径。
    fn apply_summary(&mut self, cut: usize, summary: String) -> CompactionResult {
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
        CompactionResult {
            summary,
            first_kept_entry_id,
            tokens_before,
        }
    }

    /// P96：后台预压缩（pi compaction_worker 的两阶段非阻塞思路）。
    /// 消息量达到触发阈值的 2/3 时，提前在后台线程生成摘要；
    /// 下一轮 compact() 直接取用，主循环不被 LLM 压缩调用阻塞。
    pub fn maybe_prefetch_compaction(&mut self) {
        if !self.config.enable_compaction {
            return;
        }
        if self.prefetch_in_flight.load(Ordering::Acquire) {
            return;
        }
        // 已有待用摘要（上一轮 spawn 完成、本轮 compact 尚未取用）→ 不重复 spawn
        if self.prefetch_summary.lock().unwrap().is_some() {
            return;
        }
        let budget = self
            .config
            .compaction
            .context_window
            .saturating_sub(self.config.compaction.reserve);
        // 压缩触发线是 60% 预算；达到 40%（2/3 提前量）即开始后台预压缩
        if self.estimate_tokens() < budget * 2 / 5 {
            return;
        }
        if self.messages.is_empty() {
            return;
        }
        let snapshot: Vec<Message> = self.messages.clone();
        let previous = self.previous_summary.clone();
        let cm = build_cm(&snapshot, previous.as_deref());
        self.prefetch_in_flight.store(true, Ordering::Release);
        let out = self.prefetch_summary.clone();
        let in_flight = self.prefetch_in_flight.clone();
        let primary = Arc::clone(&self.provider);
        let comp = self.compaction_provider.clone();
        std::thread::spawn(move || {
            let result = request_summary(comp.as_deref(), primary.as_ref(), &cm);
            match result {
                Ok(s) => *out.lock().unwrap() = Some(s),
                Err(e) => eprintln!("[compaction] 后台预压缩失败（下轮回退同步压缩）: {e}"),
            }
            in_flight.store(false, Ordering::Release);
        });
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

/// 构建压缩请求（chatml）：过滤易变瞬时注入（perceive/记忆/目标/nudge），
/// 附带 previous-summary（增量压缩）。同步压缩与 P96 后台预压缩共用。
fn build_cm(messages: &[Message], previous_summary: Option<&str>) -> Vec<Value> {
    let old: Vec<String> = messages
        .iter()
        .filter(|m| {
            // B3：统一用 `Message::is_transient()` 过滤全部轮间注入消息
            // （perceive/记忆/目标/nudge/警告/引导）——不进入压缩摘要。
            !m.is_transient()
        })
        .map(Agent::serialize_msg)
        .collect();
    let mut prompt = format!("<conversation>\n{}\n</conversation>\n\n", old.join("\n\n"));
    let system = if let Some(prev) = previous_summary {
        prompt.push_str(&format!(
            "<previous-summary>\n{prev}\n</previous-summary>\n\n"
        ));
        prompt.push_str(super::UPDATE_SUMMARIZATION_PROMPT);
        super::COMPACTION_SYSTEM
    } else {
        prompt.push_str(super::SUMMARIZATION_PROMPT);
        super::COMPACTION_SYSTEM
    };
    vec![system_chatml(system), Message::user(prompt).to_chatml()]
}

/// 执行压缩摘要：先专用模型（最多 3 次重试），失败回退主模型再试一次。
/// 同步压缩与 P96 后台预压缩共用。
fn request_summary(
    comp: Option<&dyn LlmProvider>,
    primary: &dyn LlmProvider,
    cm: &[Value],
) -> Result<String, String> {
    fn try_summarize(messages: &[Value], provider: &dyn LlmProvider) -> Result<String, String> {
        let mut result: Option<String> = None;
        let mut last_err: Option<String> = None;
        for attempt in 1..=3 {
            match provider.complete(messages, &[]) {
                Ok(resp) => {
                    if let Some(t) = resp.content.as_ref().filter(|t| !t.trim().is_empty()) {
                        result = Some(t.clone());
                    } else if let Some(t) = resp.reasoning.as_ref().filter(|t| !t.trim().is_empty())
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
                        std::thread::sleep(std::time::Duration::from_millis(500 * attempt as u64));
                    }
                }
            }
        }
        result.ok_or_else(|| last_err.unwrap_or_else(|| "unknown".into()))
    }

    match comp {
        Some(comp) => match try_summarize(cm, comp) {
            Ok(s) => Ok(s),
            Err(e) => {
                eprintln!("[compaction] 专用压缩模型失败，回退主模型: {e}");
                // 保留专用模型失败原因，两者都失败时如实上报（P91 回归锁定）
                try_summarize(cm, primary)
                    .map_err(|e2| format!("专用模型({e}) 与主模型({e2}) 均失败"))
            }
        },
        None => try_summarize(cm, primary),
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
        assert_eq!(
            Agent::json_byte_len(&v),
            serde_json::to_string(&v).unwrap().len()
        );
    }

    #[test]
    fn json_byte_len_matches_serde_for_nested() {
        let v = json!({
            "name": "gather",
            "args": {"item": "oak_log", "count": 5},
            "nested": {"a": [1, 2, 3], "b": "text"}
        });
        assert_eq!(
            Agent::json_byte_len(&v),
            serde_json::to_string(&v).unwrap().len()
        );
    }

    // ── msg_tokens ──

    #[test]
    fn msg_tokens_user_text_only() {
        let m = Message::user("hello world");
        let tokens = Agent::msg_tokens(&m);
        // "hello world" = 11 chars / 2 = 5 tokens
        assert!(
            (5..=10).contains(&tokens),
            "expected ~5 tokens, got {tokens}"
        );
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
        assert!(
            s.starts_with("result(gather)"),
            "should start with result(tool_name): {s}"
        );
    }

    #[test]
    fn serialize_msg_tool_error() {
        let m = Message::tool_error("c1", "gather", "block not found");
        let s = Agent::serialize_msg(&m);
        assert!(
            s.starts_with("result(gather)"),
            "error should still format as result: {s}"
        );
    }

    // ── P97：build_cm 过滤语义记忆瞬时注入 ──

    #[test]
    fn build_cm_filters_semantic_memory_injection() {
        let messages = vec![
            Message::user("【长期记忆】\n1. [策略] 钻石镐策略：用钻石镐挖钻石最快"),
            Message::user("【邻近世界记忆】\n钻石矿 @(10,12,-20)"),
            Message::user("[当前目标] 挖钻石"),
            Message::assistant_text("好的，先找钻石"),
            Message::tool_result("c1", "goto", "到达"),
        ];
        let cm = build_cm(&messages, None);
        let joined = serde_json::to_string(&cm).unwrap();
        assert!(
            !joined.contains("长期记忆"),
            "【长期记忆】是每轮重生的瞬时注入，不应进入压缩摘要"
        );
        assert!(!joined.contains("邻近世界记忆"));
        assert!(!joined.contains("当前目标"));
        assert!(joined.contains("好的，先找钻石"), "真实交互历史应保留");
    }

    // ── P96：后台预压缩（compaction_worker）──

    use crate::agent::AgentConfig;
    use crate::core::message::AssistantResponse;
    use crate::core::tool::ToolRegistry;
    use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

    #[derive(Debug)]
    struct CountingProvider {
        calls: Arc<AtomicU32>,
    }
    impl LlmProvider for CountingProvider {
        fn complete(
            &self,
            _messages: &[Value],
            _tools: &[Value],
        ) -> anyhow::Result<AssistantResponse> {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(20));
            Ok(AssistantResponse {
                content: Some("后台预压缩摘要内容".into()),
                reasoning: None,
                tool_calls: vec![],
                usage: Usage::default(),
                stop_reason: crate::core::message::StopReason::Stop,
            })
        }
    }

    fn wait_for<F: Fn() -> bool>(timeout_ms: u64, cond: F) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        while std::time::Instant::now() < deadline {
            if cond() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        cond()
    }

    #[test]
    fn p96_prefetch_compaction_runs_in_background_and_compact_skips_llm() {
        // 小预算配置：context_window=1000, reserve=200 → budget=800 → 40% 触发线 320 tokens
        let mut config = AgentConfig::new("test".into(), 1);
        config.enable_compaction = true;
        config.compaction.context_window = 1000;
        config.compaction.reserve = 200;
        config.compaction.keep_recent = 200;
        config.auto_perceive = false;
        let calls = Arc::new(AtomicU32::new(0));
        let mut agent = Agent::new(
            Box::new(CountingProvider {
                calls: calls.clone(),
            }),
            ToolRegistry::new(),
            config,
        );
        // 造 400+ tokens 的消息（40 条 × ~20 chars）
        for _ in 0..40 {
            agent
                .messages
                .push(Message::user(format!("早期对话片段 {}", "x".repeat(20))));
        }
        let est = agent.estimate_tokens();
        assert!(est >= 320, "测试前置：token 估算应超过预取线，got {est}");

        // 预取：后台线程生成摘要
        agent.maybe_prefetch_compaction();
        assert!(
            agent.prefetch_in_flight.load(AtomicOrdering::Acquire),
            "预取应在途"
        );
        assert!(
            wait_for(3000, || !agent
                .prefetch_in_flight
                .load(AtomicOrdering::Acquire)),
            "后台预压缩应在 3s 内完成"
        );
        // 摘要已产出，且只调用了一次 provider（后台线程）
        let prefetched = agent.prefetch_summary.lock().unwrap().clone();
        assert_eq!(
            prefetched.as_deref(),
            Some("后台预压缩摘要内容"),
            "后台线程应产出摘要"
        );
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 1, "仅后台线程调用 1 次");

        // compact() 直接取用预取摘要，不再调用 LLM
        let result = agent.compact().unwrap();
        assert!(!result.summary.is_empty(), "compact 应返回预取摘要");
        assert_eq!(
            calls.load(AtomicOrdering::SeqCst),
            1,
            "compact 不得再调用 provider"
        );
        assert_eq!(
            agent.previous_summary.as_deref(),
            Some("后台预压缩摘要内容")
        );
        assert!(
            agent.messages[0]
                .to_chatml()
                .to_string()
                .contains("<summary>"),
            "消息应以摘要开头"
        );
    }

    #[test]
    fn p96_prefetch_is_idempotent_until_summary_consumed() {
        let mut config = AgentConfig::new("test".into(), 1);
        config.enable_compaction = true;
        config.compaction.context_window = 1000;
        config.compaction.reserve = 200;
        config.compaction.keep_recent = 200;
        config.auto_perceive = false;
        let calls = Arc::new(AtomicU32::new(0));
        let mut agent = Agent::new(
            Box::new(CountingProvider {
                calls: calls.clone(),
            }),
            ToolRegistry::new(),
            config,
        );
        for _ in 0..40 {
            agent
                .messages
                .push(Message::user(format!("早期对话片段 {}", "x".repeat(20))));
        }
        // 第一次预取 → 后台完成
        agent.maybe_prefetch_compaction();
        assert!(
            wait_for(3000, || !agent
                .prefetch_in_flight
                .load(AtomicOrdering::Acquire)),
            "后台预压缩应在 3s 内完成"
        );
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 1);
        // 摘要未消费时再次调用 → 不重复 spawn（仍在途标志也不会被覆盖）
        agent.maybe_prefetch_compaction();
        assert!(
            !agent.prefetch_in_flight.load(AtomicOrdering::Acquire),
            "幂等：不应再次起后台任务"
        );
        assert_eq!(
            calls.load(AtomicOrdering::SeqCst),
            1,
            "幂等：provider 不应被重复调用"
        );
        // 消费（compact 取走）后可再次预取：直接取走摘要模拟 compact 消费
        agent.prefetch_summary.lock().unwrap().take();
        agent.maybe_prefetch_compaction();
        assert!(
            wait_for(3000, || !agent
                .prefetch_in_flight
                .load(AtomicOrdering::Acquire)),
            "消费后应能再次预取"
        );
        assert_eq!(
            calls.load(AtomicOrdering::SeqCst),
            2,
            "第二次预取应再次调用"
        );
    }
}
