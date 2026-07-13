"""用原始 CLIP ViT-B/32 (openai/clip-vit-base-patch32) 生成文本嵌入。"""
import numpy as np
from transformers import CLIPTextModel, CLIPTokenizer

MC_CLASSES = [
    "tree", "stone", "cow", "creeper", "grass", "dirt", "water",
    "crafting table", "flower", "leaves", "iron ore", "skeleton",
    "zombie", "chicken", "pig", "sheep", "sand", "sugar cane", "wood",
]

print("加载 CLIP ViT-B/32...")
model = CLIPTextModel.from_pretrained("openai/clip-vit-base-patch32")
tokenizer = CLIPTokenizer.from_pretrained("openai/clip-vit-base-patch32")

embeddings = []
for cls in MC_CLASSES:
    inputs = tokenizer(cls, return_tensors="pt", padding=True)
    outputs = model(**inputs)
    emb = outputs.pooler_output.detach().numpy()  # (1, 512)
    emb = emb / np.linalg.norm(emb, axis=1, keepdims=True)  # L2 normalize
    embeddings.append(emb[0])

emb = np.stack(embeddings)  # (19, 512)
print(f"嵌入形状: {emb.shape}")

# 质量检查
sim = emb @ emb.T
mask = np.eye(19, dtype=bool)
off_diag = sim[~mask]
print(f"  自相似度: 1.0")
print(f"  类间相似度 — mean={off_diag.mean():.4f}  max={off_diag.max():.4f}  min={off_diag.min():.4f}")

np.save("mc_classes_emb.npy", emb)
print(f"✅ 已保存 mc_classes_emb.npy ({emb.shape})")
