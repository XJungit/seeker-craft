//! MC 桥接 mod 的本地 TCP 客户端（JSON 行协议，localhost）。
//!
//! 仅 `mod-bridge` 特性编译。与 enigo / xcap 输入完全解耦——所有感知与动作
//! 都走 mod 在游戏进程内暴露的结构化状态（MindFlayer 式"直接读游戏数据"）。
//!
//! 协议：一行一个 JSON 对象（`\n` 结尾）。请求有 `state`（查询快照）与动作命令
//! （`Look`/`LookAt`/`Press`/`Mine`/`Move`/`MoveTo`）；响应同样一行 JSON。
//! 连接持久复用：客户端保持一条连接，按序发请求、读响应（与 enigo 的同步 sleep
//! 模型一致，单线程 agent 无并发冲突）。

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// 桥接 mod 默认监听端口（避开 GameQuery 的 25566）。
pub const DEFAULT_PORT: u16 = 25567;
/// 动作超时：path_to 逐节点走（每节点 2s），长路径可能 60s+，给 90s 余量。
const READ_TIMEOUT: Duration = Duration::from_secs(90);
/// TCP 写入超时。
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// 重连基础退避（指数增长，上限 30s）。
const BASE_BACKOFF_MS: u64 = 200;
const MAX_BACKOFF_MS: u64 = 30_000;

/// 物品栏槽位。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvSlot {
    pub slot: u32,
    /// 注册表 id，如 `minecraft:oak_log`。
    pub id: String,
    pub count: u32,
}

/// 准星所指方块（基于 MC 自带 raycast）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetedBlock {
    pub id: String,
    /// 玩家到方块中心的距离（米）。
    pub dist: f64,
    /// 方块世界坐标（整数方块坐标，可直接传给 dig_at / place_at）。
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default)]
    pub z: i32,
}

/// 附近实体（生物/掉落物/其他玩家）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearbyEntity {
    /// 注册表 id，如 `minecraft:creeper`。
    pub r#type: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// 到玩家的水平+垂直距离（米）。
    pub dist: f64,
    pub health: f32,
    /// 运动速度 [vx, vy, vz]（米/秒）。
    #[serde(default)]
    pub velocity: [f64; 3],
    /// 实体身上的状态效果（生物才有；掉落物为空数组）。
    #[serde(default)]
    pub effects: Vec<ActiveEffect>,
}

/// 状态效果（中毒 / 缓慢 / 发光 / 速度等）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActiveEffect {
    /// 注册表 id，如 `minecraft:poison`。
    pub id: String,
    /// 效果等级（0=Ⅰ级，+N 递增）。缺失按 0 处理。
    #[serde(default)]
    pub amplifier: i32,
    /// 剩余持续 tick。缺失按 0 处理。
    #[serde(default)]
    pub duration: i32,
}

/// 附近方块（mod 在半径内扫描白名单：原木/木板/工作台/石头/矿石等）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearbyBlock {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub dist: f64,
    /// 相对玩家的高度差（player.y - block.y）。正=方块在脚下，负=在头顶。缺失按 0。
    #[serde(default)]
    pub height_diff: f64,
}

