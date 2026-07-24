//! WorldMemory —— 游戏 Agent 的空间-状态长期记忆（"世界地图"）。
//!
//! 与对话历史/WorldInfo 的区别：这里存的是**世界事实**，不是聊天。
//! 典型条目：橡树林位置、造好的工作台坐标、箱子内容、村民位置、下界传送门、
//! 某区块矿石已采光等。由 `perceive`（看到即记）与 `action`（改动即更）回填，
//! 每轮把**当前位置附近的记忆**注入 prompt，供 LLM 做空间规划与长任务连续性。
//!
//! 设计要点：
//! - 坐标为主键（`MemoryPos`），同时支持**命名锚点**（`anchors`：如 "home"、"nether_portal"）。
//! - 分块索引（`chunk_key`）实现 O(1) 邻近查询（按区块 + 曼哈顿半径）。
//! - 线程安全：`Arc<Mutex<WorldMemoryInner>>`，适配器后台线程与 agent 主循环共享。
//! - 可序列化：落盘为 JSONL（sidecar）并通过 `SessionEntry::Memory` 持久化、重启回放。
//! - 世界会变：同一坐标的新记录覆盖旧记录（`upsert`），`forget_*` 可显式删除。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 世界坐标（整数块坐标，避免浮点抖动导致主键漂移）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MemoryPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl MemoryPos {
    pub fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }
    /// 区块键（每 16 格一个区块，与 MC 区块对齐）。
    pub fn chunk_key(&self) -> (i32, i32, i32) {
        (self.x >> 4, self.y >> 4, self.z >> 4)
    }
    /// 曼哈顿距离（用于邻近排序）。
    pub fn manhattan(&self, other: &MemoryPos) -> i32 {
        (self.x - other.x).abs() + (self.y - other.y).abs() + (self.z - other.z).abs()
    }
}

/// 记忆条目类别（决定渲染模板与查询过滤）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryKind {
    /// 资源点：树/矿/食物源（采集后标记 depleted）
    Resource,
    /// 已造/已存在的结构：工作台/熔炉/房子/路
    Structure,
    /// 容器：箱子/熔炉/潜影盒（含内容快照）
    Container,
    /// 生物：村民/动物/ hostile（按种类）
    Entity,
    /// 危险：岩浆/深渊/刷怪笼
    Hazard,
    /// 传送/维度门：下界门/末地门
    Portal,
    /// 其他自由标注
    Note,
}

impl MemoryKind {
    pub fn label(&self) -> &'static str {
        match self {
            MemoryKind::Resource => "资源点",
            MemoryKind::Structure => "结构",
            MemoryKind::Container => "容器",
            MemoryKind::Entity => "实体",
            MemoryKind::Hazard => "危险",
            MemoryKind::Portal => "传送门",
            MemoryKind::Note => "标注",
        }
    }
}

/// 单条记忆。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryCell {
    pub pos: MemoryPos,
    pub kind: MemoryKind,
    /// 人类可读标签（如 "橡树林"、"工具匠村民"、"箱子(32 oak_log)"）
    pub label: String,
    /// 物品/方块 id（如 "oak_log"、"crafting_table"），用于精确匹配与衰减
    pub item: Option<String>,
    /// 数量（资源剩余 / 容器内容计数），可空
    pub count: Option<u32>,
    /// 是否已耗尽（树砍光 / 矿挖完）。耗尽后不再作为有效资源推荐，但保留记录。
    pub depleted: bool,
    /// 自由备注
    pub note: Option<String>,
    /// 最近更新时间戳（毫秒）
    pub updated_at: u64,
}

/// 命名锚点（不绑定固定坐标也能记，但大多有坐标）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryAnchor {
    pub name: String,
    pub pos: Option<MemoryPos>,
    pub label: String,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct WorldMemoryInner {
    /// 主键：坐标 → 记忆
    cells: HashMap<MemoryPos, MemoryCell>,
    /// 区块 → 该区块内所有坐标（邻近查询索引）
    by_chunk: HashMap<(i32, i32, i32), Vec<MemoryPos>>,
    /// 命名锚点
    anchors: HashMap<String, MemoryAnchor>,
}

