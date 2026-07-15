"""全部 mod 命令完整测试 — 逐个验证"""
import socket, json, time, sys

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
            if not ck: break; b += ck
            if b.endswith(b"\n"): break
        s.close(); return json.loads(b.decode(errors="ignore"))
    except Exception as e: return {"error": str(e)}
    finally:
        try: s.close()
        except: pass

def snap():
    st = cmd({"type": "state"})
    if "error" in st: return None
    inv = {}
    for i in st.get("inventory", []):
        k = i["id"].replace("minecraft:", "")
        inv[k] = inv.get(k, 0) + i["count"]
    return {"inv": inv, "pos": st.get("position", [0,0,0])[:2], "pitch": st.get("pitch", 0)}

def test(name, c, check_fn=None, inv_expected=None):
    global P, F
    b = snap()
    r = cmd(c)
    time.sleep(0.1)
    a = snap()
    d = r.get("detail", "")
    e = r.get("error", "")
    if e or (isinstance(r, dict) and r.get("status") == "fail"):
        print(f"  ❌ [{name}] {e or d}")
        F += 1; return
    ok = True
    if check_fn and not check_fn(b, a):
        ok = False
    if inv_expected:
        for k, delta in inv_expected.items():
            act = a["inv"].get(k, 0) - b["inv"].get(k, 0)
            if act != delta:
                print(f"  ❌ [{name}] {k}: {b['inv'].get(k,0)}→{a['inv'].get(k,0)} exp{delta:+} got{act:+}")
                ok = False
    if ok: print(f"  ✅ [{name}] {d[:90]}")
    else: F += 1
    if ok: P += 1
    return a

print("Craft-Agent 全工具完整性测试")
print(f"连接 {HOST}:{PORT}\n")

# 1. 状态查询
print("═══ 查询 ═══")
test("perceive", {"type": "state"})

# 2. 视角控制
print("\n═══ 视角 ═══")
test("look_at (地面)", {"type": "look_at", "x": -35.0, "y": 66.0, "z": 52.0})
test("look_down", {"type": "look", "dx": 0, "dy": -80})
test("look_up", {"type": "look", "dx": 0, "dy": 80})
test("look_right", {"type": "look", "dx": 150, "dy": 0})

# 3. 移动
print("\n═══ 移动 ═══")
test("press w", {"type": "press", "keys": "w", "ticks": 20})
test("press a", {"type": "press", "keys": "a", "ticks": 20})
test("press s", {"type": "press", "keys": "s", "ticks": 20})
test("press d", {"type": "press", "keys": "d", "ticks": 20})
test("press space", {"type": "press", "keys": "space", "ticks": 5})
test("press shift", {"type": "press", "keys": "shift", "ticks": 5})
test("move_to", {"type": "move_to", "x": -35.0, "y": 68.0, "z": 52.0})

# 4. 合成 — 木材系
print("\n═══ 合成:木材系 ═══")
test("craft planks", {"type": "craft", "item": "oak_planks", "count": 4}, inv_expected={"oak_log": -1, "oak_planks": 4})
test("craft stick", {"type": "craft", "item": "stick", "count": 4}, inv_expected={"oak_planks": -2, "stick": 4})
test("craft table", {"type": "craft", "item": "crafting_table", "count": 1}, inv_expected={"oak_planks": -4, "crafting_table": 1})

# 5. 合成 — 木工具
print("\n═══ 合成:木工具 ═══")
test("craft wooden_pickaxe", {"type": "craft", "item": "wooden_pickaxe", "count": 1}, inv_expected={"oak_planks": -3, "stick": -2, "wooden_pickaxe": 1})
test("craft wooden_axe", {"type": "craft", "item": "wooden_axe", "count": 1}, inv_expected={"oak_planks": -3, "stick": -2, "wooden_axe": 1})
test("craft wooden_sword", {"type": "craft", "item": "wooden_sword", "count": 1}, inv_expected={"oak_planks": -2, "stick": -1, "wooden_sword": 1})
test("craft wooden_shovel", {"type": "craft", "item": "wooden_shovel", "count": 1}, inv_expected={"oak_planks": -1, "stick": -2, "wooden_shovel": 1})

# 6. 合成 — 石工具
print("\n═══ 合成:石工具 ═══")
test("craft stone_pickaxe", {"type": "craft", "item": "stone_pickaxe", "count": 1}, inv_expected={"cobblestone": -3, "stick": -2, "stone_pickaxe": 1})
test("craft stone_sword", {"type": "craft", "item": "stone_sword", "count": 1}, inv_expected={"cobblestone": -2, "stick": -1, "stone_sword": 1})

# 7. 合成 — 其他
print("\n═══ 合成:其他 ═══")
test("craft torch", {"type": "craft", "item": "torch", "count": 4}, inv_expected={"stick": -1, "torch": 4})
test("craft furnace", {"type": "craft", "item": "furnace", "count": 1}, inv_expected={"cobblestone": -8, "furnace": 1})
test("craft chest", {"type": "craft", "item": "chest", "count": 1}, inv_expected={"oak_planks": -8, "chest": 1})
test("craft fence", {"type": "craft", "item": "fence", "count": 3}, inv_expected={"oak_planks": -4, "stick": -2, "oak_fence": 3})
test("craft ladder", {"type": "craft", "item": "ladder", "count": 3}, inv_expected={"stick": -7, "ladder": 3})

# 8. 合成 — 护甲
print("\n═══ 合成:护甲 ═══")
test("craft leather_helmet", {"type": "craft", "item": "leather_helmet", "count": 1}, inv_expected={"leather": -5, "leather_helmet": 1})
test("craft leather_boots", {"type": "craft", "item": "leather_boots", "count": 1}, inv_expected={"leather": -4, "leather_boots": 1})

# 9. 交互
print("\n═══ 交互 ═══")
test("mine 60t", {"type": "mine", "ticks": 60})
test("attack 20t", {"type": "attack", "ticks": 20})
test("right_click 5t", {"type": "right_click", "ticks": 5})

# 10. 丢弃
print("\n═══ 丢弃 ═══")
inv = snap()
target = next((k for k in inv["inv"] if k in ("dirt","cobblestone","oak_planks")), "dirt")
test(f"discard {target}", {"type": "discard", "item": target, "num": 1}, inv_expected={target: -1})

# 11. 烧制
print("\n═══ 烧制 ═══")
test("smelt log→charcoal", {"type": "smelt", "item": "oak_log", "num": 1}, inv_expected={"oak_log": -1, "coal": -1, "charcoal": 1})

# 12. 快捷栏
print("\n═══ 快捷栏 ═══")
for i in range(1, 10):
    test(f"press {i}", {"type": "press", "keys": str(i), "ticks": 2})

# 结果
print(f"\n{'═'*40}")
print(f"  ✅ {P} 通过  ❌ {F} 失败  共 {P+F}")
if F == 0: print("  🎉 全部通过！")
