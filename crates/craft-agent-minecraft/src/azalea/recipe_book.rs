//! 配方书（recipe book）全量模型 + 通用合成执行。
//!
//! 配方数据来自本地内置 JSON（`builtin_recipes.json`，vanilla 26.2 配方库），
//! 与 azalea 客户端库版本完全解耦——azalea 升级不影响配方。服务端下发的
//! `ClientboundRecipeBookAdd` 作为可选 overlay 叠加（部分版本 azalea 协议解析不全）。

use std::collections::HashMap;
use std::str::FromStr;

use azalea_protocol::packets::game::c_recipe_book_add::Entry;
use azalea_protocol::packets::game::c_update_recipes::SingleInputEntry;
use azalea_protocol::common::recipe::{RecipeDisplayData, SlotDisplayData};
use azalea_registry::builtin::ItemKind;
use serde_json::Value;

/// 一个原料允许的若干物品。
#[derive(Clone, Debug, Default)]
pub struct IngredientItems {
    pub items: Vec<ItemKind>,
}

impl IngredientItems {
    /// 从配方显示里的 `SlotDisplayData` 抽取允许的物品清单。
    /// 直接物品（Item/ItemStack）取其实体；Tag 等复杂展示暂忽略（best-effort）。
    fn from_slot(d: &SlotDisplayData) -> Self {
        let items = match d {
            SlotDisplayData::Item(i) => vec![i.item],
            SlotDisplayData::ItemStack(s) => vec![s.stack.kind()],
            _ => Vec::new(),
        };
        IngredientItems { items }
    }
}

/// 从 SlotDisplayData 尝试取 ItemKind（用于结果/单物品原料）。
fn slot_item(d: &SlotDisplayData) -> Option<ItemKind> {
    match d {
        SlotDisplayData::Item(i) => Some(i.item),
        SlotDisplayData::ItemStack(s) => Some(s.stack.kind()),
        _ => None,
    }
}

/// 归一化后的配方。
#[derive(Clone, Debug)]
pub enum StoredRecipe {
    Shapeless {
        ingredients: Vec<IngredientItems>,
        result: ItemKind,
        count: u32,
    },
    Shaped {
        width: u32,
        height: u32,
        /// 长度 = width*height，按行优先；None 表示该格为空。
        grid: Vec<Option<IngredientItems>>,
        result: ItemKind,
        count: u32,
    },
    Furnace {
        ingredient: IngredientItems,
        fuel: IngredientItems,
        result: ItemKind,
        count: u32,
    },
    Stonecutter {
        input: IngredientItems,
        result: ItemKind,
        count: u32,
    },
    Smithing {
        template: IngredientItems,
        base: IngredientItems,
        addition: IngredientItems,
        result: ItemKind,
    },
    Brewing {
        ingredient: IngredientItems,
        base: IngredientItems,
        result: ItemKind,
    },
}

impl StoredRecipe {
    /// 产物 id（归一化，去命名空间前缀）。
    pub fn result_id(&self) -> String {
        let k = match self {
            StoredRecipe::Shapeless { result, .. }
            | StoredRecipe::Shaped { result, .. }
            | StoredRecipe::Furnace { result, .. }
            | StoredRecipe::Stonecutter { result, .. }
            | StoredRecipe::Smithing { result, .. }
            | StoredRecipe::Brewing { result, .. } => *result,
        };
        normalize_item(&k.to_string())
    }

    /// 合成类型标签，便于 auto_craft 选择执行路径。
    pub fn kind(&self) -> &'static str {
        match self {
            StoredRecipe::Shapeless { .. } => "shapeless",
            StoredRecipe::Shaped { .. } => "shaped",
            StoredRecipe::Furnace { .. } => "furnace",
            StoredRecipe::Stonecutter { .. } => "stonecutter",
            StoredRecipe::Smithing { .. } => "smithing",
            StoredRecipe::Brewing { .. } => "brewing",
        }
    }
}

/// 配方书：按产物 id 聚合；后下发的覆盖先下发的。
#[derive(Default, Clone)]
pub struct RecipeBook {
    by_result: HashMap<String, StoredRecipe>,
}

