//! 放置工具：place / build / build_blueprint / list_blueprints（P3.2 按域拆分，tools_azalea 域模块）。
use super::*;

/// 放置方块（把手持物品放到坐标旁）。
pub struct PlaceTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl PlaceTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for PlaceTool {
    fn name(&self) -> &str {
        "place"
    }
    fn description(&self) -> &str {
        "把手持物品 item 放置到世界坐标 (x,y,z) 旁（右键放置）。\n\
         需背包持有该物品；常用于放置工作台/熔炉以便后续 craft_3x3 / smelt。\n\
         item 为目标物品 id（如 \"crafting_table\"），坐标用整数。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "item": { "type": "string", "description": "物品 id，如 crafting_table / furnace" },
                "x": { "type": "integer" },
                "y": { "type": "integer" },
                "z": { "type": "integer" }
            },
            "required": ["item", "x", "y", "z"]
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
        let item = args
            .get("item")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 item"))?
            .to_string();
        let x = args
            .get("x")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow::anyhow!("缺少 x"))? as i32;
        let y = args
            .get("y")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow::anyhow!("缺少 y"))? as i32;
        let z = args
            .get("z")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow::anyhow!("缺少 z"))? as i32;
        let r = self
            .ctx
            .adapter
            .execute_shared(Action::Minecraft(MinecraftAction::Place {
                item: item.clone(),
                x,
                y,
                z,
            }))?;
        // 行动回写：仅当放置成功时才回写世界记忆。
        // P5 修复：原代码无条件 record，导致 do_place 失败后 LLM 仍能在记忆中看到
        // 「已放置 crafting_table」，下一轮 perceive 又因实际方块不存在而遗忘——
        // 这种"先记后忘"会让 LLM 困惑（记忆与感知矛盾）。
        if r.ok {
            let pos = MemoryPos::new(x, y, z);
            let kind = match item.as_str() {
                "chest" | "barrel" | "shulker_box" => MemoryKind::Container,
                "lava" | "water" | "fire" => MemoryKind::Hazard,
                "nether_portal" | "end_portal" => MemoryKind::Portal,
                _ => MemoryKind::Structure,
            };
            self.ctx
                .memory
                .record(pos, kind, Some(&item), &item.clone(), None);
        }
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 执行蓝图建造：按 JSON 描述的方块列表依次放置。
/// 格式: {"blocks":[{"x":10,"y":64,"z":20,"block":"oak_planks"}, ...]}
/// 自动检查背包是否有材料，缺材料时报错。每步先 goto 到目标位置再 place。
pub struct BuildTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl BuildTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for BuildTool {
    fn name(&self) -> &str {
        "build"
    }
    fn description(&self) -> &str {
        "按蓝图建造：JSON 格式 {\"blocks\":[{\"x\":10,\"y\":64,\"z\":20,\"block\":\"oak_planks\"}, ...]}。\
         自动 goto 到每个位置再 place。材料不足时报错。\
         例: build(blueprint=\"{\\\"blocks\\\":[{\\\"x\\\":10,\\\"y\\\":64,\\\"z\\\":20,\\\"block\\\":\\\"oak_planks\\\"}]}\")"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "blueprint": { "type": "string", "description": "JSON 蓝图，格式 {\"blocks\":[{\"x\":int,\"y\":int,\"z\":int,\"block\":\"id\"}]}" }
            },
            "required": ["blueprint"]
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
        let bp_str = args
            .get("blueprint")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 blueprint"))?;
        let bp: serde_json::Value =
            serde_json::from_str(bp_str).map_err(|e| anyhow::anyhow!("JSON 解析失败: {e}"))?;
        let blocks = bp
            .get("blocks")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("缺少 blocks 数组"))?;
        let adapter = self.ctx.adapter.0.clone();
        let mut results: Vec<String> = Vec::new();
        for (i, block) in blocks.iter().enumerate() {
            let x = block
                .get("x")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow::anyhow!("第{}个方块缺少 x", i + 1))?
                as i32;
            let y = block
                .get("y")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow::anyhow!("第{}个方块缺少 y", i + 1))?
                as i32;
            let z = block
                .get("z")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow::anyhow!("第{}个方块缺少 z", i + 1))?
                as i32;
            let block_id = block
                .get("block")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("第{}个方块缺少 block", i + 1))?;
            // 先 goto 到目标位置
            let goto_result = _exec_action(&adapter, MinecraftAction::Goto { x, y, z });
            if goto_result.starts_with("错误") {
                results.push(format!("第{}个 (goto) 失败: {goto_result}", i + 1));
                break;
            }
            // 放置方块
            let place_result = _exec_action(
                &adapter,
                MinecraftAction::Place {
                    item: block_id.to_string(),
                    x,
                    y,
                    z,
                },
            );
            if place_result.starts_with("错误") {
                results.push(format!(
                    "第{}个 (place {block_id}) 失败: {place_result}",
                    i + 1
                ));
                break;
            }
            results.push(format!("第{}个: placed {block_id} @({x},{y},{z})", i + 1));
        }
        Ok(ToolResult {
            message: results.join("\n"),
            is_error: false,
            images: vec![],
        })
    }
}

