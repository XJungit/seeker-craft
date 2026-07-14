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
    // 取第一个 { 到第一个 } 之间的内容（多个 JSON | 分隔时只取第一个）
    if let Some(s) = t.find('{')
        && let Some(e) = t[s..].find('}')
        && let Ok(v) = serde_json::from_str::<Value>(&t[s..=s + e])
    {
        return Ok(v);
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

        /// 返回 (reasoning_text, tool_calls), reasoning 为 LLM 思考内容
        pub fn chat_tools(
            &self,
            messages: &Value,
            tools: &Value,
        ) -> Result<(Option<String>, Vec<(String, String)>)> {
            let mut body = json!({
                "model": self.model,
                "messages": messages,
                "tools": tools,
                "temperature": self.temperature,
                "max_tokens": self.max_tokens,
            });
            if let (Some(Value::Object(extra)), Value::Object(base)) = (&self.extra_body, &mut body) {
                for (k, v) in extra {
                    base.insert(k.clone(), v.clone());
                }
            }
            let resp = self
                .client
                .post(&self.endpoint)
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()?
                .error_for_status()?
                .json::<Value>()?;

            let msg = &resp["choices"][0]["message"];
            let reasoning = msg["content"].as_str()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            let tool_calls = msg["tool_calls"].as_array();

            match tool_calls {
                Some(calls) => {
                    let mut result = Vec::new();
                    for tc in calls {
                        let name = tc["function"]["name"]
                            .as_str()
                            .unwrap_or("")
                            .to_string();
                        let args = tc["function"]["arguments"]
                            .as_str()
                            .unwrap_or("{}")
                            .to_string();
                        result.push((name, args));
                    }
                    Ok((reasoning, result))
                }
                None => {
                    let content = reasoning.unwrap_or_default();
                    Ok((None, vec![("text".into(), content)]))
                }
            }
        }

        fn chat_raw(&self, messages: &Value) -> Result<String> {
            let mut body = json!({
                "model": self.model,
                "messages": messages,
                "temperature": self.temperature,
                "max_tokens": self.max_tokens,
            });
            if let (Some(Value::Object(extra)), Value::Object(base)) = (&self.extra_body, &mut body) {
                for (k, v) in extra {
                    base.insert(k.clone(), v.clone());
                }
            }
            let resp = self
                .client
                .post(&self.endpoint)
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()?
                .error_for_status()?
                .json::<Value>()?;
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
            screenshot: vec![],
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
}
