//! 物品 / 容器 / GUI 交互工具（参考 Numen 容器管理 + Mindcraft 物品管理）。

use crate::tool_args::schema;
use crate::tools_mod::MinecraftModAdapter;
use crate::tools_mod::SafeLockAdapter;
use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult, ToolUpdateFn};
use serde_json::Value;
use std::sync::Arc;
use std::sync::Mutex;

pub struct ModInspectGuiTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModInspectGuiTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModInspectGuiTool {
    fn name(&self) -> &str {
        "inspect_gui"
    }
    fn description(&self) -> &str {
        "Read the currently open container/crafting GUI contents. Returns slot-by-slot inventory of the open container (chest, furnace, crafting table, etc) plus player inventory side. Use BEFORE transfer() to know slot indices."
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
        let ack = self.adapter.lock_adapter()?.inspect_gui()?;
        let has_gui = ack.has_gui.unwrap_or(false);
        if !has_gui {
            return Ok(ToolResult {
                message: "inspect_gui: no container open".into(),
                is_error: false,
                images: vec![],
            });
        }
        let slots = ack.slots.clone().unwrap_or_default();
        let carried = ack.carried_item.clone().unwrap_or_default();
        let mut lines = vec![format!("GUI slots: {} total", ack.detail)];
        if let Some(arr) = slots.as_array() {
            for slot in arr.iter().take(40) {
                let idx = slot["slot_index"].as_u64().unwrap_or(0);
                let id = slot["id"].as_str().unwrap_or("?");
                let count = slot["count"].as_u64().unwrap_or(0);
                let side = slot["side"].as_str().unwrap_or("?");
                if id != "minecraft:air" {
                    lines.push(format!(
                        "  [{idx}] {id} x{count} ({side})",
                        idx = idx,
                        id = id.replace("minecraft:", ""),
                        count = count,
                        side = side
                    ));
                }
            }
        }
        if carried != "minecraft:air" && !carried.is_empty() {
            lines.push(format!(
                "  [cursor] {} x{}",
                carried.replace("minecraft:", ""),
                ack.carried_count.unwrap_or(0)
            ));
        }
        Ok(ToolResult {
            message: lines.join("\n"),
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModTransferTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModTransferTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModTransferTool {
    fn name(&self) -> &str {
        "transfer"
    }
    fn description(&self) -> &str {
        "Move items within an open container GUI. Use inspect_gui first to get slot indices. moves: array of {from: slot_index, to?: slot_index|null}. to=null means shift-click (auto-route to opposite side). Call close_gui when done."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req("moves", "JSON array of moves: [{from: int, to?: int|null}]")
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
        let moves = args["moves"].clone();
        let ack = self.adapter.lock_adapter()?.transfer(moves)?;
        let moved = ack.moved_count.unwrap_or(0);
        Ok(ToolResult {
            message: format!("transfer: {moved} moves executed. {}", ack.detail),
            is_error: moved == 0,
            images: vec![],
        })
    }
}

pub struct ModCloseGuiTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModCloseGuiTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModCloseGuiTool {
    fn name(&self) -> &str {
        "close_gui"
    }
    fn description(&self) -> &str {
        "Close the currently open container/crafting GUI. Use after transfer() is complete."
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
        let ack = self.adapter.lock_adapter()?.close_gui()?;
        Ok(ToolResult {
            message: ack.detail,
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModEquipItemTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModEquipItemTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModEquipItemTool {
    fn name(&self) -> &str {
        "equip_item"
    }
    fn description(&self) -> &str {
        "Equip an item to a specific slot. Supports armor (head/chest/legs/feet), offhand, and mainhand. Auto-routes by item type if slot not specified. item: item name. slot: 'mainhand'|'offhand'|'head'|'chest'|'legs'|'feet' (optional, auto-detected)."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req(
                "item",
                "Item to equip, e.g. iron_helmet, shield, diamond_sword",
            )
            .str_opt(
                "slot",
                "Target slot: mainhand/offhand/head/chest/legs/feet",
                "mainhand",
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
        let item = args["item"].as_str().unwrap_or("");
        let slot = args["slot"].as_str();
        let ack = self.adapter.lock_adapter()?.equip_item(item, slot)?;
        let equipped = ack.equipped.unwrap_or(false);
        let slot_str = ack
            .slot
            .as_ref()
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(ToolResult {
            message: format!("equip_item {item} -> {slot_str} (equipped={})", equipped),
            is_error: !equipped,
            images: vec![],
        })
    }
}

pub struct ModEatItemTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModEatItemTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModEatItemTool {
    fn name(&self) -> &str {
        "eat_item"
    }
    fn description(&self) -> &str {
        "Eat a specific food item from inventory. Finds item, equips it, and consumes it. item: food name like cooked_beef, bread, apple. ticks: eat duration (32≈1.6s for most food)."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req("item", "Food to eat, e.g. cooked_beef, bread, apple")
            .int_opt("ticks", "Eat duration (32≈1.6s)", 32, 1, 100)
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
        let item = args["item"].as_str().unwrap_or("bread");
        let ticks = args["ticks"].as_u64().map(|t| t as u32);
        let ack = self.adapter.lock_adapter()?.eat_item(item, ticks)?;
        let consumed = ack.consumed.unwrap_or(false);
        Ok(ToolResult {
            message: format!("eat_item {item} (consumed={})", consumed),
            is_error: !consumed,
            images: vec![],
        })
    }
}

pub struct ModDropItemsTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModDropItemsTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModDropItemsTool {
    fn name(&self) -> &str {
        "drop_items"
    }
    fn description(&self) -> &str {
        "Drop items from inventory as ground ItemEntity (with pickup cooldown, like pressing Q). item: item name. num: how many to drop. Unlike discard(), this spawns real items on the ground that can be picked back up."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req("item", "Item to drop")
            .int_opt("num", "Count to drop", 1, 1, 64)
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
        let num = args["num"].as_u64().unwrap_or(1) as u32;
        let ack = self.adapter.lock_adapter()?.drop_items(item, num)?;
        let dropped = ack.dropped.unwrap_or(0);
        Ok(ToolResult {
            message: format!("drop_items {item} x{dropped} (ItemEntity spawned)"),
            is_error: dropped == 0,
            images: vec![],
        })
    }
}
