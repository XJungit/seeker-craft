# Adding Tools

This guide shows how to add a new Minecraft tool on the azalea-bot route.

## Tool trait

Every tool implements `GameTool`:
- `name()` — tool name (LLM-visible, must follow naming discipline, see `ARCHITECTURE.md`)
- `description()` — natural language description for the LLM
- `parameters()` — JSON schema for tool arguments
- `effects()` — `ToolEffects` bit flags used for parallel batching
- `execute()` — async fn invoked with args, returns `ToolResult`

## Example (azalea route)

```rust
use std::sync::Arc;
use tokio::sync::Mutex;
use craft_agent::{GameTool, ToolEffects, ToolResult, ToolUpdateFn};
use craft_agent_minecraft::adapter_azalea::MinecraftAzaleaAdapter;

pub struct AzaleaMyTool {
    adapter: Arc<Mutex<MinecraftAzaleaAdapter>>,
}

impl AzaleaMyTool {
    pub fn new(adapter: Arc<Mutex<MinecraftAzaleaAdapter>>) -> Self {
        Self { adapter }
    }
}

impl GameTool for AzaleaMyTool {
    fn name(&self) -> &str { "my_tool" }
    fn description(&self) -> &str { "Do something in the world." }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "target": {"type": "string", "description": "Target block id"}
            },
            "required": ["target"]
        })
    }
    fn effects(&self) -> ToolEffects { ToolEffects::write() }
    fn execute(
        &self,
        _id: &str,
        args: serde_json::Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let adapter = self.adapter.blocking_lock();
        let target = args.get("target")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing target"))?;
        // Call adapter methods or azalea bot APIs directly...
        Ok(ToolResult {
            message: format!("did something to {target}"),
            is_error: false,
            images: vec![],
        })
    }
}
```

## Register

Add the tool in `create_mc_azalea_tools()` in
`crates/craft-agent-minecraft/src/tools_azalea.rs`.

```rust
tools.push(Arc::new(AzaleaMyTool::new(adapter.clone())));
```

## Side-Effect Flags

`ToolEffects` is a bitmask carried on each tool (READ / WRITE / APPEND / NETWORK /
PROCESS / BARRIER). **Note**: the in-bot batch executor that consumed these flags was
removed with the agent loop (2026-08-14) — DSH drives tools one-by-one via
`/api/bot_tool`, so the flags are now informational/documentation only.

Use `ToolEffects::read()`, `ToolEffects::write()`, `ToolEffects::network()` helpers
or combine with `|` operator.

## Mindcraft Philosophy Rules

When implementing tools, follow [`AGENTS.md`](../../AGENTS.md) section 9-bis:

- ❌ Never auto-craft tool blocks (furnace/pickaxe/etc.) inside a tool
- ❌ Never chain-call other tools to satisfy material dependencies
- ✅ Return `Err` with complete resolution steps when prerequisites are missing
- ✅ Return `Ok` for partial success (e.g. `gather` 14/16 → Ok with hint)
- ✅ Keep tools atomic — the LLM plans multi-step synthesis

## Tips

- Keep tools focused and small.
- Return actionable messages — error messages should tell the LLM exactly what to do next.
- Use `auto_equip_best_pickaxe` / `auto_equip_best_axe` helpers before mining/chopping.
- For container-based tools, prefer the `table_flow.rs` helpers for place/open/recycle flow.