/// mod 返回的游戏状态快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModState {
    pub position: [f64; 3],
    /// 偏航角（度）。MC: 0=朝南, 正值向左转。
    pub yaw: f64,
    /// 俯仰角（度）。正值=低头看地。
    pub pitch: f64,
    pub health: f32,
    pub hunger: f32,
    pub inventory: Vec<InvSlot>,
    pub targeted_block: Option<TargetedBlock>,
    pub nearby_blocks: Vec<NearbyBlock>,
    pub entities: Vec<NearbyEntity>,
    /// 游戏内时间（tick）。
    pub time: i64,
    pub dimension: String,
    pub biome: String,
    pub gamemode: String,
    /// 运动速度 [vx, vy, vz]（米/秒）。缺失按 [0,0,0]。
    #[serde(default)]
    pub velocity: [f64; 3],
    /// 玩家状态效果。缺失按空数组。
    #[serde(default)]
    pub effects: Vec<ActiveEffect>,
    /// 经验等级。缺失按 0。
    #[serde(default)]
    pub experience_level: u32,
    /// 经验进度（0~1）。缺失按 0。
    #[serde(default)]
    pub experience_progress: f32,
    /// 是否正在下雨。缺失按 false。
    #[serde(default)]
    pub raining: bool,
    /// 是否雷暴。缺失按 false。
    #[serde(default)]
    pub thundering: bool,
    /// 天空光照等级（0~15）。缺失按 0。
    #[serde(default)]
    pub sky_light: i32,
    /// 方块光照等级（0~15）。缺失按 0。
    #[serde(default)]
    pub block_light: i32,
    /// 主手物品 id（如 minecraft:wooden_pickaxe），缺失按 air。
    #[serde(default = "default_held_item")]
    pub held_item: String,
    /// 最近威胁（敌对生物）聚合。缺失按 None。
    #[serde(default)]
    pub nearest_threat: Option<NearestThreat>,
}

/// 最近威胁聚合（帮 LLM 快速判断危险）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearestThreat {
    /// 实体注册表 id，如 `minecraft:zombie`。
    pub r#type: String,
    /// 距离（米）。
    pub dist: f64,
}

fn default_held_item() -> String {
    "minecraft:air".into()
}

