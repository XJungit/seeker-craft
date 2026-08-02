//! 对话消息类型 — pi 风格的类型化消息枚举
//!
//! 参考 pi_agent_rust src/model.rs:
//! - Message enum (User/Assistant/ToolResult/System)
//! - ToolCall struct (id + name + arguments)
//! - Usage tracking (input/output tokens)
//!
//! 与 pi 的差异: pi 的 System 消息不单独存储, 我们用 system prompt 单独管理。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::time::{SystemTime, UNIX_EPOCH};

#[inline]
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// 对话中的一条消息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
    /// 用户输入 (pi: UserMessage)
    User(UserMsg),
    /// 助手回复, 可能包含 tool_calls (pi: AssistantMessage)
    Assistant(AssistantMsg),
    /// 工具执行结果 (pi: ToolResultMessage)
    ToolResult(ToolResultMsg),
}

/// 用户消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMsg {
    pub content: String,
    /// 毫秒时间戳（持久化/检索用，不发送给 LLM）
    #[serde(default)]
    pub timestamp: i64,
    /// 可选图像段：base64 data URI。非空时 `to_chatml` 把 `content` 改为
    /// `[{type:text},{type:image_url}]` 数组，让多模态 LLM 在 user 角色下看截图。
    /// 这是比挂在 tool 角色更通用、更可移植的做法（很多 OpenAI 兼容端只认
    /// user/assistant 角色的图）。旧 JSONL 无此字段也能反序列化（默认空）。
    #[serde(default)]
    pub images: Vec<String>,
}

/// 助手消息 (pi: AssistantMessage + ThinkingContent)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantMsg {
    /// 文本回复
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// 思维链 (pi: ThinkingContent, 进历史供下轮参考)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// 工具调用列表
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// 毫秒时间戳（持久化/检索用，不发送给 LLM）
    #[serde(default)]
    pub timestamp: i64,
    /// 该条 assistant 消息产生时的 token 用量（来自 LLM 返回的 usage）。
    /// 用于上下文压缩时的精确 token 估算（参考 pi_agent_rust 的 per-message usage）。
    #[serde(default)]
    pub usage: Usage,
}

/// 工具调用 (pi: ToolCall)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// Provider 返回的原始调用 ID，后续 tool result 必须原样引用。
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// Provider 对本次生成的终止原因。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Stop,
    ToolCalls,
    Length,
    ContentFilter,
    MaxIterations,
    Error,
    Other(String),
}

/// 一次结构化 assistant 响应。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantResponse {
    pub content: Option<String>,
    pub reasoning: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    pub stop_reason: StopReason,
}

/// 工具结果消息 (pi: ToolResultMessage)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultMsg {
    pub tool_call_id: String,
    pub tool_name: String,
    /// LLM 可读的结果描述
    pub content: String,
    /// 是否执行出错
    #[serde(default)]
    pub is_error: bool,
    /// 毫秒时间戳（持久化/检索用，不发送给 LLM）
    #[serde(default)]
    pub timestamp: i64,
    /// 可选结构化元数据（pi: ToolResultMessage.details）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    /// 可选图像段：base64 data URI（如 `data:image/png;base64,...`）。
    /// 非空时 `to_chatml` 把 `content` 改为 `[{type:text},{type:image_url}]` 数组，
    /// 让多模态 LLM 直接看截图。旧版本 JSONL 无此字段也能反序列化（默认空）。
    #[serde(default)]
    pub images: Vec<String>,
}

/// Token 用量追踪 (pi: Usage)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// 整轮上下文总 token (pi: total_tokens, estimate_context_tokens 优先用它)
    pub total_tokens: u64,
    /// DeepSeek 上下文缓存命中 token 数（prompt_cache_hit_tokens）。
    /// 用于观测前缀缓存是否生效；非 DeepSeek 兼容端点无此字段时为 0。
    #[serde(default, rename = "prompt_cache_hit_tokens")]
    pub cache_hit_tokens: u64,
    /// DeepSeek 上下文缓存未命中 token 数（prompt_cache_miss_tokens）。
    #[serde(default, rename = "prompt_cache_miss_tokens")]
    pub cache_miss_tokens: u64,
}

