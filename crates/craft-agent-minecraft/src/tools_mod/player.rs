// ═══════════════════════════════════════════════════════════════
// 玩家交互 + 控制命令工具（list_players / go_to_player / give_player /
// collect_items / stop / set_goal / follow_player / search_wiki /
// villager_trades / trade_with_villager / look_at_player /
// look_at_position / clear_chat）
// 从 tools_mod.rs 拆分到本子模块（重构 ②）
// ═══════════════════════════════════════════════════════════════

use crate::tool_args::schema;
use crate::tools_mod::MinecraftModAdapter;
use crate::tools_mod::SafeLockAdapter;
use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult, ToolUpdateFn};
use serde_json::Value;
use std::sync::Arc;
use std::sync::Mutex;

pub struct ModListPlayersTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModListPlayersTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModListPlayersTool {
    fn name(&self) -> &str {
        "list_players"
    }
    fn description(&self) -> &str {
        "List all online players with name, position, and distance. Use before go_to_player or attack_player."
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
        let ack = self.adapter.lock_adapter()?.list_players()?;
        let count = ack.count.unwrap_or(0);
        let players = ack.players.clone().unwrap_or_default();
        let mut lines = vec![format!("Online players: {count}")];
        if let Some(arr) = players.as_array() {
            for p in arr {
                let name = p["name"].as_str().unwrap_or("?");
                let dist = p["dist"].as_f64().unwrap_or(0.0);
                let x = p["position"][0].as_f64().unwrap_or(0.0);
                let y = p["position"][1].as_f64().unwrap_or(0.0);
                let z = p["position"][2].as_f64().unwrap_or(0.0);
                lines.push(format!(
                    "  {name} at ({x:.0},{y:.0},{z:.0}) dist={dist:.1}m"
                ));
            }
        }
        Ok(ToolResult {
            message: lines.join("\n"),
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModGoToPlayerTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModGoToPlayerTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModGoToPlayerTool {
    fn name(&self) -> &str {
        "go_to_player"
    }
    fn description(&self) -> &str {
        "Navigate to another player's position. player_name: exact player name from list_players. closeness: how close to get (default 2.0m)."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req("player_name", "Exact player name")
            .num_opt("closeness", "How close to get (meters)", 3.0)
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
        let closeness = args["closeness"].as_f64();
        let ack = self.adapter.lock_adapter()?.go_to_player(name, closeness)?;
        let reached = ack.reached.unwrap_or(false);
        let dist = ack.final_dist.unwrap_or(0.0);
        Ok(ToolResult {
            message: format!("go_to_player {name}: reached={reached} dist={dist:.1}m"),
            is_error: !reached,
            images: vec![],
        })
    }
}

pub struct ModGivePlayerTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModGivePlayerTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModGivePlayerTool {
    fn name(&self) -> &str {
        "give_player"
    }
    fn description(&self) -> &str {
        "Give items to another player. Walks to player if far, then drops items as ItemEntity. player_name: exact name. item: item name. num: how many."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req("player_name", "Exact player name")
            .str_req("item", "Item name")
            .int_opt("num", "Count", 1, 1, 64)
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
        let item = args["item"].as_str().unwrap_or("");
        let num = args["num"].as_u64().unwrap_or(1) as u32;
        let ack = self.adapter.lock_adapter()?.give_player(name, item, num)?;
        let dropped = ack.dropped.unwrap_or(0);
        Ok(ToolResult {
            message: format!("give_player {item} x{dropped} to {name}"),
            is_error: dropped == 0,
            images: vec![],
        })
    }
}

pub struct ModCollectItemsTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModCollectItemsTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModCollectItemsTool {
    fn name(&self) -> &str {
        "collect_items"
    }
    fn description(&self) -> &str {
        "Automatically pick up nearby dropped items on the ground. Scans for ItemEntity, walks to each, and lets vanilla pickup handle collection. item_ids: filter by item names (empty = all). radius: search radius (default 16). max_count: max items to collect (default 64)."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_opt(
                "item_ids",
                "Filter items (empty=all), JSON array of strings",
                "[]",
            )
            .num_opt("radius", "Search radius", 16.0)
            .int_opt("max_count", "Max to collect", 64, 1, 256)
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
        let item_ids: Vec<String> = match &args["item_ids"] {
            Value::Array(arr) => arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            Value::String(s) => serde_json::from_str(s).unwrap_or_default(),
            _ => vec![],
        };
        let radius = args["radius"].as_f64().unwrap_or(16.0);
        let max_count = args["max_count"].as_u64().unwrap_or(64) as u32;
        let ack = self
            .adapter
            .lock_adapter()?
            .collect_items(item_ids, radius, max_count)?;
        let collected = ack.collected.unwrap_or(0);
        Ok(ToolResult {
            message: format!("collect_items: collected {collected} items"),
            is_error: collected == 0,
            images: vec![],
        })
    }
}

pub struct ModStopTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModStopTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModStopTool {
    fn name(&self) -> &str {
        "stop"
    }
    fn description(&self) -> &str {
        "Stop all current actions immediately. Use when agent is stuck or doing something wrong. Equivalent to mindcraft !stop."
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
        let ack = self.adapter.lock_adapter()?.stop()?;
        Ok(ToolResult {
            message: ack.detail,
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModSetGoalTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
    pending_goal: Option<Arc<std::sync::Mutex<Option<String>>>>,
}
impl ModSetGoalTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self {
            adapter: a,
            pending_goal: None,
        }
    }
    pub fn with_pending_goal(mut self, pg: Arc<std::sync::Mutex<Option<String>>>) -> Self {
        self.pending_goal = Some(pg);
        self
    }
}
impl GameTool for ModSetGoalTool {
    fn name(&self) -> &str {
        "set_goal"
    }
    fn description(&self) -> &str {
        "Set a persistent goal that stays active across turns. Clear with empty goal. Used by SelfPrompter for continuous motivation. goal: description of what to achieve."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_opt("goal", "Goal description (empty to clear)", "")
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
        let goal = args["goal"].as_str().unwrap_or("");
        let ack = self.adapter.lock_adapter()?.set_goal(goal)?;
        if let Some(ref pg) = self.pending_goal
            && let Ok(mut g) = pg.lock()
        {
            if goal.is_empty() {
                *g = None;
            } else {
                *g = Some(goal.to_string());
            }
        }
        Ok(ToolResult {
            message: ack.detail,
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModFollowPlayerTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModFollowPlayerTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModFollowPlayerTool {
    fn name(&self) -> &str {
        "follow_player"
    }
    fn description(&self) -> &str {
        "Endlessly follow the given player (mindcraft !followPlayer resume=true). Mod-side tick loop keeps chasing. Use stop() to cancel. player_name: target player. follow_dist: distance to maintain (default 3m)."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req("player_name", "Player name to follow")
            .num_opt("follow_dist", "Distance to maintain (default 3m)", 3.0)
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
        let dist = args["follow_dist"].as_f64();
        let ack = self.adapter.lock_adapter()?.follow_player(name, dist)?;
        Ok(ToolResult {
            message: ack.detail,
            is_error: ack.status == "fail",
            images: vec![],
        })
    }
}

pub struct ModSearchWikiTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModSearchWikiTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModSearchWikiTool {
    fn name(&self) -> &str {
        "search_wiki"
    }
    fn description(&self) -> &str {
        "Search minecraft.wiki for crafting/behavior info. Mod-side HTTP request + HTML extraction, 2000 char truncation. query: search term (e.g. 'redstone repeater recipe')."
    }
    fn parameters(&self) -> Value {
        schema::object().str_req("query", "Search query").finish()
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
        let q = args["query"].as_str().unwrap_or("");
        let ack = self.adapter.lock_adapter()?.search_wiki(q)?;
        let text = ack.wiki_text.unwrap_or(ack.detail);
        Ok(ToolResult {
            message: text,
            is_error: ack.status == "fail",
            images: vec![],
        })
    }
}

pub struct ModVillagerTradesTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModVillagerTradesTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModVillagerTradesTool {
    fn name(&self) -> &str {
        "villager_trades"
    }
    fn description(&self) -> &str {
        "Show trades of nearest villager (mindcraft !showVillagerTrades). Returns trade list with 1-indexed positions. radius: search radius (default 8m)."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .num_opt("radius", "Search radius (default 8m)", 8.0)
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
        let radius = args["radius"].as_f64();
        let ack = self.adapter.lock_adapter()?.villager_trades(radius)?;
        let trades = ack
            .trades
            .map(|t| t.to_string())
            .unwrap_or_else(|| "[]".into());
        Ok(ToolResult {
            message: format!("{} | trades={}", ack.detail, trades),
            is_error: ack.status == "fail",
            images: vec![],
        })
    }
}

pub struct ModTradeWithVillagerTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModTradeWithVillagerTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModTradeWithVillagerTool {
    fn name(&self) -> &str {
        "trade_with_villager"
    }
    fn description(&self) -> &str {
        "Trade with nearest villager (mindcraft !tradeWithVillager). index: 1-indexed trade position from villager_trades. count: how many trades (default 1). radius: search radius (default 8m)."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .int_req("index", "1-indexed trade position", 1, 100)
            .int_opt("count", "How many trades (default 1)", 1, 1, 64)
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
        let index = args["index"].as_u64().unwrap_or(1) as u32;
        let count = args["count"].as_u64().map(|n| n as u32);
        let radius = args["radius"].as_f64();
        let ack = self
            .adapter
            .lock_adapter()?
            .trade_with_villager(index, count, radius)?;
        let traded = ack.traded.unwrap_or(0);
        Ok(ToolResult {
            message: format!("traded {} of index {}", traded, index),
            is_error: ack.status == "fail" || traded == 0,
            images: vec![],
        })
    }
}

pub struct ModLookAtPlayerTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModLookAtPlayerTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModLookAtPlayerTool {
    fn name(&self) -> &str {
        "look_at_player"
    }
    fn description(&self) -> &str {
        "Look at the given player (only orientation, no movement). player_name: target player."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req("player_name", "Player name to look at")
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
        let ack = self.adapter.lock_adapter()?.look_at_player(name)?;
        Ok(ToolResult {
            message: ack.detail,
            is_error: ack.status == "fail",
            images: vec![],
        })
    }
}

pub struct ModLookAtPositionTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModLookAtPositionTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModLookAtPositionTool {
    fn name(&self) -> &str {
        "look_at_position"
    }
    fn description(&self) -> &str {
        "Look at a specific x/y/z coordinate (only orientation, no movement)."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .num_req("x", "World X")
            .num_req("y", "World Y")
            .num_req("z", "World Z")
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
        let x = args["x"].as_f64().unwrap_or(0.0);
        let y = args["y"].as_f64().unwrap_or(0.0);
        let z = args["z"].as_f64().unwrap_or(0.0);
        let ack = self.adapter.lock_adapter()?.look_at_position(x, y, z)?;
        Ok(ToolResult {
            message: ack.detail,
            is_error: ack.status == "fail",
            images: vec![],
        })
    }
}

pub struct ModClearChatTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModClearChatTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModClearChatTool {
    fn name(&self) -> &str {
        "clear_chat"
    }
    fn description(&self) -> &str {
        "Clear the chat history (mindcraft !clearChat). Starts fresh conversation from scratch. Useful after long sessions to reset context."
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
        let _ = self.adapter.lock_adapter()?.clear_chat()?;
        Ok(ToolResult {
            message:
                "Chat history cleared (mod ack only — agent runtime should clear its own history)."
                    .into(),
            is_error: false,
            images: vec![],
        })
    }
}
