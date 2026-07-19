//! 建造 / 蓝图 / 合成计划 / 维度传送工具。

use crate::blueprint;
use crate::tool_args::schema;
use crate::tools_mod::MinecraftModAdapter;
use crate::tools_mod::SafeLockAdapter;
use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult, ToolUpdateFn};
use serde_json::Value;
use std::sync::Arc;
use std::sync::Mutex;

pub struct ModPlaceTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModPlaceTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModPlaceTool {
    fn name(&self) -> &str {
        "place"
    }
    fn description(&self) -> &str {
        "Place a block from inventory at an exact coordinate. Auto-finds item in inventory, equips it, places at the coordinate. For multi-block structures use build(). Usage: place(item=\"crafting_table\", x=10, y=64, z=20)  place(item=\"torch\", x=11, y=65, z=20)"
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req(
                "item",
                "Block to place: crafting_table, torch, dirt, oak_planks, furnace, chest, etc.",
            )
            .int_req("x", "World X coordinate", -30000000, 30000000)
            .int_req("y", "World Y coordinate", -64, 320)
            .int_req("z", "World Z coordinate", -30000000, 30000000)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let item = args["item"].as_str().unwrap_or("dirt");
        let x = args["x"].as_i64().unwrap_or(0) as i32;
        let y = args["y"].as_i64().unwrap_or(0) as i32;
        let z = args["z"].as_i64().unwrap_or(0) as i32;
        let full_item = if item.contains(':') {
            item.to_string()
        } else {
            format!("minecraft:{item}")
        };
        let adapter = self.adapter.lock_adapter()?;
        // 用 place_at 精确放置（mod 侧自动找物品+切栏位+useItemOn）
        let ack = adapter.place_at(x, y, z, &full_item)?;
        let placed = ack.placed == Some(true);
        let msg = if placed {
            format!("placed {item} at ({x},{y},{z})")
        } else {
            // 借鉴 Numen 失败类型区分：用现有数据判断失败原因，给 LLM 精准恢复建议
            match adapter.reload() {
                Ok(st) => {
                    let has_item = st.inventory.iter().any(|i| i.id.contains(item));
                    let px = st.position[0];
                    let py = st.position[1];
                    let pz = st.position[2];
                    let dist = ((x as f64 - px).powi(2)
                        + (y as f64 - py).powi(2)
                        + (z as f64 - pz).powi(2))
                    .sqrt();
                    if !has_item {
                        format!(
                            "place FAILED at ({x},{y},{z}): no {item} in inventory — collect/craft it first"
                        )
                    } else if dist > 5.5 {
                        format!(
                            "place FAILED at ({x},{y},{z}): too far ({:.1}m, max 5.5m) — move_to closer first",
                            dist
                        )
                    } else {
                        format!(
                            "place FAILED at ({x},{y},{z}): no valid surface or line-of-sight blocked — try adjacent block or move to different angle"
                        )
                    }
                }
                Err(_) => format!(
                    "place FAILED at ({x},{y},{z}): could not place {item} (state reload failed)"
                ),
            }
        };
        Ok(ToolResult {
            message: msg,
            is_error: !placed,
            images: vec![],
        })
    }
}

