package com.craftagent.bridge;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import java.util.HashSet;
import net.minecraft.commands.arguments.EntityAnchorArgument;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Vec3i;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.tags.FluidTags;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;

public class MovementController {

    public static boolean isInWater(ServerPlayer player) {
        return player.isInWater() || player.isEyeInFluid(FluidTags.WATER);
    }

    public static JsonObject performMoveTo(JsonObject req) {
        JsonObject o = new JsonObject();
        if (CraftAgentBridge.serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u670d\u52a1\u5668\u672a\u5c31\u7eea");
            return o;
        }
        double tx = req.get("x").getAsDouble();
        double ty = req.get("y").getAsDouble();
        double tz = req.get("z").getAsDouble();
        int maxTicks = req.has("max_ticks") ? req.get("max_ticks").getAsInt() : 200;
        CraftAgentBridge.moveReached = false;
        CraftAgentBridge.moveFinalDist = 999.0;
        CraftAgentBridge.moveStuck = false;
        CraftAgentBridge.moveTicksLeft = maxTicks;
        CraftAgentBridge.moveTarget = new double[]{tx, ty, tz};
        CraftAgentBridge.moveStuckCounter = 0;
        ServerLevel level = CraftAgentBridge.onServer(() -> {
            ServerPlayer p = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
            return p != null ? p.level() : null;
        });
        if (level == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u65e0\u6cd5\u83b7\u53d6\u4e16\u754c");
            return o;
        }
        BlockPos targetPos = BlockPos.containing((double)tx, (double)(ty + 1.0), (double)tz);
        BlockPos fromPos = CraftAgentBridge.onServer(() -> {
            ServerPlayer p = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
            return p != null ? p.blockPosition() : targetPos;
        });
        CraftAgentBridge.moveWaypoints = AStar.findPath(level, Vec3.atCenterOf((Vec3i)fromPos), Vec3.atCenterOf((Vec3i)targetPos));
        CraftAgentBridge.moveCurrentWpIndex = 0;
        if (CraftAgentBridge.moveWaypoints == null) {
            o.addProperty("status", "ok");
            o.addProperty("reached", Boolean.valueOf(false));
            o.addProperty("stuck", Boolean.valueOf(true));
            o.addProperty("detail", "no_path");
            CraftAgentBridge.moveTarget = null;
            return o;
        }
        if (CraftAgentBridge.moveWaypoints.isEmpty()) {
            o.addProperty("status", "ok");
            o.addProperty("reached", Boolean.valueOf(true));
            o.addProperty("stuck", Boolean.valueOf(false));
            o.addProperty("detail", "already_at_target");
            CraftAgentBridge.moveTarget = null;
            return o;
        }
        boolean hasWater = false;
        boolean hasFall = false;
        double prevY = fromPos.getY();
        for (Vec3 wp : CraftAgentBridge.moveWaypoints) {
            String waterId = BuiltInRegistries.BLOCK.getKey(level.getBlockState(BlockPos.containing((double)wp.x, (double)(prevY - 1.0), (double)wp.z)).getBlock()).toString();
            if (waterId.contains("water")) {
                hasWater = true;
            }
            if (wp.y < prevY - 3.0) {
                hasFall = true;
            }
            prevY = wp.y;
        }
        String detailSuffix = (hasWater ? " [WATER]" : "") + (hasFall ? " [FALL>3]" : "");
        int hardLimit = maxTicks * 50 + 2000;
        for (int waitMs = 0; waitMs < hardLimit && CraftAgentBridge.moveWaypoints != null; waitMs += 50) {
            try {
                Thread.sleep(50L);
            }
            catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                break;
            }
        }
        if (CraftAgentBridge.moveWaypoints != null) {
            CraftAgentBridge.moveWaypoints = null;
            CraftAgentBridge.moveTarget = null;
            CraftAgentBridge.moveReached = false;
        }
        o.addProperty("status", "ok");
        o.addProperty("reached", Boolean.valueOf(CraftAgentBridge.moveReached));
        o.addProperty("final_dist", (Number)CraftAgentBridge.moveFinalDist);
        o.addProperty("stuck", Boolean.valueOf(CraftAgentBridge.moveStuck));
        o.addProperty("detail", "move_to " + tx + "," + ty + "," + tz + " (reached=" + CraftAgentBridge.moveReached + ", dist=" + String.format("%.1f", CraftAgentBridge.moveFinalDist) + "m)" + detailSuffix);
        return o;
    }

    public static JsonObject performGoToPlayer(JsonObject req) {
        JsonObject o = new JsonObject();
        if (CraftAgentBridge.serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u670d\u52a1\u5668\u672a\u5c31\u7eea");
            return o;
        }
        String targetName = req.has("player_name") ? req.get("player_name").getAsString() : "";
        String targetId = CraftAgentBridge.onServer(() -> {
            for (ServerPlayer p : CraftAgentBridge.serverInstance.getPlayerList().getPlayers()) {
                if (!p.getName().getString().equalsIgnoreCase(targetName)) continue;
                return p.getUUID().toString();
            }
            return null;
        });
        if (targetId == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "go_to_player: player '" + targetName + "' not found");
            return o;
        }
        double[] targetPos = CraftAgentBridge.onServer(() -> {
            for (ServerPlayer p : CraftAgentBridge.serverInstance.getPlayerList().getPlayers()) {
                if (!p.getUUID().toString().equals(targetId)) continue;
                return new double[]{p.getX(), p.getY(), p.getZ()};
            }
            return null;
        });
        if (targetPos == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "go_to_player: player disappeared");
            return o;
        }
        double closeness = req.has("closeness") ? req.get("closeness").getAsDouble() : 2.5;
        CraftAgentBridge.moveReached = false;
        CraftAgentBridge.moveFinalDist = 999.0;
        CraftAgentBridge.moveStuck = false;
        CraftAgentBridge.moveTicksLeft = 400;
        CraftAgentBridge.moveTarget = targetPos;
        CraftAgentBridge.moveStuckCounter = 0;
        ServerLevel level = CraftAgentBridge.onServer(() -> {
            ServerPlayer p = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
            return p != null ? p.level() : null;
        });
        if (level == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u65e0\u6cd5\u83b7\u53d6\u4e16\u754c");
            return o;
        }
        BlockPos targetBlockPos = BlockPos.containing((double)targetPos[0], (double)targetPos[1], (double)targetPos[2]);
        BlockPos fromPos = CraftAgentBridge.onServer(() -> {
            ServerPlayer p = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
            return p != null ? p.blockPosition() : targetBlockPos;
        });
        CraftAgentBridge.moveWaypoints = AStar.findPath(level, Vec3.atCenterOf((Vec3i)fromPos), Vec3.atCenterOf((Vec3i)targetBlockPos));
        CraftAgentBridge.moveCurrentWpIndex = 0;
        if (CraftAgentBridge.moveWaypoints == null) {
            o.addProperty("status", "ok");
            o.addProperty("reached", Boolean.valueOf(false));
            o.addProperty("stuck", Boolean.valueOf(true));
            o.addProperty("detail", "go_to_player " + targetName + ": no_path");
            CraftAgentBridge.moveTarget = null;
            return o;
        }
        if (CraftAgentBridge.moveWaypoints.isEmpty()) {
            double dz;
            double dx = targetPos[0] - (double)fromPos.getX();
            double dist = Math.sqrt(dx * dx + (dz = targetPos[2] - (double)fromPos.getZ()) * dz);
            CraftAgentBridge.moveReached = dist <= closeness;
            CraftAgentBridge.moveFinalDist = dist;
            o.addProperty("status", "ok");
            o.addProperty("reached", Boolean.valueOf(CraftAgentBridge.moveReached));
            o.addProperty("final_dist", (Number)CraftAgentBridge.moveFinalDist);
            o.addProperty("detail", "go_to_player " + targetName + " reached=" + CraftAgentBridge.moveReached + " dist=" + String.format("%.1f", CraftAgentBridge.moveFinalDist));
            CraftAgentBridge.moveTarget = null;
            return o;
        }
        long start = System.currentTimeMillis();
        while (CraftAgentBridge.moveWaypoints != null && System.currentTimeMillis() - start < 20000L) {
            if (CraftAgentBridge.shouldStop) {
                CraftAgentBridge.shouldStop = false;
                break;
            }
            try {
                Thread.sleep(50L);
            }
            catch (InterruptedException e) {
                break;
            }
        }
        o.addProperty("status", "ok");
        o.addProperty("reached", Boolean.valueOf(CraftAgentBridge.moveReached));
        o.addProperty("final_dist", (Number)CraftAgentBridge.moveFinalDist);
        o.addProperty("detail", "go_to_player " + targetName + " reached=" + CraftAgentBridge.moveReached + " dist=" + String.format("%.1f", CraftAgentBridge.moveFinalDist));
        return o;
    }

    public static JsonObject performDiscardSmart(JsonObject req) {
        JsonObject o = new JsonObject();
        if (CraftAgentBridge.serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u670d\u52a1\u5668\u672a\u5c31\u7eea");
            return o;
        }
        String itemName = req.has("item") ? req.get("item").getAsString() : "";
        int num = req.has("num") ? req.get("num").getAsInt() : 1;
        String search = itemName.replace("minecraft:", "").toLowerCase();
        double[] startData = CraftAgentBridge.onServer(() -> {
            ServerPlayer p = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
            if (p == null) {
                return null;
            }
            return new double[]{p.getX(), p.getY(), p.getZ(), p.getYRot()};
        });
        if (startData == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "discard_smart: no player");
            return o;
        }
        double startX = startData[0];
        double startY = startData[1];
        double startZ = startData[2];
        float startYaw = (float)startData[3];
        float awayYaw = startYaw + 180.0f;
        double awayDx = -Math.sin(Math.toRadians(awayYaw)) * 5.0;
        double awayDz = Math.cos(Math.toRadians(awayYaw)) * 5.0;
        CraftAgentBridge.moveTarget = new double[]{startX + awayDx, startY, startZ + awayDz};
        CraftAgentBridge.moveTicksLeft = 100;
        long moveStart = System.currentTimeMillis();
        while (CraftAgentBridge.moveTarget != null && System.currentTimeMillis() - moveStart < 5000L) {
            if (CraftAgentBridge.shouldStop) {
                CraftAgentBridge.shouldStop = false;
                break;
            }
            try {
                Thread.sleep(100L);
            }
            catch (InterruptedException e) {
                break;
            }
        }
        int dropped = CraftAgentBridge.onServer(() -> {
            ServerPlayer p = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
            if (p == null) {
                return 0;
            }
            Inventory inv = p.getInventory();
            int count = 0;
            for (int i = 0; i < inv.getContainerSize() && count < num; ++i) {
                String key;
                ItemStack s = inv.getItem(i);
                if (s.isEmpty() || !(key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase()).contains(search)) continue;
                int take = Math.min(s.getCount(), num - count);
                ItemStack toDrop = s.copy();
                toDrop.setCount(take);
                s.shrink(take);
                p.drop(toDrop, false);
                count += take;
            }
            p.containerMenu.broadcastChanges();
            return count;
        });
        try {
            Thread.sleep(500L);
        }
        catch (InterruptedException interruptedException) {
        }
        CraftAgentBridge.moveTarget = new double[]{startX, startY, startZ};
        CraftAgentBridge.moveTicksLeft = 100;
        long returnStart = System.currentTimeMillis();
        while (CraftAgentBridge.moveTarget != null && System.currentTimeMillis() - returnStart < 5000L) {
            if (CraftAgentBridge.shouldStop) {
                CraftAgentBridge.shouldStop = false;
                break;
            }
            try {
                Thread.sleep(100L);
            }
            catch (InterruptedException e) {
                break;
            }
        }
        o.addProperty("status", "ok");
        o.addProperty("dropped", (Number)dropped);
        o.addProperty("detail", "discard_smart " + itemName + " x" + dropped + " (moved away 5m, dropped, returned)");
        return o;
    }

    public static JsonObject performCollectItems(JsonObject req) {
        JsonObject o = new JsonObject();
        if (CraftAgentBridge.serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u670d\u52a1\u5668\u672a\u5c31\u7eea");
            return o;
        }
        JsonArray itemFilters = req.has("item_ids") ? req.get("item_ids").getAsJsonArray() : new JsonArray();
        double radius = req.has("radius") ? req.get("radius").getAsDouble() : 16.0;
        int maxCount = req.has("max_count") ? req.get("max_count").getAsInt() : 64;
        HashSet<String> filters = new HashSet<String>();
        for (int i = 0; i < itemFilters.size(); ++i) {
            filters.add(itemFilters.get(i).getAsString().toLowerCase());
        }
        int collected = 0;
        long start = System.currentTimeMillis();
        while (collected < maxCount && System.currentTimeMillis() - start < 30000L) {
            double[] myPos;
            if (CraftAgentBridge.shouldStop) {
                CraftAgentBridge.shouldStop = false;
                break;
            }
            int[] collectedThisLoop = new int[]{0};
            CraftAgentBridge.onServer(() -> {
                ServerPlayer p = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
                if (p == null) {
                    return null;
                }
                ServerLevel lvl = p.level();
                AABB pickupArea = AABB.ofSize((Vec3)p.position(), (double)1.2, (double)0.5, (double)1.2);
                int picked = 0;
                for (Entity e : lvl.getEntities((Entity)p, pickupArea)) {
                    if (!(e instanceof ItemEntity)) continue;
                    ItemEntity ie = (ItemEntity)e;
                    String itemId = BuiltInRegistries.ITEM.getKey(ie.getItem().getItem()).toString().toLowerCase();
                    if (!filters.isEmpty()) {
                        boolean match = false;
                        for (String f : filters) {
                            if (!itemId.contains(f)) continue;
                            match = true;
                            break;
                        }
                        if (!match) continue;
                    }
                    int count = ie.getItem().getCount();
                    ie.playerTouch((Player)p);
                    if ((picked += count) <= 0) continue;
                    break;
                }
                collectedThisLoop[0] = picked;
                return null;
            });
            if (collectedThisLoop[0] > 0) {
                collected += collectedThisLoop[0];
                continue;
            }
            double[] itemInfo = CraftAgentBridge.onServer(() -> {
                ServerPlayer p = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
                if (p == null) {
                    return null;
                }
                ServerLevel lvl = p.level();
                ItemEntity nearest = null;
                double minDist = Double.MAX_VALUE;
                for (Entity e : lvl.getEntities((Entity)p, AABB.ofSize((Vec3)p.position(), (double)(radius * 2.0), (double)(radius * 2.0), (double)(radius * 2.0)))) {
                    double d;
                    if (!(e instanceof ItemEntity)) continue;
                    ItemEntity ie = (ItemEntity)e;
                    String itemId = BuiltInRegistries.ITEM.getKey(ie.getItem().getItem()).toString().toLowerCase();
                    if (!filters.isEmpty()) {
                        boolean match = false;
                        for (String f : filters) {
                            if (!itemId.contains(f)) continue;
                            match = true;
                            break;
                        }
                        if (!match) continue;
                    }
                    if (!((d = (double)p.distanceTo((Entity)ie)) < minDist)) continue;
                    minDist = d;
                    nearest = ie;
                }
                if (nearest == null) {
                    return null;
                }
                return new double[]{nearest.getX(), nearest.getY(), nearest.getZ(), minDist};
            });
            if (itemInfo == null) break;
            double nx = itemInfo[0];
            double ny = itemInfo[1];
            double nz = itemInfo[2];
            double minDist = itemInfo[3];
            if (minDist > 1.2 && (myPos = CraftAgentBridge.onServer(() -> {
                ServerPlayer p = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
                if (p == null) {
                    return null;
                }
                return new double[]{p.getX(), p.getY(), p.getZ()};
            })) != null) {
                CraftAgentBridge.moveTarget = new double[]{nx, myPos[1], nz};
                CraftAgentBridge.moveTicksLeft = 100;
                long walkStart = System.currentTimeMillis();
                while (CraftAgentBridge.moveTarget != null && System.currentTimeMillis() - walkStart < 5000L) {
                    if (CraftAgentBridge.shouldStop) {
                        CraftAgentBridge.shouldStop = false;
                        break;
                    }
                    try {
                        Thread.sleep(100L);
                    }
                    catch (InterruptedException e) {
                        break;
                    }
                }
            }
            try {
                Thread.sleep(50L);
            }
            catch (InterruptedException e) {
                break;
            }
        }
        o.addProperty("status", "ok");
        o.addProperty("collected", (Number)collected);
        o.addProperty("detail", "collect_items: collected " + collected + " items");
        return o;
    }

    public static JsonObject performAttackPlayer(JsonObject req) {
        JsonObject o = new JsonObject();
        if (CraftAgentBridge.serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u670d\u52a1\u5668\u672a\u5c31\u7eea");
            return o;
        }
        String targetName = req.has("player_name") ? req.get("player_name").getAsString() : "";
        int ticks = req.has("ticks") ? req.get("ticks").getAsInt() : 60;
        String targetId = CraftAgentBridge.onServer(() -> {
            for (ServerPlayer p : CraftAgentBridge.serverInstance.getPlayerList().getPlayers()) {
                if (!p.getName().getString().equalsIgnoreCase(targetName)) continue;
                return p.getUUID().toString();
            }
            return null;
        });
        if (targetId == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "attack_player: player '" + targetName + "' not found");
            return o;
        }
        int hitCount = 0;
        long start = System.currentTimeMillis();
        long timeout = (long)ticks * 50L;
        int attackCooldown = 0;
        block4: while (System.currentTimeMillis() - start < timeout) {
            if (CraftAgentBridge.shouldStop) {
                CraftAgentBridge.shouldStop = false;
                break;
            }
            double[] info = CraftAgentBridge.onServer(() -> {
                ServerPlayer p = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
                if (p == null) {
                    return null;
                }
                ServerPlayer target = null;
                for (ServerPlayer pp : CraftAgentBridge.serverInstance.getPlayerList().getPlayers()) {
                    if (!pp.getUUID().toString().equals(targetId)) continue;
                    target = pp;
                    break;
                }
                if (target == null || !target.isAlive()) {
                    return null;
                }
                double dist = p.distanceTo(target);
                return new double[]{target.getX(), target.getY(), target.getZ(), dist, target.getHealth()};
            });
            if (info == null) break;
            double tx = info[0];
            double ty = info[1];
            double tz = info[2];
            double dist = info[3];
            if (dist > 4.5) {
                CraftAgentBridge.moveReached = false;
                CraftAgentBridge.moveFinalDist = 999.0;
                CraftAgentBridge.moveStuck = false;
                CraftAgentBridge.moveTicksLeft = 40;
                CraftAgentBridge.moveTarget = new double[]{tx, ty, tz};
                long moveStart = System.currentTimeMillis();
                while (CraftAgentBridge.moveTarget != null && System.currentTimeMillis() - moveStart < 2000L && !CraftAgentBridge.shouldStop) {
                    try {
                        Thread.sleep(50L);
                    }
                    catch (InterruptedException e) {
                        continue block4;
                    }
                }
                continue;
            }
            if (attackCooldown <= 0) {
                boolean[] hit = new boolean[]{false};
                CraftAgentBridge.onServer(() -> {
                    ServerPlayer p = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
                    if (p == null) {
                        return null;
                    }
                    ServerPlayer target = null;
                    for (ServerPlayer pp : CraftAgentBridge.serverInstance.getPlayerList().getPlayers()) {
                        if (!pp.getUUID().toString().equals(targetId)) continue;
                        target = pp;
                        break;
                    }
                    if (target != null && target.isAlive() && (double)p.distanceTo(target) <= 5.0) {
                        InventoryHelper.equipBestWeapon(p);
                        double dx = target.getX() - p.getX();
                        double dy = target.getY() + (double)target.getEyeHeight() * 0.5 - (p.getY() + (double)p.getEyeHeight());
                        double dz = target.getZ() - p.getZ();
                        p.setYRot((float)Math.toDegrees(Math.atan2(-dx, dz)));
                        double horiz = Math.sqrt(dx * dx + dz * dz);
                        p.setXRot((float)(-Math.toDegrees(Math.atan2(dy, horiz))));
                        p.swing(InteractionHand.MAIN_HAND);
                        float dmg = (float)p.getAttributeValue(Attributes.ATTACK_DAMAGE);
                        target.hurt(p.level().damageSources().playerAttack((Player)p), dmg);
                        p.containerMenu.broadcastChanges();
                        hit[0] = true;
                    }
                    return null;
                });
                if (hit[0]) {
                    ++hitCount;
                }
                attackCooldown = 10;
            } else {
                --attackCooldown;
            }
            try {
                Thread.sleep(50L);
            }
            catch (InterruptedException e) {
                break;
            }
        }
        CraftAgentBridge.moveTarget = null;
        o.addProperty("status", "ok");
        o.addProperty("hits", (Number)hitCount);
        o.addProperty("detail", "attack_player " + targetName + " hits=" + hitCount);
        return o;
    }

    public static JsonObject performFollowPlayer(JsonObject req) {
        JsonObject o = new JsonObject();
        if (CraftAgentBridge.serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u670d\u52a1\u5668\u672a\u5c31\u7eea");
            return o;
        }
        String targetName = req.has("player_name") ? req.get("player_name").getAsString() : "";
        double followDist = req.has("follow_dist") ? req.get("follow_dist").getAsDouble() : 3.0;
        int totalTicks = req.has("ticks") ? req.get("ticks").getAsInt() : 600;
        String targetId = CraftAgentBridge.onServer(() -> {
            for (ServerPlayer p : CraftAgentBridge.serverInstance.getPlayerList().getPlayers()) {
                if (!p.getName().getString().equalsIgnoreCase(targetName)) continue;
                return p.getUUID().toString();
            }
            return null;
        });
        if (targetId == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "follow_player: player '" + targetName + "' not found");
            return o;
        }
        long start = System.currentTimeMillis();
        long timeout = (long)totalTicks * 50L;
        int followTicks = 0;
        while (System.currentTimeMillis() - start < timeout) {
            double[] myPos;
            if (CraftAgentBridge.shouldStop) {
                CraftAgentBridge.shouldStop = false;
                break;
            }
            double[] targetPos = CraftAgentBridge.onServer(() -> {
                for (ServerPlayer p : CraftAgentBridge.serverInstance.getPlayerList().getPlayers()) {
                    if (!p.getUUID().toString().equals(targetId)) continue;
                    if (!p.isAlive()) {
                        return null;
                    }
                    return new double[]{p.getX(), p.getY(), p.getZ()};
                }
                return null;
            });
            if (targetPos == null || (myPos = CraftAgentBridge.onServer(() -> {
                ServerPlayer p = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
                if (p == null) {
                    return null;
                }
                return new double[]{p.getX(), p.getY(), p.getZ()};
            })) == null) break;
            double dx = targetPos[0] - myPos[0];
            double dz = targetPos[2] - myPos[2];
            double dist = Math.sqrt(dx * dx + dz * dz);
            if (dist > followDist + 0.5) {
                CraftAgentBridge.moveReached = false;
                CraftAgentBridge.moveFinalDist = 999.0;
                CraftAgentBridge.moveStuck = false;
                CraftAgentBridge.moveTicksLeft = 30;
                CraftAgentBridge.moveTarget = (double[])targetPos.clone();
                long moveStart = System.currentTimeMillis();
                while (CraftAgentBridge.moveTarget != null && System.currentTimeMillis() - moveStart < 1500L && !CraftAgentBridge.shouldStop) {
                    try {
                        Thread.sleep(50L);
                    }
                    catch (InterruptedException e) {
                        break;
                    }
                }
            } else if (dist < followDist - 0.5) {
                double backX = myPos[0] - dx / dist * 2.0;
                double backZ = myPos[2] - dz / dist * 2.0;
                CraftAgentBridge.moveReached = false;
                CraftAgentBridge.moveFinalDist = 999.0;
                CraftAgentBridge.moveStuck = false;
                CraftAgentBridge.moveTicksLeft = 20;
                CraftAgentBridge.moveTarget = new double[]{backX, myPos[1], backZ};
                long moveStart = System.currentTimeMillis();
                while (CraftAgentBridge.moveTarget != null && System.currentTimeMillis() - moveStart < 1000L && !CraftAgentBridge.shouldStop) {
                    try {
                        Thread.sleep(50L);
                    }
                    catch (InterruptedException e) {
                        break;
                    }
                }
            } else {
                try {
                    Thread.sleep(100L);
                }
                catch (InterruptedException e) {
                    break;
                }
            }
            ++followTicks;
        }
        CraftAgentBridge.moveTarget = null;
        o.addProperty("status", "ok");
        o.addProperty("followed_ticks", (Number)followTicks);
        o.addProperty("detail", "follow_player " + targetName + " for " + followTicks + " ticks");
        return o;
    }

    public static JsonObject performCombat(JsonObject req) {
        JsonObject o = new JsonObject();
        if (CraftAgentBridge.serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u670d\u52a1\u5668\u672a\u5c31\u7eea");
            return o;
        }
        String mode = req.has("mode") ? req.get("mode").getAsString() : "melee";
        int maxTicks = req.has("ticks") ? req.get("ticks").getAsInt() : 200;
        String result = "none";
        String targetType = "";
        long start = System.currentTimeMillis();
        long timeout = (long)maxTicks * 50L;
        int attackCooldown = 0;
        block12: while (System.currentTimeMillis() - start < timeout) {
            double[] myPos;
            long moveStart;
            double dz;
            double dx;
            double len;
            if (CraftAgentBridge.shouldStop) {
                CraftAgentBridge.shouldStop = false;
                break;
            }
            String[] tType = new String[]{""};
            double[] info = CraftAgentBridge.onServer(() -> {
                ServerPlayer p = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
                if (p == null) {
                    return null;
                }
                ServerLevel lvl = p.level();
                LivingEntity target = null;
                double minDist = Double.MAX_VALUE;
                AABB scanArea = AABB.ofSize((Vec3)p.position(), (double)32.0, (double)32.0, (double)32.0);
                for (Entity e : lvl.getEntities((Entity)p, scanArea)) {
                    double d;
                    if (!(e instanceof LivingEntity)) continue;
                    LivingEntity le = (LivingEntity)e;
                    String tn = BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).getPath();
                    if (!InventoryHelper.isHostile(tn) || !((d = (double)e.distanceTo((Entity)p)) < minDist)) continue;
                    minDist = d;
                    target = le;
                }
                if (target == null) {
                    return null;
                }
                tType[0] = BuiltInRegistries.ENTITY_TYPE.getKey(target.getType()).getPath();
                return new double[]{target.getX(), target.getY(), target.getZ(), minDist, target.getHealth(), p.getHealth()};
            });
            if (info == null) {
                result = "no_target";
                break;
            }
            targetType = tType[0];
            double tx = info[0];
            double ty = info[1];
            double tz = info[2];
            double dist = info[3];
            double pHp = info[5];
            if (pHp < 5.0) {
                double dz2;
                double dx2;
                double len2;
                result = "retreated";
                double[] myPos2 = CraftAgentBridge.onServer(() -> {
                    ServerPlayer p = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
                    if (p == null) {
                        return null;
                    }
                    return new double[]{p.getX(), p.getY(), p.getZ()};
                });
                if (myPos2 == null || !((len2 = Math.sqrt((dx2 = myPos2[0] - tx) * dx2 + (dz2 = myPos2[2] - tz) * dz2)) > 0.0)) break;
                CraftAgentBridge.moveReached = false;
                CraftAgentBridge.moveFinalDist = 999.0;
                CraftAgentBridge.moveTicksLeft = 100;
                CraftAgentBridge.moveTarget = new double[]{myPos2[0] + dx2 / len2 * 15.0, myPos2[1], myPos2[2] + dz2 / len2 * 15.0};
                long moveStart2 = System.currentTimeMillis();
                while (CraftAgentBridge.moveTarget != null && System.currentTimeMillis() - moveStart2 < 5000L && !CraftAgentBridge.shouldStop) {
                    try {
                        Thread.sleep(50L);
                    }
                    catch (InterruptedException e) {
                        break block12;
                    }
                }
                break;
            }
            boolean isCreeper = targetType.contains("creeper");
            if (isCreeper && dist < 6.0 && !mode.equals("retreat")) {
                double[] myPos3 = CraftAgentBridge.onServer(() -> {
                    ServerPlayer p = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
                    if (p == null) {
                        return null;
                    }
                    return new double[]{p.getX(), p.getY(), p.getZ()};
                });
                if (myPos3 == null || !((len = Math.sqrt((dx = myPos3[0] - tx) * dx + (dz = myPos3[2] - tz) * dz)) > 0.0)) continue;
                CraftAgentBridge.moveReached = false;
                CraftAgentBridge.moveFinalDist = 999.0;
                CraftAgentBridge.moveTicksLeft = 30;
                CraftAgentBridge.moveTarget = new double[]{myPos3[0] + dx / len * 8.0, myPos3[1], myPos3[2] + dz / len * 8.0};
                moveStart = System.currentTimeMillis();
                while (CraftAgentBridge.moveTarget != null && System.currentTimeMillis() - moveStart < 1500L) {
                    try {
                        Thread.sleep(50L);
                    }
                    catch (InterruptedException e) {
                        continue block12;
                    }
                }
                continue;
            }
            if (mode.equals("retreat")) {
                double[] myPos4 = CraftAgentBridge.onServer(() -> {
                    ServerPlayer p = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
                    if (p == null) {
                        return null;
                    }
                    return new double[]{p.getX(), p.getY(), p.getZ()};
                });
                if (myPos4 != null && (len = Math.sqrt((dx = myPos4[0] - tx) * dx + (dz = myPos4[2] - tz) * dz)) > 0.0 && dist < 15.0) {
                    CraftAgentBridge.moveReached = false;
                    CraftAgentBridge.moveFinalDist = 999.0;
                    CraftAgentBridge.moveTicksLeft = 50;
                    CraftAgentBridge.moveTarget = new double[]{myPos4[0] + dx / len * 18.0, myPos4[1], myPos4[2] + dz / len * 18.0};
                    moveStart = System.currentTimeMillis();
                    while (CraftAgentBridge.moveTarget != null && System.currentTimeMillis() - moveStart < 2500L && !CraftAgentBridge.shouldStop) {
                        try {
                            Thread.sleep(50L);
                        }
                        catch (InterruptedException e) {
                            break;
                        }
                    }
                }
                if (!(dist > 15.0)) continue;
                result = "retreated";
                break;
            }
            if (dist > 4.0) {
                CraftAgentBridge.moveReached = false;
                CraftAgentBridge.moveFinalDist = 999.0;
                CraftAgentBridge.moveStuck = false;
                CraftAgentBridge.moveTicksLeft = 30;
                CraftAgentBridge.moveTarget = new double[]{tx, ty, tz};
                long moveStart3 = System.currentTimeMillis();
                while (CraftAgentBridge.moveTarget != null && System.currentTimeMillis() - moveStart3 < 1500L && !CraftAgentBridge.shouldStop) {
                    try {
                        Thread.sleep(50L);
                    }
                    catch (InterruptedException e) {
                        continue block12;
                    }
                }
                continue;
            }
            if (attackCooldown <= 0) {
                boolean[] killed = new boolean[]{false};
                CraftAgentBridge.onServer(() -> {
                    ServerPlayer p = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
                    if (p == null) {
                        return null;
                    }
                    ServerLevel lvl = p.level();
                    LivingEntity target = null;
                    double minDist = Double.MAX_VALUE;
                    AABB scanArea = AABB.ofSize((Vec3)p.position(), (double)10.0, (double)10.0, (double)10.0);
                    for (Entity e : lvl.getEntities((Entity)p, scanArea)) {
                        double d;
                        if (!(e instanceof LivingEntity)) continue;
                        LivingEntity le = (LivingEntity)e;
                        String tn = BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).getPath();
                        if (!InventoryHelper.isHostile(tn) || !((d = (double)e.distanceTo((Entity)p)) < minDist)) continue;
                        minDist = d;
                        target = le;
                    }
                    if (target != null && minDist <= 5.0) {
                        InventoryHelper.equipBestWeapon(p);
                        p.lookAt(EntityAnchorArgument.Anchor.EYES, target.position().add(0.0, 1.0, 0.0));
                        p.attack(target);
                        p.containerMenu.broadcastChanges();
                        if (!target.isAlive()) {
                            killed[0] = true;
                        }
                    }
                    return null;
                });
                if (killed[0]) {
                    result = "killed";
                    break;
                }
                attackCooldown = 10;
            } else {
                --attackCooldown;
            }
            if (mode.equals("kite") && attackCooldown > 5 && (myPos = CraftAgentBridge.onServer(() -> {
                ServerPlayer p = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
                if (p == null) {
                    return null;
                }
                return new double[]{p.getX(), p.getY(), p.getZ()};
            })) != null && (len = Math.sqrt((dx = myPos[0] - tx) * dx + (dz = myPos[2] - tz) * dz)) > 0.0 && dist < 6.0) {
                CraftAgentBridge.moveReached = false;
                CraftAgentBridge.moveFinalDist = 999.0;
                CraftAgentBridge.moveTicksLeft = 15;
                CraftAgentBridge.moveTarget = new double[]{myPos[0] + dx / len * 8.0, myPos[1], myPos[2] + dz / len * 8.0};
                moveStart = System.currentTimeMillis();
                while (CraftAgentBridge.moveTarget != null && System.currentTimeMillis() - moveStart < 800L) {
                    try {
                        Thread.sleep(50L);
                    }
                    catch (InterruptedException e) {
                        break;
                    }
                }
            }
            try {
                Thread.sleep(50L);
            }
            catch (InterruptedException e) {
                break;
            }
        }
        CraftAgentBridge.moveTarget = null;
        if (result.equals("none")) {
            result = "timeout";
        }
        o.addProperty("status", "ok");
        o.addProperty("result", result);
        o.addProperty("target", targetType);
        o.addProperty("detail", "combat mode=" + mode + " -> " + result + " (target=" + targetType + ")");
        return o;
    }

    public static JsonObject performUseItem(JsonObject req) {
        JsonObject o = new JsonObject();
        if (CraftAgentBridge.serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u670d\u52a1\u5668\u672a\u5c31\u7eea");
            return o;
        }
        int ticks = req.has("ticks") ? req.get("ticks").getAsInt() : 5;
        boolean[] consumed = new boolean[]{false};
        String[] itemId = new String[]{""};
        CraftAgentBridge.onServer(() -> {
            ServerPlayer p = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
            if (p == null) {
                return null;
            }
            ItemStack held = p.getMainHandItem();
            if (held.isEmpty()) {
                return null;
            }
            itemId[0] = BuiltInRegistries.ITEM.getKey(held.getItem()).getPath();
            InteractionResult result = p.gameMode.useItem(p, (Level)p.level(), held, InteractionHand.MAIN_HAND);
            consumed[0] = result.consumesAction();
            p.containerMenu.broadcastChanges();
            return null;
        });
        if (itemId[0].isEmpty()) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "use_item: main hand empty");
            return o;
        }
        if (consumed[0] && ticks > 1) {
            try {
                Thread.sleep((long)ticks * 50L);
            }
            catch (InterruptedException interruptedException) {
            }
        }
        o.addProperty("status", "ok");
        o.addProperty("consumed", Boolean.valueOf(consumed[0]));
        o.addProperty("detail", "use_item " + itemId[0] + " (consumed=" + consumed[0] + ")");
        return o;
    }

    public static JsonObject performEatItem(JsonObject req) {
        JsonObject o = new JsonObject();
        if (CraftAgentBridge.serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u670d\u52a1\u5668\u672a\u5c31\u7eea");
            return o;
        }
        String itemName = req.has("item") ? req.get("item").getAsString() : "";
        int eatTicks = req.has("ticks") ? req.get("ticks").getAsInt() : 32;
        String search = itemName.replace("minecraft:", "").toLowerCase();
        boolean[] found = new boolean[]{false};
        boolean[] consumed = new boolean[]{false};
        CraftAgentBridge.onServer(() -> {
            ServerPlayer p = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
            if (p == null) {
                return null;
            }
            Inventory inv = p.getInventory();
            int eatSlot = -1;
            for (int i = 0; i < inv.getContainerSize(); ++i) {
                String key;
                ItemStack s = inv.getItem(i);
                if (s.isEmpty() || !(key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase()).contains(search)) continue;
                eatSlot = i;
                break;
            }
            if (eatSlot < 0) {
                return null;
            }
            found[0] = true;
            if (eatSlot < 9) {
                inv.setSelectedSlot(eatSlot);
            } else {
                int dst = 0;
                for (int i = 0; i < 9; ++i) {
                    if (!inv.getItem(i).isEmpty()) continue;
                    dst = i;
                    break;
                }
                ItemStack tmp = inv.getItem(dst);
                inv.setItem(dst, inv.getItem(eatSlot));
                inv.setItem(eatSlot, tmp);
                inv.setSelectedSlot(dst);
            }
            p.containerMenu.broadcastChanges();
            InteractionResult result = p.gameMode.useItem(p, (Level)p.level(), p.getMainHandItem(), InteractionHand.MAIN_HAND);
            consumed[0] = result.consumesAction();
            p.containerMenu.broadcastChanges();
            return null;
        });
        if (!found[0]) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "eat_item: " + itemName + " not found");
            return o;
        }
        if (consumed[0]) {
            try {
                Thread.sleep((long)eatTicks * 50L);
            }
            catch (InterruptedException interruptedException) {
            }
            CraftAgentBridge.onServer(() -> {
                ServerPlayer p = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
                if (p != null) {
                    p.containerMenu.broadcastChanges();
                }
                return null;
            });
        }
        o.addProperty("status", "ok");
        o.addProperty("consumed", Boolean.valueOf(consumed[0]));
        o.addProperty("detail", "eat_item " + itemName + " (consumed=" + consumed[0] + ")");
        return o;
    }

    public static JsonObject performPillarUp(JsonObject req) {
        Boolean ok;
        JsonObject o = new JsonObject();
        if (CraftAgentBridge.serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u670d\u52a1\u5668\u672a\u5c31\u7eea");
            return o;
        }
        int count = req.has("count") ? req.get("count").getAsInt() : 3;
        String item = req.has("item") ? req.get("item").getAsString() : "dirt";
        int placed = 0;
        for (int i = 0; i < count && (ok = CraftAgentBridge.onServer(() -> {
            BlockPos below;
            ServerPlayer p = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
            if (p == null) {
                return false;
            }
            if (CraftAgentBridge.shouldStop) {
                return false;
            }
            ServerLevel lvl = p.level();
            if (InventoryHelper.placeAt(p, lvl, (below = p.blockPosition().below()).getX(), below.getY(), below.getZ(), item)) {
                p.setDeltaMovement(p.getDeltaMovement().x, 0.42, p.getDeltaMovement().z);
                return true;
            }
            return false;
        })) != null && ok.booleanValue(); ++i) {
            ++placed;
            try {
                Thread.sleep(200L);
            }
            catch (InterruptedException e) {
                break;
            }
        }
        o.addProperty("status", "ok");
        o.addProperty("pillar_count", (Number)placed);
        o.addProperty("detail", "pillar_up count=" + count + " placed=" + placed);
        return o;
    }

    public static JsonObject performWait(JsonObject req) {
        JsonObject o = new JsonObject();
        int seconds = req.has("seconds") ? req.get("seconds").getAsInt() : 1;
        try {
            Thread.sleep((long)seconds * 1000L);
        }
        catch (InterruptedException interruptedException) {
        }
        o.addProperty("status", "ok");
        o.addProperty("detail", "wait " + seconds + "s");
        return o;
    }

    public static CombatResult combat(ServerPlayer player, ServerLevel level, String mode, int maxTicks) {
        CombatResult cr = new CombatResult();
        long start = System.currentTimeMillis();
        long timeout = (long)maxTicks * 50L;
        while (System.currentTimeMillis() - start < timeout) {
            LivingEntity target = null;
            double minDist = Double.MAX_VALUE;
            AABB scanArea = AABB.ofSize((Vec3)player.position(), (double)32.0, (double)32.0, (double)32.0);
            for (Entity e : level.getEntities((Entity)player, scanArea)) {
                double d;
                if (!(e instanceof LivingEntity)) continue;
                LivingEntity le = (LivingEntity)e;
                String tn = BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).getPath();
                if (!InventoryHelper.isHostile(tn) || !((d = (double)e.distanceTo((Entity)player)) < minDist)) continue;
                minDist = d;
                target = le;
            }
            if (target == null) {
                cr.result = "no_target";
                break;
            }
            cr.targetType = BuiltInRegistries.ENTITY_TYPE.getKey(target.getType()).getPath();
            double dist = target.distanceTo((Entity)player);
            if (player.getHealth() < 5.0f) {
                cr.result = "retreated";
                float yaw = (float)Math.toDegrees(Math.atan2(-(player.getX() - target.getX()), player.getZ() - target.getZ()));
                player.setYRot(yaw);
                player.setDeltaMovement(-Math.cos(Math.toRadians(yaw)) * 0.28, 0.0, Math.sin(Math.toRadians(yaw)) * 0.28);
                break;
            }
            if (cr.targetType.contains("creeper") && dist < 6.0) {
                float yaw = (float)Math.toDegrees(Math.atan2(-(player.getX() - target.getX()), player.getZ() - target.getZ()));
                player.setYRot(yaw);
                player.setDeltaMovement(-Math.cos(Math.toRadians(yaw)) * 0.28, 0.0, Math.sin(Math.toRadians(yaw)) * 0.28);
                continue;
            }
            if (mode.equals("retreat")) {
                float yaw = (float)Math.toDegrees(Math.atan2(-(player.getX() - target.getX()), player.getZ() - target.getZ()));
                player.setYRot(yaw);
                player.setDeltaMovement(-Math.cos(Math.toRadians(yaw)) * 0.28, 0.0, Math.sin(Math.toRadians(yaw)) * 0.28);
                if (!(dist > 15.0)) continue;
                cr.result = "retreated";
                break;
            }
            InventoryHelper.equipBestWeapon(player);
            player.lookAt(EntityAnchorArgument.Anchor.EYES, target.position().add(0.0, 1.0, 0.0));
            if (dist > 4.0) {
                float yaw = (float)Math.toDegrees(Math.atan2(-(target.getX() - player.getX()), target.getZ() - player.getZ()));
                player.setYRot(yaw);
                double nx = (target.getX() - player.getX()) / dist;
                double nz = (target.getZ() - player.getZ()) / dist;
                player.setDeltaMovement(nx * 0.28, player.getDeltaMovement().y, nz * 0.28);
            } else {
                player.attack((Entity)target);
                player.containerMenu.broadcastChanges();
                if (mode.equals("kite")) {
                    float yaw = (float)Math.toDegrees(Math.atan2(-(player.getX() - target.getX()), player.getZ() - target.getZ()));
                    player.setYRot(yaw);
                    player.setDeltaMovement(-Math.cos(Math.toRadians(yaw)) * 0.28, 0.0, Math.sin(Math.toRadians(yaw)) * 0.28);
                }
            }
            if (!target.isAlive()) {
                cr.result = "killed";
                break;
            }
            try {
                Thread.sleep(200L);
            }
            catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                break;
            }
        }
        player.setDeltaMovement(0.0, player.getDeltaMovement().y, 0.0);
        if (cr.result.equals("none")) {
            cr.result = "timeout";
        }
        return cr;
    }

    public static class CombatResult {
        String result = "none";
        String targetType = "";

        private CombatResult() {
        }
    }
}