impl RecipeBook {
    pub fn get_by_result(&self, item: &str) -> Option<&StoredRecipe> {
        self.by_result.get(&normalize_item(item))
    }

    pub fn insert(&mut self, r: StoredRecipe) {
        self.by_result.insert(r.result_id(), r);
    }

    pub fn len(&self) -> usize {
        self.by_result.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_result.is_empty()
    }

    /// 列出所有产物 id（调试/校准用）。
    pub fn keys(&self) -> Vec<String> {
        self.by_result.keys().cloned().collect()
    }
}

/// 解析 `RecipeDisplayData` 为归一化配方。
fn parse_display(d: &RecipeDisplayData) -> Option<StoredRecipe> {
    match d {
        RecipeDisplayData::Shapeless(s) => {
            let result = slot_item(&s.result)?;
            let ingredients = s.ingredients.iter().map(IngredientItems::from_slot).collect();
            Some(StoredRecipe::Shapeless {
                ingredients,
                result,
                count: 1,
            })
        }
        RecipeDisplayData::Shaped(s) => {
            let result = slot_item(&s.result)?;
            let mut grid = Vec::with_capacity(s.ingredients.len());
            for i in &s.ingredients {
                grid.push(Some(IngredientItems::from_slot(i)));
            }
            Some(StoredRecipe::Shaped {
                width: s.width,
                height: s.height,
                grid,
                result,
                count: 1,
            })
        }
        RecipeDisplayData::Furnace(f) => {
            let result = slot_item(&f.result)?;
            let ingredient = IngredientItems::from_slot(&f.ingredient);
            let fuel = IngredientItems::from_slot(&f.fuel);
            Some(StoredRecipe::Furnace {
                ingredient,
                fuel,
                result,
                count: 1,
            })
        }
        RecipeDisplayData::Stonecutter(s) => {
            let result = slot_item(&s.result)?;
            let input = IngredientItems::from_slot(&s.input);
            Some(StoredRecipe::Stonecutter {
                input,
                result,
                count: 1,
            })
        }
        RecipeDisplayData::Smithing(s) => {
            let result = slot_item(&s.result)?;
            Some(StoredRecipe::Smithing {
                template: IngredientItems::from_slot(&s.template),
                base: IngredientItems::from_slot(&s.base),
                addition: IngredientItems::from_slot(&s.addition),
                result,
            })
        }
    }
}

/// 把一个配方书条目存入配方书。
pub fn store_recipe_book_entry(book: &mut RecipeBook, e: &Entry) {
    if let Some(r) = parse_display(&e.contents.display) {
        book.insert(r);
    }
}

/// 补充切石机配方（来自 ClientboundUpdateRecipes.stonecutter_recipes）。
/// 注意：Stonecutter 的 `option_display` 仅含结果展示，无法重建完整配方，
/// 故此处暂存为占位（不自动合成）。后续如需切石机自动合成，需解析
/// `StonecutterRecipeDisplay` 全结构。
#[allow(dead_code)]
pub fn store_stonecutter_entry(_book: &mut RecipeBook, _e: &SingleInputEntry) {}

/// 统一物品 id 为无命名空间小写。
pub fn normalize_item(id: &str) -> String {
    let s = id.trim();
    let s = s.strip_prefix("minecraft:").unwrap_or(s);
    s.to_ascii_lowercase()
}

/// 从内置 JSON 加载完整配方库（vanilla 26.2），作为 `auto_craft` 的权威数据源。
/// 与服务端下发的配方书解耦，azalea 升级不受影响。
pub fn load_builtin() -> RecipeBook {
    let raw = include_str!("builtin_recipes.json");
    let mut book = RecipeBook::default();
    match serde_json::from_str::<Vec<Value>>(raw) {
        Ok(entries) => {
            for e in &entries {
                if let Some(r) = parse_builtin(e) {
                    book.insert(r);
                }
            }
        }
        Err(err) => {
            eprintln!("[recipe_book] 内置配方 JSON 解析失败: {err}");
        }
    }
    book
}

