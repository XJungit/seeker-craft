//! 蓝图系统（P2-1）：预定义可复用建筑模板，按名称调用，自动相对坐标→绝对坐标展开。
//!
//! 学习自 Mindcraft `library/world.js` 的 buildable 概念：把建筑拆成可命名的蓝图，
//! bot 调用 `build_blueprint(name, origin_x, origin_y, origin_z)` 一键建造。
//!
//! 蓝图 JSON 格式（相对坐标 `dx/dy/dz`，原点 (0,0,0) 是蓝图基准角）：
//! ```json
//! {
//!   "name": "small_shelter",
//!   "description": "3x3 木屋，含工作台和熔炉",
//!   "blocks": [
//!     {"dx": 0, "dy": 0, "dz": 0, "block": "oak_planks"},
//!     {"dx": 1, "dy": 0, "dz": 0, "block": "crafting_table"},
//!     {"dx": 2, "dy": 0, "dz": 0, "block": "furnace"}
//!   ]
//! }
//! ```
//!
//! 蓝图库存放在 `blueprints/` 目录（与 tasks/ 同级），启动时由 `BlueprintLibrary::load_dir` 载入。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// 单个方块的相对坐标 + 物品 id。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlueprintBlock {
    pub dx: i32,
    pub dy: i32,
    pub dz: i32,
    pub block: String,
}

/// 蓝图：一组相对坐标的方块集合 + 元信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Blueprint {
    /// 蓝图唯一 id（与文件名同步，不带 .json 后缀）
    pub name: String,
    /// 人类可读描述
    pub description: String,
    /// 方块列表（相对坐标）
    pub blocks: Vec<BlueprintBlock>,
}

impl Blueprint {
    /// 把蓝图实例化到原点 (origin_x, origin_y, origin_z)，返回绝对坐标的 JSON 字符串。
    /// 输出格式与 BuildTool 的 blueprint 参数一致：`{"blocks":[{"x":..,"y":..,"z":..,"block":..}]}`
    pub fn instantiate(&self, origin_x: i32, origin_y: i32, origin_z: i32) -> String {
        let blocks: Vec<serde_json::Value> = self
            .blocks
            .iter()
            .map(|b| {
                serde_json::json!({
                    "x": origin_x + b.dx,
                    "y": origin_y + b.dy,
                    "z": origin_z + b.dz,
                    "block": b.block,
                })
            })
            .collect();
        serde_json::json!({ "blocks": blocks }).to_string()
    }

    /// 计算材料清单：返回 `HashMap<物品 id, 数量>`。
    pub fn material_list(&self) -> HashMap<String, u32> {
        let mut map: HashMap<String, u32> = HashMap::new();
        for b in &self.blocks {
            *map.entry(b.block.clone()).or_insert(0) += 1;
        }
        map
    }

