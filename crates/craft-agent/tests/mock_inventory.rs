//! Mock inventory for testing tool logic without a real MC server.
#![allow(dead_code)]

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Air,
    OakLog,
    OakPlanks,
    Stick,
    Coal,
    IronOre,
    IronIngot,
    Diamond,
    WoodenPickaxe,
    StonePickaxe,
    IronPickaxe,
    WoodenAxe,
    StoneAxe,
    IronAxe,
    CraftingTable,
    Furnace,
    Torch,
    Bread,
    Dirt,
    Stone,
    Gravel,
    Sand,
    ClayBall,
    Flint,
    Leather,
    Feather,
    RawIron,
    RawGold,
    Apple,
    Sword,
    Shovel,
    Hoe,
    Pickaxe,
    Axe,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct ItemStack {
    pub kind: ItemKind,
    pub count: u32,
}

impl ItemStack {
    pub fn new(kind: ItemKind, count: u32) -> Self {
        Self { kind, count }
    }

    pub fn is_empty(&self) -> bool {
        self.kind == ItemKind::Air || self.count == 0
    }
}

/// Mock inventory for testing tool logic.
pub struct MockInventory {
    slots: HashMap<usize, ItemStack>,
    size: usize,
}

impl MockInventory {
    pub fn new(size: usize) -> Self {
        Self {
            slots: HashMap::new(),
            size,
        }
    }

    pub fn with_items(items: Vec<(usize, ItemKind, u32)>) -> Self {
        let mut inv = Self::new(36);
        for (slot, kind, count) in items {
            inv.set_slot(slot, ItemStack::new(kind, count));
        }
        inv
    }

    pub fn set_slot(&mut self, slot: usize, item: ItemStack) {
        if slot < self.size {
            self.slots.insert(slot, item);
        }
    }

    pub fn get_slot(&self, slot: usize) -> Option<&ItemStack> {
        self.slots.get(&slot)
    }

    pub fn remove_slot(&mut self, slot: usize) -> Option<ItemStack> {
        self.slots.remove(&slot)
    }

    /// Count total of an item kind across all slots.
    pub fn count_item(&self, kind: ItemKind) -> u32 {
        self.slots
            .values()
            .filter(|s| s.kind == kind)
            .map(|s| s.count)
            .sum()
    }

    /// Check if inventory has at least `count` of `kind`.
    pub fn has_item(&self, kind: ItemKind, count: u32) -> bool {
        self.count_item(kind) >= count
    }

    /// Consume `count` of `kind` from inventory. Returns true if successful.
    pub fn consume(&mut self, kind: ItemKind, count: u32) -> bool {
        if self.count_item(kind) < count {
            return false;
        }
        let mut remaining = count;
        let slots_with_kind: Vec<usize> = self
            .slots
            .iter()
            .filter(|(_, s)| s.kind == kind)
            .map(|(i, _)| *i)
            .collect();
        for slot in slots_with_kind {
            if remaining == 0 {
                break;
            }
            if let Some(item) = self.slots.get_mut(&slot) {
                let take = item.count.min(remaining);
                item.count -= take;
                remaining -= take;
                if item.count == 0 {
                    self.slots.remove(&slot);
                }
            }
        }
        true
    }

    /// Add `count` of `kind` to inventory. Returns true if successful.
    pub fn add(&mut self, kind: ItemKind, count: u32) -> bool {
        // Try to stack first
        for item in self.slots.values_mut() {
            if item.kind == kind {
                item.count += count;
                return true;
            }
        }
        // Find empty slot
        for slot in 0..self.size {
            if let std::collections::hash_map::Entry::Vacant(e) = self.slots.entry(slot) {
                e.insert(ItemStack::new(kind, count));
                return true;
            }
        }
        false // Inventory full
    }

    /// Find the first slot containing `kind`, returns (slot, count).
    pub fn find_item(&self, kind: ItemKind) -> Option<(usize, u32)> {
        self.slots
            .iter()
            .find(|(_, s)| s.kind == kind)
            .map(|(i, s)| (*i, s.count))
    }

    /// Get total number of occupied slots.
    pub fn occupied_slots(&self) -> usize {
        self.slots.len()
    }

    /// Get all items as a vector.
    pub fn all_items(&self) -> Vec<(usize, ItemStack)> {
        let mut items: Vec<_> = self.slots.iter().map(|(i, s)| (*i, s.clone())).collect();
        items.sort_by_key(|(i, _)| *i);
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_inventory_count_item() {
        let inv = MockInventory::with_items(vec![
            (0, ItemKind::OakLog, 4),
            (1, ItemKind::OakPlanks, 16),
            (5, ItemKind::OakLog, 2),
        ]);
        assert_eq!(inv.count_item(ItemKind::OakLog), 6);
        assert_eq!(inv.count_item(ItemKind::OakPlanks), 16);
        assert_eq!(inv.count_item(ItemKind::Stick), 0);
    }

    #[test]
    fn mock_inventory_consume() {
        let mut inv =
            MockInventory::with_items(vec![(0, ItemKind::OakLog, 4), (5, ItemKind::OakLog, 2)]);
        assert!(inv.consume(ItemKind::OakLog, 5));
        assert_eq!(inv.count_item(ItemKind::OakLog), 1);
        assert!(!inv.consume(ItemKind::OakLog, 5)); // Not enough
    }

    #[test]
    fn mock_inventory_add() {
        let mut inv = MockInventory::new(36);
        assert!(inv.add(ItemKind::OakLog, 4));
        assert_eq!(inv.count_item(ItemKind::OakLog), 4);
        // Add more to same stack
        assert!(inv.add(ItemKind::OakLog, 2));
        assert_eq!(inv.count_item(ItemKind::OakLog), 6);
    }

    #[test]
    fn mock_inventory_add_new_slot() {
        let mut inv = MockInventory::new(36);
        inv.add(ItemKind::OakLog, 4);
        inv.add(ItemKind::Stick, 2);
        assert_eq!(inv.occupied_slots(), 2);
    }

    #[test]
    fn mock_inventory_has_item() {
        let inv = MockInventory::with_items(vec![(0, ItemKind::OakLog, 4)]);
        assert!(inv.has_item(ItemKind::OakLog, 4));
        assert!(!inv.has_item(ItemKind::OakLog, 5));
        assert!(!inv.has_item(ItemKind::Stick, 1));
    }
}
