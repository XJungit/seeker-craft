#!/usr/bin/env bash
# 结构自审：把"不准再犯"从人工承诺变成机器门禁。
# 任一条件不满足即非零退出，CI / pre-commit 会因此变红。
set -euo pipefail

# 切到仓库根（脚本位于 scripts/ 下）
cd "$(dirname "$0")/.."

echo "[结构校验] 检查单一 Cargo.lock ..."
lock_count=$(find . -name Cargo.lock -not -path './target/*' | wc -l | tr -d ' ')
if [ "$lock_count" -ne 1 ]; then
  echo "✗ 发现 $lock_count 个 Cargo.lock，必须只有根目录 1 个（成员不得自带）"
  exit 1
fi

echo "[结构校验] 检查无残留 target/ ..."
stray_target=$(find . -name target -type d -not -path './target' -not -path './target/*' | wc -l | tr -d ' ')
if [ "$stray_target" -ne 0 ]; then
  echo "✗ 发现 $stray_target 个非根 target/ 目录，编译产物必须集中到根 target/"
  exit 1
fi

echo "[结构校验] 检查成员依赖均走 workspace.dependencies ..."
# 在成员 Cargo.toml 中，普通依赖不应直接写 version = "..."（应写 xxx.workspace = true）
# 排除根 Cargo.toml（它本就定义在 [workspace.dependencies]）；path 依赖（如 craft-agent）无 version，放行。
violations=$(grep -rnE '^[[:space:]]*[A-Za-z0-9_-]+[[:space:]]*=[[:space:]]*\{[^}]*version[[:space:]]*=' \
  crates/*/Cargo.toml phase0_verify/*/Cargo.toml 2>/dev/null || true)
if [ -n "$violations" ]; then
  echo "✗ 以下依赖未走 workspace.dependencies（应改为 xxx.workspace = true）："
  echo "$violations"
  exit 1
fi

echo "✓ 结构校验通过"
