# Craft-Agent 项目持久记忆

## 工作原则（重要）
- **任何不确定、不知道、不清楚、不准确的信息，都可以通过联网获取来补充与丰富**（WebSearch / WebFetch / 官方文档）。
- 如果网络上也没有明确答案，就自己认真思考、推理判断，并标注这是推断而非查证。
- 遇到 MC 26.2 API 疑问，优先查 NeoForged 26.2 Migration Primer / Fabric 公告 / Minecraft Wiki（见下方「MC 26.2 API 权威参考文档」），再用 `javap` 验证 jar 签名，**不要凭记忆猜**。

## 强制规则：每次改动必须 git commit（重要！！）
- **任何对源码的修改（Java / Rust / 配置）完成后，立即 `git add -A && git commit`**，不要等"全部做完"。
- 原因（血泪教训 2026-07-19）：一次 PowerShell `Set-Content -NoNewline` 误操作把 `CraftAgentBridge.java` 整个毁成 1 行，
  而该文件**从未提交过**，导致无法从 git 恢复，被迫从部署 jar 用 CFR 反编译重建（反编译会引入泛型擦除等人工修正成本）。
- 提交频率：每改完一个独立功能/修复就提交一次，提交信息写清楚（中文即可）。
- **绝不用 PowerShell `Set-Content` / `Out-File` 改 Java/Rust 源文件**（编码会破坏字符串/BOM/换行）。
  要改文件一律用 Edit 工具；要查内容用 Read/Grep；要批量字符串替换用 Edit 的 replaceAll（小范围）或先备份再 Read+Edit。
- Java 源文件是单点故障：一旦损坏且未提交，损失巨大。养成"改一点、测一下、commit 一下"的节奏。
- 反编译恢复命令（应急用，平时别用）：从部署 jar 用 CFR 提取 `CraftAgentBridge.class` 再 `java -jar cfr.jar ...` 反编译；
  反编译后需手工修：① 去掉 CFR 插入的 `(Object)` 强转（约 68 处）② 去掉 BOM ③ `CompletableFuture` 补回泛型 `<T>` ④ `Registry`/`Holder`/`Stream<Holder<Enchantment>>` 补回泛型 ⑤ `switch` 里漏赋值的 `targetLevel`。

## 关键路径
- JDK 25: `C:\Users\xj\AppData\Roaming\.minecraft\runtime\java-runtime-epsilon`
- PCL2 启动器: `D:\Game\pcl2`（.minecraft 在 `C:\Users\xj\AppData\Roaming\.minecraft`）
- Gradle 缓存: `C:\Users\xj\.gradle`

## Minecraft 版本
- **MC 26.2** "Chaos Cubed"（版本号就是 26.2，不是 1.21.x）
- Fabric API 0.154.2+26.2
- **Mojang 官方映射（不是 Yarn）**
- **26.2 无混淆** — jar 里就是原始类名/方法名，`javap` 直接查 `minecraft-merged.jar` 即可验证 API 签名

## 项目结构
- `mods/craft-agent-bridge/` — Java Fabric mod，Gradle 构建，Java 25 target
- `crates/craft-agent-minecraft/` — Rust crate，连接 Java mod 的桥接层
- `crates/craft-agent-core/` — Rust 核心逻辑

## Java Mod 编译
- 设置 `$env:JAVA_HOME = 'C:\Users\xj\AppData\Roaming\.minecraft\runtime\java-runtime-epsilon'`
- 在 `mods/craft-agent-bridge/` 下运行 `.\\gradlew.bat build`

## Rust 编译
- `cargo test --workspace` 或 `cargo build`
- `edition = "2024"`（用户明确拒绝改成 2021）

## MC 26.2 API 权威参考文档
- **NeoForged 26.2 Migration Primer**（最权威，vanilla 类改动总览）：
  `https://github.com/neoforged/.github/blob/main/primers/26.2/index.md`
