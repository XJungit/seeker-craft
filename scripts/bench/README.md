# SeekerCraft Bench Runner

一键复现工具层基准：编译 azalea_probe，连接 MC 服务器，顺序执行 `scripts/probe/`
下全部（或指定的）probe 脚本，汇总各脚本执行状态。供评审自助复现
[docs/benchmarks.md](../docs/benchmarks.md) 中工具层验证数据。

## 前置条件

- MC Java 服务器（vanilla 1.20.4+ / MC 26.2，LAN 即可），记下地址与端口
- Rust nightly（见 `rust-toolchain.toml`）
- 可选：Docker（若想隔离构建，见下方容器节）

## 用法

```bash
# 全部 probe（默认端口 4444）
./run_all.sh 4444

# 指定脚本子集（其余传给 azalea_probe --script）
./run_all.sh 4444 smoke.json till_and_sow.json

# 单脚本冒烟
cargo run -p craft-agent-minecraft --example azalea_probe --features azalea-bot -- 4444 --script scripts/probe/smoke.json
```

输出：逐脚本 PASS/FAIL 汇总，结果写入 `bench_results_<ts>.txt`。

## 解读结果

每个 probe 脚本对应 `docs/benchmarks.md` §2 表格中的一行场景：
全部 PASS = 工具层该场景在当前服务器环境复现成功。
FAIL 时的处理：先确认服务器处于干净状态（无残留怪物/方块、bot 可重生），
再对照 `docs/mindcraft-gap.md` 的对应 P 条目。

## Docker 复现（可选）

```bash
docker build -t seeker-craft-bench -f Dockerfile.bench .
# 需要 MC 服务器 host 可达；示例（Linux/macOS）：
docker run --network host seeker-craft-bench 127.0.0.1:4444
```
