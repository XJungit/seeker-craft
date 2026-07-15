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
use std::net::TcpStream;
use std::time::Duration;

/// 桥接 mod 默认监听端口（避开 GameQuery 的 25566）。
pub const DEFAULT_PORT: u16 = 25567;
/// 动作（如 mine 10s）可能耗时较长，读超时给足余量。
const READ_TIMEOUT: Duration = Duration::from_secs(30);

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
}

fn default_held_item() -> String {
    "minecraft:air".into()
}

/// 发给 mod 的动作命令（serde tag = `type` 字段，与 mod 侧小写匹配）。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum ModCommand {
    /// 相对转视角。dx>0 右转, dy>0 低头（与 enigo 模式语义一致）。
    #[serde(rename = "look")]
    Look { dx: i32, dy: i32 },
    /// 绝对朝向某世界坐标（精确对准，供 aim_and_mine）。
    #[serde(rename = "look_at")]
    LookAt { x: f64, y: f64, z: f64 },
    /// 按住按键若干 tick（20 tick≈1 秒）。keys 为单字符: w/a/s/d/space/shift/e/1-9。
    #[serde(rename = "press")]
    Press { keys: String, ticks: u32 },
    /// 按住左键挖掘若干 tick。返回前后原木数量用于成败判断。
    #[serde(rename = "mine")]
    Mine { ticks: u32 },
    /// 朝某方向移动若干 tick（dir: forward/back/left/right/up/down）。
    #[serde(rename = "move")]
    Move { dir: String, ticks: u32 },
    /// 简易寻路：转向目标并前进直到接近（水平距离 < 1.5 米）。
    #[serde(rename = "move_to")]
    MoveTo { x: f64, y: f64, z: f64 },
    /// 右键点击（放置方块/使用物品/吃食物/开箱子）。
    #[serde(rename = "right_click")]
    RightClick { ticks: u32 },
    /// 按住左键攻击（对实体造成伤害；对准方块则为挖掘，用 mine 代替）。
    #[serde(rename = "attack")]
    Attack { ticks: u32 },
    /// 合成物品：mod 侧直接操作 Inventory 扣材料加结果，零视觉依赖。
    #[serde(rename = "craft")]
    Craft { item: String, count: u32 },
}

/// mod 对动作命令的回执。
#[derive(Debug, Clone, Deserialize)]
pub struct ModAck {
    /// `ok` / `fail`。
    pub status: String,
    #[serde(default)]
    pub detail: String,
    /// mine 前原木（`*_log`）总数，用于成败判断。
    #[serde(default)]
    pub logs_before: Option<u32>,
    /// mine 后原木总数。
    #[serde(default)]
    pub logs_after: Option<u32>,
}

/// 本地桥接客户端。
pub struct McBridge {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
    host: String,
    port: u16,
}

impl McBridge {
    /// 连接本机 mod（默认端口 25567）。
    pub fn connect(host: &str, port: u16) -> Result<Self> {
        let (stream, reader) = Self::new_stream(host, port)?;
        Ok(Self {
            stream,
            reader,
            host: host.to_string(),
            port,
        })
    }

    fn new_stream(host: &str, port: u16) -> Result<(TcpStream, BufReader<TcpStream>)> {
        let stream = TcpStream::connect((host, port)).with_context(|| {
            format!(
                "连接 MC 桥接 mod 失败 {host}:{port}（确认 MC 已启动且加载了 craft-agent-bridge，端口 {DEFAULT_PORT}）"
            )
        })?;
        stream
            .set_read_timeout(Some(READ_TIMEOUT))
            .context("设置读超时失败")?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .context("设置写超时失败")?;
        let reader = BufReader::new(stream.try_clone().context("克隆 socket 失败")?);
        Ok((stream, reader))
    }

    /// 重连 mod（MC 崩溃重启后恢复连接）。
    pub fn reconnect(&mut self) -> Result<()> {
        let (stream, reader) = Self::new_stream(&self.host, self.port)?;
        self.stream = stream;
        self.reader = reader;
        Ok(())
    }

    /// 检查连接是否存活（发轻量 state 请求，如果失败返回 false）。
    pub fn is_alive(&mut self) -> bool {
        self.send_line(&serde_json::json!({"type": "state"}))
            .is_ok()
            && self.read_line().is_ok()
    }

    /// 查询最新游戏状态快照。
    pub fn query_state(&mut self) -> Result<ModState> {
        self.send_line(&serde_json::json!({"type": "state"}))?;
        let line = self.read_line()?;
        serde_json::from_str(&line).with_context(|| format!("解析 mod state 失败: {line}"))
    }

    /// 发送动作命令并等待回执。
    pub fn send(&mut self, cmd: ModCommand) -> Result<ModAck> {
        self.send_line(&serde_json::to_value(&cmd)?)?;
        let line = self.read_line()?;
        serde_json::from_str(&line).with_context(|| format!("解析 mod ack 失败: {line}"))
    }

    fn send_line(&mut self, v: &serde_json::Value) -> Result<()> {
        let mut s = serde_json::to_string(v).context("序列化命令失败")?;
        s.push('\n');
        self.stream
            .write_all(s.as_bytes())
            .and_then(|_| self.stream.flush())
            .context("发送命令到 mod 失败")?;
        Ok(())
    }

    fn read_line(&mut self) -> Result<String> {
        let mut buf = String::new();
        let n = self
            .reader
            .read_line(&mut buf)
            .context("读取 mod 响应失败（mod 可能已崩溃或卡住）")?;
        if n == 0 {
            return Err(anyhow!("mod 连接已关闭（MC 可能已退出）"));
        }
        Ok(buf.trim_end().to_string())
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

        let m = ModCommand::Mine { ticks: 60 };
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["type"], "mine");
        assert_eq!(v["ticks"], 60);
    }

    #[test]
    fn mod_state_roundtrips() {
        let json = r#"{
            "position":[1.0,64.0,2.0],
            "yaw":90.0,"pitch":10.0,
            "health":20.0,"hunger":18.0,
            "inventory":[{"slot":0,"id":"minecraft:oak_log","count":4}],
            "targeted_block":{"id":"minecraft:oak_log","dist":3.2},
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
