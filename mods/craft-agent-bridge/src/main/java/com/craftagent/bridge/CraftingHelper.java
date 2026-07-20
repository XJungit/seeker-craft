package com.craftagent.bridge;

import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.item.ItemStack;

public class CraftingHelper {

    public static int craftItem(ServerPlayer player, String targetId, int want) {
        Inventory inv = player.getInventory();
        int crafted = 0;
        String t = targetId.toLowerCase();
        if (t.contains("planks") && InventoryHelper.countItem(inv, "log") > 0) {
            for (String log : new String[]{"oak_log", "birch_log", "spruce_log", "jungle_log", "acacia_log", "dark_oak_log", "mangrove_log", "cherry_log"}) {
                while (crafted < want && InventoryHelper.countItem(inv, log) > 0) {
                    InventoryHelper.removeItem(inv, log, 1);
                    String plank = log.replace("_log", "_planks");
                    InventoryHelper.addItem(inv, plank, 4);
                    crafted += 4;
                }
            }
        }
        if (t.contains("stick")) {
            while (crafted < want && InventoryHelper.countItem(inv, "planks") >= 2) {
                InventoryHelper.removeItem(inv, "planks", 2);
                InventoryHelper.addItem(inv, "stick", 4);
                crafted += 4;
            }
        }
        if (t.contains("crafting_table")) {
            while (crafted < want && InventoryHelper.countItem(inv, "planks") >= 4) {
                InventoryHelper.removeItem(inv, "planks", 4);
                InventoryHelper.addItem(inv, "crafting_table", 1);
                ++crafted;
            }
        }
        if (t.contains("wooden_pickaxe") || t.contains("wooden_axe")) {
            while (crafted < want && InventoryHelper.countItem(inv, "planks") >= 3 && InventoryHelper.countItem(inv, "stick") >= 2) {
                InventoryHelper.removeItem(inv, "planks", 3);
                InventoryHelper.removeItem(inv, "stick", 2);
                InventoryHelper.addItem(inv, t.contains("pickaxe") ? "wooden_pickaxe" : "wooden_axe", 1);
                ++crafted;
            }
        }
        if (t.contains("wooden_sword")) {
            while (crafted < want && InventoryHelper.countItem(inv, "planks") >= 2 && InventoryHelper.countItem(inv, "stick") >= 1) {
                InventoryHelper.removeItem(inv, "planks", 2);
                InventoryHelper.removeItem(inv, "stick", 1);
                InventoryHelper.addItem(inv, "wooden_sword", 1);
                ++crafted;
            }
        }
        if (t.contains("wooden_shovel")) {
            while (crafted < want && InventoryHelper.countItem(inv, "planks") >= 1 && InventoryHelper.countItem(inv, "stick") >= 2) {
                InventoryHelper.removeItem(inv, "planks", 1);
                InventoryHelper.removeItem(inv, "stick", 2);
                InventoryHelper.addItem(inv, "wooden_shovel", 1);
                ++crafted;
            }
        }
        if (t.contains("stone_pickaxe") || t.contains("stone_axe")) {
            while (crafted < want && InventoryHelper.countItem(inv, "cobblestone") >= 3 && InventoryHelper.countItem(inv, "stick") >= 2) {
                InventoryHelper.removeItem(inv, "cobblestone", 3);
                InventoryHelper.removeItem(inv, "stick", 2);
                InventoryHelper.addItem(inv, t.contains("pickaxe") ? "stone_pickaxe" : "stone_axe", 1);
                ++crafted;
            }
        }
        if (t.contains("stone_sword")) {
            while (crafted < want && InventoryHelper.countItem(inv, "cobblestone") >= 2 && InventoryHelper.countItem(inv, "stick") >= 1) {
                InventoryHelper.removeItem(inv, "cobblestone", 2);
                InventoryHelper.removeItem(inv, "stick", 1);
                InventoryHelper.addItem(inv, "stone_sword", 1);
                ++crafted;
            }
        }
        if (t.contains("torch")) {
            while (crafted < want && InventoryHelper.countItem(inv, "stick") >= 1 && InventoryHelper.countItem(inv, "coal") >= 1) {
                InventoryHelper.removeItem(inv, "stick", 1);
                InventoryHelper.removeItem(inv, "coal", 1);
                InventoryHelper.addItem(inv, "torch", 4);
                crafted += 4;
            }
        }
        if (t.contains("furnace")) {
            while (crafted < want && InventoryHelper.countItem(inv, "cobblestone") >= 8) {
                InventoryHelper.removeItem(inv, "cobblestone", 8);
                InventoryHelper.addItem(inv, "furnace", 1);
                ++crafted;
            }
        }
        if (t.contains("chest")) {
            while (crafted < want && InventoryHelper.countItem(inv, "planks") >= 8) {
                InventoryHelper.removeItem(inv, "planks", 8);
                InventoryHelper.addItem(inv, "chest", 1);
                ++crafted;
            }
        }
        if (t.contains("iron_pickaxe") || t.contains("iron_axe")) {
            while (crafted < want && InventoryHelper.countItem(inv, "iron_ingot") >= 3 && InventoryHelper.countItem(inv, "stick") >= 2) {
                InventoryHelper.removeItem(inv, "iron_ingot", 3);
                InventoryHelper.removeItem(inv, "stick", 2);
                InventoryHelper.addItem(inv, t.contains("pickaxe") ? "iron_pickaxe" : "iron_axe", 1);
                ++crafted;
            }
        }
        if (t.contains("iron_sword")) {
            while (crafted < want && InventoryHelper.countItem(inv, "iron_ingot") >= 2 && InventoryHelper.countItem(inv, "stick") >= 1) {
                InventoryHelper.removeItem(inv, "iron_ingot", 2);
                InventoryHelper.removeItem(inv, "stick", 1);
                InventoryHelper.addItem(inv, "iron_sword", 1);
                ++crafted;
            }
        }
        if (t.contains("diamond_pickaxe") || t.contains("diamond_axe")) {
            while (crafted < want && InventoryHelper.countItem(inv, "diamond") >= 3 && InventoryHelper.countItem(inv, "stick") >= 2) {
                InventoryHelper.removeItem(inv, "diamond", 3);
                InventoryHelper.removeItem(inv, "stick", 2);
                InventoryHelper.addItem(inv, t.contains("pickaxe") ? "diamond_pickaxe" : "diamond_axe", 1);
                ++crafted;
            }
        }
        if (t.contains("diamond_sword")) {
            while (crafted < want && InventoryHelper.countItem(inv, "diamond") >= 2 && InventoryHelper.countItem(inv, "stick") >= 1) {
                InventoryHelper.removeItem(inv, "diamond", 2);
                InventoryHelper.removeItem(inv, "stick", 1);
                InventoryHelper.addItem(inv, "diamond_sword", 1);
                ++crafted;
            }
        }
        if (t.contains("iron_helmet")) {
            while (crafted < want && InventoryHelper.countItem(inv, "iron_ingot") >= 5) {
                InventoryHelper.removeItem(inv, "iron_ingot", 5);
                InventoryHelper.addItem(inv, "iron_helmet", 1);
                ++crafted;
            }
        }
        if (t.contains("iron_chestplate")) {
            while (crafted < want && InventoryHelper.countItem(inv, "iron_ingot") >= 8) {
                InventoryHelper.removeItem(inv, "iron_ingot", 8);
                InventoryHelper.addItem(inv, "iron_chestplate", 1);
                ++crafted;
            }
        }
        if (t.contains("iron_leggings")) {
            while (crafted < want && InventoryHelper.countItem(inv, "iron_ingot") >= 7) {
                InventoryHelper.removeItem(inv, "iron_ingot", 7);
                InventoryHelper.addItem(inv, "iron_leggings", 1);
                ++crafted;
            }
        }
        if (t.contains("iron_boots")) {
            while (crafted < want && InventoryHelper.countItem(inv, "iron_ingot") >= 4) {
                InventoryHelper.removeItem(inv, "iron_ingot", 4);
                InventoryHelper.addItem(inv, "iron_boots", 1);
                ++crafted;
            }
        }
        if (t.contains("diamond_helmet")) {
            while (crafted < want && InventoryHelper.countItem(inv, "diamond") >= 5) {
                InventoryHelper.removeItem(inv, "diamond", 5);
                InventoryHelper.addItem(inv, "diamond_helmet", 1);
                ++crafted;
            }
        }
        if (t.contains("diamond_chestplate")) {
            while (crafted < want && InventoryHelper.countItem(inv, "diamond") >= 8) {
                InventoryHelper.removeItem(inv, "diamond", 8);
                InventoryHelper.addItem(inv, "diamond_chestplate", 1);
                ++crafted;
            }
        }
        if (t.contains("diamond_leggings")) {
            while (crafted < want && InventoryHelper.countItem(inv, "diamond") >= 7) {
                InventoryHelper.removeItem(inv, "diamond", 7);
                InventoryHelper.addItem(inv, "diamond_leggings", 1);
                ++crafted;
            }
        }
        if (t.contains("diamond_boots")) {
            while (crafted < want && InventoryHelper.countItem(inv, "diamond") >= 4) {
                InventoryHelper.removeItem(inv, "diamond", 4);
                InventoryHelper.addItem(inv, "diamond_boots", 1);
                ++crafted;
            }
        }
        if (t.contains("shield")) {
            while (crafted < want && InventoryHelper.countItem(inv, "planks") >= 6 && InventoryHelper.countItem(inv, "iron_ingot") >= 1) {
                InventoryHelper.removeItem(inv, "planks", 6);
                InventoryHelper.removeItem(inv, "iron_ingot", 1);
                InventoryHelper.addItem(inv, "shield", 1);
                ++crafted;
            }
        }
        return crafted;
    }

    public static int smeltItem(ServerPlayer player, String itemId, int num) {
        Inventory inv = player.getInventory();
        int smelted = 0;
        String input = itemId.toLowerCase();
        String output = null;
        if (input.contains("raw_iron")) {
            output = "iron_ingot";
        } else if (input.contains("raw_copper")) {
            output = "copper_ingot";
        } else if (input.contains("raw_gold")) {
            output = "gold_ingot";
        } else if (input.contains("oak_log")) {
            output = "charcoal";
        } else if (input.contains("sand")) {
            output = "glass";
        } else if (input.contains("cobblestone")) {
            output = "stone";
        }
        if (output == null) {
            return 0;
        }
        while (smelted < num && InventoryHelper.countItem(inv, input) >= 1 && InventoryHelper.countItem(inv, "coal") >= 1) {
            InventoryHelper.removeItem(inv, input, 1);
            InventoryHelper.removeItem(inv, "coal", 1);
            InventoryHelper.addItem(inv, output, 1);
            ++smelted;
        }
        return smelted;
    }
}
