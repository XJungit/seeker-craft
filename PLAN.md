# Craft-Agent 重构计划（最终版）

## 核心架构

```
LLM ──→ {"goal": "craft iron_pickaxe", "cancel_if": "health<6"}
         ↓
    Rust (转发)
         ↓ TCP
    Java Mod: GoalEngine
         ↓
    自动分解目标 → 自动执行 → 自动容错 → 返回结果
```

**LLM 不写代码，不调工具，只发目标。** Mod 侧全自动。

---

## 为什么会超越 Mindcraft

| 对比 | Mindcraft | 我们 |
|------|-----------|------|
| LLM 写什么 | JS 代码，每步控制 | 目标，一句话 |
| 代码质量 | LLM 写，经常 bug，重试 5 次 | 预编译 Java，0 bug |
| 执行环境 | Node.js eval 沙箱 | Java Mod 原生，服务端 |
| 游戏状态 | Mineflayer 客户端模拟 | 原版服务端真实数据 |
| 容错 | LLM 自己 try-catch | Mod 自动处理 |
| 工具链 | 靠 LLM 自己组合 | Mod 自动规划 |
| 中断 | 常死循环 | 内置 cancel_if 条件 |

**Mindcraft 的局限性**：它必须让 LLM 写代码，因为 Mineflayer 只提供原子 skill，不组合就没法用。代码生成是 Mineflayer 的 crutch。

**我们的优势**：跑在服务端，可以直接调原版所有 API。不需要 LLM 写代码，Mod 自己就能做所有事情。

---

## GoalEngine 设计

### 目标格式

```json
{
  "goal": "craft iron_pickaxe",
  "cancel_if": {"health": "< 6", "hunger": "< 4"}
}
```

### 内置目标分解规则

```
"craft iron_pickaxe"
  → 检查 iron_ingot×3 + stick×2
  → 缺 stick → 子目标 "get stick"
    → 检查 planks×2
    → 缺 planks → 子目标 "get planks"
      → 砍树 → 合成 planks
  → 缺 iron_ingot → 子目标 "get iron_ingot"
    → 检查 raw_iron×3
    → 缺 raw_iron → 子目标 "get raw_iron"
      → 装备 stone_pickaxe+
      → 找 iron_ore → 挖 ×3
    → 烧 raw_iron → iron_ingot
  → 合成 iron_pickaxe
  → equip
  → 报告完成
```

### 目标列表（初期）

| 目标 | 内部行为 |
|------|---------|
| `craft <item>` | 检查材料→自动收集→自动合成→equip |
| `get <item> ×N` | 自动采集或合成 |
| `build <blueprint>` | 检查材料→自动收集→建造 |
| `hunt food` | 找动物→杀→捡→烧肉 |
| `explore cave` | 找洞→下→插火把→挖矿→回 |
| `defend base` | 检查周围→杀威胁→修复 |
| `smelt <item> ×N` | 找炉子→放料→等→取 |
| `enchant <item> <level>` | 做书架→附魔台→附魔 |

### 容错机制（内置，不需要 LLM 管）

- 血量 < 6 → 自动吃食物，目标暂停
- 饥饿 < 4 → 自动吃食物，目标暂停
- 背包满 → 自动丢弃垃圾或回家放箱子
- 工具损坏 → 自动造新的
- 被攻击 → 自动反击/逃跑
- 天黑 → 自动回安全地点
- 目标失败 → 报告原因 + 建议

---

## 实施阶段

### Phase 0: 基础设施（2 天）
- [ ] 弃用 `move_to`，统一 `nav_to`
- [ ] VanillaPathfinder 调参稳定
- [ ] autoSurvive 增强（自动吃、自动反击、自动逃跑）
- [ ] 背包管理（自动丢弃、自动整理）

### Phase 1: GoalEngine 核心（3 天）
- [ ] `GoalEngine.java` — 目标状态机框架
- [ ] 目标分解规则引擎（材料检查→子目标展开）
- [ ] 自动采集系统（`CollectController`，挖→捡一条龙）
- [ ] 自动合成系统（`CraftingController`，完整配方表）
- [ ] 容错系统（血量/饥饿/背包/中断检查）

### Phase 2: 目标覆盖（3 天）
- [ ] `craft <item>` — 完整工具/装备合成链
- [ ] `get <item> ×N` — 自动采集/合成
- [ ] `hunt food` — 打猎+烹饪
- [ ] `build <blueprint>` — 蓝图建造
- [ ] `smelt <item> ×N` — 熔炼
- [ ] `enchant <item> <level>` — 附魔

### Phase 3: 打磨 + 测试（2 天）
- [ ] smoke test 覆盖全部目标
- [ ] 边界情况处理
- [ ] 性能优化

---

## 时间线

| 阶段 | 内容 | 工时 |
|------|------|------|
| Phase 0 | 基础设施 | 2 天 |
| Phase 1 | GoalEngine 核心 | 3 天 |
| Phase 2 | 目标覆盖 | 3 天 |
| Phase 3 | 打磨测试 | 2 天 |
| **总计** | | **10 天** |

---

## 结论

不再让 LLM 写代码，不再让 LLM 调工具。**LLM 只发目标，Mod 全自动执行。**

这是超越 Mindcraft 的唯一路径——因为 Mindcraft 的架构决定了它必须让 LLM 写代码，而我们不需要。