/// 发给 mod 的动作命令（serde tag = `type` 字段，与 mod 侧小写匹配）。
///
/// ServerPlayer 架构协议：所有命令在服务端主线程原生执行，天然同步。
/// 已移除的旧命令（KeyMapping 模拟）：Press / Mine / Move / PathTo / RightClick。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum ModCommand {
    /// 相对转视角。dx>0 右转, dy>0 低头。
    #[serde(rename = "look")]
    Look { dx: i32, dy: i32 },
    /// 绝对朝向某世界坐标（精确对准）。
    #[serde(rename = "look_at")]
    LookAt { x: f64, y: f64, z: f64 },
    /// 简易寻路：转向目标并前进直到接近（水平距离 < 1.5 米）。
    /// mod 侧每 tick setDeltaMovement + 朝向，自动跳跃障碍。
    #[serde(rename = "move_to")]
    MoveTo { x: f64, y: f64, z: f64 },
    /// 精确放置方块到指定坐标（mod 侧 useItemOn，不依赖准星朝向）。
    #[serde(rename = "place_at")]
    PlaceAt {
        x: i32,
        y: i32,
        z: i32,
        item: String,
    },
    /// 精确破坏指定坐标方块（mod 侧 destroyBlock，含掉落）。
    #[serde(rename = "dig_at")]
    DigAt { x: i32, y: i32, z: i32 },
    /// 攻击最近敌对实体（单次攻击，mod 侧自动装备武器+朝向）。
    #[serde(rename = "attack")]
    Attack { ticks: u32 },
    /// 战斗 AI（melee/kite/retreat，mod 侧自主走位）。
    #[serde(rename = "combat")]
    Combat { mode: String, ticks: u32 },
    /// 合成物品：mod 侧直接操作 Inventory 扣材料加结果，零视觉依赖。
    #[serde(rename = "craft")]
    Craft { item: String, count: u32 },
    /// 丢弃物品。
    #[serde(rename = "discard")]
    Discard { item: String, num: u32 },
    /// 烧制物品（mod 侧直接转换，无需熔炉 GUI）。
    #[serde(rename = "smelt")]
    Smelt { item: String, num: u32 },
    /// 切换快捷栏选中格（mod 侧反射设置 Inventory.selected）。
    #[serde(rename = "select_slot")]
    SelectSlot { slot: u32 },
    /// 从主背包移动物品到快捷栏（mod 侧直接交换槽位）。
    #[serde(rename = "move_to_hotbar")]
    MoveToHotbar { item: String },
    /// 精确槽位移动物品（支持任意 from/to 槽位 + 可选 count 拆分）。
    /// slot 索引：0-8=hotbar，9-35=main inventory。
    /// count=None 时整组移动；count=Some(n) 时拆分 n 个到 to_slot。
    /// 若 to_slot 已有不同物品，按 MC 规则交换；同物品则叠加。
    #[serde(rename = "move_slot")]
    MoveSlot {
        from_slot: u32,
        to_slot: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        count: Option<u32>,
    },
    /// 使用主手物品（吃东西/用桶/扔珍珠等，mod 侧 useItem）。
    /// ticks: 长按时长（吃东西 32 tick ≈ 1.6s）。
    #[serde(rename = "use_item")]
    UseItem { ticks: u32 },
    /// 查询单个方块（Rust A* 寻路用）。
    #[serde(rename = "get_block")]
    GetBlock { x: i32, y: i32, z: i32 },
    /// 查询区域内所有非空方块（Rust A* 寻路用）。
    #[serde(rename = "get_blocks")]
    GetBlocks {
        x1: i32,
        y1: i32,
        z1: i32,
        x2: i32,
        y2: i32,
        z2: i32,
    },
    /// 读取当前打开的容器/GUI内容（参考 Numen inspect_gui）。
    #[serde(rename = "inspect_gui")]
    InspectGui,
    /// 关闭当前打开的容器/GUI。
    #[serde(rename = "close_gui")]
    CloseGui,
    /// 在打开的容器中进行物品转移（Shift+路由或精确槽位移）。
    /// moves: JSON 数组，每项 {from: int, to?: int|null, count?: int}
    #[serde(rename = "transfer")]
    Transfer { moves: serde_json::Value },
    /// 装备物品到指定槽位（支持盔甲/offhand/mainhand）。
    #[serde(rename = "equip_item")]
    EquipItem {
        item: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        slot: Option<String>,
    },
    /// 吃指定物品（自动切到快捷栏+useItem）。
    #[serde(rename = "eat_item")]
    EatItem {
        item: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        ticks: Option<u32>,
    },
    /// 丢弃物品为地面实体（真正生成 ItemEntity，带拾取冷却）。
    #[serde(rename = "drop_items")]
    DropItems { item: String, num: u32 },
    /// 等待指定秒数。
    #[serde(rename = "wait")]
    Wait { seconds: u32 },
    /// 原地垫方块脱困：在脚下放 blocks 并跳起，重复 count 次。
    #[serde(rename = "pillar_up")]
    PillarUp {
        #[serde(skip_serializing_if = "Option::is_none")]
        count: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        item: Option<String>,
    },
    /// 列出在线玩家（支持 goToPlayer/attackPlayer 基础）。
    #[serde(rename = "list_players")]
    ListPlayers,
    /// 按名字导航到指定玩家。
    #[serde(rename = "go_to_player")]
    GoToPlayer {
        player_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        closeness: Option<f64>,
    },
    /// 攻击指定玩家。
    #[serde(rename = "attack_player")]
    AttackPlayer { player_name: String, ticks: u32 },
    /// 给指定玩家物品。
    #[serde(rename = "give_player")]
    GivePlayer {
        player_name: String,
        item: String,
        num: u32,
    },
    /// 自动拾取附近掉落物（参考 Numen collect_items）。
    #[serde(rename = "collect_items")]
    CollectItems {
        item_ids: Vec<String>,
        radius: f64,
        max_count: u32,
    },
    /// 停止所有当前动作（参考 mindcraft !stop）。
    #[serde(rename = "stop")]
    Stop,
    /// 设置持续目标（参考 mindcraft !goal）。
    #[serde(rename = "set_goal")]
    SetGoal { goal: String },
    /// 获取当前持续目标。
    #[serde(rename = "get_goal")]
    GetGoal,
    // ═══ 第三批命令（参考 mindcraft 41 actions + 14 queries） ═══
    /// 持续跟随指定玩家（resume=true 模式，mod 侧 tick 循环追击）。
    #[serde(rename = "follow_player")]
    FollowPlayer {
        player_name: String,
        follow_dist: Option<f64>,
    },
    /// 搜索 minecraft.wiki（HTTP 请求 + HTML 提取正文，2000 字截断）。
    #[serde(rename = "search_wiki")]
    SearchWiki { query: String },
    /// 查询最近村民的交易列表（参考 mindcraft !showVillagerTrades）。
    #[serde(rename = "villager_trades")]
    VillagerTrades { radius: Option<f64> },
    /// 与村民交易（参考 mindcraft !tradeWithVillager）。
    #[serde(rename = "trade_with_villager")]
    TradeWithVillager {
        index: u32,
        count: Option<u32>,
        radius: Option<f64>,
    },
    /// 看向指定玩家（仅朝向，不移动）。
    #[serde(rename = "look_at_player")]
    LookAtPlayer { player_name: String },
    /// 看向指定坐标（仅朝向，不移动）。
    #[serde(rename = "look_at_position")]
    LookAtPosition { x: f64, y: f64, z: f64 },
    /// 右键激活指定坐标方块（朝向 + useItemOn）。
    #[serde(rename = "activate_block")]
    ActivateBlock { x: i32, y: i32, z: i32 },
    /// 对最近实体使用物品（朝向 + interactOn）。
    #[serde(rename = "use_on_entity")]
    UseOnEntity {
        entity_type: String,
        radius: Option<f64>,
    },
    /// 清空对话历史（mod 侧 ack，Rust 侧清空）。
    #[serde(rename = "clear_chat")]
    ClearChat,
    /// 激活最近的指定类型方块（搜索 + useItemOn）。
    #[serde(rename = "activate_nearest_block")]
    ActivateNearestBlock {
        block_type: String,
        radius: Option<f64>,
    },
    /// 查询合成计划（库存已有 + 缺失材料）。
    #[serde(rename = "get_crafting_plan")]
    GetCraftingPlan { item: String, count: u32 },
    /// 智能丢弃（moveAway 5m + drop + goBack，参考 mindcraft !discard）。
    #[serde(rename = "discard_smart")]
    DiscardSmart { item: String, num: u32 },
    // ═══ 第四批命令（附魔 + 维度传送） ═══
    /// 附魔物品（消耗 XP 等级）。
    #[serde(rename = "enchant")]
    Enchant { item: String, levels: u32 },
    /// 在当前坐标建造下界传送门。
    #[serde(rename = "build_portal")]
    BuildPortal,
    /// 传送到指定维度（the_nether / the_end / overworld）。
    #[serde(rename = "teleport_to")]
    TeleportToDimension { dimension: String },
    // ═══ 调试命令（smoke 测试造环境用，不暴露给 LLM） ═══
    /// 在玩家前方生成实体/掉落物。entity: zombie/pig/cow/creeper/chicken/item/villager。
    #[serde(rename = "debug_spawn")]
    DebugSpawn {
        entity: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        item: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        num: Option<u32>,
        /// 仅 villager 用：职业名（farmer/librarian/...），设置后村民带职业+工作站。
        #[serde(skip_serializing_if = "Option::is_none")]
        profession: Option<String>,
    },
    /// 直接给玩家物品。
    #[serde(rename = "debug_give")]
    DebugGive { item: String, num: u32 },
    /// 扣血。
    #[serde(rename = "debug_damage")]
    DebugDamage { amount: f32 },
    /// 回血。
    #[serde(rename = "debug_heal")]
    DebugHeal { amount: f32 },
    /// 清空背包 + 周围掉落物。
    #[serde(rename = "debug_clear")]
    DebugClear,
    /// 在世界指定坐标放置方块（造 searchForBlock/chest/activate 测试环境）。
    #[serde(rename = "debug_place")]
    DebugPlace {
        block: String,
        x: i32,
        y: i32,
        z: i32,
    },
    /// 设置玩家饱食度（造 eat_item 测试环境：<满才能吃）。
    #[serde(rename = "debug_food")]
    DebugFood { level: i32 },
    /// 程序化设置世界时间（night/day/noon/midnight 或 tick 数值），并暂停昼夜循环。
    #[serde(rename = "debug_time")]
    DebugTime { value: String },
    /// 给玩家经验等级（造 enchant 测试环境）。
    #[serde(rename = "debug_xp")]
    DebugXp { levels: i32 },
    /// 把指定真实玩家（按名，排除 bot）传送到 bot 前方，用于 *_player 工具测试。
    #[serde(rename = "debug_teleport_player")]
    DebugTeleportPlayer { name: String, dist: Option<f64> },
    /// 把 bot（被控 fakePlayer）传送到干净地面，避免卡坑/水里导致摆位异常。
    #[serde(rename = "debug_teleport_bot")]
    DebugTeleportBot { x: Option<f64>, z: Option<f64> },
    /// 绝对朝向（对齐 Mineflayer bot.look）。
    #[serde(rename = "look_abs")]
    LookAbs { yaw: f32, pitch: f32 },
    /// 钓鱼（手持 fishing_rod 抛竿/收竿）。
    #[serde(rename = "fish")]
    Fish { ticks: u32 },
    /// 骑乘控制：mount 最近的 rideable / dismount / steer 驾驶。
    #[serde(rename = "ride")]
    Ride {
        action: String,
        radius: Option<f64>,
        left: Option<f64>,
        forward: Option<f64>,
    },
    /// 睡觉跳夜（需要附近有床）。
    #[serde(rename = "sleep")]
    Sleep { radius: Option<f64> },
    /// 醒来。
    #[serde(rename = "wake")]
    Wake,
}

