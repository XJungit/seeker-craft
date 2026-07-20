package com.craftagent.bridge;

import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.Vec3i;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.Vec3;

public class InventoryHelper {

    public static boolean placeAt(ServerPlayer player, ServerLevel level, int x, int y, int z, String itemName) {
        double dist = player.position().distanceTo(Vec3.atCenterOf(new BlockPos(x, y, z)));
        if (dist > 5.5) {
            return false;
        }
        Inventory inv = player.getInventory();
        int slot = -1;
        String search = itemName.replace("minecraft:", "").toLowerCase();
        for (int i = 0; i < 9; ++i) {
            ItemStack s = inv.getItem(i);
            if (s.isEmpty()) continue;
            String key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase();
            if (!key.contains(search)) continue;
            slot = i;
            break;
        }
        if (slot == -1) {
            for (int i = 9; i < inv.getContainerSize(); ++i) {
                ItemStack s = inv.getItem(i);
                if (s.isEmpty()) continue;
                String key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase();
                if (!key.contains(search)) continue;
                slot = i;
                break;
            }
            if (slot == -1) {
                return false;
            }
            int dstSlot = 0;
            for (int i2 = 0; i2 < 9; ++i2) {
                if (!inv.getItem(i2).isEmpty()) continue;
                dstSlot = i2;
                break;
            }
            ItemStack tmp = inv.getItem(dstSlot);
            inv.setItem(dstSlot, inv.getItem(slot));
            inv.setItem(slot, tmp);
            slot = dstSlot;
        }
        inv.setSelectedSlot(slot);
        player.containerMenu.broadcastChanges();
        BlockPos pos = new BlockPos(x, y, z);
        BlockPos playerPos = player.blockPosition();
        for (Direction dir : new Direction[]{Direction.UP, Direction.NORTH, Direction.SOUTH, Direction.EAST, Direction.WEST, Direction.DOWN}) {
            BlockPos neighbor = pos.relative(dir);
            BlockState ns = level.getBlockState(neighbor);
            boolean isPlayerAnchor = neighbor.equals(playerPos);
            if ((ns.isAir() || !ns.isSolid()) && !isPlayerAnchor) continue;
            BlockHitResult hit = new BlockHitResult(Vec3.atCenterOf(pos), dir.getOpposite(), neighbor, false);
            if (!player.gameMode.useItemOn(player, level, player.getMainHandItem(), InteractionHand.MAIN_HAND, hit).consumesAction()) continue;
            return true;
        }
        ItemStack held = player.getMainHandItem();
        if (!held.isEmpty()) {
            Block b = Block.byItem(held.getItem());
            if (b != null && b != Blocks.AIR) {
                level.setBlock(pos, b.defaultBlockState(), 3);
                held.shrink(1);
                if (held.isEmpty()) {
                    inv.setItem(slot, ItemStack.EMPTY);
                }
                player.containerMenu.broadcastChanges();
                return true;
            }
        }
        return false;
    }

    public static boolean isHostile(String typeName) {
        String[] hostile = new String[]{"zombie", "skeleton", "creeper", "spider", "phantom", "witch", "enderman", "blaze", "ghast", "slime", "magma_cube", "pillager", "vindicator", "evoker", "ravager", "hoglin", "piglin", "zoglin", "warden", "wither", "dragon"};
        for (String h : hostile) {
            if (!typeName.contains(h)) continue;
            return true;
        }
        return false;
    }

    public static void equipBestWeapon(ServerPlayer player) {
        Inventory inv = player.getInventory();
        int best = -1;
        double bestDmg = -1.0;
        for (int i = 0; i < 9; ++i) {
            ItemStack s = inv.getItem(i);
            if (s.isEmpty()) continue;
            String key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString();
            if (!key.contains("sword") && (!key.contains("axe") || key.contains("pickaxe"))) continue;
            double dmg = 4.0;
            if (key.contains("diamond")) {
                dmg = 8.0;
            } else if (key.contains("iron")) {
                dmg = 6.0;
            } else if (key.contains("stone")) {
                dmg = 5.0;
            }
            if (key.contains("sword")) {
                dmg += 1.0;
            }
            if (!(dmg > bestDmg)) continue;
            bestDmg = dmg;
            best = i;
        }
        if (best >= 0) {
            inv.setSelectedSlot(best);
            player.containerMenu.broadcastChanges();
        }
    }