    /// 材料清单的人类可读字符串（供 LLM 决策"先采集哪些材料"）。
    pub fn material_summary(&self) -> String {
        let mut items: Vec<(String, u32)> = self.material_list().into_iter().collect();
        items.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        items
            .iter()
            .map(|(id, n)| format!("{id}:{n}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// 蓝图占据的边界框 (min_x, min_y, min_z, max_x, max_y, max_z)（相对坐标）。
    pub fn bounds(&self) -> (i32, i32, i32, i32, i32, i32) {
        if self.blocks.is_empty() {
            return (0, 0, 0, 0, 0, 0);
        }
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut min_z = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;
        let mut max_z = i32::MIN;
        for b in &self.blocks {
            min_x = min_x.min(b.dx);
            min_y = min_y.min(b.dy);
            min_z = min_z.min(b.dz);
            max_x = max_x.max(b.dx);
            max_y = max_y.max(b.dy);
            max_z = max_z.max(b.dz);
        }
        (min_x, min_y, min_z, max_x, max_y, max_z)
    }
}

/// 蓝图库：按名称索引的蓝图集合。
#[derive(Debug, Clone, Default)]
pub struct BlueprintLibrary {
    blueprints: HashMap<String, Blueprint>,
}

impl BlueprintLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    /// 从目录加载所有 `*.json` 蓝图。文件名（去 .json）即为蓝图 name。
    /// 文件内的 name 字段若与文件名不一致，以文件名为准（覆盖）。
    pub fn load_dir(dir: &Path) -> Self {
        let mut lib = Self::default();
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return lib,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let stem = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let text = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            match serde_json::from_str::<Blueprint>(&text) {
                Ok(mut bp) => {
                    bp.name = stem.clone();
                    lib.blueprints.insert(stem, bp);
                }
                Err(e) => {
                    eprintln!("[blueprint] 解析 {} 失败: {e}", path.display());
                }
            }
        }
        lib
    }

    /// 内置一张蓝图（程序构造，无文件）。
    pub fn insert(&mut self, bp: Blueprint) {
        self.blueprints.insert(bp.name.clone(), bp);
    }

    /// 按名称查询蓝图。
    pub fn get(&self, name: &str) -> Option<&Blueprint> {
        self.blueprints.get(name)
    }

    /// 列出所有蓝图名 + 描述（供 LLM 选择）。
    pub fn list(&self) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = self
            .blueprints
            .iter()
            .map(|(k, b)| (k.clone(), b.description.clone()))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    /// 列出所有蓝图名 + 描述的人类可读字符串。
    pub fn list_summary(&self) -> String {
        let items = self.list();
        if items.is_empty() {
            return "（无蓝图）".to_string();
        }
        items
            .iter()
            .map(|(n, d)| format!("- {n}: {d}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 蓝图数量。
    pub fn len(&self) -> usize {
        self.blueprints.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.blueprints.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_bp() -> Blueprint {
        Blueprint {
            name: "test".to_string(),
            description: "测试蓝图".to_string(),
            blocks: vec![
                BlueprintBlock {
                    dx: 0,
                    dy: 0,
                    dz: 0,
                    block: "oak_planks".into(),
                },
                BlueprintBlock {
                    dx: 1,
                    dy: 0,
                    dz: 0,
                    block: "oak_planks".into(),
                },
                BlueprintBlock {
                    dx: 0,
                    dy: 0,
                    dz: 1,
                    block: "crafting_table".into(),
                },
                BlueprintBlock {
                    dx: 1,
                    dy: 1,
                    dz: 0,
                    block: "torch".into(),
                },
            ],
        }
    }

    #[test]
    fn instantiate_translates_to_absolute_coords() {
        let bp = sample_bp();
        let json = bp.instantiate(10, 64, 20);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let blocks = v.get("blocks").unwrap().as_array().unwrap();
        assert_eq!(blocks.len(), 4);
        // 第一个 (0,0,0) → (10,64,20)
        assert_eq!(blocks[0]["x"], 10);
        assert_eq!(blocks[0]["y"], 64);
        assert_eq!(blocks[0]["z"], 20);
        assert_eq!(blocks[0]["block"], "oak_planks");
        // 第四个 (1,1,0) → (11,65,20)
        assert_eq!(blocks[3]["x"], 11);
        assert_eq!(blocks[3]["y"], 65);
        assert_eq!(blocks[3]["z"], 20);
        assert_eq!(blocks[3]["block"], "torch");
    }

    #[test]
    fn material_list_counts_correctly() {
        let bp = sample_bp();
        let ml = bp.material_list();
        assert_eq!(ml.get("oak_planks"), Some(&2));
        assert_eq!(ml.get("crafting_table"), Some(&1));
        assert_eq!(ml.get("torch"), Some(&1));
    }

    #[test]
    fn bounds_returns_min_max() {
        let bp = sample_bp();
        let (min_x, min_y, min_z, max_x, max_y, max_z) = bp.bounds();
        assert_eq!((min_x, min_y, min_z), (0, 0, 0));
        assert_eq!((max_x, max_y, max_z), (1, 1, 1));
    }

    #[test]
    fn material_summary_sorts_by_count_desc() {
        let bp = sample_bp();
        let s = bp.material_summary();
        // oak_planks:2 应排在最前
        assert!(s.starts_with("oak_planks:2"), "got: {s}");
    }

    #[test]
    fn empty_bounds_returns_zeros() {
        let bp = Blueprint {
            name: "empty".into(),
            description: "".into(),
            blocks: vec![],
        };
        assert_eq!(bp.bounds(), (0, 0, 0, 0, 0, 0));
    }

    #[test]
    fn library_load_dir_loads_json_files() {
        // 写一份临时蓝图到 tmp 目录测试 load_dir
        let tmp = std::env::temp_dir().join("craft_agent_bp_test");
        let _ = std::fs::create_dir_all(&tmp);
        let bp_path = tmp.join("hello.json");
        std::fs::write(
            &bp_path,
            r#"{"name":"hello","description":"hi","blocks":[{"dx":0,"dy":0,"dz":0,"block":"stone"}]}"#,
        )
        .unwrap();
        let lib = BlueprintLibrary::load_dir(&tmp);
        assert_eq!(lib.len(), 1);
        assert!(lib.get("hello").is_some());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn library_list_summary_handles_empty() {
        let lib = BlueprintLibrary::new();
        assert_eq!(lib.list_summary(), "（无蓝图）");
    }
}
