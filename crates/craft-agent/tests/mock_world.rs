//! Mock world for testing tool logic without a real MC server.
//! Implements just enough of azalea's World interface for tool tests.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl BlockPos {
    pub fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Air,
    Stone,
    Dirt,
    GrassBlock,
    OakLog,
    OakPlanks,
    CoalOre,
    IronOre,
    DiamondOre,
    CraftingTable,
    Furnace,
    Chest,
    Water,
    Lava,
    Sand,
    Gravel,
    BirchLog,
    SpruceLog,
    DarkOakLog,
    Unknown,
}

#[derive(Debug, Clone, Copy)]
pub struct BlockState {
    kind: BlockKind,
}

impl BlockState {
    pub fn new(kind: BlockKind) -> Self {
        Self { kind }
    }

    pub fn is_air(&self) -> bool {
        self.kind == BlockKind::Air
    }
}

impl From<BlockState> for BlockKind {
    fn from(s: BlockState) -> Self {
        s.kind
    }
}

/// Mock world storage for testing.
pub struct MockWorld {
    blocks: HashMap<(i32, i32, i32), BlockKind>,
    default: BlockKind,
}

impl MockWorld {
    pub fn new() -> Self {
        Self {
            blocks: HashMap::new(),
            default: BlockKind::Air,
        }
    }

    pub fn with_default(default: BlockKind) -> Self {
        Self {
            blocks: HashMap::new(),
            default,
        }
    }

    pub fn set_block(&mut self, pos: BlockPos, kind: BlockKind) {
        self.blocks.insert((pos.x, pos.y, pos.z), kind);
    }

    pub fn set_range(
        &mut self,
        x1: i32,
        y1: i32,
        z1: i32,
        x2: i32,
        y2: i32,
        z2: i32,
        kind: BlockKind,
    ) {
        for x in x1..=x2 {
            for y in y1..=y2 {
                for z in z1..=z2 {
                    self.set_block(BlockPos::new(x, y, z), kind);
                }
            }
        }
    }

    pub fn get_block_state(&self, pos: BlockPos) -> Option<BlockState> {
        let kind = self
            .blocks
            .get(&(pos.x, pos.y, pos.z))
            .copied()
            .unwrap_or(self.default);
        Some(BlockState::new(kind))
    }

    pub fn remove_block(&mut self, pos: BlockPos) {
        self.blocks.insert((pos.x, pos.y, pos.z), BlockKind::Air);
    }

    /// Scan for blocks of a given kind within radius, sorted by distance.
    pub fn scan_blocks(&self, center: BlockPos, kind: BlockKind, radius: i32) -> Vec<BlockPos> {
        let mut found = Vec::new();
        for dx in -radius..=radius {
            for dy in -radius..=radius {
                for dz in -radius..=radius {
                    let pos = BlockPos::new(center.x + dx, center.y + dy, center.z + dz);
                    if let Some(state) = self.get_block_state(pos) {
                        let bk: BlockKind = state.into();
                        if bk == kind {
                            found.push(pos);
                        }
                    }
                }
            }
        }
        found.sort_by_key(|p| {
            (p.x - center.x).pow(2) + (p.y - center.y).pow(2) + (p.z - center.z).pow(2)
        });
        found
    }
}

impl Default for MockWorld {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_world_set_and_get() {
        let mut world = MockWorld::new();
        world.set_block(BlockPos::new(0, 64, 0), BlockKind::Stone);
        let state = world.get_block_state(BlockPos::new(0, 64, 0)).unwrap();
        let block_kind: BlockKind = state.into();
        assert_eq!(block_kind, BlockKind::Stone);
    }

    #[test]
    fn mock_world_default() {
        let world = MockWorld::with_default(BlockKind::Stone);
        let state = world.get_block_state(BlockPos::new(100, 100, 100)).unwrap();
        let block_kind: BlockKind = state.into();
        assert_eq!(block_kind, BlockKind::Stone);
    }

    #[test]
    fn mock_world_scan_blocks() {
        let mut world = MockWorld::new();
        world.set_block(BlockPos::new(5, 64, 0), BlockKind::OakLog);
        world.set_block(BlockPos::new(10, 64, 0), BlockKind::OakLog);
        world.set_block(BlockPos::new(3, 64, 0), BlockKind::OakLog);

        let found = world.scan_blocks(BlockPos::new(0, 64, 0), BlockKind::OakLog, 16);
        assert_eq!(found.len(), 3);
        // Closest first
        assert_eq!(found[0], BlockPos::new(3, 64, 0));
        assert_eq!(found[1], BlockPos::new(5, 64, 0));
        assert_eq!(found[2], BlockPos::new(10, 64, 0));
    }

    #[test]
    fn mock_world_remove_block() {
        let mut world = MockWorld::new();
        world.set_block(BlockPos::new(0, 64, 0), BlockKind::Stone);
        world.remove_block(BlockPos::new(0, 64, 0));
        let state = world.get_block_state(BlockPos::new(0, 64, 0)).unwrap();
        assert!(state.is_air());
    }
}
