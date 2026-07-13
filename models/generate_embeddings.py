#!python
"""
用 YOLO-World 官方文本编码器（open_clip ViT-B-32）生成 MC 类别文本嵌入。

YOLO-World 论文中使用的 CLIP 文本编码器是 open_clip 的 ViT-B-32 模型，
权重来自 laion2b_s34b_b79k 预训练。

两种方法任选其一：
1. open_clip 直接生成（轻量，推荐）
2. ultralytics YOLO-World 模型内置编码器

用法：
  python models/generate_embeddings.py
"""

import numpy as np

import os

# MC 类别列表（与 detect.rs 中 MC_CLASSES 一一对应）
MC_CLASSES = [
    "tree", "oak tree", "stone", "coal ore", "iron ore", "diamond ore",
    "cow", "sheep", "pig", "zombie", "creeper", "grass", "dirt", "water",
    "wood", "log", "leaves", "flower", "crafting table",
]

OUTPUT_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "mc_classes_emb.npy")


def method_open_clip():
    """
    方法 1：open_clip ViT-B-32（YOLO-World 官方指定版本）
    
    YOLO-World 论文 §3.1："We adopt a ViT-B/32 based CLIP text encoder
    with the Laion2b-s34b-b79k pretrained checkpoint."
    """
    import open_clip
    import torch

    device = "cuda" if torch.cuda.is_available() else "cpu"
    print(f"[open_clip] 加载 CLIP ViT-B-32 (laion2b_s34b_b79k)...")
    model, _, _ = open_clip.create_model_and_transforms(
        "ViT-B-32", pretrained="laion2b_s34b_b79k", device=device
    )
    tokenizer = open_clip.get_tokenizer("ViT-B-32")

    texts = tokenizer(MC_CLASSES).to(device)
    with torch.no_grad(), torch.cuda.amp.autocast(enabled=(device == "cuda")):
        # YOLO-World 使用的是 CLIP 文本编码器的输出（不做最终投影）
        text_features = model.encode_text(texts)
        # 正常 CLIP 有 projection 层，但 YOLO-World 用的是编码器原始输出
        # 实际上 YOLO-World 论文说它用了 CLIP 文本编码器，
        # 并且在训练过程中会微调文本编码器。
        # 更准确的做法是查看 ultralytics YOLO-World 的源码。
        # 这里我们先用 encode_text 的输出。
        pass

    emb = text_features.detach().cpu().numpy().astype(np.float32)
    # L2 归一化
    norms = np.linalg.norm(emb, axis=1, keepdims=True)
    emb = emb / norms

    _report(emb, "open_clip")
    return emb


def method_ultralytics():
    """
    方法 2：从 ultralytics YOLO-World 权重提取文本编码器输出。

    加载 ultralytics 的预训练 .pt 权重，使用模型内部封装的 CLIP 文本编码器。
    这保证了与 YOLO-World 训练时使用的编码器完全一致。
    """
    from ultralytics import YOLO
    import torch

    device = "cuda" if torch.cuda.is_available() else "cpu"
    print(f"[ultralytics] 加载 YOLO-World v2 模型...")
    
    model = YOLO("yolov8s-worldv2.pt")
    model.to(device)

    # Ultralytics YOLO-World 用 WorldModel.set_classes 来注册类别
    # 这会在内部调用 CLIP 文本编码器生成嵌入
    model.set_classes(MC_CLASSES)

# 提取文本嵌入
    # ultralytics 内部：WorldModel.txt_feats -> torch.Tensor (1, num_classes, 512)
    txt_feats = model.model.txt_feats
    emb = txt_feats.detach().cpu().numpy().astype(np.float32)

    # 去掉 batch 维度: (1, 19, 512) -> (19, 512)
    if emb.ndim == 3:
        emb = emb.squeeze(0)

    # 检查是否已归一化
    norms = np.linalg.norm(emb, axis=1, keepdims=True)
    print(f"[ultralytics] 嵌入是否已 L2 归一化: {np.allclose(norms, 1.0, atol=1e-5)}")
    if not np.allclose(norms, 1.0, atol=1e-5):
        emb = emb / norms
        
    _report(emb, "ultralytics")
    return emb


def _report(emb: np.ndarray, tag: str):
    """打印嵌入质量报告"""
    print(f"\n{'='*60}")
    print(f"[{tag}] 嵌入报告")
    print(f"{'='*60}")
    print(f"  形状: {emb.shape}")
    print(f"  数值范围: [{emb.min():.6f}, {emb.max():.6f}]")
    print(f"  行范数: min={np.linalg.norm(emb, axis=1).min():.4f}  "
          f"max={np.linalg.norm(emb, axis=1).max():.4f}")

    # 余弦相似度矩阵
    sim = emb @ emb.T
    diag = np.diag(sim)
    mask = ~np.eye(sim.shape[0], dtype=bool)
    off_diag = sim[mask]

    print(f"  自相似度(对角线): {diag.round(4)}")
    print(f"  类间相似度均值: {off_diag.mean():.4f}")
    print(f"  类间相似度最大值: {off_diag.max():.4f}")
    print(f"  类间相似度最小值: {off_diag.min():.4f}")

    # 类间相似度过高说明嵌入区分度差
    if off_diag.mean() > 0.90:
        print(f"  ⚠️  警告：类间相似度均值 {off_diag.mean():.3f} > 0.90，嵌入区分度极差！")
    elif off_diag.mean() > 0.80:
        print(f"  ⚡ 注意：类间相似度均值 {off_diag.mean():.3f} > 0.80，区分度一般。")
    else:
        print(f"  ✅ 类间相似度均值 {off_diag.mean():.3f}，区分度合理。")


def main():
    import sys
    print("=" * 60)
    print("MC 类别文本嵌入生成器（YOLO-World 专用）")
    print("=" * 60)
    print(f"类别 ({len(MC_CLASSES)}): {MC_CLASSES}")
    print(f"输出: {OUTPUT_PATH}")
    print()

    method = sys.argv[1] if len(sys.argv) > 1 else "ultralytics"

    if method == "open_clip":
        emb = method_open_clip()
    elif method == "ultralytics":
        emb = method_ultralytics()
    else:
        print(f"未知方法: {method}，可选: open_clip, ultralytics")
        sys.exit(1)

    # 保存
    os.makedirs(os.path.dirname(OUTPUT_PATH), exist_ok=True)
    np.save(OUTPUT_PATH, emb)
    print(f"\n✅ 已保存到 {OUTPUT_PATH}")

    # 验证回读
    loaded = np.load(OUTPUT_PATH)
    assert loaded.shape == (len(MC_CLASSES), 512), f"形状错误: {loaded.shape}"
    assert loaded.dtype == np.float32, f"类型错误: {loaded.dtype}"
    print(f"✅ 验证通过: shape={loaded.shape}, dtype={loaded.dtype}")


if __name__ == "__main__":
    main()