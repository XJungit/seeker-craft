# Configuration

Configure LLM/VLM backends, agent behavior, and runtime parameters.

## Backends

Edit `config/agent.toml` to select active LLM/VLM backends.
Each backend can define model name, endpoint, API key env var, context window,
timeout, and max tokens.

```toml
[llm.backends.deepseek]
model = "deepseek-v4-flash"
api_base = "https://api.deepseek.com"
api_key_env = "DEEPSEEK_API_KEY"
context_window = 500000
timeout_secs = 60
max_tokens = 4096

[llm]  # active backend selector
active = "deepseek"
```

## Agent Config

Key `AgentConfig` options:

| Field | Default | Description |
|---|---|---|
| `max_iter` | 50 | Max turns per run |
| `enable_compaction` | true | Compact context when over budget |
| `enable_retry` | true | Retry LLM calls with backoff |
| `enable_skill` | true | Extract skill examples from successful tool calls |
| `enable_self_prompt` | true | Re-inject `[当前目标]` each turn |
| `compaction.context_window` | 500000 | Token budget for context |
| `compaction.reserve` | 100000 | Reserve kept free (20%) |
| `compaction.keep_recent` | 300000 | Recent tokens kept intact (60%) |
| `retry.max_attempts` | 3 | Max retry attempts on LLM error |

## Runtime Mode

The only supported runtime is **azalea-bot**. Start it via the viewer:

```bash
cargo run -p craft-agent-viewer -- --goal "..." --steps 40 --port 8080
```

The bot connects to the MC server at `localhost:4444` (configurable in
`adapter_azalea.rs`). The old `McAgentBuilder::mod_bridge` and `McAgentBuilder::real`
modes have been removed (see `docs/adr.md` ADR-004).

## Profile (system prompt)

The system prompt is loaded from `profiles/_default.json` and rendered with the
bot name. Critical constraints:

- **Byte stability**: the rendered system prompt must be byte-identical across
  all turns for DeepSeek prefix cache to hit. Dynamic variables go into
  `build_dynamic_instructions_msg()` user messages, not the system prompt.
- **P56 rule**: the system prompt explicitly forbids the LLM from declaring
  "task complete ✅" mid-progress (see `session-and-compaction.md`).

## Sessions

Always pass `--session` for long runs so the agent can resume. Session files are
plain JSONL — greppable and diffable. See `session-and-compaction.md`.