/// mod 对动作命令的回执。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ModAck {
    /// `ok` / `fail`。
    pub status: String,
    #[serde(default)]
    pub detail: String,
    /// move_to: 是否到达目标。
    #[serde(default)]
    pub reached: Option<bool>,
    /// move_to: 最终距离（米）。
    #[serde(default)]
    pub final_dist: Option<f64>,
    /// move_to: 是否卡住。
    #[serde(default)]
    pub stuck: Option<bool>,
    /// place_at: 是否放置成功。
    #[serde(default)]
    pub placed: Option<bool>,
    /// pillar_up: 实际放置的方块数。
    #[serde(default)]
    pub pillar_count: Option<u32>,
    /// dig_at: 是否破坏成功。
    #[serde(default)]
    pub broken: Option<bool>,
    /// dig_at: 破坏的方块 ID。
    #[serde(default)]
    pub block_id: Option<String>,
    /// ride: 骑乘的实体类型。
    #[serde(default)]
    pub mounted: Option<String>,
    /// combat: 战斗结果 (killed/retreated/timeout/no_target)。
    #[serde(default)]
    pub result: Option<String>,
    /// combat: 目标实体类型。
    #[serde(default)]
    pub target: Option<String>,
    /// select_slot: 实际选中的格 (0-8)。equip_item 复用此字段传装备槽名
    /// (如 "mainhand")，故用 Value 兼容整数与字符串。
    #[serde(default)]
    pub slot: Option<serde_json::Value>,
    /// select_slot / use_item: 当前手持物品 ID。
    #[serde(default)]
    pub held_item: Option<String>,
    /// move_to_hotbar: 是否移动成功。
    #[serde(default)]
    pub moved: Option<bool>,
    /// move_to_hotbar: 移动到的快捷栏格 (0-8)。
    #[serde(default)]
    pub hotbar_slot: Option<u32>,
    /// craft: 合成的数量。
    #[serde(default)]
    pub crafted: Option<u32>,
    /// use_item: 物品是否被消耗。
    #[serde(default)]
    pub consumed: Option<bool>,
    /// get_block: 方块 ID。
    #[serde(default)]
    pub id: Option<String>,
    /// get_block: 是否固体。
    #[serde(default)]
    pub solid: Option<bool>,
    /// get_block: 是否空气。
    #[serde(default)]
    pub air: Option<bool>,
    /// get_blocks: 方块列表。
    #[serde(default)]
    pub blocks: Option<serde_json::Value>,
    /// get_blocks: 方块数量。
    #[serde(default)]
    pub count: Option<u32>,
    // ═══ inspect_gui 回执 ═══
    #[serde(default)]
    pub has_gui: Option<bool>,
    #[serde(default)]
    pub slots: Option<serde_json::Value>,
    #[serde(default)]
    pub crafting_grid: Option<serde_json::Value>,
    #[serde(default)]
    pub carried_item: Option<String>,
    #[serde(default)]
    pub carried_count: Option<u32>,
    // ═══ transfer 回执 ═══
    #[serde(default)]
    pub moved_count: Option<u32>,
    // ═══ equip_item 回执 ═══
    #[serde(default)]
    pub equipped: Option<bool>,
    // ═══ drop_items 回执 ═══
    #[serde(default)]
    pub dropped: Option<u32>,
    // ═══ list_players 回执 ═══
    #[serde(default)]
    pub players: Option<serde_json::Value>,
    // ═══ attack_player 回执 ═══
    #[serde(default)]
    pub hits: Option<u32>,
    // ═══ collect_items 回执 ═══
    #[serde(default)]
    pub collected: Option<u32>,
    // ═══ get_goal 回执 ═══
    #[serde(default)]
    pub goal: Option<String>,
    // ═══ 第三批命令回执 ═══
    /// follow_player: 是否在跟随中。
    #[serde(default)]
    pub following: Option<bool>,
    /// search_wiki: 搜索结果文本。
    #[serde(default)]
    pub wiki_text: Option<String>,
    /// villager_trades: 交易列表 JSON。
    #[serde(default)]
    pub trades: Option<serde_json::Value>,
    /// villager_trades: 村民职业。
    #[serde(default)]
    pub villager_profession: Option<String>,
    /// trade_with_villager: 实际交易次数。
    #[serde(default)]
    pub traded: Option<u32>,
    /// activate_block / use_on_entity: 是否消费了动作。
    #[serde(default)]
    pub activated: Option<bool>,
    #[serde(default)]
    pub interacted: Option<bool>,
    /// get_crafting_plan: 已有数量。
    #[serde(default)]
    pub have_count: Option<u32>,
    /// get_crafting_plan: 缺失材料 JSON。
    #[serde(default)]
    pub missing: Option<serde_json::Value>,
}

