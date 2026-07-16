//! 蓝图系统 —— 参考 Mindcraft 的 JSON 蓝图格式，支持按层建造。
//!
//! 格式: { "name": "...", "offset": i32, "blocks": [[[block_name, ...], ...], ...] }
//! blocks[y][z][x] = block_name（"" 或 null 表示空）

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintDef {
    pub name: String,
    #[serde(default)]
    pub offset: i32,
    pub blocks: Vec<Vec<Vec<String>>>,
}

#[derive(Debug, Clone)]
pub enum BuildAction {
    Place(String),
    Dig,
}

#[derive(Debug, Clone)]
pub struct BuildStep {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub action: BuildAction,
}

impl BlueprintDef {
    /// 从 JSON 字符串加载蓝图
    pub fn from_json(json: &str) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(json)?)
    }

    /// 生成建造步骤列表（按 y→z→x 顺序），orientation 0-3 = 0/90/180/270 度旋转。
    pub fn build_steps(&self, ox: i32, oy: i32, oz: i32, orientation: u32) -> Vec<BuildStep> {
        let mut steps = Vec::new();
        for (y, layer) in self.blocks.iter().enumerate() {
            let sizez = layer.len() as i32;
            for (z, row) in layer.iter().enumerate() {
                let sizex = row.len() as i32;
                for (x, block_name) in row.iter().enumerate() {
                    if block_name.is_empty() || block_name == "null" || block_name == "air" {
                        continue;
                    }
                    // 旋转后的局部坐标（0-3 表示 0/90/180/270 度）
                    let (rx, rz) = rotate_xz(x as i32, z as i32, orientation, sizex, sizez);
                    let wx = ox + rx;
                    let wy = oy + self.offset + y as i32;
                    let wz = oz + rz;
                    if block_name == "dig" {
                        steps.push(BuildStep {
                            x: wx,
                            y: wy,
                            z: wz,
                            action: BuildAction::Dig,
                        });
                    } else {
                        steps.push(BuildStep {
                            x: wx,
                            y: wy,
                            z: wz,
                            action: BuildAction::Place(block_name.clone()),
                        });
                    }
                }
            }
        }
        steps
    }

    /// 统计所需材料
    pub fn materials_needed(&self) -> HashMap<String, u32> {
        let mut mats = HashMap::new();
        for layer in &self.blocks {
            for row in layer {
                for block_name in row {
                    if block_name.is_empty()
                        || block_name == "null"
                        || block_name == "air"
                        || block_name == "dig"
                    {
                        continue;
                    }
                    *mats.entry(block_name.clone()).or_insert(0) += 1;
                }
            }
        }
        mats
    }
}

fn rotate_xz(x: i32, z: i32, orientation: u32, sizex: i32, sizez: i32) -> (i32, i32) {
    match orientation % 4 {
        0 => (x, z),
        1 => (sizez - 1 - z, x),
        2 => (sizex - 1 - x, sizez - 1 - z),
        3 => (z, sizex - 1 - x),
        _ => (x, z),
    }
}

/// 内置蓝图库
pub fn builtin_blueprints() -> Vec<(&'static str, &'static str)> {
    vec![
        ("dirt_shelter", DIRT_SHELTER_JSON),
        ("wood_house", WOOD_HOUSE_JSON),
        ("stone_house", STONE_HOUSE_JSON),
        ("wall_3x3", WALL_3X3_JSON),
    ]
}

/// 查找内置蓝图
pub fn get_blueprint(name: &str) -> Option<BlueprintDef> {
    for (n, json) in builtin_blueprints() {
        if n == name {
            return BlueprintDef::from_json(json).ok();
        }
    }
    None
}

// ══════════════════════════════════════════════════════════════
// 内置蓝图 JSON
// ══════════════════════════════════════════════════════════════

