//! Tool logic tests - tests the pure functions that tools use.
//! These don't need a real MC server.

mod mock_inventory;
mod mock_world;

use mock_inventory::{ItemKind, MockInventory};
use mock_world::{BlockKind, MockWorld, BlockPos};

// ============================================================
// Recipe Lookup Tests
// ============================================================

/// Test recipe lookup logic (extracted from craft.rs).
/// Given a target item, what ingredients are needed?
#[test]
fn recipe_oak_planks_from_oak_log() {
    // 1 oak_log → 4 oak_planks
    let ingredients = vec![(ItemKind::OakLog, 1)];
    let output_per_craft = 4;

    // To get 16 planks, need 4 logs
    let target_count = 16;
    let crafts_needed = (target_count + output_per_craft - 1) / output_per_craft;
    let total_oak_log = crafts_needed * ingredients[0].1;

    assert_eq!(crafts_needed, 4);
    assert_eq!(total_oak_log, 4);
}

#[test]
fn recipe_stick_from_planks() {
    // 2 oak_planks → 4 sticks
    let ingredients = vec![(ItemKind::OakPlanks, 2)];
    let output_per_craft = 4;

    // To get 8 sticks, need 2 crafts = 4 planks
    let target_count = 8;
    let crafts_needed = (target_count + output_per_craft - 1) / output_per_craft;
    let total_planks = crafts_needed * ingredients[0].1;

    assert_eq!(crafts_needed, 2);
    assert_eq!(total_planks, 4);
}

#[test]
fn recipe_crafting_table() {
    // 4 oak_planks → 1 crafting_table
    let ingredients = vec![(ItemKind::OakPlanks, 4)];
    let output_per_craft = 1;

    let target_count = 1;
    let crafts_needed = (target_count + output_per_craft - 1) / output_per_craft;
    let total_planks = crafts_needed * ingredients[0].1;

    assert_eq!(crafts_needed, 1);
    assert_eq!(total_planks, 4);
}

#[test]
fn recipe_torch() {
    // 1 coal + 1 stick → 4 torches
    let output_per_craft = 4;

    // To get 8 torches, need 2 crafts
    let target_count = 8;
    let crafts_needed = (target_count + output_per_craft - 1) / output_per_craft;

    assert_eq!(crafts_needed, 2);
    // Need 2 coal + 2 stick
}

// ============================================================
// Craft Planning Tests
// ============================================================

/// Test craft planning: given inventory and target, can we craft?
#[test]
fn craft_plan_can_craft_planks() {
    let mut inv = MockInventory::new(36);
    inv.add(ItemKind::OakLog, 4);

    // Want to craft 16 oak_planks
    let have_logs = inv.count_item(ItemKind::OakLog);
    let need_logs = 4; // 16 planks / 4 per craft = 4 logs

    assert!(have_logs >= need_logs);

    // Simulate crafting
    inv.consume(ItemKind::OakLog, need_logs);
    inv.add(ItemKind::OakPlanks, 16);

    assert_eq!(inv.count_item(ItemKind::OakLog), 0);
    assert_eq!(inv.count_item(ItemKind::OakPlanks), 16);
}

#[test]
fn craft_plan_cannot_craft_without_materials() {
    let inv = MockInventory::new(36);
    // No oak_log in inventory
    assert!(!inv.has_item(ItemKind::OakLog, 1));
}

#[test]
fn craft_plan_need_to_gather_first() {
    // Scenario: want to craft crafting_table but no planks
    let inv = MockInventory::new(36);

    // Need 4 planks for a crafting table
    let need_planks = 4;
    let have_planks = inv.count_item(ItemKind::OakPlanks);

    assert!(have_planks < need_planks);
    // Decision: need to gather oak_log first, then craft planks
}

// ============================================================
// Block Scanning Tests
// ============================================================

#[test]
fn scan_finds_nearest_oak_log() {
    let mut world = MockWorld::new();
    world.set_block(BlockPos::new(10, 64, 0), BlockKind::OakLog);
    world.set_block(BlockPos::new(3, 64, 0), BlockKind::OakLog);
    world.set_block(BlockPos::new(5, 64, 0), BlockKind::OakLog);

    let found = world.scan_blocks(BlockPos::new(0, 64, 0), BlockKind::OakLog, 16);
    assert_eq!(found.len(), 3);
    assert_eq!(found[0], BlockPos::new(3, 64, 0)); // Nearest first
}