/// 本地桥接客户端（TCP 长连接复用）。
pub struct McBridge {
    host: String,
    port: u16,
    reader: Option<BufReader<TcpStream>>,
}

impl McBridge {
    /// 建立 TCP 长连接（实际连接延迟到首次命令）。
    pub fn connect(host: &str, port: u16) -> Result<Self> {
        let addr: SocketAddr = format!("{}:{}", host, port)
            .parse()
            .with_context(|| format!("解析地址失败 {host}:{port}"))?;
        // 验证可连接
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
            .with_context(|| format!("连接 MC 桥接 mod 超时 {host}:{port}（5s，确认 MC 已启动且加载了 craft-agent-bridge，端口 {DEFAULT_PORT}）"))?;
        stream
            .set_read_timeout(Some(READ_TIMEOUT))
            .context("设置读超时失败")?;
        stream
            .set_write_timeout(Some(WRITE_TIMEOUT))
            .context("设置写超时失败")?;
        Ok(Self {
            host: host.to_string(),
            port,
            reader: Some(BufReader::new(stream)),
        })
    }

    /// 重连 mod（MC 崩溃重启后恢复连接），含指数退避。
    /// `max_attempts`: 最多尝试次数（默认 5 次）。
    pub fn reconnect(&mut self) -> Result<()> {
        self.reconnect_with_backoff(5)
    }