/// 3x3 泥土庇护所（参考 Mindcraft dirt_shelter）
pub const DIRT_SHELTER_JSON: &str = r#"{
    "name": "dirt_shelter",
    "offset": 0,
    "blocks": [
        [
            ["dirt","dirt","dirt"],
            ["dirt","dirt","dirt"],
            ["dirt","dirt","dirt"]
        ],
        [
            ["dirt","","dirt"],
            ["","",""],
            ["dirt","","dirt"]
        ],
        [
            ["dirt","","dirt"],
            ["","","dirt"],
            ["dirt","dirt","dirt"]
        ]
    ]
}"#;

/// 5x5 木屋
pub const WOOD_HOUSE_JSON: &str = r#"{
    "name": "wood_house",
    "offset": 0,
    "blocks": [
        [
            ["oak_planks","oak_planks","oak_planks","oak_planks","oak_planks"],
            ["oak_planks","oak_planks","oak_planks","oak_planks","oak_planks"],
            ["oak_planks","oak_planks","oak_planks","oak_planks","oak_planks"],
            ["oak_planks","oak_planks","oak_planks","oak_planks","oak_planks"],
            ["oak_planks","oak_planks","oak_planks","oak_planks","oak_planks"]
        ],
        [
            ["oak_planks","oak_planks","oak_planks","oak_planks","oak_planks"],
            ["oak_planks","","","","oak_planks"],
            ["oak_planks","","crafting_table","","oak_planks"],
            ["oak_planks","","","","oak_planks"],
            ["oak_planks","oak_door","oak_planks","oak_planks","oak_planks"]
        ],
        [
            ["oak_planks","oak_planks","oak_planks","oak_planks","oak_planks"],
            ["oak_planks","","","","oak_planks"],
            ["oak_planks","","","","oak_planks"],
            ["oak_planks","","","","oak_planks"],
            ["oak_planks","oak_planks","oak_planks","oak_planks","oak_planks"]
        ],
        [
            ["oak_planks","oak_planks","oak_planks","oak_planks","oak_planks"],
            ["oak_planks","oak_planks","oak_planks","oak_planks","oak_planks"],
            ["oak_planks","oak_planks","oak_planks","oak_planks","oak_planks"],
            ["oak_planks","oak_planks","oak_planks","oak_planks","oak_planks"],
            ["oak_planks","oak_planks","oak_planks","oak_planks","oak_planks"]
        ]
    ]
}"#;

/// 5x5 石屋
pub const STONE_HOUSE_JSON: &str = r#"{
    "name": "stone_house",
    "offset": 0,
    "blocks": [
        [
            ["stone","stone","stone","stone","stone"],
            ["stone","stone","stone","stone","stone"],
            ["stone","stone","stone","stone","stone"],
            ["stone","stone","stone","stone","stone"],
            ["stone","stone","stone","stone","stone"]
        ],
        [
            ["stone","stone","stone","stone","stone"],
            ["stone","","","","stone"],
            ["stone","","furnace","","stone"],
            ["stone","","","","stone"],
            ["stone","oak_door","stone","stone","stone"]
        ],
        [
            ["stone","stone","stone","stone","stone"],
            ["stone","","","","stone"],
            ["stone","","","","stone"],
            ["stone","","","","stone"],
            ["stone","stone","stone","stone","stone"]
        ],
        [
            ["stone","stone","stone","stone","stone"],
            ["stone","stone","stone","stone","stone"],
            ["stone","stone","stone","stone","stone"],
            ["stone","stone","stone","stone","stone"],
            ["stone","stone","stone","stone","stone"]
        ]
    ]
}"#;

/// 3x3 墙壁
pub const WALL_3X3_JSON: &str = r#"{
    "name": "wall_3x3",
    "offset": 0,
    "blocks": [
        [
            ["oak_planks","oak_planks","oak_planks"],
            ["oak_planks","oak_planks","oak_planks"],
            ["oak_planks","oak_planks","oak_planks"]
        ]
    ]
}"#;