pub struct ModBuildTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModBuildTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModBuildTool {
    fn name(&self) -> &str {
        "build"
    }
    fn description(&self) -> &str {
        "Build a structure from a named blueprint at given coordinates. Auto-generates build steps layer by layer (y→z→x), calls place_at for each block. Stops on missing materials. blueprint: name from blueprints() list (dirt_shelter/wood_house/stone_house/wall_3x3). x,y,z: world coords for blueprint origin (corner). orientation: 0-3 (0=north, 1=east, 2=south, 3=west). Returns blocks placed/failed."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req(
                "blueprint",
                "Blueprint name: dirt_shelter, wood_house, stone_house, wall_3x3",
            )
            .int_req(
                "x",
                "World X coordinate for blueprint origin (corner)",
                -30000000,
                30000000,
            )
            .int_req(
                "y",
                "World Y coordinate for blueprint origin (ground level)",
                -64,
                320,
            )
            .int_req(
                "z",
                "World Z coordinate for blueprint origin (corner)",
                -30000000,
                30000000,
            )
            .int_opt(
                "orientation",
                "Rotation 0-3 (0=north,1=east,2=south,3=west)",
                0,
                0,
                3,
            )
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let bp_name = args["blueprint"].as_str().unwrap_or("dirt_shelter");
        let ox = args["x"].as_i64().unwrap_or(0) as i32;
        let oy = args["y"].as_i64().unwrap_or(0) as i32;
        let oz = args["z"].as_i64().unwrap_or(0) as i32;
        let orientation = args["orientation"].as_u64().unwrap_or(0) as u32 % 4;

        let Some(bp) = blueprint::get_blueprint(bp_name) else {
            return Ok(ToolResult {
                message: format!(
                    "build: blueprint '{bp_name}' not found. Use blueprints() to list available."
                ),
                is_error: true,
                images: vec![],
            });
        };

        // 先检查材料
        let needed = bp.materials_needed();
        let st = self.adapter.lock_adapter()?.reload()?;
        let mut missing = Vec::new();
        for (mat, need) in &needed {
            let have: u32 = st
                .inventory
                .iter()
                .filter(|i| i.id.contains(mat))
                .map(|i| i.count)
                .sum();
            if have < *need {
                missing.push(format!("{mat}: need {need}, have {have}"));
            }
        }
        if !missing.is_empty() {
            return Ok(ToolResult {
                message: format!("build {bp_name}: missing materials: {}", missing.join("; ")),
                is_error: true,
                images: vec![],
            });
        }

        // 生成建造步骤并逐步执行
        let steps = bp.build_steps(ox, oy, oz, orientation);
        let mut placed = 0u32;
        let mut failed = 0u32;
        let mut consecutive_fail = 0u32;
        let mut first_fail_reason: Option<String> = None;
        for step in &steps {
            match &step.action {
                blueprint::BuildAction::Place(item) => {
                    // 物品名补全 minecraft: 前缀（mod 侧匹配）
                    let full_item = if item.contains(':') {
                        item.clone()
                    } else {
                        format!("minecraft:{item}")
                    };
                    match self
                        .adapter
                        .lock_adapter()?
                        .place_at(step.x, step.y, step.z, &full_item)
                    {
                        Ok(ack) if ack.placed == Some(true) => {
                            placed += 1;
                            consecutive_fail = 0;
                        }
                        _ => {
                            failed += 1;
                            consecutive_fail += 1;
                            // 借鉴 Numen PlaceBlock 失败诊断：记录首个失败原因
                            if first_fail_reason.is_none() {
                                let adapter = self.adapter.lock_adapter()?;
                                if let Ok(st) = adapter.reload() {
                                    let has_item = st.inventory.iter().any(|i| i.id.contains(item));
                                    let px = st.position[0];
                                    let py = st.position[1];
                                    let pz = st.position[2];
                                    let dist = ((step.x as f64 - px).powi(2)
                                        + (step.y as f64 - py).powi(2)
                                        + (step.z as f64 - pz).powi(2))
                                    .sqrt();
                                    first_fail_reason = Some(if !has_item {
                                        format!(
                                            "no {item} in inventory at ({},{},{})",
                                            step.x, step.y, step.z
                                        )
                                    } else if dist > 5.5 {
                                        format!(
                                            "too far ({:.1}m) at ({},{},{})",
                                            dist, step.x, step.y, step.z
                                        )
                                    } else {
                                        format!("no surface at ({},{},{})", step.x, step.y, step.z)
                                    });
                                }
                            }
                            if consecutive_fail >= 5 {
                                break;
                            } // 连续失败 5 次停止
                        }
                    }
                }
                blueprint::BuildAction::Dig => {
                    match self.adapter.lock_adapter()?.dig_at(step.x, step.y, step.z) {
                        Ok(ack) if ack.broken == Some(true) => {
                            placed += 1;
                            consecutive_fail = 0;
                        }
                        _ => {
                            failed += 1;
                            consecutive_fail += 1;
                            if first_fail_reason.is_none() {
                                first_fail_reason = Some(format!(
                                    "dig failed at ({},{},{})",
                                    step.x, step.y, step.z
                                ));
                            }
                            if consecutive_fail >= 5 {
                                break;
                            }
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(150));
        }

        let msg = if failed == 0 {
            format!(
                "build {bp_name} at ({ox},{oy},{oz}) orient={orientation}: placed {placed} blocks (complete)"
            )
        } else if placed > 0 {
            format!(
                "build {bp_name} at ({ox},{oy},{oz}) orient={orientation}: placed {placed}, failed {failed} (partial) — first failure: {}",
                first_fail_reason.unwrap_or_else(|| "unknown".into())
            )
        } else {
            format!(
                "build {bp_name} at ({ox},{oy},{oz}) orient={orientation}: all {failed} blocks failed — first failure: {}",
                first_fail_reason.unwrap_or_else(|| "unknown".into())
            )
        };
        Ok(ToolResult {
            message: msg,
            is_error: placed == 0,
            images: vec![],
        })
    }
}

pub struct ModBlueprintsTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModBlueprintsTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModBlueprintsTool {
    fn name(&self) -> &str {
        "blueprints"
    }
    fn description(&self) -> &str {
        "List all available blueprints with their materials requirements. Returns name, size (layers×rows×cols), and materials needed. Use before build() to verify you have enough materials. Built-in: dirt_shelter (3x3), wood_house (5x5), stone_house (5x5), wall_3x3."
    }
    fn parameters(&self) -> Value {
        schema::no_args()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::read()
    }
    fn execute(
        &self,
        _id: &str,
        _args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let bps = blueprint::builtin_blueprints();
        let st = self.adapter.lock_adapter()?.reload()?;
        let mut lines = Vec::new();
        for (name, json) in bps {
            if let Ok(bp) = blueprint::BlueprintDef::from_json(json) {
                let layers = bp.blocks.len();
                let rows = bp.blocks.first().map(|l| l.len()).unwrap_or(0);
                let cols = bp
                    .blocks
                    .first()
                    .and_then(|l| l.first())
                    .map(|r| r.len())
                    .unwrap_or(0);
                let mats = bp.materials_needed();
                let mat_str: Vec<String> = mats
                    .iter()
                    .map(|(m, n)| {
                        let have: u32 = st
                            .inventory
                            .iter()
                            .filter(|i| i.id.contains(m))
                            .map(|i| i.count)
                            .sum();
                        let status = if have >= *n { "ok" } else { "MISSING" };
                        format!("{m}×{n}({status})")
                    })
                    .collect();
                lines.push(format!(
                    "  {name}: {layers}×{rows}×{cols} | materials: {}",
                    mat_str.join(", ")
                ));
            }
        }
        Ok(ToolResult {
            message: format!("BLUEPRINTS:\n{}", lines.join("\n")),
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModCraftingPlanTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModCraftingPlanTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModCraftingPlanTool {
    fn name(&self) -> &str {
        "getCraftingPlan"
    }
    fn description(&self) -> &str {
        "Get detailed crafting plan for an item: required materials, exact quantities, and missing materials analysis. targetItem: item to craft. quantity: how many to craft. Returns materials list (have/need/missing) and craftable count."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req(
                "targetItem",
                "Item to craft, e.g. wooden_pickaxe, torch, chest",
            )
            .int_opt("quantity", "How many to craft", 1, 1, 64)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::read()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let target = args["targetItem"].as_str().unwrap_or("oak_planks");
        let qty = args["quantity"].as_u64().unwrap_or(1) as u32;
        let st = self.adapter.lock_adapter()?.reload()?;
        type RecipeEntry<'a> = (&'a str, &'a [(&'a str, u32)], u32);
        const RECIPES: &[RecipeEntry] = &[
            ("oak_planks", &[("oak_log", 1)], 4),
            ("birch_planks", &[("birch_log", 1)], 4),
            ("stick", &[("planks", 2)], 4),
            ("crafting_table", &[("planks", 4)], 1),
            ("wooden_pickaxe", &[("planks", 3), ("stick", 2)], 1),
            ("wooden_axe", &[("planks", 3), ("stick", 2)], 1),
            ("wooden_sword", &[("planks", 2), ("stick", 1)], 1),
            ("wooden_shovel", &[("planks", 1), ("stick", 2)], 1),
            ("stone_pickaxe", &[("cobblestone", 3), ("stick", 2)], 1),
            ("stone_axe", &[("cobblestone", 3), ("stick", 2)], 1),
            ("stone_sword", &[("cobblestone", 2), ("stick", 1)], 1),
            ("iron_pickaxe", &[("iron_ingot", 3), ("stick", 2)], 1),
            ("iron_sword", &[("iron_ingot", 2), ("stick", 1)], 1),
            ("torch", &[("stick", 1), ("coal", 1)], 4),
            ("furnace", &[("cobblestone", 8)], 1),
            ("chest", &[("planks", 8)], 1),
            ("oak_door", &[("planks", 6)], 1),
        ];
        let recipe = RECIPES.iter().find(|(name, _, _)| *name == target);
        match recipe {
            Some((_, ingredients, yield_count)) => {
                let mut lines = Vec::new();
                let mut max_crafts = u32::MAX;
                let mut all_ok = true;
                for (mat, need_per) in ingredients.iter() {
                    let have: u32 = st
                        .inventory
                        .iter()
                        .filter(|i| i.id.contains(mat))
                        .map(|i| i.count)
                        .sum();
                    let need = need_per * qty;
                    let missing = need.saturating_sub(have);
                    if missing > 0 {
                        all_ok = false;
                    }
                    let can_craft = if *need_per > 0 {
                        have / need_per
                    } else {
                        u32::MAX
                    };
                    max_crafts = max_crafts.min(can_craft);
                    lines.push(format!(
                        "  {mat}: have {have}, need {need}{} (can craft {can_craft}×)",
                        if missing > 0 {
                            format!(", MISSING {missing}")
                        } else {
                            String::new()
                        }
                    ));
                }
                let total_yield = max_crafts.saturating_mul(*yield_count);
                lines.push(format!(
                    "  → yield: {yield_count} per craft, max {max_crafts} crafts = {total_yield} items"
                ));
                if all_ok {
                    lines.push(format!("  → READY: can craft {qty}× {target}"));
                } else {
                    lines.push(format!(
                        "  → BLOCKED: missing materials for {qty}× {target}"
                    ));
                }
                Ok(ToolResult {
                    message: format!("CRAFTING PLAN for {qty}× {target}:\n{}", lines.join("\n")),
                    is_error: false,
                    images: vec![],
                })
            }
            None => Ok(ToolResult {
                message: format!(
                    "getCraftingPlan: no recipe for '{target}'. Use craftable() to list all craftable items."
                ),
                is_error: true,
                images: vec![],
            }),
        }
    }
}

pub struct ModBlueprintLevelTool;
impl ModBlueprintLevelTool {
    pub fn new(_a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self
    }
}
impl GameTool for ModBlueprintLevelTool {
    fn name(&self) -> &str {
        "getBlueprintLevel"
    }
    fn description(&self) -> &str {
        "Get blueprint details for a specific level (floor). blueprint: name from blueprints(). level: floor number (0=ground level). Returns the block layout for that level."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req(
                "blueprint",
                "Blueprint name: dirt_shelter, wood_house, stone_house, wall_3x3",
            )
            .int_req("level", "Level/floor number (0=ground)", 0, 100)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::read()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let bp_name = args["blueprint"].as_str().unwrap_or("dirt_shelter");
        let level = args["level"].as_u64().unwrap_or(0) as usize;
        let Some(bp) = blueprint::get_blueprint(bp_name) else {
            return Ok(ToolResult {
                message: format!("blueprint '{bp_name}' not found"),
                is_error: true,
                images: vec![],
            });
        };
        if level >= bp.blocks.len() {
            return Ok(ToolResult {
                message: format!("level {level} out of range (0..{})", bp.blocks.len()),
                is_error: true,
                images: vec![],
            });
        }
        let layer = &bp.blocks[level];
        let mut rows = Vec::new();
        for row in layer.iter() {
            rows.push(
                row.iter()
                    .map(|b| {
                        if b.is_empty() || b == "air" {
                            "."
                        } else {
                            b.as_str()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        Ok(ToolResult {
            message: format!("BLUEPRINT {bp_name} level {level}:\n{}", rows.join("\n")),
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModGetCraftingPlanTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModGetCraftingPlanTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModGetCraftingPlanTool {
    fn name(&self) -> &str {
        "get_crafting_plan"
    }
    fn description(&self) -> &str {
        "Get crafting plan: how many you have + what's missing. item: target item. count: how many to craft. Returns have_count and missing materials list."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req("item", "Target item name")
            .int_opt("count", "How many to craft", 1, 1, 64)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::read()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let item = args["item"].as_str().unwrap_or("");
        let count = args["count"].as_u64().unwrap_or(1) as u32;
        let ack = self
            .adapter
            .lock()
            .unwrap()
            .get_crafting_plan(item, count)?;
        let have = ack.have_count.unwrap_or(0);
        let missing = ack
            .missing
            .map(|m| m.to_string())
            .unwrap_or_else(|| "[]".into());
        Ok(ToolResult {
            message: format!(
                "have {} of {}: {} | missing: {}",
                have, count, item, missing
            ),
            is_error: ack.status == "fail",
            images: vec![],
        })
    }
}

pub struct ModBuildPortalTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModBuildPortalTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModBuildPortalTool {
    fn name(&self) -> &str {
        "build_portal"
    }
    fn description(&self) -> &str {
        "Build a Nether portal at current position. Requires 10 obsidian + flint_and_steel or fire_charge in inventory. Builds 4x5 obsidian frame and lights it. Use teleport_to(the_nether) after portal is built to enter."
    }
    fn parameters(&self) -> Value {
        schema::no_args()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        _args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let ack = self.adapter.lock_adapter()?.build_portal()?;
        Ok(ToolResult {
            message: ack.detail,
            is_error: ack.status != "ok",
            images: vec![],
        })
    }
}

pub struct ModTeleportToDimensionTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModTeleportToDimensionTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModTeleportToDimensionTool {
    fn name(&self) -> &str {
        "teleport_to"
    }
    fn description(&self) -> &str {
        "Teleport player between dimensions (the_nether, the_end, overworld). Coordinate scaling: overworld↔nether 8:1. End teleports to center (0,65,0). Use build_portal first for Nether portal. Usage: teleport_to(dimension=\"the_nether\")"
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req(
                "dimension",
                "Target dimension: the_nether, the_end, overworld",
            )
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let dimension = args["dimension"].as_str().unwrap_or("the_nether");
        let ack = self.adapter.lock_adapter()?.teleport_to(dimension)?;
        Ok(ToolResult {
            message: ack.detail,
            is_error: ack.status != "ok",
            images: vec![],
        })
    }
}
