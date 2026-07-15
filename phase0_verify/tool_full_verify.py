"""逐工具验证：每步前后查状态，自动对比差异"""
import socket, json, time, sys

HOST, PORT = "127.0.0.1", 25567

def cmd(c, timeout=20):
    s = socket.socket(); s.settimeout(timeout)
    try:
        s.connect((HOST, PORT))
        s.sendall((json.dumps(c, ensure_ascii=False) + "\n").encode())
        buf = b""
        while True:
            chunk = s.recv(16384)
            if not chunk: break
            buf += chunk
            if buf.endswith(b"\n"): break
        s.close()
        return json.loads(buf.decode(errors="ignore"))
    except Exception as e:
        return {"error": str(e)}
    finally:
        try: s.close()
        except: pass

def state_snapshot():
    """获取当前背包和位置摘要"""
    st = cmd({"type": "state"})
    if "error" in st: return {"error": st["error"]}
    items = {}
    for i in st.get("inventory", []):
        k = i["id"].replace("minecraft:", "")
        items[k] = items.get(k, 0) + i["count"]
    pos = st.get("position", [0,0,0])
    hp = st.get("health", 0)
    held = st.get("held_item", "?").replace("minecraft:", "")
    return {"items": items, "pos": pos, "hp": hp, "held": held}

def diff_items(before, after):
    """比较背包变化"""
    all_keys = set(before.keys()) | set(after.keys())
    changes = []
    for k in sorted(all_keys):
        b = before.get(k, 0)
        a = after.get(k, 0)
        if b != a:
            changes.append(f"{k}: {b}→{a} ({a-b:+})")
    return changes

def check(name, command, expect_desc="", timeout=20):
    print(f"\n{'─'*60}")
    print(f"【{name}】")
    print(f"  发送: {json.dumps(command, ensure_ascii=False)[:100]}")
    
    before = state_snapshot()
    if "error" in before and "inventory" in str(command):
        # If state query fails but we're trying to check inventory, skip
        print(f"  ⚠️ 状态查询失败: {before.get('error','?')}")
        return
    
    result = cmd(command, timeout)
    time.sleep(0.3)
    after = state_snapshot()
    
    # Tool result
    detail = result.get("detail", str(result)[:80])
    status = result.get("status", "ok")
    if "error" in result:
        print(f"  ❌ 错误: {result['error']}")
        return
    
    print(f"  返回: {detail[:100]}")
    print(f"  位置: {before['pos'][:2]} → {after['pos'][:2]}")
    print(f"  血量: {before['hp']}/{before.get('hp',20)} | 手持: {before['held']} → {after['held']}")
    if "held" in str(before):
        print()
    
    # Inventory changes
    changes = diff_items(before.get("items",{}), after.get("items",{}))
    if changes:
        print(f"  背包变化:")
        for c in changes:
            print(f"    {c}")
    else:
        print(f"  背包: 无变化")

# ═══════════════════════════════════════════════════════════
# 开始测试
print("Craft-Agent 全部工具验证（自动前后对比）")
print(f"连接 {HOST}:{PORT}")

# 1. 状态查询
check("1. perceive", {"type": "state"})

# 2. 视角控制
check("2. look_at (看脚下)", {"type": "look_at", "x": -35.0, "y": 66.0, "z": 56.0})
check("3. look (抬头)", {"type": "look", "dx": 0, "dy": 50})
check("4. look (回正低头)", {"type": "look", "dx": 0, "dy": -50})

# 3. 移动
check("5. press w (前进20t=1s)", {"type": "press", "keys": "w", "ticks": 20})
check("6. press a (左移20t)", {"type": "press", "keys": "a", "ticks": 20})
check("7. press s (后退20t)", {"type": "press", "keys": "s", "ticks": 20})
check("8. move_to", {"type": "move_to", "x": -35.0, "y": 68.0, "z": 52.0})

# 4. 合成
check("9. craft planks (橡木→木板)", {"type": "craft", "item": "oak_planks", "count": 4})
check("10. craft stick (木板→木棍)", {"type": "craft", "item": "stick", "count": 4})
check("11. craft table (木板→工作台)", {"type": "craft", "item": "crafting_table", "count": 1})

# 5. 挖掘
check("12. mine (挖地下 60t)", {"type": "mine", "ticks": 60})

# 6. 交互
check("13. attack (攻击 20t)", {"type": "attack", "ticks": 20})
check("14. right_click (右键 5t)", {"type": "right_click", "ticks": 5})

# 7. 丢弃
check("15. discard (丢泥土 x1)", {"type": "discard", "item": "dirt", "num": 1})

# 8. 最终状态
print(f"\n{'═'*60}")
print("全部 15 项测试完毕")
final = state_snapshot()
print(f"最终位置: {final['pos']}")
print(f"最终背包: {len(final.get('items',{}))} 种物品")
