#!/usr/bin/env python3
"""
MiniCPM-V 缩放 vs 不缩放 速率基准
- 同一张 MC 截图，client 端用 PIL(LANCZOS) 把最长边缩到 768（与 Rust downscale_png 的 Lanczos3 等价）
- 交替打 MiniCPM 端点（消除时段偏置），多轮取统计
- 记录端到端墙钟延迟 + payload(base64)大小 + 输出字符数
端点：OpenAI 兼容 /chat/completions，图走 base64 data URI。
key：MINICPM_API_KEY 环境变量，缺省回退到面壁官方文档公开试用 key。
"""
import base64
import io
import json
import os
import statistics
import sys
import time
import urllib.error
import urllib.request
from PIL import Image

IMG = sys.argv[1] if len(sys.argv) > 1 else "phase0_verify/enigo_mc_test/mc_capture.png"
ROUNDS = int(sys.argv[2]) if len(sys.argv) > 2 else 5
MAX_SIDE = int(sys.argv[3]) if len(sys.argv) > 3 else 768
MAX_TOKENS = 512
TIMEOUT = 120

PROMPT = (
    "这是一张 Minecraft 游戏截图。请以游戏 AI agent 视角简洁分点回答："
    "1) 当前是什么界面；2) 画面里有哪些可交互 UI 元素及大致方位；"
    "3) 玩家状态（血量/饥饿/物品栏/准星）；4) 若目标是砍树，下一步做什么。"
)

URL = "https://api.modelbest.co/v1/chat/completions"
MODEL = "MiniCPM-V-4.6-Instruct"
KEY = os.environ.get(
    "MINICPM_API_KEY",
    "lis_sk_298cf78155f231c7_DkrDcNLHnK8dJRnfFrJCd4JGDbBLMkHrC3T-wLpvC9zy0BPemsyFuQ",
)


def load_png(path):
    with open(path, "rb") as f:
        return f.read()


def downscale_png(png_bytes, max_side):
    im = Image.open(io.BytesIO(png_bytes)).convert("RGB")
    w, h = im.size
    if max_side and max(w, h) > max_side:
        scale = max_side / max(w, h)
        im = im.resize((int(w * scale + 0.5), int(h * scale + 0.5)), Image.LANCZOS)
    buf = io.BytesIO()
    im.save(buf, format="PNG")
    return buf.getvalue()


def to_data_uri(png_bytes):
    return "data:image/png;base64," + base64.b64encode(png_bytes).decode(), len(png_bytes)


def call(data_uri):
    payload = {
        "model": MODEL,
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
    req = urllib.request.Request(
        URL,
        data=json.dumps(payload).encode(),
        headers={"Authorization": "Bearer " + KEY, "Content-Type": "application/json"},
        method="POST",
    )
    t0 = time.perf_counter()
    try:
        r = urllib.request.urlopen(req, timeout=TIMEOUT)
        j = json.loads(r.read().decode())
        dt = time.perf_counter() - t0
        msg = j["choices"][0]["message"]
        content = msg.get("content") or ""
        usage = j.get("usage", {})
        return {
            "ok": True,
            "dt": dt,
            "chars": len(content),
            "ptok": usage.get("prompt_tokens"),
            "ctok": usage.get("completion_tokens"),
            "sample": content[:60].replace("\n", " "),
        }
    except urllib.error.HTTPError as e:
        return {"ok": False, "dt": time.perf_counter() - t0, "err": f"HTTP {e.code}: {e.read().decode()[:150]}"}
    except Exception as e:  # noqa: BLE001
        return {"ok": False, "dt": time.perf_counter() - t0, "err": f"{type(e).__name__}: {e}"}


def main():
    raw = load_png(IMG)
    full_uri, full_n = to_data_uri(raw)
    scaled = downscale_png(raw, MAX_SIDE)
    scale_uri, scale_n = to_data_uri(scaled)
    print(f"图片: {IMG}")
    print(f"  原图: {Image.open(io.BytesIO(raw)).size}  PNG={full_n}B  base64≈{len(full_uri)}ch")
    print(f"  缩放: {Image.open(io.BytesIO(scaled)).size}  PNG={scale_n}B  base64≈{len(scale_uri)}ch")
    print(f"轮数={ROUNDS}  max_side={MAX_SIDE}  max_tokens={MAX_TOKENS}\n")

    full_r = []
    scale_r = []
    for rnd in range(1, ROUNDS + 1):
        r = call(full_uri)
        full_r.append(r)
        print(
            f"[R{rnd}] 不缩放 {r['dt']:6.2f}s  "
            + (f"{r['chars']}字 ptok={r['ptok']} \"{r['sample']}...\"" if r["ok"] else f"失败 {r['err']}")
        )
        r = call(scale_uri)
        scale_r.append(r)
        print(
            f"[R{rnd}] 缩放   {r['dt']:6.2f}s  "
            + (f"{r['chars']}字 ptok={r['ptok']} \"{r['sample']}...\"" if r["ok"] else f"失败 {r['err']}")
        )

    print("\n" + "=" * 70)
    print(f"{'模式':<10}{'成功':>5}{'中位':>9}{'均值':>8}{'最快':>7}{'最慢':>7}{'均tok/s':>9}{'省payload':>10}")
    print("-" * 70)
    for name, rs, n in [("不缩放", full_r, full_n), ("缩放", scale_r, scale_n)]:
        ok = [r for r in rs if r["ok"]]
        if not ok:
            print(f"{name:<10} 0/{len(rs)} 全部失败")
            continue
        dts = [r["dt"] for r in ok]
        tpsv = [r["ctok"] / r["dt"] for r in ok if r["ctok"]]
        tps = f"{statistics.mean(tpsv):.1f}" if tpsv else ""
        print(
            f"{name:<10}{len(ok):>3}/{len(rs)} {statistics.median(dts):>7.2f}s "
            f"{statistics.mean(dts):>7.2f}s {min(dts):>6.2f}s {max(dts):>6.2f}s {tps:>9} "
            f"{100 * (1 - n / full_n):>8.1f}%"
        )
    print("=" * 70)
    fd = [r["dt"] for r in full_r if r["ok"]]
    sd = [r["dt"] for r in scale_r if r["ok"]]
    if fd and sd:
        imp = statistics.median(fd) / statistics.median(sd)
        print(
            f"中位延迟 不缩放/缩放 = {imp:.2f}×  → 缩放比不缩放"
            f"{'快' if imp > 1 else '慢'} {abs(imp - 1) * 100:.0f}%"
        )
        print("注：第 1 轮含冷启动；稳态更接近中位/最快值。")


if __name__ == "__main__":
    main()
