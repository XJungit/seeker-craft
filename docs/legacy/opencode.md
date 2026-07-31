# Craft-Agent Project Instructions

## Mission
Beat Minecraft Ender Dragon using LLM-driven bot. Continuously optimize the autonomous workflow.

## Project Structure
- `crates/craft-agent/` - Core agent framework
- `crates/craft-agent-minecraft/` - MC adapter (azalea protocol)
- `crates/craft-agent-model/` - LLM client
- `crates/craft-agent-viewer/` - Web dashboard
- `crates/craft-agent-test-harness/` - Test toolchain
- `crates/craft-agent-autopilot/` - **Autonomous workflow (NEW)**

## Workflow Autopilot
Entry point: `cargo run -p craft-agent-autopilot`

The autopilot runs in infinite loop:
1. Build + Test (cargo check/test)
2. LLM Real-Machine Test (viewer + azalea bot)
3. Anomaly Detection (session analysis)
4. Root Cause Analysis (pattern matching)
5. Knowledge沉淀 (learning)
6. Git Commit (checkpoint)

## Key Constraints
- Only `vendor/` is off-limits (third-party azalea)
- All other files can be auto-modified
- Git commit every round for rollback capability
- Bot name: CraftAgent (fixed, preserves inventory)

## Monitoring
```powershell
# Real-time autopilot output
Get-Content "tools/autopilot_out.log" -Tail 20

# Session analysis (LLM behavior)
Get-Content "sessions/mc_run.jsonl" -Tail 10

# Viewer logs
Get-Content "tools/viewer_err.log" -Tail 10
```

## Self-Evolution Principles
1. **Record everything** - events.jsonl, metrics, knowledge_base
2. **Detect anomalies** - statistical, pattern, trend
3. **Root cause analysis** - timeline, correlation, web search
4. **Experiment** - minimal change + A/B test
5. **Learn** - store successful fixes in knowledge_base
6. **Prune** - remove low-value knowledge

## Current Issues to Fix
- LLM session too short (need longer timeout)
- Session analysis module not integrated into autopilot
- No progress detection (steps ≠ progress)
- Need better LLM prompt for survival priorities

## Success Path
Wood tools → Stone tools → Iron tools → Diamond → Nether → End → Ender Dragon
