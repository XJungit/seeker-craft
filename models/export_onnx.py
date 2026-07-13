from ultralytics import YOLO

print("[export] 加载 yolov8s-worldv2.pt ...")
model = YOLO("yolov8s-worldv2.pt")

print("[export] 设置类别...")
model.set_classes([
    "tree", "stone", "cow", "creeper", "grass", "dirt", "water",
    "crafting table", "flower", "leaves", "iron ore", "skeleton",
    "zombie", "chicken", "pig", "sheep", "sand", "sugar cane", "wood",
])

print("[export] 导出 ONNX (dynamic, imgsz=640)...")
model.export(format="onnx", imgsz=640, dynamic=True)
print("done")