// ── 构造器 (pi: Message::assistant / Message::tool_result 风格) ──

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self::User(UserMsg {
            content: content.into(),
            timestamp: now_ms(),
            images: vec![],
        })
    }

    /// 构造带图像段的用户消息（多模态 LLM 直读场景）。
    /// `images` 为 base64 data URI 列表，非空时 `to_chatml` 输出图像内容段。
    pub fn user_with_images(content: impl Into<String>, images: Vec<String>) -> Self {
        Self::User(UserMsg {
            content: content.into(),
            timestamp: now_ms(),
            images,
        })
    }

    pub fn assistant_text(content: impl Into<String>) -> Self {
        Self::Assistant(AssistantMsg {
            content: Some(content.into()),
            reasoning: None,
            tool_calls: vec![],
            timestamp: now_ms(),
            usage: Usage::default(),
        })
    }

    pub fn assistant_tool_call(
        id: impl Into<String>,
        name: impl Into<String>,
        args: Value,
    ) -> Self {
        Self::Assistant(AssistantMsg {
            content: None,
            reasoning: None,
            tool_calls: vec![ToolCall {
                id: id.into(),
                name: name.into(),
                arguments: args,
            }],
            timestamp: now_ms(),
            usage: Usage::default(),
        })
    }

    /// 从 provider 响应构造 assistant 消息，保留原始 tool-call ID。
    pub fn assistant_response(response: &AssistantResponse) -> Self {
        Self::Assistant(AssistantMsg {
            content: response.content.clone(),
            reasoning: response.reasoning.clone(),
            tool_calls: response.tool_calls.clone(),
            timestamp: now_ms(),
            usage: response.usage.clone(),
        })
    }

    pub fn assistant_with_reasoning(
        content: impl Into<String>,
        reasoning: impl Into<String>,
    ) -> Self {
        Self::Assistant(AssistantMsg {
            content: Some(content.into()),
            reasoning: Some(reasoning.into()),
            tool_calls: vec![],
            timestamp: now_ms(),
            usage: Usage::default(),
        })
    }

    pub fn tool_result(
        id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self::ToolResult(ToolResultMsg {
            tool_call_id: id.into(),
            tool_name: name.into(),
            content: content.into(),
            is_error: false,
            timestamp: now_ms(),
            details: None,
            images: vec![],
        })
    }

    /// 构造带图像段的工具结果（多模态 LLM 直读场景）。
    /// `images` 为 base64 data URI 列表，非空时 `to_chatml` 会输出图像内容段。
    pub fn tool_result_with_images(
        id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
        images: Vec<String>,
    ) -> Self {
        Self::ToolResult(ToolResultMsg {
            tool_call_id: id.into(),
            tool_name: name.into(),
            content: content.into(),
            is_error: false,
            timestamp: now_ms(),
            details: None,
            images,
        })
    }

    pub fn tool_error(
        id: impl Into<String>,
        name: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        Self::ToolResult(ToolResultMsg {
            tool_call_id: id.into(),
            tool_name: name.into(),
            content: error.into(),
            is_error: true,
            timestamp: now_ms(),
            details: None,
            images: vec![],
        })
    }
}

// ── 转换为 OpenAI ChatML 格式 (发送给 LLM) ──

