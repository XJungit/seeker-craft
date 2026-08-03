//! 感知工具：perceive / memory / search_wiki（P3.2 按域拆分，tools_azalea 域模块）。
use super::*;

/// 感知：读取结构化世界状态（坐标/背包/附近玩家），无需 VLM。
pub struct PerceiveTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl PerceiveTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for PerceiveTool {
    fn name(&self) -> &str {
        "perceive"
    }
    fn description(&self) -> &str {
        "读取当前世界结构化状态：坐标、维度、生命/饥饿、背包、附近方块/实体、激活传送门和服务端击杀统计。返回文本描述，供决策使用。无参数。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({})
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::read()
    }
    fn execute(
        &self,
        _call_id: &str,
        _args: Value,
        _on_update: Option<craft_agent::core::tool::ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let st = self.ctx.adapter.perceive_shared()?;
        Ok(ToolResult {
            message: st.self_hint.to_string(),
            is_error: false,
            images: vec![],
        })
    }
}

/// 世界记忆工具：让 LLM 显式记录/查询/遗忘空间记忆（资源点/结构/容器/锚点等）。
///
/// 动作：
/// - save: 在 (x,y,z) 记录一条记忆（kind 取 resource/structure/container/entity/hazard/portal/note；item 可选）
/// - anchor: 设置命名锚点（name, x, y, z, label）
/// - query: 查询（around 半径内邻近；或 by_item 按物品过滤；或 by_anchor 查锚点）
/// - forget: 按坐标遗忘；或 forget_anchor 按名称遗忘
pub struct MemoryTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl MemoryTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for MemoryTool {
    fn name(&self) -> &str {
        "memory"
    }
    fn description(&self) -> &str {
        "世界长期记忆：记录/查询/遗忘空间事实（资源点、结构、容器、村民、传送门、锚点）。\
         action=save 记录坐标记忆；action=anchor 设命名锚点；action=query 查询；action=forget 删除。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["save", "anchor", "query", "forget"] },
                "x": { "type": "integer" },
                "y": { "type": "integer" },
                "z": { "type": "integer" },
                "kind": { "type": "string", "enum": ["resource","structure","container","entity","hazard","portal","note"] },
                "item": { "type": "string", "description": "方块/物品 id，如 oak_log" },
                "label": { "type": "string" },
                "name": { "type": "string", "description": "锚点名称" },
                "radius": { "type": "integer", "description": "query 邻近半径，默认 64" },
                "by_item": { "type": "string", "description": "query 按物品过滤" },
                "by_anchor": { "type": "string", "description": "query 按锚点名查询" }
            },
            "required": ["action"]
        })
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _call_id: &str,
        args: Value,
        _on_update: Option<craft_agent::core::tool::ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let mem = &self.ctx.memory;
        let action = args["action"].as_str().unwrap_or("");
        let res = match action {
            "save" => {
                let pos = MemoryPos::new(
                    args["x"].as_i64().unwrap_or(0) as i32,
                    args["y"].as_i64().unwrap_or(0) as i32,
                    args["z"].as_i64().unwrap_or(0) as i32,
                );
                let kind = match args["kind"].as_str() {
                    Some("structure") => MemoryKind::Structure,
                    Some("container") => MemoryKind::Container,
                    Some("entity") => MemoryKind::Entity,
                    Some("hazard") => MemoryKind::Hazard,
                    Some("portal") => MemoryKind::Portal,
                    Some("note") => MemoryKind::Note,
                    _ => MemoryKind::Resource,
                };
                let label = args["label"].as_str().unwrap_or("记忆点");
                let item = args["item"].as_str();
                mem.record(pos, kind, item, label, None);
                format!(
                    "已记录记忆 @({},{},{}) kind={:?} label={}",
                    pos.x, pos.y, pos.z, kind, label
                )
            }
            "anchor" => {
                let pos = MemoryPos::new(
                    args["x"].as_i64().unwrap_or(0) as i32,
                    args["y"].as_i64().unwrap_or(0) as i32,
                    args["z"].as_i64().unwrap_or(0) as i32,
                );
                let name = args["name"].as_str().unwrap_or("anchor");
                let label = args["label"].as_str().unwrap_or(name);
                mem.set_anchor(name, Some(pos), label);
                format!("已设锚点 {name} @({},{},{})", pos.x, pos.y, pos.z)
            }
            "query" => {
                if let Some(an) = args["by_anchor"].as_str() {
                    return Ok(ToolResult {
                        message: match mem.find_anchor(an) {
                            Some(a) => format!("锚点 {an}: {} {:?}", a.label, a.pos),
                            None => format!("未找到锚点 {an}"),
                        },
                        is_error: false,
                        images: vec![],
                    });
                }
                if let Some(item) = args["by_item"].as_str() {
                    let v = mem.query(None, Some(item));
                    let s = if v.is_empty() {
                        format!("无 {item} 相关记忆")
                    } else {
                        v.iter()
                            .map(|c| format!("{} @({},{},{})", c.label, c.pos.x, c.pos.y, c.pos.z))
                            .collect::<Vec<_>>()
                            .join("\n")
                    };
                    return Ok(ToolResult {
                        message: s,
                        is_error: false,
                        images: vec![],
                    });
                }
                let around = mem
                    .find_anchor("__self__")
                    .and_then(|a| a.pos)
                    .unwrap_or(MemoryPos::new(0, 64, 0));
                let r = args["radius"].as_i64().unwrap_or(64) as i32;
                mem.render_nearby(around, r)
            }
            "forget" => {
                if let Some(an) = args["name"].as_str() {
                    mem.forget_anchor(an);
                    format!("已遗忘锚点 {an}")
                } else {
                    let pos = MemoryPos::new(
                        args["x"].as_i64().unwrap_or(0) as i32,
                        args["y"].as_i64().unwrap_or(0) as i32,
                        args["z"].as_i64().unwrap_or(0) as i32,
                    );
                    mem.forget_pos(pos);
                    format!("已遗忘坐标 ({},{},{})", pos.x, pos.y, pos.z)
                }
            }
            other => format!("memory 未知 action: {other}"),
        };
        Ok(ToolResult {
            message: res,
            is_error: false,
            images: vec![],
        })
    }
}

