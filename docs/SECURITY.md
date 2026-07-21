# Security Policy

Craft-Agent connects to external LLM/VLM APIs and executes game actions via Minecraft mod bridge or OS-level input simulation. This policy covers security-relevant aspects of these operations.

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| main    | :white_check_mark: |

## API Key Security

- All LLM/VLM API keys are loaded from `config/agent.toml` or environment variables.
- API keys are passed as HTTP Bearer tokens; never logged or exposed in session files.
- The `craft-agent-model` crate does not persist credentials; they exist only in memory during the session.

## Mod-bridge Security

- The TCP bridge binds to `127.0.0.1` (localhost only) by default.
- No authentication is enforced on the bridge port; do not expose it to external networks.
- The bridge accepts structured commands only; it does not evaluate arbitrary Java code.

## Real Path (OS-level input)

- The `real` feature uses `enigo` for keyboard/mouse simulation at the OS level.
- Only active while the agent is running; does not persist or install background hooks.

## Reporting a Vulnerability

If you discover a security vulnerability, please open a private security advisory
in GitHub once available, or contact the maintainers directly through the
repository's private channel.

Please do not open public issues for security vulnerabilities.
