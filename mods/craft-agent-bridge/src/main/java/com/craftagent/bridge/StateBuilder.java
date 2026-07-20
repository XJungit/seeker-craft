package com.craftagent.bridge;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Vec3i;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.effect.MobEffect;
import net.minecraft.world.effect.MobEffectInstance;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.Mob;
import net.minecraft.world.entity.monster.Monster;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.LightLayer;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.HitResult;
import net.minecraft.world.phys.Vec3;

public class StateBuilder {

    public static JsonObject buildState() {
        JsonObject o = new JsonObject();
        MinecraftServer server = CraftAgentBridge.serverInstance;
        if (server == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u670d\u52a1\u5668\u672a\u5c31\u7eea");
            return o;
        }
        ServerPlayer player = FakePlayerManager.getFirstPlayer(server);
        if (player == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u6ca1\u6709\u5728\u7ebf\u73a9\u5bb6\uff08\u8bf7\u5148\u8fdb\u5165\u4e16\u754c\uff09");
            return o;
        }
        ServerLevel level = player.level();
        Vec3 pos = player.position();
        o.add("position", CraftAgentBridge.arr(pos.x, pos.y, pos.z));
        o.addProperty("yaw", player.getYRot());
        o.addProperty("pitch", player.getXRot());
        o.addProperty("health", player.getHealth());
        o.addProperty("hunger", player.getFoodData().getFoodLevel());
        o.addProperty("gamemode", player.gameMode.getGameModeForPlayer().getName());
        o.addProperty("time", level.getOverworldClockTime());
        o.addProperty("dimension", level.dimension().toString());
        o.addProperty("biome", level.getBiomeManager().getBiome(player.blockPosition()).unwrapKey().map(k -> k.identifier().toString()).orElse("?"));
        long time = level.getOverworldClockTime() % 24000L;
        int hour = (int)((time / 1000L + 6L) % 24L);
        int minute = (int)(time % 1000L * 60L / 1000L);
        boolean isDay = time < 12000L || time >= 23000L;
        o.addProperty("time_str", String.format("%02d:%02d (%s)", hour, minute, isDay ? "day" : "night"));
        Vec3 vel = player.getDeltaMovement();
        o.add("velocity", CraftAgentBridge.arr(vel.x, vel.y, vel.z));
        JsonArray effects = new JsonArray();
        for (MobEffectInstance me : player.getActiveEffects()) {
            MobEffect effect = me.getEffect().value();
            String id = BuiltInRegistries.MOB_EFFECT.getKey(effect).toString();
            JsonObject eo = new JsonObject();
            eo.addProperty("id", id);
            eo.addProperty("amplifier", me.getAmplifier());
            eo.addProperty("duration", me.getDuration());
            effects.add(eo);
        }
        o.add("effects", effects);
        o.addProperty("experience_level", player.experienceLevel);
        o.addProperty("experience_progress", player.experienceProgress);
        o.addProperty("raining", level.isRaining());
        o.addProperty("thundering", level.isThundering());
        BlockPos pp = player.blockPosition();
        int skyLight = level.getLightEngine().getLayerListener(LightLayer.SKY).getLightValue(pp);
        int blockLight = level.getLightEngine().getLayerListener(LightLayer.BLOCK).getLightValue(pp);
        o.addProperty("sky_light", skyLight);
        o.addProperty("block_light", blockLight);
        JsonArray inv = new JsonArray();
        Inventory inventory = player.getInventory();
        int size = inventory.getContainerSize();
        for (int i = 0; i < size; ++i) {
            ItemStack stack = inventory.getItem(i);
            if (stack.isEmpty()) continue;
            String id = BuiltInRegistries.ITEM.getKey(stack.getItem()).toString();
            JsonObject s = new JsonObject();
            s.addProperty("slot", i);
            s.addProperty("id", id);
            s.addProperty("count", stack.getCount());
            inv.add(s);
        }
        o.add("inventory", inv);
        ItemStack held = player.getMainHandItem();
        int selectedSlot = inventory.getSelectedSlot();
        o.addProperty("held_item", held.isEmpty() ? "minecraft:air" : BuiltInRegistries.ITEM.getKey(held.getItem()).toString());
        o.addProperty("selected_slot", selectedSlot);
        HitResult hit = player.pick(6.0, 0.0f, false);
        if (hit != null && hit.getType() == HitResult.Type.BLOCK) {
            BlockPos bp = ((BlockHitResult)hit).getBlockPos();
            BlockState bs = level.getBlockState(bp);
            String id = BuiltInRegistries.BLOCK.getKey(bs.getBlock()).toString();
            double dist = player.position().distanceTo(Vec3.atCenterOf(bp));
            JsonObject tb = new JsonObject();
            tb.addProperty("id", id);
            tb.addProperty("dist", dist);
            tb.addProperty("x", bp.getX());
            tb.addProperty("y", bp.getY());
            tb.addProperty("z", bp.getZ());
            o.add("targeted_block", tb);
        } else {
            o.add("targeted_block", null);
        }
        JsonArray blocks = new JsonArray();
        BlockPos pc = player.blockPosition();
        for (BlockPos bp : BlockPos.betweenClosed(pc.getX() - 16, pc.getY() - 16, pc.getZ() - 16, pc.getX() + 16, pc.getY() + 16, pc.getZ() + 16)) {
            BlockState bs = level.getBlockState(bp);
            if (bs.isAir()) continue;
            String id = BuiltInRegistries.BLOCK.getKey(bs.getBlock()).toString();
            if (!matchesWhitelist(id)) continue;
            double dist = player.position().distanceTo(Vec3.atCenterOf(bp));
            JsonObject b = new JsonObject();
            b.addProperty("id", id);
            b.addProperty("x", bp.getX());
            b.addProperty("y", bp.getY());
            b.addProperty("z", bp.getZ());
            b.addProperty("dist", dist);
            b.addProperty("height_diff", player.getY() - (double)bp.getY());
            blocks.add(b);
        }
        o.add("nearby_blocks", blocks);
        JsonArray ents = new JsonArray();
        AABB scanArea = AABB.ofSize(player.position(), 32.0, 32.0, 32.0);
        for (Entity e : level.getEntities(player, scanArea)) {
            if (e == player) continue;
            String tid = BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString();
            Vec3 ep = e.position();
            double dist = player.distanceTo(e);
            JsonObject en = new JsonObject();
            en.addProperty("type", tid);
            en.addProperty("x", ep.x);
            en.addProperty("y", ep.y);
            en.addProperty("z", ep.z);
            en.addProperty("dist", dist);
            float hp = (e instanceof LivingEntity) ? ((LivingEntity)e).getHealth() : 0.0f;
            en.addProperty("health", hp);
            ents.add(en);
        }
        o.add("entities", ents);
        String nearestThreatType = null;
        double nearestThreatDist = Double.MAX_VALUE;
        for (Entity e : level.getEntities(player, scanArea)) {
            if (e == player || !(e instanceof Mob)) continue;
            Mob mob = (Mob)e;
            boolean hostile = e instanceof Monster;
            if (!hostile && mob.getTarget() == player) {
                hostile = true;
            }
            if (!hostile) continue;
            double d = player.distanceTo(e);
            if (!(d < nearestThreatDist)) continue;
            nearestThreatDist = d;
            nearestThreatType = BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString();
        }
        if (nearestThreatType != null) {
            JsonObject nt = new JsonObject();
            nt.addProperty("type", nearestThreatType);
            nt.addProperty("dist", nearestThreatDist);
            o.add("nearest_threat", nt);
        } else {
            o.add("nearest_threat", null);
        }
        o.addProperty("status", "ok");
        return o;
    }

    private static boolean matchesWhitelist(String id) {
        String lower = id.toLowerCase();
        for (String k : CraftAgentBridge.BLOCK_WHITELIST) {
            if (!lower.contains(k)) continue;
            return true;
        }
        return false;
    }
}
