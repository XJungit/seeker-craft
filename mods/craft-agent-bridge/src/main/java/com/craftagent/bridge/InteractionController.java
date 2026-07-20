package com.craftagent.bridge;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import net.minecraft.commands.arguments.EntityAnchorArgument;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.Vec3i;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.Vec3;

public class InteractionController {

    public static JsonObject actLook(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        int dx = req.has("dx") ? req.get("dx").getAsInt() : 0;
        int dy = req.has("dy") ? req.get("dy").getAsInt() : 0;
        float yaw = player.getYRot() - (float) dx * 0.3f;
        float pitch = CraftAgentBridge.clamp(player.getXRot() + (float) dy * 0.3f, -90.0f, 90.0f);
        player.setYRot(yaw);
        player.setXRot(pitch);
        o.addProperty("detail", "look dx=" + dx + " dy=" + dy);
        return o;
    }

    public static JsonObject actLookAbs(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        float yaw = req.has("yaw") ? req.get("yaw").getAsFloat() : player.getYRot();
        float pitch = req.has("pitch") ? req.get("pitch").getAsFloat() : player.getXRot();
        player.setYRot(yaw);
        player.setXRot(CraftAgentBridge.clamp(pitch, -90.0f, 90.0f));
        o.addProperty("detail", "look_abs yaw=" + yaw + " pitch=" + pitch);
        return o;
    }

    public static JsonObject actLookAt(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        double tx = req.get("x").getAsDouble();
        double ty = req.get("y").getAsDouble();
        double tz = req.get("z").getAsDouble();
        Vec3 eye = player.getEyePosition();
        double bx = Math.abs(tx % 1.0) < 0.01 ? tx + 0.5 : tx;
        double by = Math.abs(ty % 1.0) < 0.01 ? ty + 0.5 : ty;
        double bz = Math.abs(tz % 1.0) < 0.01 ? tz + 0.5 : tz;
        double ddx = bx - eye.x;
        double ddy = by - eye.y;
        double ddz = bz - eye.z;
        double len = Math.sqrt(ddx * ddx + ddy * ddy + ddz * ddz);
        if (len < 0.001) return o;
        float yaw = (float) Math.toDegrees(Math.atan2(-ddx, ddz));
        float pitch = (float) Math.toDegrees(-Math.asin(CraftAgentBridge.clamp(ddy / len, -1.0, 1.0)));
        player.setYRot(yaw);
        player.setXRot(CraftAgentBridge.clamp(pitch, -90.0f, 90.0f));
        o.addProperty("detail", "look_at(" + tx + "," + ty + "," + tz + ")");
        return o;
    }

    public static JsonObject actDigAt(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        int tx = req.get("x").getAsInt();
        int ty = req.get("y").getAsInt();
        int tz = req.get("z").getAsInt();
        BlockPos pos = new BlockPos(tx, ty, tz);
        BlockState state = level.getBlockState(pos);
        if (state.isAir()) {
            o.addProperty("broken", Boolean.valueOf(false));
            o.addProperty("detail", "dig_at: block is air");
            return o;
        }
        double dist = player.position().distanceTo(Vec3.atCenterOf((Vec3i) pos));
        if (dist > 5.5) {
            o.addProperty("broken", Boolean.valueOf(false));
            o.addProperty("detail", "dig_at: too far (" + String.format("%.1f", dist) + "m)");
            return o;
        }
        String blockId = BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString();
        InventoryHelper.equipBestTool(player, blockId);
        boolean ok = player.level().destroyBlock(pos, true);
        player.containerMenu.broadcastChanges();
        o.addProperty("broken", Boolean.valueOf(ok));
        o.addProperty("block_id", blockId);
        o.addProperty("detail", "dig_at " + tx + "," + ty + "," + tz + " (broken=" + ok + ", block=" + blockId + ")");
        return o;
    }

    public static JsonObject actPlaceAt(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        int tx = req.get("x").getAsInt();
        int ty = req.get("y").getAsInt();
        int tz = req.get("z").getAsInt();
        String item = req.has("item") ? req.get("item").getAsString() : "dirt";
        boolean placed = InventoryHelper.placeAt(player, level, tx, ty, tz, item);
        player.containerMenu.broadcastChanges();
        o.addProperty("placed", Boolean.valueOf(placed));
        o.addProperty("detail", "place_at " + tx + "," + ty + "," + tz + " item=" + item + " (placed=" + placed + ")");
        return o;
    }

    public static JsonObject actGetBlock(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        int x = req.get("x").getAsInt();
        int y = req.get("y").getAsInt();
        int z = req.get("z").getAsInt();
        BlockState state = level.getBlockState(new BlockPos(x, y, z));
        String id = BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString();
        o.addProperty("id", id);
        o.addProperty("solid", Boolean.valueOf(!state.isAir() && !state.canBeReplaced()));
        o.addProperty("air", Boolean.valueOf(state.isAir()));
        return o;
    }