/// 可序列化快照（落盘/回放用）。用 Vec 而非 HashMap，规避 JSON 键必须为字符串的限制。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct WorldMemorySnapshot {
    cells: Vec<MemoryCell>,
    anchors: Vec<MemoryAnchor>,
}

/// 对外暴露的线程安全记忆库。
#[derive(Debug, Clone)]
pub struct WorldMemory {
    inner: std::sync::Arc<std::sync::Mutex<WorldMemoryInner>>,
}

impl Default for WorldMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl WorldMemory {
    pub fn new() -> Self {
        Self {
            inner: std::sync::Arc::new(std::sync::Mutex::new(WorldMemoryInner::default())),
        }
    }

    /// 记录/更新一条坐标记忆（同坐标同种类覆盖）。
    pub fn upsert(&self, cell: MemoryCell) {
        let mut g = self.inner.lock().unwrap();
        let ck = cell.pos.chunk_key();
        g.by_chunk.entry(ck).or_default().push(cell.pos);
        g.cells.insert(cell.pos, cell);
    }

    /// 便捷构造：资源点。
    pub fn record_resource(&self, pos: MemoryPos, item: &str, label: &str, count: Option<u32>) {
        let now = now_ms();
        self.upsert(MemoryCell {
            pos,
            kind: MemoryKind::Resource,
            label: label.to_string(),
            item: Some(item.to_string()),
            count,
            depleted: false,
            note: None,
            updated_at: now,
        });
    }

    /// 记录结构（工作台/熔炉/建筑）。
    pub fn record_structure(&self, pos: MemoryPos, item: &str, label: &str) {
        let now = now_ms();
        self.upsert(MemoryCell {
            pos,
            kind: MemoryKind::Structure,
            label: label.to_string(),
            item: Some(item.to_string()),
            count: None,
            depleted: false,
            note: None,
            updated_at: now,
        });
    }

    /// 记录容器及其内容摘要。
    pub fn record_container(&self, pos: MemoryPos, label: &str, content: &str) {
        let now = now_ms();
        self.upsert(MemoryCell {
            pos,
            kind: MemoryKind::Container,
            label: label.to_string(),
            item: None,
            count: None,
            depleted: false,
            note: Some(content.to_string()),
            updated_at: now,
        });
    }

    /// 记录实体（村民/动物/怪物）。
    pub fn record_entity(&self, pos: MemoryPos, kind_item: &str, label: &str) {
        let now = now_ms();
        self.upsert(MemoryCell {
            pos,
            kind: MemoryKind::Entity,
            label: label.to_string(),
            item: Some(kind_item.to_string()),
            count: None,
            depleted: false,
            note: None,
            updated_at: now,
        });
    }

    /// 记录危险/传送门/标注。
    pub fn record(&self, pos: MemoryPos, kind: MemoryKind, item: Option<&str>, label: &str, note: Option<&str>) {
        let now = now_ms();
        self.upsert(MemoryCell {
            pos,
            kind,
            label: label.to_string(),
            item: item.map(|s| s.to_string()),
            count: None,
            depleted: false,
            note: note.map(|s| s.to_string()),
            updated_at: now,
        });
    }

    /// 标记某坐标记忆为耗尽（资源采光）。
    pub fn mark_depleted(&self, pos: MemoryPos, depleted: bool) {
        let mut g = self.inner.lock().unwrap();
        if let Some(c) = g.cells.get_mut(&pos) {
            c.depleted = depleted;
            c.updated_at = now_ms();
        }
    }

    /// 按物品 id 标记所有相关资源点为耗尽（如整片树林采光）。
    pub fn mark_depleted_by_item(&self, item: &str, depleted: bool) {
        let mut g = self.inner.lock().unwrap();
        for c in g.cells.values_mut() {
            if c.kind == MemoryKind::Resource && c.item.as_deref() == Some(item) {
                c.depleted = depleted;
                c.updated_at = now_ms();
            }
        }
    }