    public static void equipBestTool(ServerPlayer player, String blockId) {
        String toolType;
        String b = blockId.toLowerCase();
        if (b.contains("stone") || b.contains("cobble") || b.contains("ore") || b.contains("obsidian") || b.contains("granite") || b.contains("diorite") || b.contains("andesite") || b.contains("basalt") || b.contains("bricks") || b.contains("netherrack")) {
            toolType = "pickaxe";
        } else if (b.contains("log") || b.contains("planks") || b.contains("wood") || b.contains("leaves") || b.contains("crafting_table") || b.contains("chest") || b.contains("bookshelf")) {
            toolType = "axe";
        } else if (b.contains("dirt") || b.contains("grass") || b.contains("sand") || b.contains("gravel") || b.contains("snow") || b.contains("clay") || b.contains("podzol") || b.contains("mycelium")) {
            toolType = "shovel";
        } else {
            return;
        }
        Inventory inv = player.getInventory();
        int best = -1;
        int bestTier = -1;
        for (int i = 0; i < 9; ++i) {
            int tier = toolTier(inv.getItem(i), toolType);
            if (tier <= bestTier) continue;
            bestTier = tier;
            best = i;
        }
        if (bestTier <= 0) {
            for (int i = 9; i < inv.getContainerSize(); ++i) {
                int tier = toolTier(inv.getItem(i), toolType);
                if (tier <= bestTier) continue;
                int dstSlot = 0;
                for (int j = 0; j < 9; ++j) {
                    if (!inv.getItem(j).isEmpty()) continue;
                    dstSlot = j;
                    break;
                }
                ItemStack tmp = inv.getItem(dstSlot);
                inv.setItem(dstSlot, inv.getItem(i));
                inv.setItem(i, tmp);
                best = dstSlot;
                bestTier = tier;
                break;
            }
        }
        if (best >= 0 && bestTier > 0) {
            inv.setSelectedSlot(best);
            player.containerMenu.broadcastChanges();
        }
    }

    public static int toolTier(ItemStack stack, String toolType) {
        if (stack.isEmpty()) {
            return 0;
        }
        String key = BuiltInRegistries.ITEM.getKey(stack.getItem()).toString().toLowerCase();
        if (!key.contains(toolType)) {
            return 0;
        }
        if (key.contains("diamond")) {
            return 4;
        }
        if (key.contains("iron")) {
            return 3;
        }
        if (key.contains("stone")) {
            return 2;
        }
        if (key.contains("wooden") || key.contains("wood")) {
            return 1;
        }
        return 0;
    }

    public static int discardItem(ServerPlayer player, String itemId, int num) {
        Inventory inv = player.getInventory();
        int discarded = 0;
        String search = itemId.toLowerCase();
        for (int i = 0; i < inv.getContainerSize() && discarded < num; ++i) {
            ItemStack s = inv.getItem(i);
            if (s.isEmpty()) continue;
            String key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase();
            if (!key.contains(search)) continue;
            int take = Math.min(s.getCount(), num - discarded);
            s.shrink(take);
            discarded += take;
        }
        return discarded;
    }

    public static int countItem(Inventory inv, String id) {
        String search = id.toLowerCase();
        int n = 0;
        for (int i = 0; i < inv.getContainerSize(); ++i) {
            ItemStack s = inv.getItem(i);
            if (s.isEmpty()) continue;
            String key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase();
            if (!key.endsWith(":" + search) && (search.contains(":") || !key.contains(search))) continue;
            n += s.getCount();
        }
        return n;
    }

    public static void addItem(Inventory inv, String id, int count) {
        Item exact = null;
        Item fallback = null;
        String search = id.toLowerCase();
        for (Item item : BuiltInRegistries.ITEM) {
            String key = BuiltInRegistries.ITEM.getKey(item).toString().toLowerCase();
            if (key.endsWith(":" + search)) {
                exact = item;
                break;
            }
            if (fallback != null || !key.contains(search) || key.contains("sticky")) continue;
            fallback = item;
        }
        Item target = exact != null ? exact : fallback;
        if (target != null) {
            inv.add(new ItemStack(target, count));
        }
    }

    public static void removeItem(Inventory inv, String id, int count) {
        String search = id.toLowerCase();
        for (int i = 0; i < inv.getContainerSize() && count > 0; ++i) {
            ItemStack s = inv.getItem(i);
            if (s.isEmpty()) continue;
            String key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase();
            if (!key.endsWith(":" + search) && !key.contains(search)) continue;
            int take = Math.min(s.getCount(), count);
            s.shrink(take);
            count -= take;
        }
    }
}
