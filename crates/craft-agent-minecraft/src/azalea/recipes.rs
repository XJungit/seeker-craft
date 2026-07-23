//! 配方知识图（数据驱动）：驱动通用 `auto_craft`。
//!
//! 每条配方描述「产物 → 输入列表 + 合成方式」。azalea 未暴露配方查询 API，
//! 故这里手写维护常见 26.2 配方（覆盖早期/中期游戏）。新增配方只需加一行。

#[derive(Debug, Clone, PartialEq)]
pub enum Method {
    /// 2×2 背包合成（无需容器）。
    Craft2x2,
    /// 3×3 工作台合成（auto_craft 会自动造/放/开工作台）。
    Craft3x3,
    /// 熔炼（auto_craft 会自动造/放/开熔炉）。
    Smelt { fuel: &'static str },
    /// 直接采集的方块（树/石/矿/沙等）。
    Gather,
}

#[derive(Debug, Clone)]
pub struct Recipe {
    pub output: &'static str,
    pub inputs: &'static [( &'static str, u32 )],
    pub method: Method,
}

/// 全部配方（按产物查）。注意：同产物只列一种主流合成路径。
pub const RECIPES: &[Recipe] = &[
    // ---- 2×2 ----
    Recipe { output: "oak_planks", inputs: &[("oak_log", 1)], method: Method::Craft2x2 },
    Recipe { output: "stick", inputs: &[("oak_planks", 2)], method: Method::Craft2x2 },
    Recipe { output: "crafting_table", inputs: &[("oak_planks", 4)], method: Method::Craft2x2 },
    Recipe { output: "torch", inputs: &[("coal", 1), ("stick", 1)], method: Method::Craft2x2 },
    Recipe { output: "torch", inputs: &[("charcoal", 1), ("stick", 1)], method: Method::Craft2x2 },
    // ---- 3×3 ----
    Recipe { output: "chest", inputs: &[("oak_planks", 8)], method: Method::Craft3x3 },
    Recipe { output: "furnace", inputs: &[("cobblestone", 8)], method: Method::Craft3x3 },
    Recipe { output: "ladder", inputs: &[("stick", 7)], method: Method::Craft3x3 },
    Recipe { output: "oak_door", inputs: &[("oak_planks", 6)], method: Method::Craft3x3 },
    Recipe { output: "oak_trapdoor", inputs: &[("oak_planks", 4)], method: Method::Craft3x3 },
    Recipe { output: "oak_fence", inputs: &[("oak_planks", 4), ("stick", 2)], method: Method::Craft3x3 },
    Recipe { output: "crafting_table", inputs: &[("oak_planks", 4)], method: Method::Craft3x3 },
    // ---- 熔炼 ----
    Recipe { output: "iron_ingot", inputs: &[("iron_ore", 1)], method: Method::Smelt { fuel: "coal" } },
    Recipe { output: "iron_ingot", inputs: &[("raw_iron", 1)], method: Method::Smelt { fuel: "coal" } },
    Recipe { output: "copper_ingot", inputs: &[("copper_ore", 1)], method: Method::Smelt { fuel: "coal" } },
    Recipe { output: "gold_ingot", inputs: &[("gold_ore", 1)], method: Method::Smelt { fuel: "coal" } },
    Recipe { output: "glass", inputs: &[("sand", 1)], method: Method::Smelt { fuel: "coal" } },
    Recipe { output: "stone", inputs: &[("cobblestone", 1)], method: Method::Smelt { fuel: "coal" } },
    Recipe { output: "smooth_stone", inputs: &[("stone", 1)], method: Method::Smelt { fuel: "coal" } },
    Recipe { output: "charcoal", inputs: &[("oak_log", 1)], method: Method::Smelt { fuel: "oak_log" } },
    // ---- 可直接采集的方块 ----
    Recipe { output: "oak_log", inputs: &[], method: Method::Gather },
    Recipe { output: "cobblestone", inputs: &[], method: Method::Gather },
    Recipe { output: "coal", inputs: &[], method: Method::Gather },
    Recipe { output: "iron_ore", inputs: &[], method: Method::Gather },
    Recipe { output: "copper_ore", inputs: &[], method: Method::Gather },
    Recipe { output: "gold_ore", inputs: &[], method: Method::Gather },
    Recipe { output: "raw_iron", inputs: &[], method: Method::Gather },
    Recipe { output: "sand", inputs: &[], method: Method::Gather },
    Recipe { output: "stone", inputs: &[], method: Method::Gather },
];

/// 查产物配方（取第一条匹配）。
pub fn lookup(output: &str) -> Option<&'static Recipe> {
    let norm = if output.starts_with("minecraft:") {
        output.to_string()
    } else {
        format!("minecraft:{output}")
    };
    RECIPES
        .iter()
        .find(|r| r.output == norm || r.output == output)
}