    public static JsonObject actGetBlocks(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        int x1 = req.get("x1").getAsInt();
        int y1 = req.get("y1").getAsInt();
        int z1 = req.get("z1").getAsInt();
        int x2 = req.get("x2").getAsInt();
        int y2 = req.get("y2").getAsInt();
        int z2 = req.get("z2").getAsInt();
        JsonArray blocks = new JsonArray();
        for (BlockPos bp : BlockPos.betweenClosed((int) x1, (int) y1, (int) z1, (int) x2, (int) y2, (int) z2)) {
            BlockState state = level.getBlockState(bp);
            if (state.isAir()) continue;
            String id = BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString();
            JsonObject b = new JsonObject();
            b.addProperty("x", (Number) bp.getX());
            b.addProperty("y", (Number) bp.getY());
            b.addProperty("z", (Number) bp.getZ());
            b.addProperty("id", id);
            b.addProperty("solid", Boolean.valueOf(!state.canBeReplaced()));
            blocks.add((JsonElement) b);
        }
        o.add("blocks", (JsonElement) blocks);
        o.addProperty("count", (Number) blocks.size());
        return o;
    }

    public static JsonObject actAttack(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        LivingEntity target = null;
        double minDist = Double.MAX_VALUE;
        AABB scanArea = AABB.ofSize((Vec3)player.position(), (double)16.0, (double)16.0, (double)16.0);
        for (Entity e4 : level.getEntities((Entity)player, scanArea)) {
            double d;
            if (!(e4 instanceof LivingEntity)) continue;
            LivingEntity le = (LivingEntity)e4;
            String tn = BuiltInRegistries.ENTITY_TYPE.getKey(e4.getType()).getPath();
            if (!InventoryHelper.isHostile(tn) || !((d = (double)e4.distanceTo((Entity)player)) < minDist)) continue;
            minDist = d;
            target = le;
        }
        if (target == null) {
            o.addProperty("detail", "attack: no hostile entity nearby");
            return o;
        }
        InventoryHelper.equipBestWeapon(player);
        player.lookAt(EntityAnchorArgument.Anchor.EYES, target.position().add(0.0, 1.0, 0.0));
        player.attack(target);
        player.containerMenu.broadcastChanges();
        o.addProperty("detail", "attack " + BuiltInRegistries.ENTITY_TYPE.getKey(target.getType()).getPath() + " dist=" + String.format("%.1f", minDist) + "m");
        return o;
    }

