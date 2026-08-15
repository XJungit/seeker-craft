//! 核心世界状态与动作定义（与 game-agent-design.md §4 对齐）

use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// 截图像素（RGBA 原始字节）。Arc 使 WorldState 克隆零成本（O(1) refcount 而不拷贝 MB 级数据）。
pub type Screenshot = Arc<Vec<u8>>;

pub(crate) mod screenshot_serde {
    use super::*;
    pub fn serialize<S: Serializer>(v: &Arc<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        v.as_ref().serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<Vec<u8>>, D::Error> {
        Vec::<u8>::deserialize(d).map(Arc::new)
    }
}

/// 可交互元素（2D 界面，如背包/合成界面按钮）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Element {
    pub id: u32,
    pub label: String,
    /// `[x, y, w, h]`，以游戏窗口左上角为基准
    pub bbox: [i32; 4],
    pub center: (i32, i32),
}

/// 3D 世界中的检测目标（如树、矿石）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub label: String,
    pub bbox: [i32; 4],
    /// 目标中心相对屏幕准星的偏移 `(dx, dy)`
    pub offset_from_crosshair: (i32, i32),
}

/// 统一世界状态：感知层输出，决策层输入
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldState {
    /// VLM 场景描述（"前方 3 格有橡木树，背包有 2 木头…"）
    pub scene_desc: String,
    /// 标记元素表（可点编号）
    pub marked_elements: Vec<Element>,
    /// 3D 目标检测结果
    pub detected_targets: Vec<Target>,
    /// 血量/饥饿/背包等 HUD 视觉读取
    pub self_hint: String,
    /// 原始截图（RGBA）
    #[serde(with = "screenshot_serde")]
    pub screenshot: Screenshot,
    /// 结构化游戏状态（前端面板可视化用，非 VLM 路线可填充）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hunger: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experience_level: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experience_progress: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<Vec<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub yaw: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pitch: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub biome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gamemode: Option<String>,
    /// 当前维度，如 `minecraft:overworld` / `minecraft:the_nether` / `minecraft:the_end`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimension: Option<String>,
    /// 感知范围内是否存在已激活的下界传送门方块。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub portal_active: Option<bool>,
    /// 服务端累计实体击杀统计，键为实体 id。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kill_counts: Option<Vec<(String, u32)>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inventory: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub held_item: Option<String>,
    /// 已穿戴盔甲（顺序：头盔/胸甲/护腿/靴子，未穿为 "无" 或空串）。
    /// 结构化字段——`scene_desc` 里也有文本摘要，但 API 消费者应读此字段，
    /// 避免解析文本误判"装备丢失"。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub armor: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_slot: Option<usize>,
    /// 世界记忆（邻近记忆 + 锚点），供前端"世界记忆库"面板可视化。
    /// 结构：`{ "cells": [...], "anchors": [...] }`，缺省不序列化。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<serde_json::Value>,
}

/// 移动方向（WASD 映射）
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Direction {
    Forward,
    Back,
    Left,
    Right,
    Up,
    Down,
}

/// 抽象动作：决策层输出
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Action {
    /// 点击带编号的可交互元素
    Click { element_id: u32 },
    /// 对准并挖掘某目标 (已废弃, 用 look+mining 替代)
    AimAndMine { target: String },
    /// WASD 移动若干 tick
    Move { dir: Direction, ticks: u32 },
    /// 转视角（相对移动）
    Look { dx: i32, dy: i32 },
    /// 按下任意按键
    Press { keys: String, ticks: u32 },
    /// 原地挖掘
    Mine { ticks: u32 },
    /// Minecraft 专属动作（azalea 客户端协议层路线）。
    /// 结构化精确控制，不依赖截图/VLM。
    Minecraft(MinecraftAction),
}