/// 搜索 Minecraft Wiki（中文源，国内可访问）。
/// 使用 Bilibili 游戏 Wiki（wiki.biligame.com/mc）的 MediaWiki 搜索 API。
#[allow(dead_code)]
pub struct SearchWikiTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl SearchWikiTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for SearchWikiTool {
    fn name(&self) -> &str {
        "search_wiki"
    }
    fn description(&self) -> &str {
        "搜索 Minecraft Wiki（中文），查询方块/物品/生物/机制等游戏知识。\
         参数 query 为搜索关键词（中文）。返回最多 3 条结果，含标题和摘要。\
         例: search_wiki(query=\"铁砧\") / search_wiki(query=\"how to make a pickaxe\")"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "搜索关键词" }
            },
            "required": ["query"]
        })
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::read()
    }
    fn execute(
        &self,
        _call_id: &str,
        args: Value,
        _on_update: Option<craft_agent::core::tool::ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 query"))?;
        let params = [
            ("action", "opensearch"),
            ("search", query),
            ("limit", "3"),
            ("format", "json"),
        ];
        let url = reqwest::Url::parse_with_params("https://wiki.biligame.com/mc/api.php", &params)
            .map_err(|e| anyhow::anyhow!("URL error: {e}"))?;
        let resp = reqwest::blocking::get(url)?.text()?;
        let json: serde_json::Value = serde_json::from_str(&resp)?;
        let results = json
            .as_array()
            .and_then(|arr| arr.get(1))
            .and_then(|v| v.as_array());
        let urls = json
            .as_array()
            .and_then(|arr| arr.get(3))
            .and_then(|v| v.as_array());
        match results {
            Some(items) if !items.is_empty() => {
                let mut lines: Vec<String> = Vec::new();
                for (i, item) in items.iter().enumerate() {
                    let title = item.as_str().unwrap_or("?");
                    let link = urls
                        .and_then(|u| u.get(i))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    lines.push(format!("{}. {} ({})", i + 1, title, link));
                }
                Ok(ToolResult {
                    message: format!("Wiki 搜索结果 ({}):\n{}", query, lines.join("\n")),
                    is_error: false,
                    images: vec![],
                })
            }
            _ => Ok(ToolResult {
                message: format!("Wiki 搜索无结果: {query}"),
                is_error: false,
                images: vec![],
            }),
        }
    }
}