    /// 删除某坐标记忆。
    pub fn forget_pos(&self, pos: MemoryPos) {
        let mut g = self.inner.lock().unwrap();
        if let Some(c) = g.cells.remove(&pos) {
            let ck = c.pos.chunk_key();
            if let Some(v) = g.by_chunk.get_mut(&ck) {
                v.retain(|p| *p != pos);
            }
        }
    }

    /// 设置/更新命名锚点。
    pub fn set_anchor(&self, name: &str, pos: Option<MemoryPos>, label: &str) {
        let now = now_ms();
        self.inner.lock().unwrap().anchors.insert(
            name.to_string(),
            MemoryAnchor {
                name: name.to_string(),
                pos,
                label: label.to_string(),
                updated_at: now,
            },
        );
    }

    /// 移除锚点。
    pub fn forget_anchor(&self, name: &str) {
        self.inner.lock().unwrap().anchors.remove(name);
    }

    /// 邻近查询：返回以 `around` 为中心、曼哈顿半径 `radius` 内的记忆，按距离升序。
    /// `include_depleted` 控制是否包含已耗尽资源。
    pub fn nearby(&self, around: MemoryPos, radius: i32, include_depleted: bool) -> Vec<MemoryCell> {
        let g = self.inner.lock().unwrap();
        let mut out: Vec<MemoryCell> = g
            .cells
            .values()
            .filter(|c| around.manhattan(&c.pos) <= radius)
            .filter(|c| include_depleted || !c.depleted)
            .cloned()
            .collect();
        out.sort_by_key(|c| around.manhattan(&c.pos));
        out
    }

    /// 按种类 + 物品过滤查询（全局）。
    pub fn query(&self, kind: Option<MemoryKind>, item: Option<&str>) -> Vec<MemoryCell> {
        let g = self.inner.lock().unwrap();
        g.cells
            .values()
            .filter(|c| kind.map_or(true, |k| c.kind == k))
            .filter(|c| item.map_or(true, |i| c.item.as_deref() == Some(i)))
            .cloned()
            .collect()
    }

    /// 取全部锚点。
    pub fn anchors(&self) -> Vec<MemoryAnchor> {
        self.inner.lock().unwrap().anchors.values().cloned().collect()
    }

    /// 锚点查询（按名称前缀或标签包含）。
    pub fn find_anchor(&self, name: &str) -> Option<MemoryAnchor> {
        let g = self.inner.lock().unwrap();
        g.anchors.get(name).cloned()
    }

    /// 渲染邻近记忆为提示文本（注入 system/context）。
    pub fn render_nearby(&self, around: MemoryPos, radius: i32) -> String {
        let cells = self.nearby(around, radius, false);
        if cells.is_empty() {
            return String::new();
        }
        let mut s = String::from("[已知世界记忆·邻近]");
        for c in &cells {
            let d = around.manhattan(&c.pos);
            let dep = if c.depleted { "（已耗尽）" } else { "" };
            let cnt = c.count.map(|n| format!(" x{n}")).unwrap_or_default();
            let note = c.note.as_ref().map(|n| format!(" | {n}")).unwrap_or_default();
            s.push_str(&format!(
                "\n- {}: {} [{}{}] @({},{},{}) 距离{}格{}{}",
                c.kind.label(),
                c.label,
                c.item.as_deref().unwrap_or("-"),
                cnt,
                c.pos.x, c.pos.y, c.pos.z,
                d, dep, note
            ));
        }
        // 锚点也一并给出（若不在邻近半径也可能有用）
        let anchors = self.anchors();
        if !anchors.is_empty() {
            s.push_str("\n[锚点]");
            for a in &anchors {
                let p = a.pos.map(|p| format!("({},{},{})", p.x, p.y, p.z)).unwrap_or_default();
                s.push_str(&format!("\n- {}: {} {}", a.name, a.label, p));
            }
        }
        s
    }

    /// 序列化为 JSON（落盘用）。使用 Vec 表示，避免 HashMap 非字符串键无法序列化。
    pub fn to_json(&self) -> String {
        let g = self.inner.lock().unwrap();
        let snap = WorldMemorySnapshot {
            cells: g.cells.values().cloned().collect(),
            anchors: g.anchors.values().cloned().collect(),
        };
        serde_json::to_string(&snap).unwrap_or_else(|_| "{\"cells\":[],\"anchors\":[]}".to_string())
    }

