# Adding Tools

This guide shows how to add a new Minecraft tool.

## Tool trait

Every tool implements `GameTool`:
- `name()`
- `description()`
- `parameters()`
- `effects()`
- `execute()`

## Example

```rust
pub struct ModMyTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}

impl ModMyTool {
    pub fn new(adapter: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter }
    }
}

impl GameTool for ModMyTool {
    fn name(&self) -> &str { "my_tool" }
    fn description(&self) -> &str { "Do something." }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{}})
    }
    fn effects(&self) -> ToolEffects { ToolEffects::write() }
    fn execute(&self, _id: &str, _args: Value, _on_update: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let adapter = self.adapter.lock().unwrap();
        Ok(ToolResult { message: "done".into(), is_error: false, images: vec![] })
    }
}
```

## Register

Add the tool in `create_mc_mod_tools()` in `tools_mod.rs`.

## Tips

- Keep tools focused and small.
- Return actionable messages.
- Use `survival_precheck()` before risky actions.