impl Message {
    /// 转换为 OpenAI Chat Completions 的 message 格式
    pub fn to_chatml(&self) -> Value {
        match self {
            Self::User(m) => {
                let mut obj = Map::new();
                obj.insert("role".into(), Value::String("user".into()));
                obj.insert("content".into(), Value::String(m.content.clone()));
                Value::Object(obj)
            }
            Self::Assistant(m) => {
                let mut obj = Map::new();
                obj.insert("role".into(), Value::String("assistant".into()));
                obj.insert(
                    "content".into(),
                    match &m.content {
                        Some(c) => Value::String(c.clone()),
                        None => Value::Null,
                    },
                );
                if let Some(r) = &m.reasoning {
                    obj.insert("reasoning_content".into(), Value::String(r.clone()));
                }
                if !m.tool_calls.is_empty() {
                    let calls: Vec<Value> = m
                        .tool_calls
                        .iter()
                        .map(|tc| {
                            let mut func = Map::new();
                            func.insert("name".into(), Value::String(tc.name.clone()));
                            func.insert(
                                "arguments".into(),
                                Value::String(tc.arguments.to_string()),
                            );
                            let mut call = Map::new();
                            call.insert("id".into(), Value::String(tc.id.clone()));
                            call.insert("type".into(), Value::String("function".into()));
                            call.insert("function".into(), Value::Object(func));
                            Value::Object(call)
                        })
                        .collect();
                    obj.insert("tool_calls".into(), Value::Array(calls));
                }
                Value::Object(obj)
            }
            Self::ToolResult(m) => {
                // P92：失败结果统一加【失败】前缀，LLM 一眼识别失败，不被长文本淹没。
                // 内容已带标记时不重复叠加（P89 【已中止】占位等兼容）。
                let content = if m.is_error && !m.content.starts_with("【失败】") {
                    format!("【失败】{}", m.content)
                } else {
                    m.content.clone()
                };
                let content = Value::String(content);
                let mut obj = Map::new();
                obj.insert("role".into(), Value::String("tool".into()));
                obj.insert("tool_call_id".into(), Value::String(m.tool_call_id.clone()));
                obj.insert("content".into(), content);
                Value::Object(obj)
            }
        }
    }
}

