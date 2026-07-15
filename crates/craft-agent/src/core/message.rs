//! 对话消息类型 — pi 风格的类型化消息枚举
//!
//! 参考 pi_agent_rust src/model.rs:
//! - Message enum (User/Assistant/ToolResult/System)
//! - ToolCall struct (id + name + arguments)
//! - Usage tracking (input/output tokens)
//!
//! 与 pi 的差异: pi 的 System 消息不单独存储, 我们用 system prompt 单独管理。

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
}

// ── 构造器 (pi: Message::assistant / Message::tool_result 风格) ──

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self::User(UserMsg {
            content: content.into(),
            images: vec![],
        })
    }

    /// 构造带图像段的用户消息（多模态 LLM 直读场景）。
    /// `images` 为 base64 data URI 列表，非空时 `to_chatml` 输出图像内容段。
    pub fn user_with_images(content: impl Into<String>, images: Vec<String>) -> Self {
        Self::User(UserMsg {
            content: content.into(),
            images,
        })
    }

    pub fn assistant_text(content: impl Into<String>) -> Self {
        Self::Assistant(AssistantMsg {
            content: Some(content.into()),
            reasoning: None,
            tool_calls: vec![],
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
        })
    }

    /// 从 provider 响应构造 assistant 消息，保留原始 tool-call ID。
    pub fn assistant_response(response: &AssistantResponse) -> Self {
        Self::Assistant(AssistantMsg {
            content: response.content.clone(),
            reasoning: response.reasoning.clone(),
            tool_calls: response.tool_calls.clone(),
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
                let content = if m.images.is_empty() {
                    serde_json::json!(m.content)
                } else {
                    let mut parts = vec![serde_json::json!({
                        "type": "text",
                        "text": m.content
                    })];
                    for img in &m.images {
                        parts.push(serde_json::json!({
                            "type": "image_url",
                            "image_url": { "url": img }
                        }));
                    }
                    serde_json::Value::Array(parts)
                };
                serde_json::json!({
                    "role": "user",
                    "content": content
                })
            }
            Self::Assistant(m) => {
                // 推理链作为独立字段 reasoning_content 回传 (KEEP 策略)。
                // DeepSeek/MiMo/MiniMax 在 Agent(工具调用) 场景要求回传 reasoning_content,
                // 否则 400; 纯多轮场景忽略或保留均安全。因此不再把推理拼进 content。
                let mut obj = serde_json::Map::new();
                obj.insert("role".to_string(), serde_json::json!("assistant"));
                obj.insert(
                    "content".to_string(),
                    match &m.content {
                        Some(c) => serde_json::json!(c),
                        None => serde_json::Value::Null,
                    },
                );
                if let Some(r) = &m.reasoning {
                    obj.insert("reasoning_content".to_string(), serde_json::json!(r));
                }
                if !m.tool_calls.is_empty() {
                    obj.insert(
                        "tool_calls".to_string(),
                        serde_json::json!(
                            m.tool_calls
                                .iter()
                                .map(|tc| serde_json::json!({
                                    "id": tc.id,
                                    "type": "function",
                                    "function": {
                                        "name": tc.name,
                                        "arguments": tc.arguments.to_string()
                                    }
                                }))
                                .collect::<Vec<_>>()
                        ),
                    );
                }
                serde_json::Value::Object(obj)
            }
            Self::ToolResult(m) => {
                let content = if m.images.is_empty() {
                    serde_json::json!(m.content)
                } else {
                    // 多模态直读：content 必须是数组，首段文本 + 后续图像段。
                    // 这是 OpenAI Chat Completions 工具结果支持的 image_url 格式。
                    let mut parts = vec![serde_json::json!({
                        "type": "text",
                        "text": m.content
                    })];
                    for img in &m.images {
                        parts.push(serde_json::json!({
                            "type": "image_url",
                            "image_url": { "url": img }
                        }));
                    }
                    serde_json::Value::Array(parts)
                };
                serde_json::json!({
                    "role": "tool",
                    "tool_call_id": m.tool_call_id,
                    "content": content
                })
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
    fn user_with_images_emits_image_url_segments() {
        // 多模态 user 消息：content 应为数组，首段文本 + 一个 image_url 段。
        let msg = Message::user_with_images(
            "请直接看图回答",
            vec!["data:image/png;base64,CCCC".to_string()],
        );
        let chatml = msg.to_chatml();
        assert_eq!(chatml["role"], "user");
        let content = chatml["content"].as_array().expect("content 应为数组");
        assert_eq!(content.len(), 2, "文本段 + 图像段");
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "请直接看图回答");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,CCCC");
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
    fn tool_result_with_images_emits_image_url_segments() {
        // 多模态直读：content 应为数组，首段文本 + 一个 image_url 段。
        let msg = Message::tool_result_with_images(
            "call_2",
            "perceive",
            "[截图已附上，请直接看图]",
            vec!["data:image/png;base64,AAAA".to_string()],
        );
        let chatml = msg.to_chatml();
        assert_eq!(chatml["role"], "tool");
        assert_eq!(chatml["tool_call_id"], "call_2");
        let content = chatml["content"].as_array().expect("content 应为数组");
        assert_eq!(content.len(), 2, "文本段 + 图像段");
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "[截图已附上，请直接看图]");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,AAAA");
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
}
