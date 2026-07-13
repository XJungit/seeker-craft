import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.patches import FancyBboxPatch, FancyArrowPatch

plt.rcParams["font.family"] = ["Microsoft YaHei", "SimHei", "SimSun", "DejaVu Sans"]
plt.rcParams["axes.unicode_minus"] = False

BG = "#0d1117"
LAYER = "#161b22"
TITLE_C = "#e6edf3"
SUB_C = "#8b949e"
ARR = "#6e7681"
WM_C = "#d29922"
GAME_C = "#2f81f7"
GND_C = "#3fb950"
DEC_C = "#8957e5"

fig, ax = plt.subplots(figsize=(12.6, 9.8))
fig.patch.set_facecolor(BG)
ax.set_facecolor(BG)
ax.axis("off")
ax.set_xlim(0, 13)
ax.set_ylim(0, 11)


def box(cx, cy, w, h, title, sub, edge, fs=12, sub_c=SUB_C):
    b = FancyBboxPatch((cx - w / 2, cy - h / 2), w, h,
                       boxstyle="round,pad=0.02,rounding_size=0.09",
                       linewidth=1.7, edgecolor=edge, facecolor=LAYER)
    ax.add_patch(b)
    ax.text(cx, cy + (0.16 if sub else 0), title, ha="center", va="center",
            color=TITLE_C, fontsize=fs, fontweight="bold")
    if sub:
        ax.text(cx, cy - 0.24, sub, ha="center", va="center",
                color=sub_c, fontsize=8.8)


def arrow(x1, y1, x2, y2, color=ARR, lw=1.8, style="-|>", rad=0, ls="-"):
    ax.add_patch(FancyArrowPatch((x1, y1), (x2, y2), arrowstyle=style,
        mutation_scale=16, color=color, lw=lw, linestyle=ls,
        connectionstyle=f"arc3,rad={rad}"))


ax.text(6.5, 10.55, "通用游戏 Agent — 纯视觉闭环架构",
        ha="center", color=TITLE_C, fontsize=16, fontweight="bold")
ax.text(6.5, 10.15, "VLM 识别  →  LLM 决策  →  键鼠控制   （不碰游戏 API / 内存）",
        ha="center", color=SUB_C, fontsize=11)
ax.text(6.5, 9.78, "Rust 主框架（ONNX 推理 + HTTP API） · 世界模型接口预留",
        ha="center", color="#8b949e", fontsize=9.2)

# 顶：游戏画面
box(6.5, 9.3, 4.6, 0.82, "Minecraft 画面", "窗口化固定分辨率", GAME_C, fs=13)

# 截图
box(6.5, 8.05, 3.0, 0.7, "截图采集 (xcap)", "", GAME_C, fs=12)
arrow(6.5, 9.3 - 0.41, 6.5, 8.05 + 0.35)

# 感知双支路：VLM + Grounding
box(3.4, 6.7, 4.0, 1.0, "VLM 视觉理解", "agnes-vision / GPT-4V\n场景语义描述", "#2f81f7", fs=12)
box(9.6, 6.7, 4.2, 1.0, "Grounding 定位", "Set-of-Mark 标编号①②③\nort 检测 + imageproc 画框", GND_C, fs=12)
arrow(6.5 - 0.9, 8.05 - 0.35, 3.9, 6.7 + 0.5)
arrow(6.5 + 0.9, 8.05 - 0.35, 9.2, 6.7 + 0.5)

# 世界状态
box(6.5, 5.25, 5.6, 0.92, "统一世界状态 WorldState", "场景描述 + 标记元素表 + 检测目标 + HUD", TITLE_C, fs=12)
arrow(3.4, 6.7 - 0.5, 5.3, 5.25 + 0.46)
arrow(9.6, 6.7 - 0.5, 7.7, 5.25 + 0.46)

# 记忆 / 规划 / 决策 竖链
box(6.5, 4.0, 5.0, 0.82, "记忆层  情景 / 语义 / 技能库", "qdrant 向量检索", "#4c8dff", fs=12)
box(6.5, 2.9, 5.0, 0.82, "规划层  目标 → 子目标 → 动作", "层次分解 + 自动课程", "#6e7bff", fs=12)
box(6.5, 1.8, 5.0, 0.88, "决策层 LLM  选编号 / 动作", "Critic 自验证 · 防幻觉乱点", DEC_C, fs=12)
arrow(6.5, 5.25 - 0.46, 6.5, 4.0 + 0.41)
arrow(6.5, 4.0 - 0.41, 6.5, 2.9 + 0.41)
arrow(6.5, 2.9 - 0.41, 6.5, 1.8 + 0.44)

# 执行层
box(6.5, 0.62, 5.6, 0.8, "执行层  键鼠控制 (enigo)", "点击编号 · 转视角对准 · 挖掘/合成", "#db6d28", fs=12)
arrow(6.5, 1.8 - 0.44, 6.5, 0.62 + 0.4)

# 执行 -> 游戏（右侧大回环）
arrow(6.5 + 2.8, 0.62, 11.9, 9.3, color="#db6d28", lw=1.9, rad=-0.42)
ax.text(12.4, 5.0, "动作\n注入", ha="center", va="center", color="#db6d28", fontsize=9)

# 游戏 -> 反思 -> 记忆（左侧回环）
box(1.35, 3.3, 1.9, 1.0, "反思层", "成败评估\n回写记忆", "#f0883e", fs=11)
arrow(4.2, 9.3, 1.35, 3.3 + 0.5, color=GAME_C, lw=1.7, rad=0.4)
ax.text(0.75, 6.6, "画面\n反馈", ha="center", va="center", color=GAME_C, fontsize=9)
arrow(1.35, 3.3 - 0.5, 4.0, 4.0, color="#f0883e", lw=1.6, rad=-0.2)

# 世界模型（预留，右下虚线）
box(11.4, 2.4, 2.5, 1.1, "世界模型(预留)", "DreamerV3 / V-JEPA2\nGenie", WM_C, fs=10)
arrow(11.4 - 1.25, 2.7, 6.5 + 2.5, 2.9, color=WM_C, lw=1.3, ls="--")
ax.text(9.9, 2.15, "想象 / 校验", ha="center", color=WM_C, fontsize=8.5)

plt.savefig("D:/Craft-Agent/architecture.png", dpi=160, facecolor=BG, bbox_inches="tight")
print("saved")