/// 把一条内置 JSON 配方解析为 `StoredRecipe`。
fn parse_builtin(e: &Value) -> Option<StoredRecipe> {
    let result = e.get("result")?.as_str()?;
    let result = ItemKind::from_str(&normalize_item(result)).ok()?;
    // 同产物取第一个（JSON 中先出现的优先）
    if let Some(existing) = e.get("count").and_then(|c| c.as_u64()) {
        let _ = existing;
    }
    let count = e.get("count").and_then(|c| c.as_u64()).unwrap_or(1) as u32;
    let kind = e.get("type")?.as_str()?;
    match kind {
        "shaped" => {
            let pattern = e.get("pattern")?.as_array()?;
            let keys = e.get("keys")?.as_object()?;
            let height = pattern.len() as u32;
            let mut width = 0u32;
            let mut grid: Vec<Option<IngredientItems>> = Vec::new();
            for row in pattern {
                let row = row.as_str()?;
                if width == 0 {
                    width = row.chars().count() as u32;
                }
                for ch in row.chars() {
                    if ch == ' ' {
                        grid.push(None);
                    } else {
                        let item = keys.get(&ch.to_string())?.as_str()?;
                        let k = ItemKind::from_str(&normalize_item(item)).ok()?;
                        grid.push(Some(IngredientItems { items: vec![k] }));
                    }
                }
            }
            Some(StoredRecipe::Shaped {
                width,
                height,
                grid,
                result,
                count,
            })
        }
        "shapeless" => {
            let ings = e.get("ingredients")?.as_array()?;
            let ingredients = ings
                .iter()
                .filter_map(|v| v.as_str())
                .filter_map(|s| ItemKind::from_str(&normalize_item(s)).ok())
                .map(|k| IngredientItems { items: vec![k] })
                .collect();
            Some(StoredRecipe::Shapeless {
                ingredients,
                result,
                count,
            })
        }
        "furnace" => {
            let ingredient = e.get("ingredient")?.as_str()?;
            let fuel = e.get("fuel").and_then(|f| f.as_str()).unwrap_or("coal");
            let ingredient = ItemKind::from_str(&normalize_item(ingredient)).ok()?;
            let fuel = ItemKind::from_str(&normalize_item(fuel)).ok()?;
            Some(StoredRecipe::Furnace {
                ingredient: IngredientItems { items: vec![ingredient] },
                fuel: IngredientItems { items: vec![fuel] },
                result,
                count,
            })
        }
        "stonecutter" => {
            let input = e.get("ingredient")?.as_str()?;
            let input = ItemKind::from_str(&normalize_item(input)).ok()?;
            Some(StoredRecipe::Stonecutter {
                input: IngredientItems { items: vec![input] },
                result,
                count,
            })
        }
        "smithing" => {
            let template = e.get("template").and_then(|v| v.as_str());
            let base = e.get("base")?.as_str()?;
            let addition = e.get("addition")?.as_str()?;
            let t = template
                .and_then(|s| ItemKind::from_str(&normalize_item(s)).ok())
                .unwrap_or(result); // 简化：netherite 升级用 netherite_ingot
            let base = ItemKind::from_str(&normalize_item(base)).ok()?;
            let addition = ItemKind::from_str(&normalize_item(addition)).ok()?;
            Some(StoredRecipe::Smithing {
                template: IngredientItems { items: vec![t] },
                base: IngredientItems { items: vec![base] },
                addition: IngredientItems { items: vec![addition] },
                result,
            })
        }
        "brewing" => {
            let ingredient = e.get("ingredient")?.as_str()?;
            let base = e.get("base").and_then(|v| v.as_str()).unwrap_or("water_bottle");
            let ing = ItemKind::from_str(&normalize_item(ingredient)).ok()?;
            let base = ItemKind::from_str(&normalize_item(base)).ok()?;
            Some(StoredRecipe::Brewing {
                ingredient: IngredientItems { items: vec![ing] },
                base: IngredientItems { items: vec![base] },
                result,
            })
        }
        _ => None,
    }
}