- **Fabric for Minecraft 26.2 公告**：`https://fabricmc.net/2026/06/15/262.html`
- **Minecraft Wiki 26.2 开发版本**：`https://minecraft.wiki/w/Java_Edition_26.2`
- **ViaVersion 改名 commit**（证明 `ResourceLocation`→`Identifier`）：
  `https://github.com/ViaVersion/ViaVersion/commit/43bb38f4cff6a59ee9739024cf3db3a158048b42`
- 遇到未知 API，**优先查上面三个文档**，再用 `javap` 验证 jar 签名（不要凭记忆猜）。

## 编译问题记录
- `net.minecraft.enchantment` → 实际包路径 `net.minecraft.world.item.enchantment`
- `net.minecraft.registry` → 实际包路径 `net.minecraft.core.registries`
- `getRegistryManager()` → `registryAccess()`
- `RegistryKeys.ENCHANTMENT` → `Registries.ENCHANTMENT`
- `teleportTo(ServerLevel,double,double,double,float,float)` → 实际签名 `teleportTo(ServerLevel,double,double,double,Set<Relative>,float,float,boolean)`
- `EnchantmentHelper.enchant()` → `EnchantmentHelper.enchantItem()`
- `enchReg.streamEntries()` → `lookup().listElements()`（返回 `Stream<Holder.Reference<T>>`，需 `.map()` 转 `Stream<Holder<T>>`）
- `ItemEnchantments.forEach()` → `keySet()` 或 `entrySet()`
- `ResourceKey.location()` → `ResourceKey.identifier()`

### MC 26.2 重大重命名（已核实）
- **`ResourceLocation` → `Identifier`**（`net.minecraft.resources.Identifier`）
  - 构造：`Identifier.fromNamespaceAndPath("minecraft", "oak_log")` 或 `Identifier.tryParse("minecraft:oak_log")`
  - 旧 `new ResourceLocation(ns, path)` 已不存在——`javap` 确认 jar 里只有 `Identifier.class`
- **`Registry.get(Identifier)` 返回 `Optional<Holder.Reference<T>>`**（不是直接的 `T`）
  - 取实际对象：`registry.get(id).get().value()`；判空用 `.isEmpty()`
  - 例：`BuiltInRegistries.ITEM.get(Identifier.fromNamespaceAndPath("minecraft","oak_log)).get().value()`
- **`EntityType.create` 签名改为** `create(Level, EntitySpawnReason)`
  - `EntitySpawnReason` 在 `net.minecraft.world.entity`（枚举：`MOB_SUMMONED` / `COMMAND` / `NATURAL` 等）
  - 旧 `EntityType.create(Level)` / `EntityType.PLAYER` 引用均不可用
- **`Identifier` 不再有 `ResourceLocation` 包**——`net.minecraft.resources` 下只剩 `Identifier`/`ResourceKey`
- **时间 API 重构**：26.1 snap3 起 `/time` 基于 world clocks；`Level.setDayTime(long)` 已删除
  - `ServerLevel`/`MinecraftServer` 上也没有 `setDayTime`/`setTime`（已 `javap` 核实）
  - 程序化设时间需走 world clock API（非生存层核心），`debug_settime` 命令已移除

## 全 63 工具 smoke 测试（fixture 驱动）
- 入口：`crates/craft-agent-minecraft/examples/smoke_test.rs`
  运行：`cargo run -p craft-agent-minecraft --example smoke_test --features mod-bridge`
  （MC + craft-agent-bridge 加载、玩家进世界后执行）
- **debug 命令与 LLM 工具隔离**：`debug_*` 只定义在 `ModCommand` 枚举（`bridge.rs`），
  **不**注册进 `create_mc_mod_tools`，bot 从工具列表拿不到、调不到。只有 smoke 测试
  通过 `adapter.send_debug(...)` 发命令造环境。
- Java mod `performAction` 支持的 debug 命令：`debug_spawn`(entity[item+num]) /
  `debug_give`(item+num) / `debug_damage`(amount) / `debug_heal` / `debug_clear` /
  `debug_place`(block+x+y+z) / `debug_food`(level)。
  `debug_spawn` 接受**任意**实体类型（不只 zombie/pig/cow…，villager 也可，但村民无交易）。