    /// 带指数退避的重连。
    pub fn reconnect_with_backoff(&mut self, max_attempts: u32) -> Result<()> {
        let mut delay = BASE_BACKOFF_MS;
        for attempt in 1..=max_attempts {
            self.reader = None;
            if self.connect_stream().is_err() {
                if attempt == max_attempts {
                    return Err(anyhow!(
                        "重连失败（已尝试 {max_attempts} 次, 最后错误: 连接超时）"
                    ));
                }
                std::thread::sleep(Duration::from_millis(delay));
                delay = (delay * 2).min(MAX_BACKOFF_MS);
                continue;
            }
            match self.try_ping() {
                Ok(()) => return Ok(()),
                Err(e) => {
                    if attempt == max_attempts {
                        return Err(anyhow!("重连失败（已尝试 {max_attempts} 次）: {e}"));
                    }
                    std::thread::sleep(Duration::from_millis(delay));
                    delay = (delay * 2).min(MAX_BACKOFF_MS);
                }
            }
        }
        Err(anyhow!("重连失败（未进入尝试循环）"))
    }

    /// 检查连接是否存活。
    pub fn is_alive(&mut self) -> bool {
        self.try_ping().is_ok()
    }

    /// 心跳 ping。
    pub fn ping(&mut self) -> Result<()> {
        self.try_ping()
    }