#[test]
fn scan_finds_iron_ore() {
    let mut world = MockWorld::new();
    world.set_block(BlockPos::new(5, 30, 0), BlockKind::IronOre);
    world.set_block(BlockPos::new(8, 30, 0), BlockKind::IronOre);

    let found = world.scan_blocks(BlockPos::new(0, 30, 0), BlockKind::IronOre, 16);
    assert_eq!(found.len(), 2);
}

#[test]
fn scan_no_blocks_found() {
    let world = MockWorld::new();
    let found = world.scan_blocks(BlockPos::new(0, 64, 0), BlockKind::OakLog, 16);
    assert!(found.is_empty());
}

// ============================================================
// Tool Selection Tests
// ============================================================

/// Given a block type, what tool is needed?
#[test]
fn tool_selection_for_stone() {
    let block = BlockKind::Stone;
    // Stone needs a pickaxe
    let needs_pickaxe = matches!(block, BlockKind::Stone | BlockKind::IronOre | BlockKind::DiamondOre | BlockKind::CoalOre);
    assert!(needs_pickaxe);
}

#[test]
fn tool_selection_for_wood() {
    let block = BlockKind::OakLog;
    // Wood needs an axe (or can be mined by hand, just slower)
    let needs_axe = matches!(block, BlockKind::OakLog | BlockKind::BirchLog | BlockKind::SpruceLog | BlockKind::DarkOakLog);
    assert!(needs_axe);
}

#[test]
fn tool_selection_for_dirt() {
    let block = BlockKind::Dirt;
    // Dirt can be mined by hand (or shovel for speed)
    let needs_tool = matches!(block, BlockKind::Stone | BlockKind::IronOre | BlockKind::DiamondOre);
    assert!(!needs_tool);
}

// ============================================================
// Inventory Management Tests
// ============================================================

#[test]
fn inventory_crafting_workflow() {
    // Simulate: gather oak_log → craft planks → craft sticks → craft crafting_table
    let mut inv = MockInventory::new(36);

    // Step 1: Gather 4 oak_log
    inv.add(ItemKind::OakLog, 4);
    assert_eq!(inv.count_item(ItemKind::OakLog), 4);

    // Step 2: Craft 16 oak_planks (4 logs → 16 planks)
    inv.consume(ItemKind::OakLog, 4);
    inv.add(ItemKind::OakPlanks, 16);
    assert_eq!(inv.count_item(ItemKind::OakPlanks), 16);

    // Step 3: Craft 4 sticks (2 planks → 4 sticks)
    inv.consume(ItemKind::OakPlanks, 2);
    inv.add(ItemKind::Stick, 4);
    assert_eq!(inv.count_item(ItemKind::Stick), 4);
    assert_eq!(inv.count_item(ItemKind::OakPlanks), 14);

    // Step 4: Craft 1 crafting_table (4 planks → 1 table)
    inv.consume(ItemKind::OakPlanks, 4);
    inv.add(ItemKind::CraftingTable, 1);
    assert_eq!(inv.count_item(ItemKind::CraftingTable), 1);
    assert_eq!(inv.count_item(ItemKind::OakPlanks), 10);
}

#[test]
fn inventory_smelt_workflow() {
    // Simulate: smelt iron_ore with coal
    let mut inv = MockInventory::new(36);

    // Start with 8 iron_ore and 2 coal
    inv.add(ItemKind::IronOre, 8);
    inv.add(ItemKind::Coal, 2);

    // Smelt 2 iron_ore (each coal smelts 8 items, but we only have 2 ore)
    let to_smelt = 2;
    inv.consume(ItemKind::IronOre, to_smelt);
    inv.consume(ItemKind::Coal, 1); // 1 coal can smelt 8 items
    inv.add(ItemKind::IronIngot, to_smelt);

    assert_eq!(inv.count_item(ItemKind::IronOre), 6);
    assert_eq!(inv.count_item(ItemKind::Coal), 1);
    assert_eq!(inv.count_item(ItemKind::IronIngot), 2);
}

