# Configuration

> **现状说明**：DSH 桥接模式（2026-08-14 起）下，LLM 后端与上下文/提示词装配由 **DSH 大脑**
> 负责；Rust 侧不再有 `craft-agent-model`（该 crate 已随阶段3清理删除）。下文
> `agent.toml` 的 LLM 后端配置段是 in-bot 循环时代的历史参考，**DSH 模式下不需要也不使用**；
> 运行时（viewer/connect）配置见「Runtime Mode」。

## Backends（历史参考，DSH 模式不使用）

Edit `data/config/agent.toml` to select active LLM/VLM backends.
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

> `data/config/agent.toml` 是 gitignored 的本地遗留配置，DSH 模式下 LLM 由 DSH 大脑提供，
> 无需也不应配置此文件。

## Agent Config（历史参考，已移除）

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

The only supported runtime is **azalea-bot**. Start the viewer via `craft-agent-ctl`
(DSH bridge mode — the bot is driven by DSH, not an in-bot loop):

```bash
cargo run -p craft-agent-ctl -- viewer "goal text" 0   # steps=0 infinite
cargo run -p craft-agent-ctl -- start                  # connect bot via /api/connect
```

The bot connects to the MC server at `localhost:4444` (configurable via `-Mc`).
The old `McAgentBuilder::mod_bridge` and `McAgentBuilder::real` modes have been
removed (see `docs/adr.md` ADR-004).

## Profile (system prompt)

The system prompt is loaded from `data/profiles/_default.json` and rendered with the
bot name. Critical constraints:

- **Byte stability**: the rendered system prompt must be byte-identical across
  all turns for DeepSeek prefix cache to hit. Dynamic variables go into user
  messages. In DSH era (2026-08-14+) the **DSH brain** owns assembly; the
  dsh-bridge plugin feeds dynamic state as a user-context snapshot
  (`systemPrompt.context`), keeping the system prompt byte-stable.
- **Premature-completion governance** (P56 legacy): the in-bot nudge was removed;
  the craft-bot preset persona (guardrail) forbids declaring "task complete ✅"
  mid-progress.

## Sessions

Viewer session JSONL (`sessions/mc_run.jsonl`) is written as an archive and shown
via `/api/session`. In DSH era, context/session management lives in DSH.
See `session-and-compaction.md` (historical).