    /// 轻量连接检查。
    fn try_ping(&mut self) -> Result<()> {
        self.send_one_shot(&serde_json::json!({"type": "state"}))?;
        Ok(())
    }

    /// 查询最新游戏状态快照。
    pub fn query_state(&mut self) -> Result<ModState> {
        let line = self.send_one_shot(&serde_json::json!({"type": "state"}))?;
        serde_json::from_str(&line).with_context(|| format!("解析 mod state 失败: {line}"))
    }

    /// 发送动作命令并等待回执。
    pub fn send(&mut self, cmd: ModCommand) -> Result<ModAck> {
        let line = self.send_one_shot(&serde_json::to_value(&cmd)?)?;
        serde_json::from_str(&line).with_context(|| format!("解析 mod ack 失败: {line}"))
    }

    /// TCP 长连接发送+接收：复用连接，用 to_writer 避免中间 String。
    fn send_one_shot(&mut self, v: &serde_json::Value) -> Result<String> {
        if self.reader.is_none() {
            self.connect_stream()?;
        }
        let result = self.send_raw(v);
        if result.is_err() {
            self.reader = None;
            if self.connect_stream().is_ok() {
                return self.send_raw(v);
            }
        }
        result
    }

    /// 裸发送（无重连逻辑）。
    fn send_raw(&mut self, v: &serde_json::Value) -> Result<String> {
        let reader = self.reader.as_mut().unwrap();
        let stream = reader.get_mut();
        serde_json::to_writer(&mut *stream, v).context("序列化/发送命令到 mod 失败")?;
        stream.write_all(b"\n").context("发送换行到 mod 失败")?;
        stream.flush().context("flush 命令失败")?;
        let mut buf = String::new();
        reader
            .read_line(&mut buf)
            .context("读取 mod 响应失败（mod 可能已崩溃或卡住）")?;
        Ok(buf.trim_end().to_string())
    }

    /// 建立新 TCP 连接，设置超时。
    fn connect_stream(&mut self) -> Result<()> {
        let addr: SocketAddr = format!("{}:{}", self.host, self.port)
            .parse()
            .with_context(|| format!("解析地址失败 {}:{}", self.host, self.port))?;
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
            .with_context(|| format!("连接 MC 桥接 mod 超时 {}:{}（5s）", self.host, self.port))?;
        stream
            .set_read_timeout(Some(READ_TIMEOUT))
            .context("设置读超时失败")?;
        stream
            .set_write_timeout(Some(WRITE_TIMEOUT))
            .context("设置写超时失败")?;
        self.reader = Some(BufReader::new(stream));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_command_serializes_with_type_tag() {
        let c = ModCommand::Look { dx: 300, dy: -100 };
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["type"], "look");
        assert_eq!(v["dx"], 300);
        assert_eq!(v["dy"], -100);

        let m = ModCommand::DigAt { x: 1, y: 64, z: 2 };
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["type"], "dig_at");
        assert_eq!(v["x"], 1);

