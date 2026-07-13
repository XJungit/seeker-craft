#!/usr/bin/env python3
"""
VLM 速度基准：Agnes vs MiniCPM-V
- 同一张 MC 截图、同一个 prompt、同样 max_tokens
- 交替打两个端点（消除时段偏置），多轮取统计
- 记录端到端墙钟延迟 + 输出 token/字符数
两端均为 OpenAI 兼容 /chat/completions，图走 base64 data URI。
"""
import base64
import json
import statistics
import sys
import time
import urllib.request
import urllib.error

IMG = sys.argv[1] if len(sys.argv) > 1 else "D:/Craft-Agent/phase0_verify/enigo_mc_test/mc_capture.png"
ROUNDS = int(sys.argv[2]) if len(sys.argv) > 2 else 5
MAX_TOKENS = 700  # 放开以容纳 Agnes 的 reasoning tokens，保证 content 非空
TIMEOUT = 180

PROMPT = (
    "这是一张 Minecraft 游戏截图。请以游戏 AI agent 的视角简洁分点回答："
    "1) 当前是什么界面；2) 画面里有哪些可交互 UI 元素及大致方位；"
    "3) 玩家状态（血量/饥饿/物品栏/准星）；4) 若目标是砍树，下一步做什么。"
)

ENDPOINTS = [
    {
        "name": "Agnes-flash(关思考)",
        "url": "https://apihub.agnes-ai.com/v1/chat/completions",
        "model": "agnes-2.0-flash",
        "key": "sk-REDACTED_FROM_HISTORY",
        # Agnes 是推理型模型，关思考以求实时速度（本轮重点验证）
        "extra": {"chat_template_kwargs": {"enable_thinking": False}},
    },
    {
        "name": "MiniCPM-V-4.6",
        "url": "https://api.modelbest.co/v1/chat/completions",
        "model": "MiniCPM-V-4.6-Instruct",
        "key": "lis_sk_298cf78155f231c7_DkrDcNLHnK8dJRnfFrJCd4JGDbBLMkHrC3T-wLpvC9zy0BPemsyFuQ",
    },
]


def load_data_uri(path):
    with open(path, "rb") as f:
        b = f.read()
    return "data:image/png;base64," + base64.b64encode(b).decode(), len(b)


def call(ep, data_uri):
    payload = {
        "model": ep["model"],
        "messages": [
            {
                "role": "user",
                "content": [
                    {"type": "text", "text": PROMPT},
                    {"type": "image_url", "image_url": {"url": data_uri}},
                ],
            }
        ],
        "temperature": 0.2,
        "max_tokens": MAX_TOKENS,
    }
    if ep.get("extra"):
        payload.update(ep["extra"])
    req = urllib.request.Request(
        ep["url"],
        data=json.dumps(payload).encode(),
        headers={"Authorization": "Bearer " + ep["key"], "Content-Type": "application/json"},
        method="POST",
    )
    t0 = time.perf_counter()
    try:
        r = urllib.request.urlopen(req, timeout=TIMEOUT)
        j = json.loads(r.read().decode())
        dt = time.perf_counter() - t0
        msg = j["choices"][0]["message"]
        content = msg.get("content") or ""
        if not content:  # 推理型模型 content 可能为空，兜底用 reasoning_content
            content = msg.get("reasoning_content") or ""
        usage = j.get("usage", {})
        rtok = (usage.get("completion_tokens_details") or {}).get("reasoning_tokens")
        return {
            "ok": True,
            "dt": dt,
            "chars": len(content),
            "ctok": usage.get("completion_tokens"),
            "rtok": rtok,
            "ptok": usage.get("prompt_tokens"),
            "sample": content[:80].replace("\n", " "),
        }
    except urllib.error.HTTPError as e:
        return {"ok": False, "dt": time.perf_counter() - t0, "err": f"HTTP {e.code}: {e.read().decode()[:200]}"}
    except Exception as e:
        return {"ok": False, "dt": time.perf_counter() - t0, "err": f"{type(e).__name__}: {e}"}


def main():
    data_uri, nbytes = load_data_uri(IMG)
    print(f"图片: {IMG}  ({nbytes} bytes, base64≈{len(data_uri)} chars)")
    print(f"轮数: {ROUNDS}  max_tokens: {MAX_TOKENS}  prompt字数: {len(PROMPT)}\n")

    results = {ep["name"]: [] for ep in ENDPOINTS}
    for rnd in range(1, ROUNDS + 1):
        for ep in ENDPOINTS:
            r = call(ep, data_uri)
            results[ep["name"]].append(r)
            if r["ok"]:
                ctok = r["ctok"] if r["ctok"] is not None else "?"
                rtok = f" 推理={r['rtok']}" if r.get("rtok") else ""
                print(f"[R{rnd}] {ep['name']:<18} {r['dt']:6.2f}s  出={r['chars']:>3}字 tok={ctok}{rtok}  \"{r['sample']}...\"")
            else:
                print(f"[R{rnd}] {ep['name']:<18} {r['dt']:6.2f}s  失败: {r['err']}")

    print("\n" + "=" * 70)
    print(f"{'模型':<18} {'成功':>4} {'中位延迟':>9} {'均值':>8} {'最快':>7} {'最慢':>7} {'均tok/s':>9}")
    print("-" * 70)
    for name, rs in results.items():
        ok = [r for r in rs if r["ok"]]
        if not ok:
            print(f"{name:<18}   0/{len(rs)}  全部失败")
            continue
        dts = [r["dt"] for r in ok]
        toks = [r["ctok"] for r in ok if r["ctok"]]
        tps = ""
        if toks:
            tps_vals = [r["ctok"] / r["dt"] for r in ok if r["ctok"]]
            tps = f"{statistics.mean(tps_vals):.1f}"
        print(
            f"{name:<18} {len(ok):>2}/{len(rs)} "
            f"{statistics.median(dts):>8.2f}s {statistics.mean(dts):>7.2f}s "
            f"{min(dts):>6.2f}s {max(dts):>6.2f}s {tps:>9}"
        )
    print("=" * 70)
    print("注：第 1 轮含冷启动，实时 agent 稳态更接近中位/最快值。")


if __name__ == "__main__":
    main()
