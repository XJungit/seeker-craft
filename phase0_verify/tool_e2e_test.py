"""工具逐一测试脚本 — 直连 mod TCP 端口，用户 MC 观察 + 脚本看返回"""
import socket, json, time, sys

HOST, PORT = "127.0.0.1", 25567

def send_cmd(cmd: dict, timeout: float = 20) -> dict | None:
    """发送一行 JSON 命令到 mod，读取响应。"""
    try:
        s = socket.socket()
        s.settimeout(timeout)
        s.connect((HOST, PORT))
        payload = json.dumps(cmd, ensure_ascii=False) + "\n"
        s.sendall(payload.encode())
        # Read response
        buf = b""
        while True:
            chunk = s.recv(4096)
            if not chunk: break
            buf += chunk
            if b"\n" in buf: break
        s.close()
        line = buf.decode(errors="ignore").strip()
        if line:
            return json.loads(line)
        return None
    except Exception as e:
        return {"error": str(e)}

def test(name: str, cmd: dict):
    print(f"\n{'='*60}")
    print(f"  [{name}] — 发送: {json.dumps(cmd, ensure_ascii=False)[:120]}")
    result = send_cmd(cmd)
    if result is None:
        print(f"  ❌ 无响应")
    elif "error" in result:
        print(f"  ❌ 错误: {result['error']}")
    else:
        detail = result.get("detail", result.get("position", "?"))[:200]
        status = result.get("status", "ok")
        print(f"  {'✅' if status == 'ok' else '⚠️'} {detail}")

print("Craft-Agent 工具逐测——请在 MC 中观察效果")
print(f"连接 {HOST}:{PORT}\n")

# ── 1. 查询状态 ──
test("1. perceive (state查询)", {"type": "state"})

# ── 2. 视角控制 ──
test("2. look (抬头)", {"type": "look", "dx": 0, "dy": 100})
time.sleep(0.5)
test("2b. look (回正)", {"type": "look", "dx": 0, "dy": -100})
time.sleep(0.5)
test("3. look_at (看坐标 0,68,0)", {"type": "look_at", "x": 0.0, "y": 68.0, "z": 0.0})

# ── 3. 移动 ──
test("4. press (前进 40t=2s)", {"type": "press", "keys": "w", "ticks": 40})
test("5. move_to (目标坐标)", {"type": "move_to", "x": -35.0, "y": 68.0, "z": 56.0})

# ── 4. 挖掘 ──
test("6. mine (挖脚下方块 140t)", {"type": "mine", "ticks": 140})

# ── 5. 合成 ──
test("7. craft (橡木→木板)", {"type": "craft", "item": "oak_planks", "count": 4})
test("8. craft (木板→木棍)", {"type": "craft", "item": "stick", "count": 4})

# ── 6. 攻击/使用 ──
test("9. attack (攻击 30t)", {"type": "attack", "ticks": 30})
test("10. right_click (右键 5t)", {"type": "right_click", "ticks": 5})

# ── 7. 丢弃 ──
test("11. discard (丢泥土 1个)", {"type": "discard", "item": "dirt", "num": 1})

# 最终状态
test("12. 最终状态确认", {"type": "state"})

print("\n" + "="*60)
print("全部工具测试完毕。请对照 MC 画面确认每个操作的效果。")