/// Minecraft 专属动作（azalea 客户端协议层）。
/// 决策层用自然语言/坐标描述意图，adapter 翻译成 bot 命令。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MinecraftAction {
    /// 走到世界坐标 (x, y, z)。
    Goto { x: i32, y: i32, z: i32 },
    /// 挖掉指定方块 (x, y, z)。
    MineBlock { x: i32, y: i32, z: i32 },
    /// 挖掉 bot 脚下方块（向下挖矿井）。
    MineBelow,
    /// 向上挖：从 bot 头顶逐格挖到空气，用于地下脱困/上到地表。
    /// 与 MineBelow 反向——MineBelow 是向下挖矿井，MineAbove 是向上挖竖井脱困。
    MineAbove,
    /// 对着指定方块交互（放置/右键）。
    InteractBlock { x: i32, y: i32, z: i32 },
    /// 犁地+播种（P84）：目标 (x,y,z) 需为 dirt/grass_block/farmland，
    /// 自动持锄头犁地、持种子播种并验证。seed 如 "wheat_seeds"。
    TillAndSow {
        x: i32,
        y: i32,
        z: i32,
        seed: String,
    },
    /// 睡觉跳夜（P85）：找附近床 → 靠近 → 空主手 → 右键上床 → 睡到自然醒。
    Sleep,
    /// 收割成熟作物（P86）：自动扫描附近成熟小麦/胡萝卜/土豆/甜菜/下界疣并挖取。
    Harvest,
    /// 发送聊天消息（也用作 LLM 指令回显）。
    Chat { content: String },
    /// 攻击最近的生物（用于自卫/狩猎）。target 为 "nearest" 或实体种类关键词（如 "zombie"）。
    /// 当前实现攻击最近的「非玩家」实体；无法指定具体实体 id。
    Attack { target: String },
    /// 合成物品（2×2 背包网格，无需工作台）。item 为配方 id（如 "oak_planks"），count 为数量。
    Craft { item: String, count: u32 },
    /// 3×3 工作台合成（P1-4：自动放收桌）。
    /// item 为配方 id（如 "furnace"），count 为数量。
    /// table_pos=Some((x,y,z)) 时使用该坐标的现有工作台；None 时 bot 自动放置+打开+关闭工作台。
    Craft3x3 {
        item: String,
        count: u32,
        table_pos: Option<(i32, i32, i32)>,
    },
    /// 熔炼（P1-4：自动放收炉）。
    /// output 为产物 id（如 "iron_ingot"），fuel 为燃料 id（如 "coal"），count 为数量。
    /// table_pos=Some((x,y,z)) 时使用该坐标的现有熔炉；None 时 bot 自动放置+打开+关闭熔炉。
    Smelt {
        output: String,
        fuel: String,
        count: u32,
        table_pos: Option<(i32, i32, i32)>,
    },
    /// 采集最近的指定方块（如 "oak_log" / "stone" / "coal_ore"）并挖掘，直到背包有 count 个。
    Gather { item: String, count: u32 },
    /// 自动造黑曜石（P67）：bot 需手持 water_bucket 且附近有岩浆源；自动放水生成黑曜石并挖下 count 块。
    MakeObsidian { count: u32 },
    /// 放置：把手持物品 item 放到世界坐标 (x,y,z) 旁（右键放置）。
    Place {
        item: String,
        x: i32,
        y: i32,
        z: i32,
    },
    /// 打开容器：打开世界坐标 (x,y,z) 处的容器（工作台/熔炉/箱子等）。
    OpenContainer { x: i32, y: i32, z: i32 },
    /// 高层自动合成（木链）：采集→2×2→放置工作台→开→3×3，一键造木制品（chest 等）。
    AutoCraft { item: String, count: u32 },
    /// 附魔：在已打开的附魔台中给背包物品 item 附魔（需背包有 item 与青金石 lapis_lazuli）。
    /// level 为 1/2/3，对应附魔台三个选项槽。
    Enchant { item: String, level: u32 },
    /// 村民交易：与最近的村民交易，选第 offer 个报价（0 起）。bot 自动打开村民。
    Trade { offer: u32 },
    /// 实体右键交互（打开村民/动物/展示框等）。kind 为实体种类关键词（如 "villager"）。
    InteractEntity { kind: String },
    /// 捡起附近掉落物：bot 走 4 个方向扫一圈，让物理引擎吸取掉落物。
    Pickup,
    /// 自动防御：等待 5 秒让 handler self_defense mode 攻击附近敌人。
    Defend,
    /// 装备背包中的指定物品到指定槽位。
    /// item 为物品 id（如 "wooden_pickaxe"），slot 为 "hand"/"helmet"/"chestplate"/"leggings"/"boots"。
    Equip { item: String, slot: String },
    /// 丢弃背包中的指定物品。item 为物品 id，count 为丢弃数量（默认全部）。
    Discard { item: String, count: u32 },
    /// 消耗（吃/喝）背包中的指定物品。item 如 "cooked_beef"/"bread"/"apple"/"potion"。
    Consume { item: String },
    /// 查看世界坐标 (x,y,z) 处容器（箱子/熔炉等）的物品列表。
    /// 打开容器 → 读槽位 → 关闭，返回 "iron_ingot:32, coal:16, ..." 格式。
    ChestView { x: i32, y: i32, z: i32 },
    /// 从世界坐标 (x,y,z) 处容器取出 item（count 个）到 bot 背包。
    /// count=0 表示取全部。bot 会打开容器、shift_click 移动物品、关闭容器。
    ChestWithdraw {
        x: i32,
        y: i32,
        z: i32,
        item: String,
        count: u32,
    },
    /// 把背包中的 item（count 个）存入世界坐标 (x,y,z) 处容器。
    /// count=0 表示存全部。bot 会打开容器、shift_click 移动物品、关闭容器。
    ChestDeposit {
        x: i32,
        y: i32,
        z: i32,
        item: String,
        count: u32,
    },
    /// P68：跟随玩家。target 为玩家名（None=最近的玩家）。
    Follow { target: Option<String> },
    /// P111：按玩家名单次导航（对齐 Mindcraft goToPlayer）。target 为玩家名
    ///（None=最近的玩家）。只导航一次到目标当前坐标，不持续跟随（持续跟随用 Follow）。
    GotoPlayer { target: Option<String> },
    /// P112：搜索指定方块在半径内的全部坐标（对齐 Mindcraft searchForBlock）。
    /// 只返回坐标列表供规划，不挖掘（要挖用 gather）。
    SearchBlock { item: String, radius: u32 },
    /// P113：向远离指定实体的方向移动（对齐 Mindcraft moveAway）。
    /// target 为实体名（None=最近的非玩家实体）；distance 为反向移动距离。
    MoveAway {
        target: Option<String>,
        distance: u32,
    },
    /// P68：停止跟随。
    StopFollow,
    /// P116：开关自动反应式模式（对齐 Mindcraft setMode）。mode 为模式名
    ///（self_preservation/self_defense/cowardice/hunting/item_collecting），
    /// enabled=true 开（默认）/false 关。关闭后 handler 不再自动执行该模式动作。
    SetMode { mode: String, enabled: bool },
    /// P118：使用/投掷手持物品（对齐 MC 右键使用）。item 为目标物品 id
    ///（末影之眼投掷定位要塞/雪球等），yaw/pitch 可选（不传=保持当前视角）。
    /// 装备物品 → 可选转视角 → 右键使用一次（消耗 1 个）。
    UseItem {
        item: String,
        yaw: Option<f32>,
        pitch: Option<f32>,
    },
    /// P119：拉弓射箭（龙战远程必需）。target 为实体名（None=朝当前视角方向射）。
    /// 装备弓 → 检查箭 → 可选转向目标 → 拉弦 ~1s → 放箭（ReleaseUseItem）。
    Shoot { target: Option<String> },
    /// P68：把物品丢在玩家脚边（给予）。item 为物品 id，count 为数量（0=全部），
    /// target 为玩家名（None=最近的玩家）。
    Give {
        item: String,
        count: u32,
        target: Option<String>,
    },
}

/// 动作执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    pub ok: bool,
    pub detail: String,
}