/// 将系统提示词包装为 ChatML system 消息
pub fn system_chatml(prompt: &str) -> Value {
    serde_json::json!({
        "role": "system",
        "content": prompt
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_message_to_chatml() {
        let msg = Message::user("hello");
        let chatml = msg.to_chatml();
        assert_eq!(chatml["role"], "user");
        assert_eq!(chatml["content"], "hello");
    }

    #[test]
    fn user_with_images_strips_images_in_chatml() {
        // 决策 LLM 不应收到图，to_chatml 应只保留文字。
        let msg = Message::user_with_images(
            "请直接看图回答",
            vec!["data:image/png;base64,CCCC".to_string()],
        );
        let chatml = msg.to_chatml();
        assert_eq!(chatml["role"], "user");
        assert_eq!(chatml["content"], "请直接看图回答");
    }

    #[test]
    fn assistant_tool_call_to_chatml() {
        let msg = Message::assistant_tool_call("call_1", "perceive", serde_json::json!({}));
        let chatml = msg.to_chatml();
        assert_eq!(chatml["role"], "assistant");
        assert_eq!(chatml["content"], serde_json::Value::Null);
        assert_eq!(chatml["tool_calls"][0]["function"]["name"], "perceive");
    }

    #[test]
    fn tool_result_to_chatml() {
        let msg = Message::tool_result("call_1", "perceive", "检测到tree,stone");
        let chatml = msg.to_chatml();
        assert_eq!(chatml["role"], "tool");
        assert_eq!(chatml["tool_call_id"], "call_1");
        assert_eq!(chatml["content"], "检测到tree,stone");
    }

    #[test]
    fn tool_error_to_chatml_gets_unified_failure_prefix() {
        // P92：失败结果统一【失败】前缀，LLM 一眼识别
        let msg = Message::tool_error("call_1", "craft", "背包没有 oak_planks");
        let chatml = msg.to_chatml();
        assert_eq!(chatml["role"], "tool");
        assert_eq!(
            chatml["content"], "【失败】背包没有 oak_planks",
            "is_error=true 应加统一失败前缀"
        );
    }

    #[test]
    fn tool_error_prefix_does_not_duplicate_on_preexisting_marker() {
        // 已带【失败】开头的内容不应二次叠加前缀（P89 占位文本兼容）
        let msg = Message::tool_error("call_1", "craft", "【失败】背包没有 oak_planks");
        let chatml = msg.to_chatml();
        assert_eq!(chatml["content"], "【失败】背包没有 oak_planks");
    }

    #[test]
    fn tool_result_with_images_strips_images_in_chatml() {
        // 决策 LLM 不应收到图，to_chatml 应只发文字。
        let msg = Message::tool_result_with_images(
            "call_2",
            "perceive",
            "[截图已附上，请直接看图]",
            vec!["data:image/png;base64,AAAA".to_string()],
        );
        let chatml = msg.to_chatml();
        assert_eq!(chatml["role"], "tool");
        assert_eq!(chatml["tool_call_id"], "call_2");
        assert_eq!(chatml["content"], "[截图已附上，请直接看图]");
    }

    #[test]
    fn tool_result_with_images_roundtrips_via_json() {
        // 持久化往返：images 随 JSON 写入/读出，旧 JSONL（无 images）也能补默认值。
        let msg = Message::tool_result_with_images(
            "call_2",
            "perceive",
            "x",
            vec!["data:image/png;base64,BBBB".to_string()],
        );
        let json = serde_json::to_string(&msg).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        match back {
            Message::ToolResult(r) => {
                assert_eq!(r.images, vec!["data:image/png;base64,BBBB".to_string()]);
            }
            _ => panic!("应反序列化为 ToolResult"),
        }
        // 旧格式（无 images 字段）反序列化 images 应为空
        let legacy =
            r#"{"role":"toolresult","tool_call_id":"c1","tool_name":"perceive","content":"old"}"#;
        let back: Message = serde_json::from_str(legacy).unwrap();
        match back {
            Message::ToolResult(r) => assert!(r.images.is_empty(), "旧 JSONL 应补空 images"),
            _ => panic!("应反序列化为 ToolResult"),
        }
    }

    #[test]
    fn assistant_reasoning_returns_separate_field() {
        // 推理链必须作为独立 reasoning_content 字段回传, 不能拼进 content。
        let msg = Message::assistant_with_reasoning("最终答案", "先思考一下");
        let chatml = msg.to_chatml();
        assert_eq!(chatml["role"], "assistant");
        assert_eq!(chatml["content"], "最终答案");
        assert_eq!(chatml["reasoning_content"], "先思考一下");
        // 旧实现会把 [思考] 拼进 content, 这里必须没有
        assert!(!chatml["content"].as_str().unwrap_or("").contains("[思考]"));
    }

    #[test]
    fn assistant_reasoning_with_tool_calls_returns_field() {
        // Agent(工具调用) 场景: reasoning_content 与 tool_calls 同在, 且 content 为 null。
        // 这正是 DeepSeek/MiMo/MiniMax 要求"必须回传 reasoning_content"的情形。
        let mut msg = Message::assistant_tool_call("call_1", "perceive", serde_json::json!({}));
        if let Message::Assistant(a) = &mut msg {
            a.reasoning = Some("推理过程".into());
        }
        let chatml = msg.to_chatml();
        assert_eq!(chatml["role"], "assistant");
        assert_eq!(chatml["reasoning_content"], "推理过程");
        assert_eq!(chatml["tool_calls"][0]["function"]["name"], "perceive");
        assert_eq!(chatml["content"], serde_json::Value::Null);
    }

    #[test]
    fn assistant_plain_text_has_no_reasoning_field() {
        // 无推理的普通回复不应出现 reasoning_content 字段。
        let msg = Message::assistant_text("你好");
        let chatml = msg.to_chatml();
        assert_eq!(chatml["content"], "你好");
        assert!(chatml.get("reasoning_content").is_none());
    }

    #[test]
    fn usage_cache_fields_default_to_zero_when_absent() {
        // 非 DeepSeek 兼容端点不返回 prompt_cache_hit_tokens 时，
        // 反序列化应补 0 而非报错（避免破坏旧 JSONL 回放）。
        let json = r#"{"input_tokens":10,"output_tokens":2,"total_tokens":12}"#;
        let u: Usage = serde_json::from_str(json).unwrap();
        assert_eq!(u.cache_hit_tokens, 0);
        assert_eq!(u.cache_miss_tokens, 0);
        // 带字段时正确解析
        let json2 = r#"{"input_tokens":10,"output_tokens":2,"total_tokens":12,"prompt_cache_hit_tokens":7,"prompt_cache_miss_tokens":5}"#;
        let u2: Usage = serde_json::from_str(json2).unwrap();
        assert_eq!(u2.cache_hit_tokens, 7);
        assert_eq!(u2.cache_miss_tokens, 5);
    }
}
