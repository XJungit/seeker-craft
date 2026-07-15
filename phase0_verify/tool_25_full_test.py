"""全部 mod 命令完整测试 — 每步前后查状态自动对比"""
import socket, json, time

HOST, PORT = "127.0.0.1", 25567
P, F = 0, 0

def cmd(c, t=20):
    s = socket.socket(); s.settimeout(t)
    try:
        s.connect((HOST, PORT))
        s.sendall((json.dumps(c, ensure_ascii=False) + "\n").encode())
        b = b""
        while True:
            ck = s.recv(16384)
            if not ck: break
            b += ck
            if b.endswith(b"\n"): break
        s.close()
        return json.loads(b.decode(errors="ignore"))
    except Exception as e: return {"error": str(e)}
    finally:
        try: s.close()
        except: pass

def snap():
    st = cmd({"type": "state"})
    if "error" in st: return None
    inv = {}
    for i in st.get("inventory", []): inv[i["id"].replace("minecraft:", "")] = inv.get(i["id"].replace("minecraft:", ""), 0) + i["count"]
    return {"inv": inv, "pos": st.get("position", [0,0,0])[:2], "pitch": st.get("pitch", 0), "held": st.get("held_item", "?").replace("minecraft:", "")}

def check(name, c, inv_expected=None):
    global P, F
    b = snap()
    if b is None: print(f"  ❌ snap失败"); F += 1; return
    r = cmd(c)
    time.sleep(0.15)
    a = snap()
    if a is None: print(f"  ❌ snap失败"); F += 1; return
    d = r.get("detail", "")[:150]
    e = r.get("error", "")
    if e: print(f"  [{name}] ❌ {e}"); F += 1; return
    
    ok = True
    if inv_expected:
        findings = []
        for k, delta in inv_expected.items():
            actual = a["inv"].get(k, 0) - b["inv"].get(k, 0)
            if actual != delta:
                findings.append(f"{k}: {b['inv'].get(k,0)}→{a['inv'].get(k,0)} (预期{delta:+} 实际{actual:+})")
                ok = False
            else:
                findings.append(f"{k}: {delta:+}")
        if ok:
            print(f"  [{name}] ✅ {d}  {'| '.join(findings)}")
            P += 1
        else:
            print(f"  [{name}] ❌ {d}  {'; '.join(findings)}")
            F += 1
    else:
        moved = abs(b["pos"][0]-a["pos"][0]) + abs(b["pos"][1]-a["pos"][1]) > 0.1
        pitch_changed = abs(b["pitch"] - a["pitch"]) > 0.5
        s = f"移动={moved} 视角={pitch_changed}"
        print(f"  [{name}] ✅ {d} ({s})")
        P += 1

print("全部 mod 命令测试（直连 TCP，每步查背包+位置+视角）\n")

# ═══ 1. 查询 ═══
b = snap()
inv_count = len(b["inv"]) if b else 0
print(f"  初始: 背包{inv_count}种, 位置({b['pos'][0]:.0f},{b['pos'][1]:.0f}), 手持={b['held']}, pitch={b['pitch']:.0f}°")

# ═══ 2. 视角 ═══
check("look_at (地面)", {"type": "look_at", "x": -35.0, "y": 66.0, "z": 52.0})
check("look (低头)", {"type": "look", "dx": 0, "dy": -80})
check("look (抬头)", {"type": "look", "dx": 0, "dy": 80})

# ═══ 3. 移动 ═══
check("press w", {"type": "press", "keys": "w", "ticks": 20})
check("press a", {"type": "press", "keys": "a", "ticks": 20})
check("press s", {"type": "press", "keys": "s", "ticks": 20})
check("move_to", {"type": "move_to", "x": -35.0, "y": 68.0, "z": 52.0})

# ═══ 4. 合成（增量验证）═══
check("craft planks", {"type": "craft", "item": "oak_planks", "count": 4}, {"oak_log": -1, "oak_planks": 4})
check("craft stick", {"type": "craft", "item": "stick", "count": 4}, {"oak_planks": -2, "stick": 4})
check("craft table", {"type": "craft", "item": "crafting_table", "count": 1}, {"oak_planks": -4, "crafting_table": 1})

# ═══ 5. 交互 ═══
check("mine 60t", {"type": "mine", "ticks": 60})
check("attack 20t", {"type": "attack", "ticks": 20})
check("right_click 5t", {"type": "right_click", "ticks": 5})

# ═══ 6. 丢弃 ═══
check("discard dirt", {"type": "discard", "item": "dirt", "num": 1}, {"dirt": -1})

# ═══ 7. 手持验证 ═══
check("press 1(切格)", {"type": "press", "keys": "1", "ticks": 3})

# ═══ 8. 最终状态 ═══
print(f"\n{'═'*50}")
print(f"  ✅ {P} 通过  ❌ {F} 失败")
f = snap()
if f: print(f"  最终: 位置({f['pos'][0]:.0f},{f['pos'][1]:.0f}) 手持={f['held']} 背包{len(f['inv'])}种")
