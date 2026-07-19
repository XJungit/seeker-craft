# Configuration

Configure backends, agent behavior, and runtime mode.

## Backends

Edit `config/agent.toml` to select active LLM/VLM backends.
Each backend can define model name, endpoint, and optional image scaling.

## Agent Config

Key `AgentConfig` options:
- `max_iter`
- `compaction`
- `retry`
- capability switches like `enable_compaction`, `enable_retry`, `enable_skill`

## Runtime Mode

Use `McAgentBuilder` to select runtime mode:
- `McAgentBuilder::mod_bridge(host, port)`
- `McAgentBuilder::real(vlm, capture, fullscreen)`

## Sessions

Always pass `--session` for long runs so the agent can resume.
