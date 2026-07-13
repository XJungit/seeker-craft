"""
用 onnxruntime 直接推理 YOLO-World ONNX 模型，对比有/无 txt_feats 的差异。
"""
import numpy as np
import onnxruntime as ort
from PIL import Image

# 加载 ONNX 模型
sess = ort.InferenceSession("models/yolov8s-worldv2.onnx", providers=["DmlExecutionProvider", "CPUExecutionProvider"])

# 加载截图
img = Image.open("mc_screenshot.png").convert("RGB")
img_resized = img.resize((640, 640), Image.BICUBIC)

# 构建 NCHW 张量
tensor_data = np.array(img_resized, dtype=np.float32) / 255.0  # [H, W, C]
tensor_data = np.transpose(tensor_data, (2, 0, 1))            # [C, H, W]
tensor_data = np.expand_dims(tensor_data, axis=0)             # [1, C, H, W]

# 方案 A：加载 ultralytics 生成的正确嵌入
emb_ultra = np.load("models/mc_classes_emb.npy")  # [19, 512]
emb_ultra_batch = np.expand_dims(emb_ultra, axis=0).astype(np.float32)  # [1, 19, 512]

# 方案 B：用全零嵌入（文本信息被消掉）
emb_zeros = np.zeros((1, 19, 512), dtype=np.float32)

# 方案 C：随机嵌入
emb_random = np.random.randn(1, 19, 512).astype(np.float32)
# 归一化
emb_random = emb_random / np.linalg.norm(emb_random, axis=2, keepdims=True)

for label, emb in [("ultralytics 嵌入", emb_ultra_batch), ("全零嵌入", emb_zeros), ("随机嵌入", emb_random)]:
    outputs = sess.run(["output0"], {"images": tensor_data, "txt_feats": emb})
    output = outputs[0]  # shape (1, 23, 8400)

    num_anchors = output.shape[2]
    num_classes = output.shape[1] - 4

    # 计算所有锚框在每个类的分数
    scores = []
    for a in range(num_anchors):
        best_score = 0.0
        best_cls = 0
        for c in range(num_classes):
            logit = output[0, 4 + c, a]
            score = 1.0 / (1.0 + np.exp(-logit))
            if score > best_score:
                best_score = score
                best_cls = c
        scores.append(best_score)

    scores = np.array(scores)
    print(f"\n{'='*60}")
    print(f"[{label}]")
    print(f"  分数分布: mean={scores.mean():.4f} std={scores.std():.4f}")
    print(f"  分数范围: [{scores.min():.4f}, {scores.max():.4f}]")
    print(f"  最高分锚框 Top-5:")
    idxs = np.argsort(scores)[-5:][::-1]
    for i, idx in enumerate(idxs):
        cx = output[0, 0, idx]
        cy = output[0, 1, idx]
        w = output[0, 2, idx]
        h = output[0, 3, idx]
        cls_scores = [1.0 / (1.0 + np.exp(-output[0, 4+c, idx])) for c in range(num_classes)]
        best_c = np.argmax(cls_scores)
        print(f"    #{i+1}: score={scores[idx]:.4f} class={best_c} bbox=({cx:.1f},{cy:.1f},{w:.1f},{h:.1f})")