    public static JsonObject actLookAtPlayer(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String targetName = req.has("player_name") ? req.get("player_name").getAsString() : "";
        ServerPlayer target = null;
        for (ServerPlayer p : CraftAgentBridge.serverInstance.getPlayerList().getPlayers()) {
            if (!p.getName().getString().equalsIgnoreCase(targetName)) continue;
            target = p;
            break;
        }
        if (target == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "look_at_player: player '" + targetName + "' not found");
            return o;
        }
        double dx = target.getX() - player.getX();
        double dy = target.getY() + (double)target.getEyeHeight() - (player.getY() + (double)player.getEyeHeight());
        double dz = target.getZ() - player.getZ();
        double horiz = Math.sqrt(dx * dx + dz * dz);
        player.setYRot((float)Math.toDegrees(Math.atan2(-dx, dz)));
        player.setXRot((float)Math.toDegrees(-Math.atan2(dy, horiz)));
        o.addProperty("detail", "look_at_player: looking at " + targetName);
        return o;
    }

    public static JsonObject actLookAtPosition(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        double tx = req.has("x") ? req.get("x").getAsDouble() : player.getX();
        double ty = req.has("y") ? req.get("y").getAsDouble() : player.getY();
        double tz = req.has("z") ? req.get("z").getAsDouble() : player.getZ();
        double dx = tx - player.getX();
        double dy = ty - (player.getY() + (double)player.getEyeHeight());
        double dz = tz - player.getZ();
        double horiz = Math.sqrt(dx * dx + dz * dz);
        player.setYRot((float)Math.toDegrees(Math.atan2(-dx, dz)));
        player.setXRot((float)Math.toDegrees(-Math.atan2(dy, horiz)));
        o.addProperty("detail", "look_at_position: looking at (" + tx + "," + ty + "," + tz + ")");
        return o;
    }

    public static JsonObject actActivateBlock(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        int x = req.get("x").getAsInt();
        int y = req.get("y").getAsInt();
        int z = req.get("z").getAsInt();
        BlockPos bp = new BlockPos(x, y, z);
        double dist = player.position().distanceTo(Vec3.atCenterOf((Vec3i)bp));
        if (dist > 5.5) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "activate_block: too far (" + String.format("%.1f", dist) + "m)");
            return o;
        }
        BlockState state = level.getBlockState(bp);
        if (state.isAir()) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "activate_block: air at (" + x + "," + y + "," + z + ")");
            return o;
        }
        double dx = (double)x + 0.5 - player.getX();
        double dy = (double)y + 0.5 - (player.getY() + (double)player.getEyeHeight());
        double dz = (double)z + 0.5 - player.getZ();
        double horiz = Math.sqrt(dx * dx + dz * dz);
        player.setYRot((float)Math.toDegrees(Math.atan2(-dx, dz)));
        player.setXRot((float)Math.toDegrees(-Math.atan2(dy, horiz)));
        BlockHitResult hit = new BlockHitResult(Vec3.atCenterOf((Vec3i)bp), Direction.getNearest((int)((int)Math.round(dx)), (int)((int)Math.round(dy)), (int)((int)Math.round(dz)), (Direction)Direction.UP), bp, false);
        InteractionResult result = player.gameMode.useItemOn(player, (Level)level, player.getMainHandItem(), InteractionHand.MAIN_HAND, hit);
        player.containerMenu.broadcastChanges();
        o.addProperty("activated", Boolean.valueOf(result.consumesAction()));
        o.addProperty("detail", "activate_block (" + x + "," + y + "," + z + ") consumed=" + result.consumesAction());
        return o;
    }

    public static JsonObject actUseOnEntity(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String entityType = req.has("entity_type") ? req.get("entity_type").getAsString() : "";
        double radius = req.has("radius") ? req.get("radius").getAsDouble() : 8.0;
        Entity nearest = null;
        double minDist = Double.MAX_VALUE;
        for (Entity e8 : level.getEntities((Entity)player, AABB.ofSize((Vec3)player.position(), (double)(radius * 2.0), (double)(radius * 2.0), (double)(radius * 2.0)))) {
            double d;
            String eName;
            if (e8 instanceof ServerPlayer || !(eName = BuiltInRegistries.ENTITY_TYPE.getKey(e8.getType()).toString().toLowerCase()).contains(entityType.toLowerCase()) || !((d = (double)player.distanceTo(e8)) < minDist)) continue;
            minDist = d;
            nearest = e8;
        }
        if (nearest == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "use_on_entity: no '" + entityType + "' within " + radius + "m");
            return o;
        }
        double dx = nearest.getX() - player.getX();
        double dy = nearest.getY() + (double)nearest.getEyeHeight() - (player.getY() + (double)player.getEyeHeight());
        double dz = nearest.getZ() - player.getZ();
        double horiz = Math.sqrt(dx * dx + dz * dz);
        player.setYRot((float)Math.toDegrees(Math.atan2(-dx, dz)));
        player.setXRot((float)Math.toDegrees(-Math.atan2(dy, horiz)));
        InteractionResult result = player.interactOn(nearest, InteractionHand.MAIN_HAND, nearest.position());
        player.containerMenu.broadcastChanges();
        o.addProperty("interacted", Boolean.valueOf(result.consumesAction()));
        o.addProperty("detail", "use_on_entity " + entityType + " consumed=" + result.consumesAction());
        return o;
    }

    public static JsonObject actActivateNearestBlock(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        double radius = req.has("radius") ? req.get("radius").getAsDouble() : 5.0;
        String blockType = req.has("block_type") ? req.get("block_type").getAsString() : "";
        BlockPos nearest = null;
        double minDist = Double.MAX_VALUE;
        BlockPos pp = player.blockPosition();
        for (BlockPos bp : BlockPos.betweenClosed((BlockPos)pp.offset(-((int)radius), -2, -((int)radius)), (BlockPos)pp.offset((int)radius, 2, (int)radius))) {
            double d;
            BlockState s = level.getBlockState(bp);
            if (s.isAir()) continue;
            String id = BuiltInRegistries.BLOCK.getKey(s.getBlock()).toString().toLowerCase();
            if (!blockType.isEmpty() && !id.contains(blockType.toLowerCase()) || !((d = player.position().distanceTo(Vec3.atCenterOf((Vec3i)bp))) < minDist) || !(d < 5.5)) continue;
            minDist = d;
            nearest = bp.immutable();
        }
        if (nearest == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "activate_nearest_block: no '" + blockType + "' within " + radius + "m");
            return o;
        }
        double dx = (double)nearest.getX() + 0.5 - player.getX();
        double dy = (double)nearest.getY() + 0.5 - (player.getY() + (double)player.getEyeHeight());
        double dz = (double)nearest.getZ() + 0.5 - player.getZ();
        double horiz = Math.sqrt(dx * dx + dz * dz);
        player.setYRot((float)Math.toDegrees(Math.atan2(-dx, dz)));
        player.setXRot((float)Math.toDegrees(-Math.atan2(dy, horiz)));
        BlockHitResult hit = new BlockHitResult(Vec3.atCenterOf((Vec3i)nearest), Direction.getNearest((int)((int)Math.round(dx)), (int)((int)Math.round(dy)), (int)((int)Math.round(dz)), (Direction)Direction.UP), nearest, false);
        InteractionResult result = player.gameMode.useItemOn(player, (Level)level, player.getMainHandItem(), InteractionHand.MAIN_HAND, hit);
        player.containerMenu.broadcastChanges();
        o.addProperty("activated", Boolean.valueOf(result.consumesAction()));
        o.addProperty("x", (Number)nearest.getX());
        o.addProperty("y", (Number)nearest.getY());
        o.addProperty("z", (Number)nearest.getZ());
        o.addProperty("detail", "activate_nearest_block (" + nearest.getX() + "," + nearest.getY() + "," + nearest.getZ() + ") consumed=" + result.consumesAction());
        return o;
    }
}
