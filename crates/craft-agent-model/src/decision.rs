//! 决策客户端：WorldState + 技能提示 → 抽象 Action
//!
//! 决策腿是「文本 LLM」：VLM 已经把画面看成结构化的 [`WorldState`]（场景描述 +
//! 编号元素 + 检测目标 + 自身状态），决策层只需在这份文本状态上推理出下一步
//! [`Action`]，**不再吃图**——这样又快又省，且与感知腿解耦。
//!
//! 真实客户端 [`real::OpenAiLlmClient`] 与视觉腿同构：同一套配置驱动
//! （[`crate::config::BackendConfig`]）、同一套 Windows http1 修复、`extra_body`
//! 透传，换后端只改 TOML。

use anyhow::{Result, anyhow, bail};
use craft_agent::core::message::{AssistantResponse, StopReason, ToolCall, Usage};
use craft_agent::core::types::{Action, Direction, WorldState};
use serde_json::Value;

/// 决策客户端接口
pub trait DecisionClient {
    /// 依据世界状态与可用技能提示，产出下一步抽象动作
    fn decide(&self, state: &WorldState, skills_hint: &str) -> Result<Action>;
}

/// 离线 mock：默认空转（Look 0,0）。真实实现由 LLM 产出 Action
pub struct MockDecisionClient;

impl DecisionClient for MockDecisionClient {
    fn decide(&self, _state: &WorldState, _skills_hint: &str) -> Result<Action> {
        Ok(Action::Look { dx: 0, dy: 0 })
    }
}

// ───────────────────────── Action JSON 契约（离线可测，不依赖网络）─────────────────────────
//
// 为什么不用 serde 直接反序列化 `Action`：`Action` 的默认（外部标签）表示是
// `{"Click": {"element_id": 5}}` 这种嵌套形式，对 LLM 很不友好、极易出错。
// 我们改用「扁平 + 判别字段」的线格式，让 LLM 更容易稳定产出：
//   {"action": "Click", "element_id": 5}
//   {"action": "AimAndMine", "target": "oak_tree"}
//   {"action": "Move", "dir": "Forward", "ticks": 20}
//   {"action": "Look", "dx": 100, "dy": 0}
// 解析全程容错（大小写、别名、被 markdown/散文包裹的 JSON）。

/// 从 LLM 的自由文本回复里提取出 JSON 对象。
///
/// 容忍三种常见污染：① ```json ... ``` 代码围栏；② JSON 前后夹带解释性散文；
/// ③ 整段就是纯 JSON。策略：优先截取第一个 `{` 到最后一个 `}` 的子串再解析。
pub fn extract_json(text: &str) -> Result<Value> {
    let t = text.trim();
    // 优先：整段纯 JSON
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return Ok(v);
    }
    // 取第一个 { 到匹配的 } 之间的内容（支持嵌套）
    if let Some(s) = t.find('{') {
        let mut depth = 0i32;
        for (i, c) in t[s..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                let e = s + i;
                if let Ok(v) = serde_json::from_str::<Value>(&t[s..=e]) {
                    return Ok(v);
                }
                break;
            }
        }
    }
    serde_json::from_str::<Value>(t)
        .map_err(|e| anyhow!("无法从 LLM 输出解析 JSON: {e}；原文: {text}"))
}

/// 方向解析：容忍大小写、中英文别名、WASD/键位说法。
pub fn parse_direction(s: &str) -> Result<Direction> {
    match s.trim().to_ascii_lowercase().as_str() {
        "forward" | "front" | "w" | "前" | "前进" => Ok(Direction::Forward),
        "back" | "backward" | "s" | "后" | "后退" => Ok(Direction::Back),
        "left" | "a" | "左" => Ok(Direction::Left),
        "right" | "d" | "右" => Ok(Direction::Right),
        "up" | "jump" | "space" | "上" | "跳" => Ok(Direction::Up),
        "down" | "sneak" | "shift" | "下" | "潜行" => Ok(Direction::Down),
        other => bail!("未知移动方向: {other}"),
    }
}