/// 按预定义蓝图名称建造（P2-1）。
/// 蓝图存放在 `blueprints/` 目录，bot 调用 `build_blueprint(name, x, y, z)` 即可
/// 在原点 (x,y,z) 实例化蓝图（相对坐标→绝对坐标自动展开）。
pub struct BuildBlueprintTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl BuildBlueprintTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for BuildBlueprintTool {
    fn name(&self) -> &str {
        "build_blueprint"
    }
    fn description(&self) -> &str {
        "按预定义蓝图名称建造（P2-1）。蓝图文件在 `blueprints/` 目录，\
         内置：small_shelter（3x3 木屋）/ farm_plot（9x9 农田）/ storage_corner（储物角）/ torch_pillar（标记柱）。\n\
         bot 自动：1) 查蓝图 → 2) 计算材料清单 → 3) 逐方块 goto+place。\n\
         参数：name 蓝图名，x/y/z 蓝图原点（相对坐标 dx/dy/dz 加上原点 = 实际世界坐标）。\n\
         示例：build_blueprint(name=\"torch_pillar\", x=100, y=64, z=-50) 在该坐标立一根火把柱。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "蓝图名（如 small_shelter / farm_plot / storage_corner / torch_pillar）" },
                "x": { "type": "integer", "description": "蓝图原点 X 坐标（dx=0 的实际位置）" },
                "y": { "type": "integer", "description": "蓝图原点 Y 坐标" },
                "z": { "type": "integer", "description": "蓝图原点 Z 坐标" }
            },
            "required": ["name", "x", "y", "z"]
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
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 name"))?
            .to_string();
        let x = args
            .get("x")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow::anyhow!("缺少 x"))? as i32;
        let y = args
            .get("y")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow::anyhow!("缺少 y"))? as i32;
        let z = args
            .get("z")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow::anyhow!("缺少 z"))? as i32;

        let bp = self
            .ctx
            .blueprints
            .get(&name)
            .ok_or_else(|| {
                let avail = self.ctx.blueprints.list_summary();
                anyhow::anyhow!("未知蓝图 '{name}'。可用蓝图：\n{avail}")
            })?
            .clone();

        // 先把材料清单回给 LLM（让它决定是否先采集）
        let materials = bp.material_summary();
        let bounds = bp.bounds();
        let abs_json = bp.instantiate(x, y, z);

        // 复用 BuildTool 的执行逻辑：把蓝图实例化的 JSON 当作普通 blueprint 参数执行
        let adapter = self.ctx.adapter.0.clone();
        let bp_value: serde_json::Value = serde_json::from_str(&abs_json)
            .map_err(|e| anyhow::anyhow!("蓝图实例化 JSON 解析失败: {e}"))?;
        let blocks = bp_value
            .get("blocks")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("蓝图实例化后无 blocks 数组"))?;

        let mut results: Vec<String> = Vec::new();
        results.push(format!(
            "蓝图 '{name}' @({x},{y},{z}) 边界 dx{}..{} dy{}..{} dz{}..{} | 材料: {materials}",
            bounds.0, bounds.3, bounds.1, bounds.4, bounds.2, bounds.5
        ));

        for (i, block) in blocks.iter().enumerate() {
            let bx = block.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let by = block.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let bz = block.get("z").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let block_id = block.get("block").and_then(|v| v.as_str()).unwrap_or("");
            let goto_result = _exec_action(
                &adapter,
                MinecraftAction::Goto {
                    x: bx,
                    y: by,
                    z: bz,
                },
            );
            if goto_result.starts_with("错误") {
                results.push(format!("第{}个 (goto) 失败: {goto_result}", i + 1));
                break;
            }
            let place_result = _exec_action(
                &adapter,
                MinecraftAction::Place {
                    item: block_id.to_string(),
                    x: bx,
                    y: by,
                    z: bz,
                },
            );
            if place_result.starts_with("错误") {
                results.push(format!(
                    "第{}个 (place {block_id}) 失败: {place_result}",
                    i + 1
                ));
                break;
            }
            results.push(format!(
                "第{}个: placed {block_id} @({bx},{by},{bz})",
                i + 1
            ));
        }

        Ok(ToolResult {
            message: results.join("\n"),
            is_error: false,
            images: vec![],
        })
    }
}

/// 列出所有可用蓝图（P2-1）。
pub struct ListBlueprintsTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl ListBlueprintsTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for ListBlueprintsTool {
    fn name(&self) -> &str {
        "list_blueprints"
    }
    fn description(&self) -> &str {
        "列出所有可用蓝图名 + 描述 + 材料清单（P2-1）。无参数。\n\
         返回示例：\n\
         - small_shelter: 3x3 木屋 | 材料: oak_planks:5, oak_log:4, ...\n\
         - torch_pillar: 标记柱 | 材料: cobblestone:3, torch:1\n\n\
         用 build_blueprint(name=..., x=..., y=..., z=...) 实例化。"
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
        let items = self.ctx.blueprints.list();
        if items.is_empty() {
            return Ok(ToolResult {
                message: "无可用蓝图（blueprints/ 目录为空或未加载）".to_string(),
                is_error: false,
                images: vec![],
            });
        }
        let mut lines: Vec<String> = Vec::new();
        for (name, desc) in items {
            let materials = self
                .ctx
                .blueprints
                .get(&name)
                .map(|bp| bp.material_summary())
                .unwrap_or_default();
            lines.push(format!("- {name}: {desc} | 材料: {materials}"));
        }
        Ok(ToolResult {
            message: format!("可用蓝图 {} 个：\n{}", lines.len(), lines.join("\n")),
            is_error: false,
            images: vec![],
        })
    }
}
