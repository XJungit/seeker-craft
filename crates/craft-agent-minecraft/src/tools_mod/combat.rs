//! 战斗 / 实体交互工具。

use crate::tool_args::schema;
use crate::tools_mod::MinecraftModAdapter;
use crate::tools_mod::SafeLockAdapter;
use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult, ToolUpdateFn};
use serde_json::Value;
use std::sync::Arc;
use std::sync::Mutex;

pub struct ModCombatTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModCombatTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModCombatTool {
    fn name(&self) -> &str {
        "combat"
    }
    fn description(&self) -> &str {
        "Autonomous combat: Mod finds nearest hostile → navigates → attacks with player.attack() (vanilla hit detection). melee=aggressive, kite=hit-and-run, retreat=flee. Auto-equips best weapon. Runs in background — non-blocking. Use combat_status to check progress. Returns killed/retreated/timeout/no_target. Usage: combat(mode=\"melee\", ticks=200)"
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req(
                "mode",
                "melee (aggressive), kite (hit-and-run), retreat (flee)",
            )
            .int_opt(
                "ticks",
                "Duration in ticks (200 ≈ 10s, max 500)",
                200,
                20,
                500,
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
        let mode = args["mode"].as_str().unwrap_or("melee");
        let ticks = args["ticks"].as_u64().unwrap_or(200).min(500) as u32;
        let adapter = self.adapter.lock_adapter()?;
        let ack = adapter.combat_start(mode, ticks)?;
        Ok(ToolResult {
            message: format!(
                "combat {} for {} ticks started: {}",
                mode, ticks, ack.detail
            ),
            is_error: ack.status != "ok",
            images: vec![],
        })
    }
}

pub struct ModCombatStatusTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModCombatStatusTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModCombatStatusTool {
    fn name(&self) -> &str {
        "combat_status"
    }
    fn description(&self) -> &str {
        "Check the status of active combat. Returns current state (running/idle) with target info. Use after combat() to check progress."
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
        let ack = self.adapter.lock_adapter()?.combat_status()?;
        Ok(ToolResult {
            message: ack.detail,
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModSearchEntityTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModSearchEntityTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModSearchEntityTool {
    fn name(&self) -> &str {
        "searchForEntity"
    }
    fn description(&self) -> &str {
        "Find nearest entity of given type and walk to it (no attack). Uses move_to for navigation. type: entity type like cow, pig, villager, sheep, chicken. Returns entity type, coordinates, and distance."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req(
                "type",
                "Entity type to find, e.g. cow, pig, villager, sheep, chicken, zombie",
            )
            .num_opt("search_range", "Max search distance (default 64)", 64.0)
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
        let target = args["type"].as_str().unwrap_or("cow");
        let range = args["search_range"].as_f64().unwrap_or(64.0);
        let adapter = self.adapter.lock_adapter()?;
        let st = adapter.reload()?;
        let target_clean = target.replace("minecraft:", "");
        let nearest = st
            .entities
            .iter()
            .filter(|e| {
                let ty = e.r#type.replace("minecraft:", "");
                ty.contains(&target_clean) && e.dist <= range
            })
            .min_by(|a, b| {
                a.dist
                    .partial_cmp(&b.dist)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        match nearest {
            Some(ent) => {
                let ty = ent.r#type.replace("minecraft:", "");
                drop(adapter);
                let _ = self.adapter.lock_adapter()?.move_to(ent.x, ent.y, ent.z)?;
                Ok(ToolResult {
                    message: format!(
                        "walked to {} at ({:.1},{:.1},{:.1}) dist={:.1}m",
                        ty, ent.x, ent.y, ent.z, ent.dist
                    ),
                    is_error: false,
                    images: vec![],
                })
            }
            None => {
                let all: Vec<String> = st
                    .entities
                    .iter()
                    .map(|e| format!("{}@{:.1}m", e.r#type, e.dist))
                    .collect();
                let dbg = if all.is_empty() {
                    "entities=[]".into()
                } else {
                    format!("entities=[{}]", all.join(", "))
                };
                Ok(ToolResult {
                    message: format!("searchForEntity: no {target} within {range}m ({dbg})"),
                    is_error: true,
                    images: vec![],
                })
            }
        }
    }
}

pub struct ModAttackPlayerTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModAttackPlayerTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModAttackPlayerTool {
    fn name(&self) -> &str {
        "attack_player"
    }
    fn description(&self) -> &str {
        "Attack a specific player by name. Automatically approaches if too far, stops if target dies. player_name: exact name from list_players. ticks: attack duration (60≈3s)."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req("player_name", "Exact player name")
            .int_opt("ticks", "Attack duration", 60, 1, 200)
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
        let name = args["player_name"].as_str().unwrap_or("");
        let ticks = args["ticks"].as_u64().unwrap_or(60) as u32;
        let ack = self.adapter.lock_adapter()?.attack_player(name, ticks)?;
        let hits = ack.hits.unwrap_or(0);
        Ok(ToolResult {
            message: format!("attack_player {name}: {hits} hits"),
            is_error: hits == 0,
            images: vec![],
        })
    }
}

pub struct ModUseOnTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModUseOnTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModUseOnTool {
    fn name(&self) -> &str {
        "useOn"
    }
    fn description(&self) -> &str {
        "Use (right-click) current item on nearest entity or block. tool_name: item to equip first (or 'hand' for empty hand). target: entity type (cow, sheep, villager) or block type (crafting_table, furnace) or 'nothing' to use in air. Used for: shearing sheep, milking cows, trading with villagers, opening crafting table/furnace/chest."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req("tool_name", "Item to equip first, e.g. shears, bucket, or 'hand' for empty hand")
            .str_req("target", "Entity type (cow, sheep, villager) or block type (crafting_table, furnace), or 'nothing'")
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
        let tool = args["tool_name"].as_str().unwrap_or("hand");
        let target = args["target"].as_str().unwrap_or("nothing");
        let adapter = self.adapter.lock_adapter()?;
        let st = adapter.reload()?;

        // 装备工具（ServerPlayer 架构：直接反射设置 Inventory.selected）
        if tool != "hand" {
            let slot = st
                .inventory
                .iter()
                .find(|i| i.id.contains(tool) && i.slot < 9 && i.count > 0)
                .map(|i| i.slot + 1);
            if let Some(s) = slot {
                let _ = adapter.select_slot(s - 1)?;
                std::thread::sleep(std::time::Duration::from_millis(100));
            } else {
                return Ok(ToolResult {
                    message: format!("useOn: {tool} not found in hotbar"),
                    is_error: true,
                    images: vec![],
                });
            }
        }

        // 找目标
        let target_clean = target.replace("minecraft:", "");
        let ent = st
            .entities
            .iter()
            .filter(|e| {
                let ty = e.r#type.replace("minecraft:", "");
                ty.contains(&target_clean) && e.dist <= 5.0
            })
            .min_by(|a, b| {
                a.dist
                    .partial_cmp(&b.dist)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        let block = st
            .nearby_blocks
            .iter()
            .filter(|b| b.id.contains(&target_clean) && b.dist <= 5.0)
            .min_by(|a, b| {
                a.dist
                    .partial_cmp(&b.dist)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

        if let Some(e) = ent {
            adapter.look_at(e.x, e.y + 1.0, e.z)?;
            std::thread::sleep(std::time::Duration::from_millis(150));
            let _ = adapter.use_item(10)?;
            Ok(ToolResult {
                message: format!(
                    "used {tool} on {} at ({:.1},{:.1},{:.1})",
                    e.r#type, e.x, e.y, e.z
                ),
                is_error: false,
                images: vec![],
            })
        } else if let Some(b) = block {
            adapter.look_at(b.x, b.y + 0.5, b.z)?;
            std::thread::sleep(std::time::Duration::from_millis(150));
            let _ = adapter.use_item(10)?;
            Ok(ToolResult {
                message: format!(
                    "used {tool} on {} at ({:.0},{:.0},{:.0})",
                    b.id, b.x, b.y, b.z
                ),
                is_error: false,
                images: vec![],
            })
        } else if target == "nothing" {
            let _ = adapter.use_item(10)?;
            Ok(ToolResult {
                message: format!("used {tool} in air"),
                is_error: false,
                images: vec![],
            })
        } else {
            Ok(ToolResult {
                message: format!("useOn: no {target} within 5m"),
                is_error: true,
                images: vec![],
            })
        }
    }
}

pub struct ModActivateBlockTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModActivateBlockTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModActivateBlockTool {
    fn name(&self) -> &str {
        "activate_block"
    }
    fn description(&self) -> &str {
        "Right-click activate a block at x/y/z (orient + useItemOn). Opens doors, activates levers/buttons, opens furnace/chest GUI. Must be within 5m."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .int_req("x", "World X", -30000000, 30000000)
            .int_req("y", "World Y", -64, 320)
            .int_req("z", "World Z", -30000000, 30000000)
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
        let x = args["x"].as_i64().unwrap_or(0) as i32;
        let y = args["y"].as_i64().unwrap_or(0) as i32;
        let z = args["z"].as_i64().unwrap_or(0) as i32;
        let ack = self.adapter.lock_adapter()?.activate_block(x, y, z)?;
        let activated = ack.activated.unwrap_or(false);
        Ok(ToolResult {
            message: ack.detail,
            is_error: !activated && ack.status == "fail",
            images: vec![],
        })
    }
}

pub struct ModUseOnEntityTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModUseOnEntityTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModUseOnEntityTool {
    fn name(&self) -> &str {
        "use_on_entity"
    }
    fn description(&self) -> &str {
        "Use held item on nearest entity of given type (orient + interactOn). entity_type: entity ID substring (e.g. 'villager', 'horse', 'cow'). radius: search radius (default 8m)."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req("entity_type", "Entity type ID substring")
            .num_opt("radius", "Search radius (default 8m)", 8.0)
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
        let et = args["entity_type"].as_str().unwrap_or("");
        let radius = args["radius"].as_f64();
        let ack = self.adapter.lock_adapter()?.use_on_entity(et, radius)?;
        let interacted = ack.interacted.unwrap_or(false);
        Ok(ToolResult {
            message: ack.detail,
            is_error: !interacted && ack.status == "fail",
            images: vec![],
        })
    }
}

pub struct ModActivateNearestBlockTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModActivateNearestBlockTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModActivateNearestBlockTool {
    fn name(&self) -> &str {
        "activate_nearest_block"
    }
    fn description(&self) -> &str {
        "Activate nearest block of given type within 5m (mindcraft !activateNearestBlock). Auto-searches + uses useItemOn. block_type: block ID substring (e.g. 'door', 'lever', 'button')."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req("block_type", "Block ID substring")
            .num_opt("radius", "Search radius (default 5m)", 5.0)
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
        let bt = args["block_type"].as_str().unwrap_or("");
        let radius = args["radius"].as_f64();
        let ack = self
            .adapter
            .lock_adapter()?
            .activate_nearest_block(bt, radius)?;
        let activated = ack.activated.unwrap_or(false);
        Ok(ToolResult {
            message: ack.detail,
            is_error: !activated && ack.status == "fail",
            images: vec![],
        })
    }
}
