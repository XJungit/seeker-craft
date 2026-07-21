# Changelog

This file tracks notable changes to Craft-Agent.

## 2026-07-18

- **DeepSeek cache optimization**: Moved jailbreak variables (obs_streak, bootstrap)
  out of system prompt into dynamic user messages. System prompt now byte-identical
  across all turns for 94%+ prefix cache hit rate.
- **Crate-level READMEs**: Added `README.md` to all 4 crates (`craft-agent`,
  `craft-agent-minecraft`, `craft-agent-model`, `craft-agent-viewer`) and
  `mods/craft-agent-bridge/`.
- **Doc updates**: RELEASING.md, SECURITY.md, troubleshooting guide, session
  & compaction doc updated with DeepSeek cache guidance.
- **Outdated doc markers**: `game-agent-design.md` and `docs/design/mod-bridge.md`
  tagged with ⚠️ warnings pointing to current docs.

## 2026-07-16

- Added `docs/` with tutorials and design archive.
- Added `McAgentBuilder` for unified mod-bridge and real paths.
- Made tool implementations `Send + Sync`.
- Cleaned compiler warnings in `craft-agent-minecraft`.

## 2026-07-15

- Upgraded tool coverage toward Mindcraft parity.
- Added survival checks and inventory helpers.

## 2026-07-14

- Audited existing implementation gaps vs pi reference.
- Added session compaction and retry improvements.
