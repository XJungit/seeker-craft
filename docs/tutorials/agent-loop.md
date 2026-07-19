# Agent Loop

This guide explains the main agent loop and how a turn is executed.

## Turn Flow

1. Build context from messages, world info, and skill examples.
2. Call the LLM with tools.
3. Handle tool calls in parallel batches.
4. Persist tool results and session entries.
5. Apply compaction if needed.

## Extension Points

- `queue_steering()` injects follow-up instructions.
- `queue_follow_up()` queues follow-up turns.
- `abort()` stops retry loops.

## Observability

- `AgentEvent` emits start/update/end events for tool execution.
- `Usage` tracks LLM token usage.
