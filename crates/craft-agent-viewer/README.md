# craft-agent-viewer

Agent 运行可视化 Web 仪表盘。基于 `Axum` HTTP 服务器 + `SSE`（Server-Sent Events）
实时推送，浏览器打开即用，无需额外客户端。

## 功能

- 实时流式显示 LLM 回复与工具调用（SSE 推送）
- Agent 事件日志（压缩、重试、模式触发、plain_text_reply 检测等）
- 启停控制（启动/暂停/单步/重置）
- 会话 JSONL 回放与浏览
- 不参与 bot 决策，仅用于观察 bot 状态、工具调用、消息流

## 启动

```bash
# 主入口：LLM 驱动 bot + Web 仪表盘（默认连入本地 MC server localhost:4444）
cargo run -p craft-agent-viewer -- --goal "收集木头做工作台" --steps 40 --port 8080
# 浏览器打开 http://127.0.0.1:8080
```

### 命令行参数

| 参数 | 默认 | 说明 |
|---|---|---|
| `--goal` | 必填 | bot 目标描述（自然语言） |
| `--steps` | 50 | 最大执行轮数 |
| `--port` | 8080 | Web 仪表盘端口 |
| `--session` | `sessions/mc_run.jsonl` | 会话持久化路径 |

## 架构

使用 `AgentEvent` 通道驱动的发布-订阅模式：

```
Agent run_one_turn() → events: Vec<AgentEvent>
                              ↓
                      SSE 推送到浏览器
                              ↓
                        JavaScript 渲染
```

不依赖具体游戏适配器，可配合任意 `craft-agent` 实例。

## 端到端测试

```bash
# 启动 MC server（端口 4444）后
cargo run -p craft-agent-viewer -- --goal "挖铁矿并熔炼成铁锭" --steps 40 --port 8080
```

会话日志写入 `sessions/mc_run.jsonl`，可用 `tools/scan_run.ps1` 分析工具调用统计、
`tools/auto_diag.ps1` 跑全自动化诊断循环。
