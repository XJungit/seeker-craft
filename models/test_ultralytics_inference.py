"""用 ultralytics YOLO-World 直接推理 MC 截图，验证模型本身是否工作正常。"""
from ultralytics import YOLO

model = YOLO("yolov8s-worldv2.pt")
model.set_classes(["tree", "stone", "cow", "creeper", "grass", "dirt", "water", "crafting table"])

results = model("mc_screenshot.png", conf=0.01)
r = results[0]
print(f"检测到 {len(r.boxes)} 个目标")
if len(r.boxes) > 0:
    for i, box in enumerate(r.boxes):
        cls_id = int(box.cls[0])
        conf = float(box.conf[0])
        cls_name = r.names[cls_id]
        xyxy = box.xyxy[0].tolist()
        print(f"  #{i+1}: {cls_name} conf={conf:.4f} bbox={xyxy}")
else:
    print("  (无目标)")