#[test]
fn inventory_pickaxe_crafting_workflow() {
    // Simulate: craft stone pickaxe (3 stone + 2 sticks)
    let mut inv = MockInventory::new(36);

    // Need 3 stone and 2 sticks
    inv.add(ItemKind::Stone, 3);
    inv.add(ItemKind::Stick, 2);

    assert!(inv.has_item(ItemKind::Stone, 3));
    assert!(inv.has_item(ItemKind::Stick, 2));

    // Craft pickaxe
    inv.consume(ItemKind::Stone, 3);
    inv.consume(ItemKind::Stick, 2);
    inv.add(ItemKind::StonePickaxe, 1);

    assert_eq!(inv.count_item(ItemKind::StonePickaxe), 1);
    assert_eq!(inv.count_item(ItemKind::Stone), 0);
    assert_eq!(inv.count_item(ItemKind::Stick), 0);
}

// ============================================================
// Scenario Tests (Decision Logic)
// ============================================================

/// Scenario: Bot spawns in forest, needs to gather wood.
/// Expected decision: gather("oak_log", 4)
#[test]
fn scenario_forest_gather_wood() {
    let mut world = MockWorld::new();
    world.set_block(BlockPos::new(5, 64, 0), BlockKind::OakLog);
    world.set_block(BlockPos::new(8, 64, 0), BlockKind::OakLog);

    let inv = MockInventory::new(36);

    // Bot has no wood, world has oak_log
    assert_eq!(inv.count_item(ItemKind::OakLog), 0);
    let logs = world.scan_blocks(BlockPos::new(0, 64, 0), BlockKind::OakLog, 16);
    assert!(!logs.is_empty());

    // Decision: gather oak_log
    // (In real bot, this would be the LLM's tool call)
}

/// Scenario: Bot has wood, needs to craft tools.
/// Expected decision: craft("oak_planks", 4) → craft("stick", 4) → craft("wooden_pickaxe", 1)
#[test]
fn scenario_craft_first_tools() {
    let mut inv = MockInventory::new(36);
    inv.add(ItemKind::OakLog, 4);

    // Step 1: Craft planks
    inv.consume(ItemKind::OakLog, 4);
    inv.add(ItemKind::OakPlanks, 16);
    assert_eq!(inv.count_item(ItemKind::OakPlanks), 16);

    // Step 2: Craft sticks
    inv.consume(ItemKind::OakPlanks, 2);
    inv.add(ItemKind::Stick, 4);
    assert_eq!(inv.count_item(ItemKind::Stick), 4);

    // Step 3: Craft crafting table
    inv.consume(ItemKind::OakPlanks, 4);
    inv.add(ItemKind::CraftingTable, 1);
    assert_eq!(inv.count_item(ItemKind::CraftingTable), 1);
}

/// Scenario: Bot is in cave, needs to mine stone.
/// Expected decision: gather("stone", 8) → craft("stone_pickaxe", 1)
#[test]
fn scenario_cave_mine_stone() {
    let world = MockWorld::with_default(BlockKind::Stone);
    let inv = MockInventory::new(36);

    // Bot has no stone, world is full of stone
    assert_eq!(inv.count_item(ItemKind::Stone), 0);
    let stone = world.scan_blocks(BlockPos::new(0, 30, 0), BlockKind::Stone, 8);
    assert!(!stone.is_empty());

    // Decision: gather stone (needs pickaxe, but can mine by hand slowly)
    // Better: craft wooden pickaxe first, then mine stone
}

/// Scenario: Bot has iron ore and coal, needs to smelt.
/// Expected decision: smelt("iron_ingot", "coal", 3)
#[test]
fn scenario_smelt_iron() {
    let mut inv = MockInventory::new(36);
    inv.add(ItemKind::IronOre, 3);
    inv.add(ItemKind::Coal, 1);

    assert!(inv.has_item(ItemKind::IronOre, 3));
    assert!(inv.has_item(ItemKind::Coal, 1));

    // Smelt 3 iron ore
    inv.consume(ItemKind::IronOre, 3);
    inv.consume(ItemKind::Coal, 1);
    inv.add(ItemKind::IronIngot, 3);

    assert_eq!(inv.count_item(ItemKind::IronIngot), 3);
}