- smoke 设计：每个工具执行前按 `fixture_for(name)` 造环境（spawn 僵尸/cow、place 方块、
  give 物品、降饱食度），造完后 `Wait 1s` 等实体/方块注册进 state 快照；执行后
  `debug_clear` + 补 oak_log 重置。默认参数从 schema `properties` 推导（坐标给整数、
  ticks=32、radius=8、search_range=64 等），并对占位参数("test")做 per-tool 覆盖。
- **最新结果（2026-07-18）**：PASS=44 / FAIL=10 / SKIP=9 / total=63。
  - SKIP 9（单人世界无法造）：5 个 `*_player` 工具、trade_with_villager、villager_trades、
    transfer（需跨工具开着的容器 GUI）、build_portal（需开阔 4×5 地形）、goToBed（床不能悬空放）、
    eat_item（进食是多 tick 消费，单命令无法验证 consumed）、collect_items（导航式拾取）。
  - 剩余 4 真实 fixture 缺口（非控制通路崩溃）：`combat`/`searchForEntity`（僵尸 spawn 后
    在 Wait/状态轮询间未稳定进入实体列表，而 `attack`/`collect` 能命中）、`chest`
    （debug_place 放 chest 是 BlockEntity，单方块 defaultBlockState 可能不被 nearby_blocks 识别）、
    `digDown`（脚下缺可破方块 / 落地后下方为空气）。这 4 个的控制通路本身已验证可执行不挂起。

## 缓存优化记录（2026-07-18）
- DeepSeek prefix cache 按 API Key 隔离，同 key 下所有请求共享
- system prompt 已改为完全静态（jailbreak 变量移出为 user message）
- 动态指令（obs_streak 警告、首轮引导）通过 `build_dynamic_instructions_msg()` 注入
- Reasonix 参考：append-only loop + 末尾截断（不用 summarize compaction），94-99%+ 缓存命中率
- 缓存命中率监控：API 返回 `usage.prompt_cache_hit_tokens` / `prompt_cache_miss_tokens`
- TTL：实测 session 内空闲 5-10 分钟缓存仍在，30 分钟+ 可能 evict

## smoke 测试平台 / bot 落点（重要，2026-07-19 复盘）
- **bot 落点根因**：`createFakePlayer` 在 `(0.5, 64.0, 0.5)` 生成。smoke 的 `build_platform()` 在原点 9×9 建
  y=63/64 两层 dirt 平台。但**若 smoke 连上时 MC 世界尚未就绪**，`debug_place` 全部静默失败（不放块也不报错），
  bot 从 y=64 掉进真实地表（原点附近地表在 **y=44 stone/water**），`state` 显示 bot 在 `y=60` 地下、竖井里。
- `debug_teleport_bot` 地面扫描逻辑：扫 `(tx,tz)` 列最高"非空气+上方2格空气"的方块，bot 放其上方 1 格。
  **若 bot 在 digDown 竖井里，扫描会找到竖井 dirt（y=60），传回 y=61（仍在地下）——不会自动回到平台。**
  所以必须保证**平台先建出来**，传送才有效。
- **教训**：smoke 跑之前务必确认平台已建（或让 smoke 建平台后校验 `state.position` 的 y 是否≈65）；
  连 MC 要等世界完全加载（进世界、地表生成完）再启动 smoke，否则平台建失败导致 bot 卡地下、后续工具全乱。
- `debug_teleport_bot` / `debug_teleport_player` 的 `teleportTo` **必须在 `performAction` 内直接调用**
  （performAction 本身已跑在 `runOnServerThread` 服务端线程）。**切勿再套 `onServer()`**——`executeIfPossible`
  在自身任务内排队会死锁/30s 超时，导致传送失效（bot 卡地下、每个工具白等 30s）。
- 手动救场命令（bot 卡地下时）：连 TCP 发 162 个 `debug_place`(y=63,64) 建平台 + `debug_teleport_bot{x:0.5,z:0.5}`。

