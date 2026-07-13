"""Phase 0 验证：在 RTX 3050 上实测 ONNX 检测推理延迟 (CPU vs DirectML)。

模型为合成的小检测骨干代理（非真实 YOLO，但计算量级相近）：
  输入 1x3x640x640 -> 5x[Conv-BN-ReLU] -> GlobalAvgPool -> Gemm -> 输出
用于判断 ort/DirectML 路径在本机是否可行、单帧多少毫秒。
"""
import time
import numpy as np
import onnx
from onnx import helper, TensorProto
import onnxruntime as ort


def build_synthetic_detector(path, in_h=640, in_w=640):
    """构造一个模拟小检测骨干的 ONNX 模型并保存。"""
    nodes = []
    initializers = []
    # 通道序列：3 -> 16 -> 32 -> 64 -> 128 -> 128
    channels = [3, 16, 32, 64, 128, 128]
    cur = "input"
    inp_shape = [1, 3, in_h, in_w]
    # 输入
    inputs = [helper.make_tensor_value_info("input", TensorProto.FLOAT, inp_shape)]
    np.random.seed(0)
    for i in range(len(channels) - 1):
        cin, cout = channels[i], channels[i + 1]
        k = 3
        w = np.random.randn(cout, cin, k, k).astype(np.float32) * 0.1
        b = np.random.randn(cout).astype(np.float32) * 0.1
        # BN 参数
        scale = np.random.rand(cout).astype(np.float32) + 0.5
        bias = np.random.randn(cout).astype(np.float32) * 0.1
        mean = np.random.randn(cout).astype(np.float32) * 0.1
        var = np.random.rand(cout).astype(np.float32) + 0.5
        w_name, b_name = f"w{i}", f"b{i}"
        s_name, bi_name, m_name, v_name = f"s{i}", f"bn_b{i}", f"m{i}", f"v{i}"
        initializers += [
            helper.make_tensor(w_name, TensorProto.FLOAT, [cout, cin, k, k], w.flatten().tolist()),
            helper.make_tensor(b_name, TensorProto.FLOAT, [cout], b.flatten().tolist()),
            helper.make_tensor(s_name, TensorProto.FLOAT, [cout], scale.flatten().tolist()),
            helper.make_tensor(bi_name, TensorProto.FLOAT, [cout], bias.flatten().tolist()),
            helper.make_tensor(m_name, TensorProto.FLOAT, [cout], mean.flatten().tolist()),
            helper.make_tensor(v_name, TensorProto.FLOAT, [cout], var.flatten().tolist()),
        ]
        conv_out = f"conv{i}"
        bn_out = f"bn{i}"
        act_out = f"relu{i}"
        nodes.append(helper.make_node("Conv", [cur, w_name, b_name], [conv_out],
                                      pads=[1, 1, 1, 1], strides=[2, 2]))
        nodes.append(helper.make_node("BatchNormalization",
                                      [conv_out, s_name, bi_name, m_name, v_name], [bn_out],
                                      epsilon=1e-5))
        nodes.append(helper.make_node("Relu", [bn_out], [act_out]))
        cur = act_out
    # 检测头：1x1 卷积输出 84 通道 + 全局平均池化（标准检测头，避免 Gemm 歧义）
    cout = 128
    head_w = np.random.randn(84, cout, 1, 1).astype(np.float32) * 0.1
    head_b = np.random.randn(84).astype(np.float32) * 0.1
    initializers += [
        helper.make_tensor("head_w", TensorProto.FLOAT, [84, cout, 1, 1], head_w.flatten().tolist()),
        helper.make_tensor("head_b", TensorProto.FLOAT, [84], head_b.flatten().tolist()),
    ]
    conv_head = "conv_head"
    nodes.append(helper.make_node("Conv", [cur, "head_w", "head_b"], [conv_head]))
    pool_out = "gap"
    nodes.append(helper.make_node("GlobalAveragePool", [conv_head], [pool_out]))
    # Flatten -> [1,84] 即输出
    nodes.append(helper.make_node("Flatten", [pool_out], ["output"], axis=1))
    graph = helper.make_graph(nodes, "synthetic_detector", inputs,
                              [helper.make_tensor_value_info("output", TensorProto.FLOAT, [1, 84])],
                              initializers)
    model = helper.make_model(graph, opset_imports=[helper.make_opsetid("", 17)])
    onnx.save(model, path)
    return path


def bench(session, feed, n_warm=10, n_run=50):
    for _ in range(n_warm):
        session.run(None, feed)
    ts = []
    for _ in range(n_run):
        t0 = time.perf_counter()
        session.run(None, feed)
        ts.append((time.perf_counter() - t0) * 1000.0)
    ts = np.array(ts)
    return ts.mean(), float(np.median(ts)), ts.min(), ts.max(), ts.std()


def main():
    import os
    here = os.path.dirname(os.path.abspath(__file__))
    model_path = os.path.join(here, "synthetic_detector.onnx")
    build_synthetic_detector(model_path, 640, 640)
    print(f"[模型] {model_path}  (输入 1x3x640x640, 合成小检测骨干)")

    feed = {"input": np.random.randn(1, 3, 640, 640).astype(np.float32)}

    so = ort.SessionOptions()
    so.intra_op_num_threads = 4  # 模拟 agent 不独占全部核心

    results = {}
    for ep in ["CPUExecutionProvider", "DmlExecutionProvider"]:
        try:
            sess = ort.InferenceSession(model_path, so, providers=[ep])
            mean, med, mn, mx, sd = bench(sess, feed)
            results[ep] = (mean, med, mn, mx, sd)
            print(f"[{ep}]  mean={mean:7.2f}ms  median={med:7.2f}ms  min={mn:6.2f}  max={mx:7.2f}  std={sd:5.2f}")
        except Exception as e:
            print(f"[{ep}] 不可用: {e}")

    if "DmlExecutionProvider" in results and "CPUExecutionProvider" in results:
        speedup = results["CPUExecutionProvider"][0] / results["DmlExecutionProvider"][0]
        print(f"\n[DML 加速比 vs CPU]  {speedup:.1f}x  (单帧 DML={results['DmlExecutionProvider'][1]:.2f}ms)")
        # 折算每秒可处理帧数（纯推理，不含截图/VLM）
        fps = 1000.0 / results["DmlExecutionProvider"][1]
        print(f"[吞吐估算] DML 纯推理约 {fps:.0f} 帧/秒（检测这一步不会成为瓶颈）")


if __name__ == "__main__":
    main()
