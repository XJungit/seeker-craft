# craft-agent-viewer

Web 仪表盘 + **DSH 桥**。基于 `Axum` HTTP 服务器 + `SSE`（Server-Sent Events）实时推送，
浏览器打开即用，无需额外客户端。**不含任何 LLM/Agent 逻辑**（in-bot 循环已随阶段3清理移除，
2026-08-14 起 DSH 是唯一大脑）。

## 功能

- **DSH 桥**：`/api/connect`（azalea 客户端连上 MC，账号 CraftAgent）/ `/api/bot_tool`
  （派发 54 工具之一，`GameTool::execute`）/ `/api/game-state`（实时 BotState 快照，
  perceive 格式）/ `/api/goal`（更新运营目标）
- **实时状态呈现**：SSE 推送 bot 状态 / 工具调用 / 事件流，浏览器可视化
- **会话 JSONL 展示**：`/api/session` 读取 `sessions/mc_run.jsonl`（只读归档）
- **不参与 bot 决策**：仅桥接 DSH 大脑与 bot 工具层，观察 bot 状态、工具调用、消息流

## 启动

```bash
# 用 ctl 启动（推荐，Windows 进程组独立，viewer 不随 ctl 退出被回收）
cargo run -p craft-agent-ctl -- viewer "目标文本" 0   # steps=0 无限循环
# 浏览器打开 http://127.0.0.1:8080
# 随后由 autopilot 或 DSH 经 /api/connect 连接 bot；日志在 %TEMP%\opencode\viewer_run.log(.err)（可用 SEEKER_LOG_DIR 覆盖）
```

### 命令行参数

| 参数 | 默认 | 说明 |
|---|---|---|
| `--goal` | 必填 | bot 目标描述（自然语言，显示用；DSH 可随时经 `/api/goal` 更新） |
| `--steps` | 50 | 最大执行轮数（0=无限循环；viewer 本身不执行轮次，仅为显示步数） |
| `--port` | 8080 | Web 仪表盘端口 |
| `--session` | `sessions/mc_run.jsonl` | 会话持久化路径（只读归档） |

## 架构

```
DSH 大脑 ──HTTP──► viewer（Axum）
  │  /api/connect     → craft-agent-minecraft 连接 MC
  │  /api/bot_tool    → 派发 54 工具（GameTool::execute，含 P100/P101/P102/P132 自动修正）
  │  /api/game-state  → 实时 BotState 快照（perceive 格式）
  │  /api/goal        → 更新运营目标
  ▼
SSE 推送到浏览器（实时状态/工具调用/事件流）
```

## 端到端测试

```bash
# 启动 MC server（端口 4444）后，用 ctl 起 viewer，再 /api/connect 连接 bot：
cargo run -p craft-agent-ctl -- viewer "挖铁矿并熔炼成铁锭" 0
# 会话日志写入 sessions/mc_run.jsonl；运维用 craft-agent-ctl（status/tail/session）查看
```
