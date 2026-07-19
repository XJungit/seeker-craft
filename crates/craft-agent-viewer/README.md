# craft-agent-viewer

Agent 运行可视化工具。基于 `ratatui`（TUI）的终端仪表盘，实时展示 Agent 内部状态。

## 功能

- 实时流式显示 LLM 回复与工具调用
- 会话 JSONL 回放与浏览
- Agent 事件日志（压缩、重试、模式触发等）

## 启动

```bash
cargo run -p craft-agent-viewer -- --session sessions/mc_run.jsonl
```

## 架构

使用 `AgentEvent` 通道驱动的发布-订阅模式：

```
Agent run_one_turn() → events: Vec<AgentEvent> → TUI 渲染
```

不依赖具体游戏适配器，可配合任意 `craft-agent` 实例。
