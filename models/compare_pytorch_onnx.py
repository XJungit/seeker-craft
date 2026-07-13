"""
对比 PyTorch 和 ONNX 的输出，确认嵌入空间是否匹配。
实验：
  1. PyTorch set_classes → 取出 txt_feats
  2. ONNX 用相同的 txt_feats 做推理（比较输出分布）
  3. ONNX 用旧嵌入做推理（对比）
"""
import numpy as np
import onnxruntime as ort
import torch
from ultralytics import YOLO
from PIL import Image

# ========== 0. 加载截图 ==========
img = Image.open("mc_screenshot.png").convert("RGB")
img_resized = img.resize((640, 640), Image.BICUBIC)
tensor_data = np.array(img_resized, dtype=np.float32) / 255.0
tensor_data = np.transpose(tensor_data, (2, 0, 1))
tensor_data = np.expand_dims(tensor_data, axis=0).astype(np.float32)

# ========== 1. PyTorch 模型 ==========
print("=" * 60)
print("1. PyTorch 模型推理")
device = "cuda" if torch.cuda.is_available() else "cpu"
model = YOLO("yolov8s-worldv2.pt")
CLASSES = [
    "tree", "oak tree", "stone", "coal ore", "iron ore", "diamond ore",
    "cow", "sheep", "pig", "zombie", "creeper", "grass", "dirt", "water",
    "wood", "log", "leaves", "flower", "crafting table",
]
NUM_CLASSES = len(CLASSES)
model.set_classes(CLASSES)

# set_classes 内部生成的 txt_feats
txt_feats_pt = model.model.txt_feats  # Tensor [1, N, 512]
print(f"txt_feats shape: {txt_feats_pt.shape}")
print(f"  range=[{txt_feats_pt.min().item():.4f}, {txt_feats_pt.max().item():.4f}]")
print(f"  mean={txt_feats_pt.mean().item():.4f} std={txt_feats_pt.std().item():.4f}")

# 保存
txt_feats_np = txt_feats_pt.detach().cpu().numpy().astype(np.float32)

# PyTorch 推理
results = model("mc_screenshot.png", conf=0.01)
r = results[0]
print(f"PyTorch 检测: {len(r.boxes)} 个目标")
if len(r.boxes) > 0:
    for i, box in enumerate(r.boxes):
        cls_id = int(box.cls[0])
        conf = float(box.conf[0])
        print(f"  #{i+1}: {r.names[cls_id]} conf={conf:.4f}")
else:
    print("  (无目标)")
    # 从模型拿原始输出分布
    import torch.nn.functional as F
    model.model.eval()
    with torch.no_grad():
        x = torch.from_numpy(tensor_data).to(device)
        txt = txt_feats_pt.to(device)
        # WorldModel 的 forward 签名是 (x, txt_feats, ...)
        pred = model.model(x, txt_feats=txt, verbose=False)
        if isinstance(pred, (list, tuple)):
            pred = pred[0]
        # pred: [1, 4+N, 8400]
        pred_np = pred.detach().cpu().numpy()
        scores_pt = []
        for a in range(pred_np.shape[2]):
            best = 0.0
            for c in range(NUM_CLASSES):
                logit = pred_np[0, 4 + c, a]
                score = 1.0 / (1.0 + np.exp(-logit))
                if score > best:
                    best = score
            scores_pt.append(best)
        scores_pt = np.array(scores_pt)
        print(f"  输出分数: mean={scores_pt.mean():.4f} std={scores_pt.std():.4f}")
        print(f"  范围: [{scores_pt.min():.4f}, {scores_pt.max():.4f}]")

# ========== 2. ONNX 用相同的 txt_feats ==========
print("\n" + "=" * 60)
print("2. ONNX 用 PyTorch 的 txt_feats")
sess = ort.InferenceSession("models/yolov8s-worldv2.onnx", providers=["CPUExecutionProvider"])

outputs = sess.run(["output0"], {"images": tensor_data, "txt_feats": txt_feats_np})
output_onnx = outputs[0]

scores_onnx = []
for a in range(output_onnx.shape[2]):
    best = 0.0
    for c in range(NUM_CLASSES):
        logit = output_onnx[0, 4 + c, a]
        score = 1.0 / (1.0 + np.exp(-logit))
        if score > best:
            best = score
    scores_onnx.append(best)
scores_onnx = np.array(scores_onnx)
print(f"得分分布: mean={scores_onnx.mean():.4f} std={scores_onnx.std():.4f}")
print(f"范围: [{scores_onnx.min():.4f}, {scores_onnx.max():.4f}]")

# Top-10
idxs = np.argsort(scores_onnx)[-10:][::-1]
print("Top-10:")
for i, idx in enumerate(idxs):
    cls_scores = [1.0/(1.0+np.exp(-output_onnx[0,4+c,idx])) for c in range(NUM_CLASSES)]
    best_c = int(np.argmax(cls_scores))
    print(f"  #{i+1}: score={scores_onnx[idx]:.4f} class={best_c}")

# ========== 3. 对比之前文件嵌入 ==========
print("\n" + "=" * 60)
print("3. ONNX 用旧嵌入文件")
old_emb = np.load("models/mc_classes_emb.npy")  # [19, 512], 需要加上 batch 维度
old_emb_batch = np.expand_dims(old_emb, axis=0).astype(np.float32)
outputs_old = sess.run(["output0"], {"images": tensor_data, "txt_feats": old_emb_batch})
output_old = outputs_old[0]

scores_old = []
for a in range(output_old.shape[2]):
    best = 0.0
    for c in range(NUM_CLASSES):
        logit = output_old[0, 4 + c, a]
        score = 1.0 / (1.0 + np.exp(-logit))
        if score > best:
            best = score
    scores_old.append(best)
scores_old = np.array(scores_old)
print(f"得分: mean={scores_old.mean():.4f} std={scores_old.std():.4f}")
print(f"范围: [{scores_old.min():.4f}, {scores_old.max():.4f}]")

# ========== 4. 嵌入数值对比 ==========
print("\n" + "=" * 60)
print("4. 嵌入数值对比")
old_sq = old_emb  # [19, 512]
new_sq = txt_feats_np.squeeze(0)  # [19, 512]
diff = np.abs(new_sq - old_sq)
print(f"新: range=[{new_sq.min():.4f},{new_sq.max():.4f}] mean={new_sq.mean():.4f} std={new_sq.std():.4f}")
print(f"旧: range=[{old_sq.min():.4f},{old_sq.max():.4f}] mean={old_sq.mean():.4f} std={old_sq.std():.4f}")
print(f"差值: mean={diff.mean():.6f} max={diff.max():.6f}")
print(f"完全相同: {np.allclose(new_sq, old_sq, atol=1e-5)}")

# ========== 5. PyTorch vs ONNX 输出对比 ==========
print("\n" + "=" * 60)
print("5. PyTorch vs ONNX (同 txt_feats) 输出数值对比")
if 'scores_pt' in dir():
    corr = np.corrcoef(scores_pt, scores_onnx)[0,1]
    print(f"相关系数: {corr:.4f}")

    # 对全输出做逐元素比较
    if 'pred_np' in dir() and pred_np.shape == output_onnx.shape:
        diff_out = np.abs(pred_np - output_onnx)
        print(f"输出逐元素差异: mean={diff_out.mean():.6f} max={diff_out.max():.6f}")