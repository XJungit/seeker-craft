package com.craftagent.bridge;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import java.util.Optional;
import net.minecraft.core.Registry;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.world.Container;
import net.minecraft.world.SimpleContainer;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.inventory.AbstractContainerMenu;
import net.minecraft.world.inventory.ContainerInput;
import net.minecraft.world.inventory.CraftingContainer;
import net.minecraft.world.inventory.Slot;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.enchantment.Enchantment;
import net.minecraft.world.item.enchantment.EnchantmentHelper;
import net.minecraft.core.Holder;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.level.Level;

public class ContainerController {

    public static JsonObject actEnchant(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String itemSearch = req.has("item") ? req.get("item").getAsString() : "";
        int levels = req.has("levels") ? req.get("levels").getAsInt() : 30;
        levels = Math.max(1, Math.min(30, levels));
        if (itemSearch.isEmpty() && (itemSearch = BuiltInRegistries.ITEM.getKey(player.getMainHandItem().getItem()).getPath()).equals("air")) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "enchant: no item specified and main hand is empty");
            return o;
        }
        String search = itemSearch.replace("minecraft:", "").toLowerCase();
        Inventory inv = player.getInventory();
        int slot = -1;
        for (int i = 0; i < inv.getContainerSize(); ++i) {
            String key2;
            ItemStack s = inv.getItem(i);
            if (s.isEmpty() || !(key2 = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase()).contains(search)) continue;
            slot = i;
            break;
        }
        if (slot < 0) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "enchant: " + itemSearch + " not found");
            return o;
        }
        if (player.experienceLevel < levels) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "enchant: need " + levels + " XP levels, have " + player.experienceLevel);
            return o;
        }
        if (slot < 9) {
            inv.setSelectedSlot(slot);
        }
        ItemStack stack = inv.getItem(slot);
        Registry<Enchantment> enchReg = player.level().registryAccess().lookup(Registries.ENCHANTMENT).orElse(null);
        if (enchReg == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "enchant: no enchantment registry");
            return o;
        }
        var possible = enchReg.listElements()
            .map(h -> (Holder<Enchantment>) h)
            .filter(e -> e.value().canEnchant(stack));
        ItemStack enchanted = EnchantmentHelper.enchantItem(player.getRandom(), stack.copy(), levels, possible);
        inv.setItem(slot, enchanted);
        player.experienceLevel -= levels;
        player.containerMenu.broadcastChanges();
        StringBuilder enchNames = new StringBuilder();
        for (Holder<Enchantment> holder : enchanted.getEnchantments().keySet()) {
            holder.unwrapKey().ifPresentOrElse(key -> enchNames.append(" ").append(key.identifier().getPath()), () -> enchNames.append(" ?"));
        }
        o.addProperty("detail", "enchant " + itemSearch + " lvl=" + levels + ":" + String.valueOf(enchNames));
        return o;
    }

    public static JsonObject actSelectSlot(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        int slot = req.get("slot").getAsInt();
        player.getInventory().setSelectedSlot(slot);
        player.containerMenu.broadcastChanges();
        int actual = player.getInventory().getSelectedSlot();
        ItemStack held = player.getMainHandItem();
        String heldId = held.isEmpty() ? "minecraft:air" : BuiltInRegistries.ITEM.getKey(held.getItem()).toString();
        o.addProperty("slot", (Number)actual);
        o.addProperty("held_item", heldId);
        o.addProperty("detail", "select_slot " + slot + " (actual=" + actual + ", held=" + heldId + ")");
        return o;
    }

    public static JsonObject actMoveToHotbar(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String item = req.has("item") ? req.get("item").getAsString() : "";
        String search = item.replace("minecraft:", "").toLowerCase();
        Inventory inv = player.getInventory();
        int srcSlot = -1;
        for (int i = 9; i < inv.getContainerSize(); ++i) {
            String key3;
            ItemStack s = inv.getItem(i);
            if (s.isEmpty() || !(key3 = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase()).contains(search)) continue;
            srcSlot = i;
            break;
        }
        if (srcSlot == -1) {
            o.addProperty("moved", Boolean.valueOf(false));
            o.addProperty("detail", "move_to_hotbar: " + item + " not found in main inventory");
            return o;
        }
        int dstSlot = -1;
        for (int i = 0; i < 9; ++i) {
            if (!inv.getItem(i).isEmpty()) continue;
            dstSlot = i;
            break;
        }
        if (dstSlot < 0) {
            dstSlot = 0;
        }
        ItemStack tmp = inv.getItem(dstSlot);
        inv.setItem(dstSlot, inv.getItem(srcSlot));
        inv.setItem(srcSlot, tmp);
        player.containerMenu.broadcastChanges();
        o.addProperty("moved", Boolean.valueOf(true));
        o.addProperty("hotbar_slot", (Number)dstSlot);
        o.addProperty("detail", "move_to_hotbar " + item + " -> slot " + dstSlot);
        return o;
    }

    public static JsonObject actMoveSlot(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        int fromSlot = req.has("from_slot") ? req.get("from_slot").getAsInt() : -1;
        int toSlot = req.has("to_slot") ? req.get("to_slot").getAsInt() : -1;
        int wantCount = req.has("count") ? req.get("count").getAsInt() : -1;
        Inventory inv = player.getInventory();
        int size = inv.getContainerSize();
        if (fromSlot < 0 || fromSlot >= size || toSlot < 0 || toSlot >= size) {
            o.addProperty("moved", Boolean.valueOf(false));
            o.addProperty("detail", "move_slot: invalid slot index (from=" + fromSlot + ", to=" + toSlot + ", size=" + size + ")");
            return o;
        }
        ItemStack fromStack = inv.getItem(fromSlot);
        if (fromStack.isEmpty()) {
            o.addProperty("moved", Boolean.valueOf(false));
            o.addProperty("detail", "move_slot: source slot " + fromSlot + " is empty");
            return o;
        }
        ItemStack toStack = inv.getItem(toSlot);
        int fromCount = fromStack.getCount();
        int moveCount = wantCount <= 0 ? fromCount : Math.min(wantCount, fromCount);
        String fromId = BuiltInRegistries.ITEM.getKey(fromStack.getItem()).toString();
        String toId = toStack.isEmpty() ? "minecraft:air" : BuiltInRegistries.ITEM.getKey(toStack.getItem()).toString();
        if (toStack.isEmpty()) {
            inv.setItem(toSlot, fromStack.split(moveCount));
            if (fromStack.isEmpty()) {
                inv.setItem(fromSlot, ItemStack.EMPTY);
            }
        } else if (ItemStack.isSameItemSameComponents((ItemStack)fromStack, (ItemStack)toStack)) {
            int max = toStack.getMaxStackSize();
            int canAdd = Math.min(max - toStack.getCount(), moveCount);
            if (canAdd <= 0) {
                o.addProperty("moved", Boolean.valueOf(false));
                o.addProperty("detail", "move_slot: target slot " + toSlot + " already full");
                return o;
            }
            toStack.grow(canAdd);
            fromStack.shrink(canAdd);
            if (fromStack.isEmpty()) {
                inv.setItem(fromSlot, ItemStack.EMPTY);
            }
            moveCount = canAdd;
        } else {
            if (moveCount < fromCount) {
                o.addProperty("moved", Boolean.valueOf(false));
                o.addProperty("detail", "move_slot: cannot split " + moveCount + " of " + fromId + " into slot " + toSlot + " holding " + toId + " (different items, swap only)");
                return o;
            }
            inv.setItem(toSlot, fromStack.copy());
            inv.setItem(fromSlot, toStack.copy());
        }
        player.containerMenu.broadcastChanges();
        o.addProperty("moved", Boolean.valueOf(true));
        o.addProperty("from_slot", (Number)fromSlot);
        o.addProperty("to_slot", (Number)toSlot);
        o.addProperty("count", (Number)moveCount);
        o.addProperty("from_item", fromId);
        o.addProperty("to_item", toId);
        o.addProperty("detail", "move_slot " + fromId + " x" + moveCount + " from slot " + fromSlot + " to slot " + toSlot);
        return o;
    }

    public static JsonObject actCraft(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String item = req.get("item").getAsString();
        int want = req.has("count") ? req.get("count").getAsInt() : 1;
        int crafted = CraftingHelper.craftItem(player, item, want);
        player.containerMenu.broadcastChanges();
        o.addProperty("crafted", (Number)crafted);
        o.addProperty("detail", "craft " + item + " x" + crafted);
        return o;
    }

    public static JsonObject actDiscard(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String item = req.get("item").getAsString();
        int num = req.has("num") ? req.get("num").getAsInt() : 1;
        int discarded = InventoryHelper.discardItem(player, item, num);
        player.containerMenu.broadcastChanges();
        o.addProperty("detail", "discarded " + discarded + " x " + item);
        return o;
    }

    public static JsonObject actSmelt(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String item = req.get("item").getAsString();
        int num = req.has("num") ? req.get("num").getAsInt() : 1;
        int smelted = CraftingHelper.smeltItem(player, item, num);
        player.containerMenu.broadcastChanges();
        o.addProperty("detail", "smelted " + smelted + " x " + item);
        return o;
    }

    public static JsonObject actInspectGui(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        ItemStack carried;
        AbstractContainerMenu menu = player.containerMenu;
        boolean hasGui = menu != player.inventoryMenu;
        o.addProperty("has_gui", Boolean.valueOf(hasGui));
        if (!hasGui) {
            o.addProperty("detail", "inspect_gui: no container open");
            return o;
        }
        JsonArray slots = new JsonArray();
        JsonArray craftingGrid = new JsonArray();
        boolean hasCrafting = false;
        for (int i = 0; i < menu.slots.size(); ++i) {
            Slot slot = menu.getSlot(i);
            ItemStack stack = slot.getItem();
            JsonObject so = new JsonObject();
            so.addProperty("slot_index", (Number)i);
            so.addProperty("id", stack.isEmpty() ? "minecraft:air" : BuiltInRegistries.ITEM.getKey(stack.getItem()).toString());
            so.addProperty("count", (Number)stack.getCount());
            boolean isPlayerInv = slot.container == player.getInventory();
            so.addProperty("side", isPlayerInv ? "player" : "container");
            if (slot.container instanceof CraftingContainer) {
                hasCrafting = true;
                JsonObject co = new JsonObject();
                co.addProperty("slot_index", (Number)i);
                co.addProperty("id", stack.isEmpty() ? "minecraft:air" : BuiltInRegistries.ITEM.getKey(stack.getItem()).toString());
                co.addProperty("count", (Number)stack.getCount());
                craftingGrid.add((JsonElement)co);
            }
            slots.add((JsonElement)so);
        }
        o.add("slots", (JsonElement)slots);
        if (hasCrafting) {
            o.add("crafting_grid", (JsonElement)craftingGrid);
        }
        o.addProperty("carried_item", (carried = menu.getCarried()).isEmpty() ? "minecraft:air" : BuiltInRegistries.ITEM.getKey(carried.getItem()).toString());
        o.addProperty("carried_count", (Number)carried.getCount());
        o.addProperty("detail", "inspect_gui: " + menu.slots.size() + " slots");
        return o;
    }

    public static JsonObject actCloseGui(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        if (player.containerMenu != player.inventoryMenu) {
            player.closeContainer();
            o.addProperty("detail", "close_gui: container closed");
            return o;
        }
        o.addProperty("detail", "close_gui: no container open");
        return o;
    }

    public static JsonObject actTransfer(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        if (player.containerMenu == player.inventoryMenu) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "transfer: no container open");
            return o;
        }
        AbstractContainerMenu menu = player.containerMenu;
        if (!req.has("moves") || !req.get("moves").isJsonArray()) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "transfer: moves array required");
            return o;
        }
        JsonArray moves = req.get("moves").getAsJsonArray();
        int movedTotal = 0;
        for (int mi = 0; mi < moves.size(); ++mi) {
            JsonObject mv = moves.get(mi).getAsJsonObject();
            int fromSlot = mv.get("from").getAsInt();
            Integer toSlot = mv.has("to") && !mv.get("to").isJsonNull() ? Integer.valueOf(mv.get("to").getAsInt()) : null;
            if (fromSlot < 0 || fromSlot >= menu.slots.size() || toSlot != null && (toSlot < 0 || toSlot >= menu.slots.size())) continue;
            if (toSlot == null) {
                menu.clicked(fromSlot, 0, ContainerInput.QUICK_MOVE, (Player)player);
                ++movedTotal;
                continue;
            }
            menu.clicked(fromSlot, 0, ContainerInput.PICKUP, (Player)player);
            menu.clicked(toSlot.intValue(), 0, ContainerInput.PICKUP, (Player)player);
            ++movedTotal;
        }
        player.containerMenu.broadcastChanges();
        o.addProperty("moved_count", (Number)movedTotal);
        o.addProperty("detail", "transfer: " + movedTotal + " moves executed");
        return o;
    }

    public static JsonObject actEquipItem(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String itemName = req.has("item") ? req.get("item").getAsString() : "";
        String slotName = req.has("slot") ? req.get("slot").getAsString() : "auto";
        String search = itemName.replace("minecraft:", "").toLowerCase();
        Inventory inv = player.getInventory();
        ItemStack targetStack = ItemStack.EMPTY;
        int foundSlot = -1;
        for (int i = 0; i < inv.getContainerSize(); ++i) {
            String key4;
            ItemStack s = inv.getItem(i);
            if (s.isEmpty() || !(key4 = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase()).contains(search)) continue;
            targetStack = s.copy();
            foundSlot = i;
            break;
        }
        if (targetStack.isEmpty()) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "equip_item: " + itemName + " not found");
            return o;
        }
        EquipmentSlot equipSlot = null;
        if (!slotName.equals("auto")) {
            switch (slotName.toLowerCase()) {
                case "mainhand": 
                case "main_hand": {
                    equipSlot = EquipmentSlot.MAINHAND;
                    break;
                }
                case "offhand": 
                case "off_hand": {
                    equipSlot = EquipmentSlot.OFFHAND;
                    break;
                }
                case "head": 
                case "helmet": {
                    equipSlot = EquipmentSlot.HEAD;
                    break;
                }
                case "chest": 
                case "chestplate": {
                    equipSlot = EquipmentSlot.CHEST;
                    break;
                }
                case "legs": 
                case "leggings": {
                    equipSlot = EquipmentSlot.LEGS;
                    break;
                }
                case "feet": 
                case "boots": {
                    equipSlot = EquipmentSlot.FEET;
                    break;
                }
            }
        }
        if (equipSlot == null) {
            String key5 = BuiltInRegistries.ITEM.getKey(targetStack.getItem()).toString().toLowerCase();
            equipSlot = key5.contains("helmet") || key5.contains("cap") ? EquipmentSlot.HEAD : (key5.contains("chestplate") || key5.contains("jacket") ? EquipmentSlot.CHEST : (key5.contains("leggings") || key5.contains("pants") ? EquipmentSlot.LEGS : (key5.contains("boots") ? EquipmentSlot.FEET : (key5.contains("shield") ? EquipmentSlot.OFFHAND : EquipmentSlot.MAINHAND))));
        }
        boolean equipped = false;
        if (equipSlot == EquipmentSlot.MAINHAND) {
            if (foundSlot < 9) {
                inv.setSelectedSlot(foundSlot);
            } else {
                int dst = 0;
                for (int i = 0; i < 9; ++i) {
                    if (!inv.getItem(i).isEmpty()) continue;
                    dst = i;
                    break;
                }
                ItemStack tmp = inv.getItem(dst);
                inv.setItem(dst, inv.getItem(foundSlot));
                inv.setItem(foundSlot, tmp);
                inv.setSelectedSlot(dst);
            }
            equipped = true;
        } else {
            InteractionResult result;
            if (foundSlot < 9) {
                inv.setSelectedSlot(foundSlot);
            }
            if ((result = player.gameMode.useItem(player, (Level)level, player.getMainHandItem(), InteractionHand.MAIN_HAND)).consumesAction()) {
                equipped = true;
            } else {
                ItemStack current = player.getItemBySlot(equipSlot);
                player.setItemSlot(equipSlot, targetStack.copy());
                if (!current.isEmpty() && !inv.add(current)) {
                    player.drop(current, false);
                }
                inv.getItem(foundSlot).shrink(targetStack.getCount());
                equipped = true;
            }
        }
        player.containerMenu.broadcastChanges();
        o.addProperty("equipped", Boolean.valueOf(equipped));
        o.addProperty("slot", equipSlot.getName());
        o.addProperty("detail", "equip_item " + itemName + " -> " + equipSlot.getName() + " (equipped=" + equipped + ")");
        return o;
    }

    public static JsonObject actDropItems(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String itemName = req.has("item") ? req.get("item").getAsString() : "";
        int num = req.has("num") ? req.get("num").getAsInt() : 1;
        String search = itemName.replace("minecraft:", "").toLowerCase();
        Inventory inv = player.getInventory();
        int dropped = 0;
        for (int i = 0; i < inv.getContainerSize() && dropped < num; ++i) {
            String key6;
            ItemStack s = inv.getItem(i);
            if (s.isEmpty() || !(key6 = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase()).contains(search)) continue;
            int take = Math.min(s.getCount(), num - dropped);
            ItemStack toDrop = s.copy();
            toDrop.setCount(take);
            s.shrink(take);
            player.drop(toDrop, false);
            dropped += take;
        }
        player.containerMenu.broadcastChanges();
        o.addProperty("dropped", (Number)dropped);
        o.addProperty("detail", "drop_items " + itemName + " x" + dropped + " (ItemEntity spawned)");
        return o;
    }

    public static JsonObject actGetCraftingPlan(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String targetItem = req.has("item") ? req.get("item").getAsString() : "";
        int quantity = req.has("quantity") ? req.get("quantity").getAsInt() : 1;
        Inventory inv = player.getInventory();
        int have = 0;
        for (int i = 0; i < inv.getContainerSize(); ++i) {
            String key7;
            ItemStack s = inv.getItem(i);
            if (s.isEmpty() || !(key7 = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase()).contains(targetItem.toLowerCase())) continue;
            have += s.getCount();
        }
        if (have >= quantity) {
            o.addProperty("detail", "get_crafting_plan: already have " + have + " " + targetItem + " (need " + quantity + ")");
        } else {
            o.addProperty("detail", "get_crafting_plan: have " + have + " " + targetItem + ", need " + quantity + " more. Use craft tool to make them.");
        }
        o.addProperty("have", (Number)have);
        o.addProperty("need", (Number)quantity);
        o.addProperty("missing", (Number)Math.max(0, quantity - have));
        return o;
    }
}
