//! Schema builder + structured arg parsing (借鉴 Numen Schema + Record-args 模式)
//!
//! # Schema builder
//! ```
//! use craft_agent_minecraft::tool_args::schema;
//! fn params() -> serde_json::Value {
//!     schema::object()
//!         .str_req("target", "Block ID substring")
//!         .int_opt("count", "Number to collect (1-64)", 1, 1, 64)
//!         .finish()
//! }
//! ```

use serde_json::{Map, Value, json};

// ═══════════════════════════════════════════════════════════════
// Schema builder
// ═══════════════════════════════════════════════════════════════

pub struct ObjectSchema {
    props: Vec<(&'static str, Value)>,
    required: Vec<&'static str>,
}

pub fn object() -> ObjectSchema {
    ObjectSchema {
        props: vec![],
        required: vec![],
    }
}

impl ObjectSchema {
    fn add(mut self, name: &'static str, v: Value, req: bool) -> Self {
        self.props.push((name, v));
        if req {
            self.required.push(name);
        }
        self
    }

    pub fn str_req(self, name: &'static str, desc: &'static str) -> Self {
        self.add(name, json!({"type":"string","description":desc}), true)
    }

    pub fn str_opt(self, name: &'static str, desc: &'static str, default: &'static str) -> Self {
        self.add(
            name,
            json!({"type":"string","description":desc,"default":default}),
            false,
        )
    }

    pub fn int_req(self, name: &'static str, desc: &'static str, min: i64, max: i64) -> Self {
        self.add(
            name,
            json!({"type":"integer","description":desc,"minimum":min,"maximum":max}),
            true,
        )
    }

    pub fn int_opt(
        self,
        name: &'static str,
        desc: &'static str,
        default: i64,
        min: i64,
        max: i64,
    ) -> Self {
        self.add(name, json!({"type":"integer","description":desc,"default":default,"minimum":min,"maximum":max}), false)
    }

    pub fn num_req(self, name: &'static str, desc: &'static str) -> Self {
        self.add(name, json!({"type":"number","description":desc}), true)
    }

    pub fn num_opt(self, name: &'static str, desc: &'static str, default: f64) -> Self {
        self.add(
            name,
            json!({"type":"number","description":desc,"default":default}),
            false,
        )
    }

    pub fn bool_opt(self, name: &'static str, desc: &'static str, default: bool) -> Self {
        self.add(
            name,
            json!({"type":"boolean","description":desc,"default":default}),
            false,
        )
    }

    pub fn finish(self) -> Value {
        let props: Map<String, Value> = self
            .props
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        json!({
            "type": "object",
            "properties": props,
            "required": self.required,
        })
    }
}

/// 空参数 schema（无参工具用）
pub fn no_args() -> Value {
    json!({"type":"object","properties":{},"required":[]})
}

/// 工具执行结果构建辅助（借鉴 Numen 结构化错误反馈模式）
pub fn ok_msg(msg: impl Into<String>) -> Value {
    json!({"message": msg.into(), "is_error": false, "images": []})
}

pub fn err_msg(msg: impl Into<String>) -> Value {
    json!({"message": msg.into(), "is_error": true, "images": []})
}

/// 解析参数为指定类型（需实现 serde::Deserialize）。
/// 使用方式：
/// ```ignore
/// #[derive(serde::Deserialize)]
/// struct MyArgs { target: String, count: Option<u32> }
/// let a: MyArgs = tool_args::parse(args)?;
/// ```
pub fn parse<T: serde::de::DeserializeOwned>(args: Value) -> anyhow::Result<T> {
    serde_json::from_value::<T>(args).map_err(|e| anyhow::anyhow!("{}", e))
}

/// Re-export for `use tool_args::schema; schema::object()...` convenience.
pub mod schema {
    pub use super::{ObjectSchema, no_args, object};
}