/// 把线格式 JSON 映射成内部 [`Action`]。判别字段接受 `action` 或 `type`。
pub fn value_to_action(v: &Value) -> Result<Action> {
    let kind = v["action"]
        .as_str()
        .or_else(|| v["type"].as_str())
        .or_else(|| v["tool"].as_str())
        .ok_or_else(|| anyhow!("JSON 缺少 action/type/tool 字段: {v}"))?;
    match kind.trim().to_ascii_lowercase().as_str() {
        "click" => {
            let id = v["element_id"]
                .as_u64()
                .or_else(|| v["id"].as_u64())
                .ok_or_else(|| anyhow!("Click 缺少 element_id: {v}"))? as u32;
            Ok(Action::Click { element_id: id })
        }
        "aimandmine" | "aim_and_mine" | "mine" => {
            let target = v["target"]
                .as_str()
                .or_else(|| v["name"].as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            Ok(Action::AimAndMine { target })
        }
        "move" => {
            let dir = v["dir"]
                .as_str()
                .or_else(|| v["direction"].as_str())
                .ok_or_else(|| anyhow!("Move 缺少 dir: {v}"))?;
            let dir = parse_direction(dir)?;
            // ticks 缺省给 20（约 1 秒），避免 LLM 漏填导致失败
            let ticks = v["ticks"].as_u64().unwrap_or(20) as u32;
            Ok(Action::Move { dir, ticks })
        }
        "look" => {
            let dx = v["dx"].as_i64().unwrap_or(0) as i32;
            let dy = v["dy"].as_i64().unwrap_or(0) as i32;
            Ok(Action::Look { dx, dy })
        }
        other => bail!("未知 action 类型: {other}（可选 Click/AimAndMine/Move/Look）"),
    }
}

/// 解析 OpenAI 兼容 chat/completions 响应为结构化 assistant 响应。
/// 作为纯函数单测真实响应形状，避免“假 provider 通过、真实链路失效”。
pub fn parse_chat_tools_response(resp: &Value) -> Result<AssistantResponse> {
    let choice = resp["choices"]
        .as_array()
        .and_then(|choices| choices.first())
        .ok_or_else(|| anyhow!("LLM 响应缺少 choices[0]: {resp}"))?;
    let msg = &choice["message"];
    let content = msg["content"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let reasoning = msg["reasoning_content"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    let mut tool_calls = Vec::new();
    if let Some(calls) = msg["tool_calls"].as_array() {
        for tc in calls {
            let id = tc["id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow!("tool_call 缺少 id: {tc}"))?;
            let name = tc["function"]["name"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow!("tool_call 缺少 function.name: {tc}"))?;
            let raw_args = tc["function"]["arguments"]
                .as_str()
                .ok_or_else(|| anyhow!("tool_call 缺少 function.arguments: {tc}"))?;
            let arguments: Value = serde_json::from_str(raw_args)
                .map_err(|e| anyhow!("tool_call 参数不是合法 JSON: {e}; call={tc}"))?;
            tool_calls.push(ToolCall {
                id: id.to_owned(),
                name: name.to_owned(),
                arguments,
            });
        }
    }

    let usage_json = &resp["usage"];
    let usage = Usage {
        input_tokens: usage_json["prompt_tokens"].as_u64().unwrap_or(0),
        output_tokens: usage_json["completion_tokens"].as_u64().unwrap_or(0),
        total_tokens: usage_json["total_tokens"].as_u64().unwrap_or(0),
    };
    let raw_reason = choice["finish_reason"].as_str().unwrap_or("stop");
    let stop_reason = match raw_reason {
        "stop" => StopReason::Stop,
        "tool_calls" | "function_call" => StopReason::ToolCalls,
        "length" => StopReason::Length,
        "content_filter" => StopReason::ContentFilter,
        other => StopReason::Other(other.to_owned()),
    };

    Ok(AssistantResponse {
        content,
        reasoning,
        tool_calls,
        usage,
        stop_reason,
    })
}

/// 把 [`WorldState`] 渲染成给 LLM 的紧凑文本状态。
pub fn render_state(state: &WorldState) -> String {
    let mut s = String::new();
    s.push_str("【场景描述】\n");
    s.push_str(if state.scene_desc.is_empty() {
        "（无）"
    } else {
        &state.scene_desc
    });
    s.push('\n');

    s.push_str("\n【可点击元素（编号→标签→中心坐标）】\n");
    if state.marked_elements.is_empty() {
        s.push_str("（无标记元素）\n");
    } else {
        for e in &state.marked_elements {
            s.push_str(&format!(
                "  #{} {} @({}, {})\n",
                e.id, e.label, e.center.0, e.center.1
            ));
        }
    }

    s.push_str("\n【3D 检测目标（标签→相对准星偏移 dx,dy）】\n");
    if state.detected_targets.is_empty() {
        s.push_str("（无检测目标）\n");
    } else {
        for t in &state.detected_targets {
            s.push_str(&format!(
                "  {} @偏移({}, {})\n",
                t.label, t.offset_from_crosshair.0, t.offset_from_crosshair.1
            ));
        }
    }

    s.push_str("\n【自身状态】\n");
    s.push_str(if state.self_hint.is_empty() {
        "（无）"
    } else {
        &state.self_hint
    });
    s.push('\n');
    s
}

/// 组装完整决策 prompt：角色 + 状态 + 技能提示 + 输出契约（仅 JSON）。
pub fn build_decision_prompt(state: &WorldState, skills_hint: &str) -> String {
    format!(
        "你是一个 Minecraft 游戏 AI，负责在纯视觉信息下决定下一步操作。\n\
         下面是当前由视觉系统解析出的世界状态：\n\
         ────────────────\n{state}────────────────\n\n\
         可用技能/提示：\n{skills}\n\n\
         请从以下 4 种动作里选择**唯一**下一步，并严格只输出一个 JSON 对象（不要解释、不要代码围栏）：\n\
         1) 点击编号元素：{{\"action\":\"Click\",\"element_id\":<编号>}}\n\
         2) 对准并挖掘目标：{{\"action\":\"AimAndMine\",\"target\":\"<标签,如 oak_tree>\"}}\n\
         3) 移动：{{\"action\":\"Move\",\"dir\":\"Forward|Back|Left|Right|Up|Down\",\"ticks\":<整数>}}\n\
         4) 转视角（相对像素）：{{\"action\":\"Look\",\"dx\":<整数>,\"dy\":<整数>}}\n\n\
         只输出 JSON：",
        state = render_state(state),
        skills = if skills_hint.is_empty() {
            "（无）"
        } else {
            skills_hint
        },
    )
}

#[cfg(feature = "real")]
pub mod real {
    use super::*;
    use crate::config::BackendConfig;
    use reqwest::blocking::Client;
    use serde_json::json;

    /// 通用 **OpenAI 兼容** 决策客户端（文本 chat/completions）。
    ///
    /// 与 [`crate::vision::real::OpenAiVisionClient`] 同构：配置驱动、`extra_body`
    /// 透传、共用 [`crate::config::build_http_client`]（含 Windows http1 修复）。
    /// 输入 [`WorldState`] → prompt → LLM → 解析为 [`Action`]。
    pub struct OpenAiLlmClient {
        endpoint: String,
        model: String,
        api_key: String,
        temperature: f32,
        max_tokens: u32,
        extra_body: Option<Value>,
        client: Client,
    }

    impl OpenAiLlmClient {
        /// 折叠多轮 tool-calling 历史为纯文本，适配不支持多轮 tool 的上游端点。
        /// - 删除 `role:"tool"` 消息（上游不认）
        /// - 剥除 assistant 的 `tool_calls` 字段（仅留 content）
        /// - 把工具结果以文本追进对应 assistant 的 content，保留语义
        fn fold_tool_history(messages: &Value) -> Value {
            let Some(arr) = messages.as_array() else { return messages.clone() };
            let mut out: Vec<Value> = Vec::new();
            // 收集 tool_call_id -> 工具结果文本，供回写 assistant content。
            let mut tool_results: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();
            for m in arr {
                if m.get("role").and_then(|r| r.as_str()) == Some("tool") {
                    let id = m
                        .get("tool_call_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let content = m
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    tool_results.insert(id, content);
                    continue; // 丢弃 role:tool 消息本身
                }
                out.push(m.clone());
            }
            // 第二遍：剥 assistant 的 tool_calls，并把对应结果回写 content。
            for m in out.iter_mut() {
                if m.get("role").and_then(|r| r.as_str()) != Some("assistant") {
                    continue;
                }
                let calls = m.get("tool_calls").and_then(|v| v.as_array()).cloned();
                if let Some(calls) = calls {
                    let mut summary = String::new();
                    for c in &calls {
                        let name = c
                            .get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("?");
                        let args = c
                            .get("function")
                            .and_then(|f| f.get("arguments"))
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        let id = c
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let result = tool_results
                            .get(id)
                            .cloned()
                            .unwrap_or_else(|| "(无结果)".to_string());
                        summary.push_str(&format!(
                            "\n[工具 {name} 参数 {args} → 结果 {result}]"
                        ));
                    }
                    // 剥除 tool_calls，结果并入 content。
                    if let Value::Object(map) = m {
                        map.remove("tool_calls");
                        let existing = map
                            .get("content")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let merged = if existing.is_empty() {
                            summary.trim_start().to_string()
                        } else {
                            format!("{existing}{summary}")
                        };
                        map.insert("content".into(), Value::String(merged));
                        if map.get("content").and_then(|v| v.as_str()) == Some("") {
                            map.insert("content".into(), Value::String("（已执行工具）".into()));
                        }
                    }
                }
            }
            Value::Array(out)
        }

        /// 直接用三要素构造（temperature/max_tokens 取默认，无 extra_body）。
        pub fn new(
            endpoint: impl Into<String>,
            model: impl Into<String>,
            api_key: impl Into<String>,
        ) -> Self {
            Self {
                endpoint: endpoint.into(),
                model: model.into(),
                api_key: api_key.into(),
                temperature: 0.2,
                max_tokens: 512,
                extra_body: None,
                client: crate::config::build_http_client(180, true),
            }
        }

        /// **推荐入口**：从配置后端构造。密钥按 `api_key`/`api_key_env` 解析。
        pub fn from_config(cfg: &BackendConfig) -> Result<Self> {
            let api_key = cfg.resolve_api_key()?;
            Ok(Self {
                endpoint: cfg.chat_endpoint(),
                model: cfg.model.clone(),
                api_key,
                temperature: cfg.temperature,
                max_tokens: cfg.max_tokens,
                extra_body: cfg.extra_body.clone(),
                client: crate::config::build_http_client(cfg.timeout_secs, cfg.force_http1),
            })
        }

        /// 从环境变量构造（快速本地测试用）：
        /// - `LLM_API_KEY`（必需）
        /// - `LLM_API_BASE`（可选，默认 `https://apihub.agnes-ai.com/v1`）
        /// - `LLM_MODEL`（可选，默认 `agnes-2.0-flash`）
        pub fn from_env() -> Result<Self> {
            let api_key =
                std::env::var("LLM_API_KEY").map_err(|_| anyhow!("未设置环境变量 LLM_API_KEY"))?;
            let base = std::env::var("LLM_API_BASE")
                .unwrap_or_else(|_| "https://apihub.agnes-ai.com/v1".to_string());
            let endpoint = format!("{}/chat/completions", base.trim_end_matches('/'));
            let model =
                std::env::var("LLM_MODEL").unwrap_or_else(|_| "agnes-2.0-flash".to_string());
            Ok(Self::new(endpoint, model, api_key))
        }

        /// 纯文本 chat：prompt → 模型文本回复（供决策/反思等复用）。
        pub fn chat_text(&self, prompt: &str) -> Result<String> {
            self.chat_raw(&json!([{"role": "user", "content": prompt}]))
        }

        /// 带工具的结构化 chat。保留 assistant 正文、推理、provider 原始 call id、usage 与终止原因。
        ///
        /// 上游适配：部分端点（如本地 OC-DSV4F 代理背后的 deepseek-v4）不支持
        /// 多轮 tool-calling 历史——只要 messages 里出现 `tool_calls` 或 `role:"tool"`
        /// 就返回 invalid_request_error。这里在发送前把这类历史折叠为纯文本：
        /// 删去 `role:"tool"` 消息，剥除 assistant 的 `tool_calls` 字段（仅留 content），
        /// 并把工具结果以文本追进对应 assistant 的 content，保留语义不丢上下文。
        /// agent 核心的多轮协议不受影响（它读的是自身内存的 messages）。
        pub fn chat_tools(&self, messages: &Value, tools: &Value) -> Result<AssistantResponse> {
            let messages = Self::fold_tool_history(messages);
            let mut body = json!({
                "model": self.model,
                "messages": messages,
                "temperature": self.temperature,
                "max_tokens": self.max_tokens,
            });
            // 仅当 tools 非空才发送 tools 字段：部分兼容端（如 stepfun step-3.7-flash）
            // 收到 "tools":[] 时会把正文塞进 reasoning_content 且 content 为空
            // (finish_reason=length)，导致纯文本/无工具场景拿不到正文。
            if tools.as_array().is_some_and(|arr| !arr.is_empty()) {
                body["tools"] = tools.clone();
            }
            if let (Some(Value::Object(extra)), Value::Object(base)) = (&self.extra_body, &mut body)
            {
                for (k, v) in extra {
                    base.insert(k.clone(), v.clone());
                }
            }
            let mut last_err = None;
            for attempt in 0..3 {
                let resp_text = self
                    .client
                    .post(&self.endpoint)
                    .bearer_auth(&self.api_key)
                    .header("Accept", "application/json")
                    .json(&body)
                    .send()?;
                match resp_text.error_for_status_ref() {
                    Ok(_) => {
                        let resp_text = resp_text.text()?;
                        let clean = resp_text.trim_end_matches("data: [DONE]");
                        let resp: Value = serde_json::from_str(clean)?;
                        return parse_chat_tools_response(&resp);
                    }
                    Err(e) => {
                        // 上游偶发限流/过载（invalid_request_error + Upstream request
                        // failed）对相同合法请求瞬时拒绝，退避重试可恢复。
                        let status = resp_text.status().as_u16();
                        if status == 400 || status == 429 {
                            last_err = Some(e.into());
                            if attempt < 2 {
                                std::thread::sleep(std::time::Duration::from_millis(
                                    500 * (attempt as u64 + 1) + 500,
                                ));
                                continue;
                            }
                        } else {
                            return Err(e.into());
                        }
                    }
                }
            }
            Err(last_err.unwrap_or_else(|| anyhow!("LLM 请求失败")))
        }

        fn chat_raw(&self, messages: &Value) -> Result<String> {
            let mut body = json!({
                "model": self.model,
                "messages": messages,
                "temperature": self.temperature,
                "max_tokens": self.max_tokens,
            });
            if let (Some(Value::Object(extra)), Value::Object(base)) = (&self.extra_body, &mut body)
            {
                for (k, v) in extra {
                    base.insert(k.clone(), v.clone());
                }
            }
            let resp_text = self
                .client
                .post(&self.endpoint)
                .bearer_auth(&self.api_key)
                .header("Accept", "application/json")
                .json(&body)
                .send()?
                .error_for_status()?
                .text()?;
            let clean = resp_text.trim_end_matches("data: [DONE]");
            let resp: Value = serde_json::from_str(clean)?;
            let msg = &resp["choices"][0]["message"];
            let content = msg["content"].as_str().filter(|s| !s.is_empty());
            let content = content
                .or_else(|| msg["reasoning_content"].as_str())
                .ok_or_else(|| anyhow!("LLM 响应缺少 content: {resp}"))?;
            Ok(content.to_string())
        }
    }

    impl DecisionClient for OpenAiLlmClient {
        fn decide(&self, state: &WorldState, skills_hint: &str) -> Result<Action> {
            let prompt = build_decision_prompt(state, skills_hint);
            let reply = self.chat_text(&prompt)?;
            let v = extract_json(&reply)?;
            value_to_action(&v)
        }
    }

    /// 向后兼容别名：旧代码里的 `AgnesLlmClient` 现指向通用客户端。
    pub type AgnesLlmClient = OpenAiLlmClient;
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_agent::core::types::{Element, Target};

    fn fake_state() -> WorldState {
        WorldState {
            scene_desc: "前方有一棵橡木树".into(),
            marked_elements: vec![Element {
                id: 1,
                label: "crafting_table".into(),
                bbox: [10, 20, 30, 40],
                center: (25, 40),
            }],
            detected_targets: vec![Target {
                label: "oak_tree".into(),
                bbox: [0, 0, 1, 1],
                offset_from_crosshair: (12, -3),
            }],
            self_hint: "血量满，快捷栏有斧头".into(),
            screenshot: std::sync::Arc::new(vec![]),
        }
    }

    #[test]
    fn mock_decision_returns_action() {
        let d = MockDecisionClient;
        let a = d.decide(&fake_state(), "").unwrap();
        assert!(matches!(a, Action::Look { .. }));
    }

    #[test]
    fn parse_plain_json_click() {
        let v = extract_json(r#"{"action":"Click","element_id":3}"#).unwrap();
        assert!(matches!(
            value_to_action(&v).unwrap(),
            Action::Click { element_id: 3 }
        ));
    }

    #[test]
    fn parse_json_wrapped_in_prose_and_fences() {
        let raw = "好的，我的决定是：\n```json\n{\"action\": \"AimAndMine\", \"target\": \"oak_tree\"}\n```\n希望有用";
        let v = extract_json(raw).unwrap();
        match value_to_action(&v).unwrap() {
            Action::AimAndMine { target } => assert_eq!(target, "oak_tree"),
            other => panic!("期望 AimAndMine，得到 {other:?}"),
        }
    }

    #[test]
    fn parse_move_with_alias_and_default_ticks() {
        // dir 用中文别名、缺 ticks → 应容错并给默认 20
        let v = extract_json(r#"{"action":"move","direction":"前进"}"#).unwrap();
        match value_to_action(&v).unwrap() {
            Action::Move { dir, ticks } => {
                assert!(matches!(dir, Direction::Forward));
                assert_eq!(ticks, 20);
            }
            other => panic!("期望 Move，得到 {other:?}"),
        }
    }

    #[test]
    fn parse_look_and_type_field_alias() {
        // 用 type 代替 action、部分字段缺省
        let v = extract_json(r#"{"type":"Look","dx":150}"#).unwrap();
        assert!(matches!(
            value_to_action(&v).unwrap(),
            Action::Look { dx: 150, dy: 0 }
        ));
    }

    #[test]
    fn unknown_action_errors() {
        let v = extract_json(r#"{"action":"Fly"}"#).unwrap();
        assert!(value_to_action(&v).is_err());
    }

    #[test]
    fn render_state_contains_key_fields() {
        let s = render_state(&fake_state());
        assert!(s.contains("crafting_table"));
        assert!(s.contains("oak_tree"));
        assert!(s.contains("斧头"));
    }

    #[test]
    fn parses_plain_text_completion_without_fake_tool() {
        let raw = serde_json::json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {"role":"assistant", "content":"任务完成", "reasoning_content":"检查完成"}
            }],
            "usage": {"prompt_tokens":10, "completion_tokens":2, "total_tokens":12}
        });
        let r = parse_chat_tools_response(&raw).unwrap();
        assert_eq!(r.content.as_deref(), Some("任务完成"));
        assert_eq!(r.reasoning.as_deref(), Some("检查完成"));
        assert!(r.tool_calls.is_empty(), "纯文本绝不能伪造成 text 工具");
        assert_eq!(r.stop_reason, StopReason::Stop);
        assert_eq!(r.usage.total_tokens, 12);
    }

    #[test]
    fn preserves_provider_tool_call_id_and_arguments() {
        let raw = serde_json::json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {"role":"assistant", "content":null, "reasoning_content":"先观察", "tool_calls":[{
                    "id":"call_from_provider", "type":"function",
                    "function":{"name":"perceive", "arguments":"{\"prompt\":\"scene\"}"}
                }]}
            }],
            "usage": {"prompt_tokens":20, "completion_tokens":5, "total_tokens":25}
        });
        let r = parse_chat_tools_response(&raw).unwrap();
        assert_eq!(r.tool_calls[0].id, "call_from_provider");
        assert_eq!(r.tool_calls[0].arguments["prompt"], "scene");
        assert_eq!(r.stop_reason, StopReason::ToolCalls);
    }

    #[test]
    fn stepfun_empty_tools_yields_empty_content_not_reasoning() {
        // 回归：stepfun step-3.7-flash 收到 "tools":[] 时会把正文塞进 reasoning_content、
        // content 为空、finish_reason=length（已在 chat_tools 改为空 tools 不发送该字段规避）。
        // 此处锁定解析层：即便遇到这种畸形响应，content 必须为空而非误取 reasoning，
        // 防止将来有人为“修 content 空”而把 reasoning 当 content fallback（会污染 agent 决策语义）。
        let raw = serde_json::json!({
            "choices": [{
                "finish_reason": "length",
                "message": {"role":"assistant", "content":null, "reasoning_content":"let me think step by step..."}
            }],
            "usage": {"prompt_tokens":1, "completion_tokens":1, "total_tokens":2}
        });
        let r = parse_chat_tools_response(&raw).unwrap();
        assert!(
            r.content.is_none(),
            "畸形响应 content 应为空，不能误取 reasoning"
        );
        assert!(r.reasoning.is_some());
        assert_eq!(r.stop_reason, StopReason::Length);
    }

    #[test]
    fn rejects_invalid_tool_arguments_json() {
        let raw = serde_json::json!({
            "choices": [{"finish_reason":"tool_calls", "message":{"tool_calls":[{
                "id":"bad", "function":{"name":"mine", "arguments":"not-json"}
            }]}}]
        });
        assert!(parse_chat_tools_response(&raw).is_err());
    }
}
