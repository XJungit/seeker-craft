# Phase 0 验证报告（第一部分：本机可自动验证项）

生成日期：2026-07-12 ｜ 机器：AMD Ryzen 5 6600H / 16GB / RTX 3050 4GB (驱动 610.62)

## 1. 待验证的不确定项与目标

纯视觉游戏 agent 有三个工程风险点，按风险排序：
1. **enigo 能否驱动 MC 视角旋转**（raw input 相对移动）— 最高风险，需真机
2. **xcap 能否截到 MC 窗口并定位坐标基准** — 需真机
3. **ort 本地推理在 RTX 3050 上是否可行、多快** — 本机可自动验证 ✅

本报告覆盖第 3 项（已钉死）。

## 2. 环境探针结果

| 项 | 结果 |
|---|---|
| Rust 工具链 | cargo 1.95.0 / rustc 1.95.0 ✅ |
| NVIDIA GPU | RTX 3050 Laptop GPU, 4096 MiB, 驱动 610.62（nvidia-smi 可见）✅ |
| CUDA 工具链 | 未单独确认（显示驱动在，但 cuDNN 等不一定装）⚠️ |
| onnxruntime (Python) | 经 `onnxruntime-directml 1.24.4` 安装，可用 EP：`DmlExecutionProvider`, `CPUExecutionProvider` ✅ |

**关键发现**：DirectML 后端**无需 CUDA 工具链/cuDNN** 即可在 Windows 上用 GPU 推理。
对 Rust `ort` 而言，DirectML 是 Windows 首选 EP（feature = `directml`），与本次验证一致。

## 3. ONNX 检测推理延迟实测

**方法**：用 `onnx` 构造合成小检测骨干（输入 1×3×640×640 → 5×[Conv-BN-ReLU, stride2] →
1×1 卷积头 84 通道 → 全局平均池化 → Flatten），模拟真实小检测模型的计算量级；
用 onnxruntime 分别走 CPU（4 线程）与 DmlExecutionProvider，warmup 10 + 实测 50 次。

> 注：合成模型非真实 YOLO，但卷积量级相近，用于判断"本地 GPU 推理是否可行、延迟量级"。

| 后端 | mean | median | min | max | std | 吞吐 |
|---|---|---|---|---|---|---|
| CPU (4 线程) | 3.16 ms | 2.97 ms | 2.36 | 4.43 | 0.55 | ~336 fps |
| **DML (RTX 3050)** | **1.64 ms** | **1.52 ms** | 1.30 | 3.83 | 0.45 | **~660 fps** |

**DML 相对 CPU 加速：1.9×。**

## 4. 结论

- ✅ **ort + DirectML 在 RTX 3050 上完全可用**，单帧检测 ~1.5ms，无需装 CUDA 工具链。
- ✅ **检测推理不会成为瓶颈**。即使换成更重的真实 YOLO（量级大几倍到十几倍），
  单帧仍在十几~几十 ms，远快于 VLM/LLM 的云端往返（数百 ms~秒级）。
- ✅ 路线可行：本地只跑轻量检测（DML 加速），重理解走云端 API，符合方案设计。
- ⚠️ 4GB 显存**不足以本地跑 7B 量级 VLM**——但本方案本就不本地跑 VLM，无影响。

## 5. 下一步

- 第 1、2 项（enigo 视角 / xcap 截图）需在**真实运行 MC** 时由人眼验证，
  见 `MC_VERIFY_CHECKLIST.md`，配套脚手架 `enigo_mc_test/`。
- 两项通过后进入 Phase 1（2D 界面点编号合成）。