    /// 从 JSON 载入（合并式：不清除已有，仅插入/覆盖）。
    pub fn load_json(&self, json: &str) {
        if let Ok(snap) = serde_json::from_str::<WorldMemorySnapshot>(json) {
            let mut lock = self.inner.lock().unwrap();
            for v in snap.cells {
                lock.by_chunk.entry(v.pos.chunk_key()).or_default().push(v.pos);
                lock.cells.insert(v.pos, v);
            }
            for v in snap.anchors {
                lock.anchors.insert(v.name.clone(), v);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_queries_nearby() {
        let m = WorldMemory::new();
        m.record_resource(MemoryPos::new(0, 64, 0), "oak_log", "橡树林", Some(6));
        m.record_structure(MemoryPos::new(10, 64, 0), "crafting_table", "我的工作台");
        let near = m.nearby(MemoryPos::new(2, 64, 0), 16, false);
        assert_eq!(near.len(), 2);
        // 锚点
        m.set_anchor("home", Some(MemoryPos::new(0, 64, 0)), "出生点");
        assert!(m.find_anchor("home").is_some());
        // 耗尽过滤
        m.mark_depleted(MemoryPos::new(0, 64, 0), true);
        assert_eq!(m.nearby(MemoryPos::new(2, 64, 0), 16, false).len(), 1);
        assert_eq!(m.nearby(MemoryPos::new(2, 64, 0), 16, true).len(), 2);
    }

    #[test]
    fn json_roundtrip() {
        let m = WorldMemory::new();
        m.record_resource(MemoryPos::new(5, 70, -3), "coal_ore", "煤", None);
        m.set_anchor("mine", Some(MemoryPos::new(5, 70, -3)), "矿洞");
        let j = m.to_json();
        let m2 = WorldMemory::new();
        m2.load_json(&j);
        assert!(!m2.is_empty());
        assert!(m2.find_anchor("mine").is_some());
    }

    /// 回归：扫描写入大量（175+）方块时 to_json 必须输出真实内容（曾因
    /// HashMap<MemoryPos> 非字符串键导致 serde_json 失败返回 "{}"）。
    #[test]
    fn json_roundtrip_many_cells_not_empty() {
        let m = WorldMemory::new();
        for dx in -8..=8i32 {
            for dy in -8..=8i32 {
                for dz in -8..=8i32 {
                    let p = MemoryPos::new(dx, 64 + dy, dz);
                    m.record_resource(p, "dark_oak_log", "树木/原木", None);
                }
            }
        }
        let j = m.to_json();
        // 必须不是 "{}" 空洞
        assert!(j.len() > 100, "to_json 返回了过短内容: {j}");
        assert!(!j.contains("\"cells\":[]"));
        let m2 = WorldMemory::new();
        m2.load_json(&j);
        assert_eq!(m2.len(), m.len(), "回放后条目数应一致");
    }

    /// 行动回写闭环：挖掉坐标后该资源记忆被移除（与 tools_azalea 的 Mine/Place 行为一致）。
    #[test]
    fn mine_forgets_cell_place_records() {
        let m = WorldMemory::new();
        let p = MemoryPos::new(3, 65, -2);
        m.record_resource(p, "oak_log", "橡树", None);
        assert_eq!(m.len(), 1);
        // 模拟 MineTool：挖掉后 forget
        m.forget_pos(p);
        assert_eq!(m.len(), 0);

        // 模拟 PlaceTool：放置工作台后记录为结构
        let q = MemoryPos::new(1, 64, 1);
        m.record(q, MemoryKind::Structure, Some("crafting_table"), "crafting_table", None);
        assert_eq!(m.len(), 1);
        let near = m.nearby(MemoryPos::new(0, 64, 0), 16, false);
        assert_eq!(near.len(), 1);
        assert_eq!(near[0].item.as_deref(), Some("crafting_table"));
    }
}
