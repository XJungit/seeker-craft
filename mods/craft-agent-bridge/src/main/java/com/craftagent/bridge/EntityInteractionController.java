package com.craftagent.bridge;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import java.util.Set;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.Vec3i;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.vehicle.boat.Boat;
import net.minecraft.world.entity.animal.equine.AbstractHorse;
import net.minecraft.world.entity.vehicle.minecart.AbstractMinecart;
import net.minecraft.world.entity.animal.pig.Pig;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.trading.Merchant;
import net.minecraft.world.item.trading.MerchantOffer;
import net.minecraft.world.item.trading.MerchantOffers;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.BedBlock;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.BedPart;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.Vec3;

public class EntityInteractionController {

    static JsonObject actVillagerTrades(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        double radius = req.has("radius") ? req.get("radius").getAsDouble() : 8.0;
        Merchant nearest = null;
        double minDist = Double.MAX_VALUE;
        for (Entity e6 : level.getEntities((Entity)player, AABB.ofSize((Vec3)player.position(), (double)(radius * 2.0), (double)(radius * 2.0), (double)(radius * 2.0)))) {
            double d;
            if (!(e6 instanceof Merchant) || !((d = (double)player.distanceTo(e6)) < minDist)) continue;
            minDist = d;
            nearest = (Merchant)e6;
        }
        if (nearest == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "villager_trades: no villager within " + radius + "m");
            return o;
        }
        Entity villagerEntity = (Entity)nearest;
        o.addProperty("villager_id", (Number)villagerEntity.getId());
        o.addProperty("villager_type", BuiltInRegistries.ENTITY_TYPE.getKey(villagerEntity.getType()).toString());
        o.addProperty("villager_profession", "merchant");
        JsonArray trades = new JsonArray();
        MerchantOffers merchantOffers = nearest.getOffers();
        for (int i = 0; i < merchantOffers.size(); ++i) {
            MerchantOffer offer = (MerchantOffer)merchantOffers.get(i);
            JsonObject to = new JsonObject();
            to.addProperty("index", (Number)(i + 1));
            to.addProperty("input_a", offer.getCostA().isEmpty() ? "air" : BuiltInRegistries.ITEM.getKey(offer.getCostA().getItem()).toString());
            to.addProperty("input_a_count", (Number)offer.getCostA().getCount());
            if (!offer.getCostB().isEmpty()) {
                to.addProperty("input_b", BuiltInRegistries.ITEM.getKey(offer.getCostB().getItem()).toString());
                to.addProperty("input_b_count", (Number)offer.getCostB().getCount());
            }
            to.addProperty("output", BuiltInRegistries.ITEM.getKey(offer.getResult().getItem()).toString());
            to.addProperty("output_count", (Number)offer.getResult().getCount());
            trades.add((JsonElement)to);
        }
        o.add("trades", (JsonElement)trades);
        o.addProperty("detail", "villager_trades: " + trades.size() + " trades from " + String.valueOf(BuiltInRegistries.ENTITY_TYPE.getKey(villagerEntity.getType())));
        return o;
    }

    static JsonObject actTradeWithVillager(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        MerchantOffer offer;
        double radius = req.has("radius") ? req.get("radius").getAsDouble() : 8.0;
        int tradeIndex = req.has("index") ? req.get("index").getAsInt() : 1;
        int count = req.has("count") ? req.get("count").getAsInt() : 1;
        Merchant nearest = null;
        double minDist = Double.MAX_VALUE;
        for (Entity e7 : level.getEntities((Entity)player, AABB.ofSize((Vec3)player.position(), (double)(radius * 2.0), (double)(radius * 2.0), (double)(radius * 2.0)))) {
            double d;
            if (!(e7 instanceof Merchant) || !((d = (double)player.distanceTo(e7)) < minDist)) continue;
            minDist = d;
            nearest = (Merchant)e7;
        }
        if (nearest == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "trade_with_villager: no villager within " + radius + "m");
            return o;
        }
        MerchantOffers offers = nearest.getOffers();
        if (tradeIndex < 1 || tradeIndex > offers.size()) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "trade_with_villager: invalid trade index " + tradeIndex + " (1-" + offers.size() + ")");
            return o;
        }
        int traded = 0;
        for (int i = 0; i < count && !(offer = (MerchantOffer)offers.get(tradeIndex - 1)).isOutOfStock(); ++i) {
            int take;
            ItemStack s;
            int j;
            ItemStack costA = offer.getCostA();
            ItemStack costB = offer.getCostB();
            int haveA = 0;
            int haveB = 0;
            Inventory inv = player.getInventory();
            for (int j2 = 0; j2 < inv.getContainerSize(); ++j2) {
                ItemStack s2 = inv.getItem(j2);
                if (s2.isEmpty()) continue;
                if (s2.getItem() == costA.getItem()) {
                    haveA += s2.getCount();
                }
                if (costB.isEmpty() || s2.getItem() != costB.getItem()) continue;
                haveB += s2.getCount();
            }
            if (haveA < costA.getCount() || !costB.isEmpty() && haveB < costB.getCount()) break;
            int needA = costA.getCount();
            int needB = costB.isEmpty() ? 0 : costB.getCount();
            for (j = 0; j < inv.getContainerSize() && needA > 0; ++j) {
                s = inv.getItem(j);
                if (s.isEmpty() || s.getItem() != costA.getItem()) continue;
                take = Math.min(s.getCount(), needA);
                s.shrink(take);
                needA -= take;
            }
            for (j = 0; j < inv.getContainerSize() && needB > 0; ++j) {
                s = inv.getItem(j);
                if (s.isEmpty() || s.getItem() != costB.getItem()) continue;
                take = Math.min(s.getCount(), needB);
                s.shrink(take);
                needB -= take;
            }
            ItemStack result = offer.getResult().copy();
            if (!inv.add(result)) {
                player.drop(result, false);
            }
            offer.increaseUses();
            ++traded;
        }
        player.containerMenu.broadcastChanges();
        o.addProperty("traded", (Number)traded);
        o.addProperty("detail", "trade_with_villager: " + traded + " trades of index " + tradeIndex);
        return o;
    }

    static JsonObject actFish(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        boolean hasRod = false;
        for (int s = 0; s < player.getInventory().getContainerSize(); ++s) {
            ItemStack it = player.getInventory().getItem(s);
            if (it.isEmpty() || !BuiltInRegistries.ITEM.getKey(it.getItem()).toString().toLowerCase().contains("fishing_rod")) continue;
            player.getInventory().setSelectedSlot(s < 9 ? s : 0);
            if (s >= 9) {
                ItemStack tmp = player.getInventory().getItem(0);
                player.getInventory().setItem(0, player.getInventory().getItem(s));
                player.getInventory().setItem(s, tmp);
            }
            hasRod = true;
            break;
        }
        if (!hasRod) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "fish: no fishing_rod in inventory");
            return o;
        }
        int ticks = req.has("ticks") ? req.get("ticks").getAsInt() : 100;
        boolean castBefore = false;
        try {
            player.startUsingItem(InteractionHand.MAIN_HAND);
            castBefore = player.fishing != null;
        }
        catch (Exception tmp) {
        }
        try {
            Thread.sleep((long)ticks * 50L);
        }
        catch (InterruptedException e9) {
            Thread.currentThread().interrupt();
        }
        if (player.fishing != null) {
            player.stopUsingItem();
            player.resetSentInfo();
        }
        o.addProperty("cast", Boolean.valueOf(castBefore));
        o.addProperty("detail", "fish ticks=" + ticks + " cast=" + castBefore);
        return o;
    }

    static JsonObject actRide(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String action;
        String string = action = req.has("action") ? req.get("action").getAsString() : "mount";
        if ("dismount".equals(action)) {
            player.stopRiding();
            o.addProperty("detail", "ride dismount");
            return o;
        }
        if ("steer".equals(action)) {
            Entity dx;
            double forward;
            double left = req.has("left") ? req.get("left").getAsDouble() : 0.0;
            double d = forward = req.has("forward") ? req.get("forward").getAsDouble() : 1.0;
            if (player.getVehicle() != null && (dx = player.getVehicle()) instanceof Boat) {
                Boat boat = (Boat)dx;
                boolean fwd = forward > 0.1;
                boolean back = forward < -0.1;
                boolean lft = left < -0.1;
                boolean rgt = left > 0.1;
                boat.setInput(fwd, lft, back, rgt);
            }
            o.addProperty("detail", "ride steer left=" + left + " forward=" + forward);
            return o;
        }
        double radius = req.has("radius") ? req.get("radius").getAsDouble() : 8.0;
        Entity target = null;
        double minD = Double.MAX_VALUE;
        for (Entity e10 : level.getEntities((Entity)player, AABB.ofSize((Vec3)player.position(), (double)(radius * 2.0), (double)(radius * 2.0), (double)(radius * 2.0)))) {
            double d;
            if (e10 == player || !(e10 instanceof AbstractHorse) && !(e10 instanceof Boat) && !(e10 instanceof AbstractMinecart) && !(e10 instanceof Pig) || !((d = (double)player.distanceTo(e10)) < minD)) continue;
            minD = d;
            target = e10;
        }
        if (target == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "ride: no rideable entity within " + radius + "m");
            return o;
        }
        player.startRiding(target);
        o.addProperty("mounted", BuiltInRegistries.ENTITY_TYPE.getKey(target.getType()).toString());
        o.addProperty("detail", "ride mount " + BuiltInRegistries.ENTITY_TYPE.getKey(target.getType()).toString());
        return o;
    }

    static JsonObject actSleep(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        double radius = req.has("radius") ? req.get("radius").getAsDouble() : 8.0;
        BlockPos bed = null;
        for (int r = 1; r <= (int)radius; ++r) {
            for (int dx = -r; dx <= r; ++dx) {
                for (int dz = -r; dz <= r; ++dz) {
                    for (int dy = -1; dy <= 1; ++dy) {
                        BlockPos bp = BlockPos.containing((double)(player.getX() + (double)dx), (double)(player.getY() + (double)dy), (double)(player.getZ() + (double)dz));
                        BlockState bs = level.getBlockState(bp);
                        String id = BuiltInRegistries.BLOCK.getKey(bs.getBlock()).toString().toLowerCase();
                        if (!id.contains("bed") || id.contains("bedrock") || bs.getValue((Property)BedBlock.PART) != BedPart.FOOT) continue;
                        bed = bp;
                    }
                }
            }
            if (bed != null) break;
        }
        if (bed == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "sleep: no bed nearby");
            return o;
        }
        player.teleportTo(level, (double)bed.getX() + 0.5, (double)bed.getY() + 1.0, (double)bed.getZ() + 1.0, Set.of(), 0.0f, 0.0f, true);
        try {
            Thread.sleep(100L);
        }
        catch (InterruptedException e11) {
            Thread.currentThread().interrupt();
        }
        player.startSleeping(bed);
        o.addProperty("slept", Boolean.valueOf(player.isSleeping()));
        o.addProperty("detail", "sleep at " + bed.getX() + "," + bed.getY() + "," + bed.getZ() + " sleeping=" + player.isSleeping());
        return o;
    }

    static JsonObject actWake(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        player.stopSleeping();
        o.addProperty("detail", "wake (was sleeping=false)");
        return o;
    }

    static JsonObject actBuildPortal(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        int[][] frame;
        String item = req.has("item") ? req.get("item").getAsString() : "obsidian";
        String search = item.replace("minecraft:", "").toLowerCase();
        Inventory inv = player.getInventory();
        int obsidianCount = 0;
        for (int i = 0; i < inv.getContainerSize(); ++i) {
            String key8;
            ItemStack s = inv.getItem(i);
            if (s.isEmpty() || !(key8 = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase()).contains(search)) continue;
            obsidianCount += s.getCount();
        }
        if (obsidianCount < 10) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "build_portal: need 10 " + search + ", have " + obsidianCount);
            return o;
        }
        boolean hasFlint = false;
        for (int i = 0; i < inv.getContainerSize(); ++i) {
            String key9;
            ItemStack s = inv.getItem(i);
            if (s.isEmpty() || !(key9 = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase()).contains("flint_and_steel") && !key9.contains("fire_charge")) continue;
            hasFlint = true;
            if (!key9.contains("flint_and_steel") || i >= 9) break;
            inv.setSelectedSlot(i);
            break;
        }
        if (!hasFlint) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "build_portal: need flint_and_steel or fire_charge to light portal");
            return o;
        }
        BlockPos pp = player.blockPosition();
        double yawRad = Math.toRadians(player.getYRot());
        int dx = (int)Math.round(-Math.sin(yawRad));
        int dz = (int)Math.round(Math.cos(yawRad));
        int ox = pp.getX() + dx * 3;
        int oz = pp.getZ() + dz * 3;
        int oy = pp.getY();
        int placed = 0;
        for (int[] f : frame = new int[][]{{0, 0}, {1, 0}, {2, 0}, {3, 0}, {0, 1}, {3, 1}, {0, 2}, {3, 2}, {0, 3}, {3, 3}, {0, 4}, {1, 4}, {2, 4}, {3, 4}}) {
            int bx = ox + (dx != 0 ? 0 : f[0]);
            int bz = oz + (dz != 0 ? 0 : f[1]);
            int by = oy + (dx != 0 ? f[1] : f[0]);
            if (dx != 0) {
                bx = ox + f[1] * 0;
                bz = oz + f[0];
                by = oy + f[0];
            }
            bx = ox + (dx != 0 ? 0 : f[0]);
            bz = oz + (dz != 0 ? 0 : f[1]);
            by = oy + (dx != 0 ? f[1] : f[0]);
            if (Math.abs(dx) > Math.abs(dz)) {
                bx = ox;
                bz = oz + f[0];
                by = oy + f[1];
            } else {
                bx = ox + f[0];
                bz = oz;
                by = oy + f[1];
            }
            BlockPos framePos = new BlockPos(bx, by, bz);
            BlockState existing = level.getBlockState(framePos);
            if (!existing.isAir()) continue;
            // Direct setBlock: bypass placeAt's useItemOn distance check in MC 26.2
            Block heldBlock = Block.byItem(player.getMainHandItem().getItem());
            if (heldBlock == null || heldBlock == Blocks.AIR) {
                // find obsidian in inventory
                boolean found = false;
                for (int si = 0; si < player.getInventory().getContainerSize(); si++) {
                    ItemStack s = player.getInventory().getItem(si);
                    if (s.isEmpty()) continue;
                    Block b = Block.byItem(s.getItem());
                    if (b == null || b == Blocks.AIR) continue;
                    String key = BuiltInRegistries.BLOCK.getKey(b).toString().toLowerCase();
                    if (!key.contains(search)) continue;
                    player.getInventory().setSelectedSlot(si);
                    found = true;
                    break;
                }
                if (!found) continue;
            }
            level.setBlock(framePos, Blocks.OBSIDIAN.defaultBlockState(), 3);
            player.getMainHandItem().shrink(1);
            ++placed;
        }
        if (placed < 10) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "build_portal: only placed " + placed + "/10 obsidian, need more space");
            return o;
        }
        int lightX = ox + (Math.abs(dx) > Math.abs(dz) ? 0 : 1);
        int lightZ = oz + (Math.abs(dz) > 0 ? 0 : 1);
        int lightY = oy + 1;
        if (Math.abs(dx) > Math.abs(dz)) {
            lightX = ox;
            lightZ = oz + 1;
            lightY = oy + 1;
        } else {
            lightX = ox + 1;
            lightZ = oz;
            lightY = oy + 1;
        }
        BlockPos lightPos = new BlockPos(lightX, lightY, lightZ);
        for (int i = 0; i < 9; ++i) {
            String key10;
            ItemStack s = inv.getItem(i);
            if (s.isEmpty() || !(key10 = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase()).contains("flint_and_steel") && !key10.contains("fire_charge")) continue;
            inv.setSelectedSlot(i);
            break;
        }
        BlockHitResult hit = new BlockHitResult(Vec3.atCenterOf((Vec3i)lightPos), Direction.UP, lightPos, false);
        InteractionResult result = player.gameMode.useItemOn(player, (Level)level, player.getMainHandItem(), InteractionHand.MAIN_HAND, hit);
        if (result.consumesAction()) {
            o.addProperty("detail", "build_portal: built and lit at (" + ox + "," + oy + "," + oz + ")");
            return o;
        }
        o.addProperty("detail", "build_portal: placed obsidian frame at (" + ox + "," + oy + "," + oz + ") but lighting failed (try activate_block manually)");
        return o;
    }
}
