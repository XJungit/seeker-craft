# Security Policy

SeekerCraft connects to external LLM/VLM APIs and joins Minecraft servers as a
protocol-level player. This policy covers security-relevant aspects.

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| main    | :white_check_mark: |

## API Key Security

- All LLM/VLM API keys are loaded from `data/config/agent.toml`, which is
  **gitignored** — the repository only ships `agent.example.toml` with placeholders.
- Keys can be injected via environment variables using `api_key_env` instead of
  embedding them in the file.
- Keys are passed as HTTP Bearer tokens; they are never logged and never written
  to session files.

## Minecraft Connection

- The bot connects as a regular player; the server owner is responsible for
  whitelist/OP policy.
- The bot executes only protocol-level actions (move, mine, place, craft, chat,
  trade) — no OS-level input simulation, no client mods.

## Operations Surface

- The viewer dashboard (default `127.0.0.1:8080`) is a local debug UI; do not
  expose it to untrusted networks.
- `craft-agent-ctl` talks to local processes only.

## Reporting a Vulnerability

Open a private security advisory on GitHub, or contact the maintainers directly.
Do **not** open public issues for security vulnerabilities.