        let g = ModCommand::GetBlock { x: 0, y: 64, z: 0 };
        let v = serde_json::to_value(&g).unwrap();
        assert_eq!(v["type"], "get_block");
    }

    #[test]
    fn mod_state_roundtrips() {
        let json = r#"{
            "position":[1.0,64.0,2.0],
            "yaw":90.0,"pitch":10.0,
            "health":20.0,"hunger":18.0,
            "inventory":[{"slot":0,"id":"minecraft:oak_log","count":4}],
            "targeted_block":{"id":"minecraft:oak_log","dist":3.2,"x":4,"y":64,"z":2},
            "nearby_blocks":[{"id":"minecraft:birch_log","x":5.0,"y":64.0,"z":2.0,"dist":4.0}],
            "entities":[{"type":"minecraft:creeper","x":10.0,"y":64.0,"z":10.0,"dist":12.0,"health":20.0}],
            "time":1200,"dimension":"minecraft:overworld","biome":"minecraft:plains","gamemode":"survival"
        }"#;
        let st: ModState = serde_json::from_str(json).unwrap();
        assert_eq!(st.inventory[0].id, "minecraft:oak_log");
        assert_eq!(st.inventory[0].count, 4);
        assert_eq!(st.targeted_block.unwrap().dist, 3.2);
        assert_eq!(st.nearby_blocks.len(), 1);
        assert_eq!(st.entities[0].r#type, "minecraft:creeper");
    }

    #[test]
    fn mod_state_parses_extended_fields() {
        let json = r#"{
            "position":[0.0,64.0,0.0],"yaw":0.0,"pitch":0.0,
            "health":20.0,"hunger":20.0,"inventory":[],
            "targeted_block":null,"nearby_blocks":[],"entities":[],
            "time":0,"dimension":"minecraft:overworld","biome":"minecraft:plains","gamemode":"survival",
            "velocity":[0.1,-0.05,0.2],
            "effects":[{"id":"minecraft:poison","amplifier":1,"duration":120}],
            "experience_level":3,"experience_progress":0.5,
            "raining":true,"thundering":false,"sky_light":12,"block_light":4
        }"#;
        let st: ModState = serde_json::from_str(json).unwrap();
        assert_eq!(st.velocity, [0.1, -0.05, 0.2]);
        assert_eq!(st.effects.len(), 1);
        assert_eq!(st.effects[0].id, "minecraft:poison");
        assert_eq!(st.effects[0].amplifier, 1);
        assert_eq!(st.effects[0].duration, 120);
        assert_eq!(st.experience_level, 3);
        assert_eq!(st.experience_progress, 0.5);
        assert!(st.raining);
        assert!(!st.thundering);
        assert_eq!(st.sky_light, 12);
        assert_eq!(st.block_light, 4);
    }

    #[test]
    fn mod_state_defaults_missing_extended_fields() {
        // 旧版 mod（不含扩展字段）返回的状态仍应解析成功，缺失字段取默认值。
        let json = r#"{
            "position":[0.0,64.0,0.0],"yaw":0.0,"pitch":0.0,
            "health":20.0,"hunger":20.0,"inventory":[],
            "targeted_block":null,"nearby_blocks":[],"entities":[],
            "time":0,"dimension":"minecraft:overworld","biome":"minecraft:plains","gamemode":"survival"
        }"#;
        let st: ModState = serde_json::from_str(json).unwrap();
        assert_eq!(st.velocity, [0.0, 0.0, 0.0]);
        assert!(st.effects.is_empty());
        assert_eq!(st.experience_level, 0);
        assert_eq!(st.sky_light, 0);
        assert!(!st.raining);
        assert!(!st.thundering);
    }

    #[test]
    fn nearby_entity_parses_velocity_and_effects() {
        let json = r#"[{"type":"minecraft:zombie","x":1.0,"y":64.0,"z":2.0,"dist":2.0,"health":20.0,"velocity":[0.0,0.0,0.0],"effects":[{"id":"minecraft:speed","amplifier":0,"duration":60}]}]"#;
        let ents: Vec<NearbyEntity> = serde_json::from_str(json).unwrap();
        assert_eq!(ents[0].velocity, [0.0, 0.0, 0.0]);
        assert_eq!(ents[0].effects.len(), 1);
        assert_eq!(ents[0].effects[0].id, "minecraft:speed");
    }
}
