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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
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
        })
    }

    pub fn assistant_text(content: impl Into<String>) -> Self {
        Self::Assistant(AssistantMsg {
            content: Some(content.into()),
            reasoning: None,
            tool_calls: vec![],
        })
    }

    pub fn assistant_tool_call(id: impl Into<String>, name: impl Into<String>, args: Value) -> Self {
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

    /// 一条 assistant 消息携带**多个** tool_calls (pi: 一轮可返回多个 tool_call)
    /// id 自动编号为 `{id_prefix}_{i}`, 与后续 tool_result 的 id 对应。
    pub fn assistant_tool_calls(
        id_prefix: &str,
        calls: &[(String, String)],
        reasoning: Option<String>,
    ) -> Self {
        Self::Assistant(AssistantMsg {
            content: None,
            reasoning,
            tool_calls: calls
                .iter()
                .enumerate()
                .map(|(i, (n, a))| ToolCall {
                    id: format!("{id_prefix}_{i}"),
                    name: n.clone(),
                    arguments: serde_json::from_str(a).unwrap_or(Value::Null),
                })
                .collect(),
        })
    }

    pub fn assistant_with_reasoning(content: impl Into<String>, reasoning: impl Into<String>) -> Self {
        Self::Assistant(AssistantMsg {
            content: Some(content.into()),
            reasoning: Some(reasoning.into()),
            tool_calls: vec![],
        })
    }

    pub fn tool_result(id: impl Into<String>, name: impl Into<String>, content: impl Into<String>) -> Self {
        Self::ToolResult(ToolResultMsg {
            tool_call_id: id.into(),
            tool_name: name.into(),
            content: content.into(),
            is_error: false,
        })
    }

    pub fn tool_error(id: impl Into<String>, name: impl Into<String>, error: impl Into<String>) -> Self {
        Self::ToolResult(ToolResultMsg {
            tool_call_id: id.into(),
            tool_name: name.into(),
            content: error.into(),
            is_error: true,
        })
    }
}

// ── 转换为 OpenAI ChatML 格式 (发送给 LLM) ──

impl Message {
    /// 转换为 OpenAI Chat Completions 的 message 格式
    pub fn to_chatml(&self) -> Value {
        match self {
            Self::User(m) => serde_json::json!({
                "role": "user",
                "content": m.content
            }),
            Self::Assistant(m) => {
                let text = if let Some(r) = &m.reasoning {
                    Some(format!("[思考] {r}\n{}", m.content.as_deref().unwrap_or("")))
                } else {
                    m.content.clone()
                };
                if m.tool_calls.is_empty() {
                    serde_json::json!({
                        "role": "assistant",
                        "content": text.unwrap_or_default()
                    })
                } else {
                    serde_json::json!({
                        "role": "assistant",
                        "content": text,
                        "tool_calls": m.tool_calls.iter().map(|tc| serde_json::json!({
                            "id": tc.id,
                            "type": "function",
                            "function": {
                                "name": tc.name,
                                "arguments": tc.arguments.to_string()
                            }
                        })).collect::<Vec<_>>()
                    })
                }
            }
            Self::ToolResult(m) => serde_json::json!({
                "role": "tool",
                "tool_call_id": m.tool_call_id,
                "content": m.content
            }),
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
}
