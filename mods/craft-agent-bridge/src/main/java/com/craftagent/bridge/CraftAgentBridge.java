/*
 * Decompiled with CFR 0.152.
 * 
 * Could not load the following classes:
 *  com.google.gson.Gson
 *  com.google.gson.JsonArray
 *  com.google.gson.JsonElement
 *  com.google.gson.JsonObject
 *  com.mojang.authlib.GameProfile
 *  net.fabricmc.api.ModInitializer
 *  net.fabricmc.fabric.api.event.lifecycle.v1.ServerLifecycleEvents
 *  net.fabricmc.fabric.api.event.lifecycle.v1.ServerTickEvents
 *  net.minecraft.commands.arguments.EntityAnchorArgument$Anchor
 *  net.minecraft.core.BlockPos
 *  net.minecraft.core.Direction
 *  net.minecraft.core.Holder
 *  net.minecraft.core.Holder$Reference
 *  net.minecraft.core.Registry
 *  net.minecraft.core.UUIDUtil
 *  net.minecraft.core.Vec3i
 *  net.minecraft.core.registries.BuiltInRegistries
 *  net.minecraft.core.registries.Registries
 *  net.minecraft.network.Connection
 *  net.minecraft.network.protocol.Packet
 *  net.minecraft.network.protocol.PacketFlow
 *  net.minecraft.network.protocol.game.ClientboundEntityPositionSyncPacket
 *  net.minecraft.network.protocol.game.ClientboundPlayerInfoUpdatePacket
 *  net.minecraft.network.protocol.game.ClientboundPlayerInfoUpdatePacket$Action
 *  net.minecraft.network.protocol.game.ClientboundRotateHeadPacket
 *  net.minecraft.resources.Identifier
 *  net.minecraft.resources.ResourceKey
 *  net.minecraft.server.MinecraftServer
 *  net.minecraft.server.level.ClientInformation
 *  net.minecraft.server.level.ServerLevel
 *  net.minecraft.server.level.ServerPlayer
 *  net.minecraft.server.network.CommonListenerCookie
 *  net.minecraft.server.players.NameAndId
 *  net.minecraft.tags.FluidTags
 *  net.minecraft.util.RandomSource
 *  net.minecraft.world.InteractionHand
 *  net.minecraft.world.InteractionResult
 *  net.minecraft.world.effect.MobEffect
 *  net.minecraft.world.effect.MobEffectInstance
 *  net.minecraft.world.effect.MobEffects
 *  net.minecraft.world.entity.Entity
 *  net.minecraft.world.entity.EntitySpawnReason
 *  net.minecraft.world.entity.EntitySpawnRequest
 *  net.minecraft.world.entity.EntityType
 *  net.minecraft.world.entity.EquipmentSlot
 *  net.minecraft.world.entity.LivingEntity
 *  net.minecraft.world.entity.Mob
 *  net.minecraft.world.entity.ai.attributes.Attributes
 *  net.minecraft.world.entity.animal.equine.AbstractHorse
 *  net.minecraft.world.entity.animal.pig.Pig
 *  net.minecraft.world.entity.item.ItemEntity
 *  net.minecraft.world.entity.monster.Monster
 *  net.minecraft.world.entity.npc.villager.Villager
 *  net.minecraft.world.entity.player.Inventory
 *  net.minecraft.world.entity.player.Player
 *  net.minecraft.world.entity.vehicle.boat.Boat
 *  net.minecraft.world.entity.vehicle.minecart.AbstractMinecart
 *  net.minecraft.world.inventory.AbstractContainerMenu
 *  net.minecraft.world.inventory.ContainerInput
 *  net.minecraft.world.inventory.CraftingContainer
 *  net.minecraft.world.inventory.Slot
 *  net.minecraft.world.item.Item
 *  net.minecraft.world.item.ItemStack
 *  net.minecraft.world.item.Items
 *  net.minecraft.world.item.enchantment.Enchantment
 *  net.minecraft.world.item.enchantment.EnchantmentHelper
 *  net.minecraft.world.item.trading.ItemCost
 *  net.minecraft.world.item.trading.Merchant
 *  net.minecraft.world.item.trading.MerchantOffer
 *  net.minecraft.world.item.trading.MerchantOffers
 *  net.minecraft.world.level.GameType
 *  net.minecraft.world.level.ItemLike
 *  net.minecraft.world.level.Level
 *  net.minecraft.world.level.LightLayer
 *  net.minecraft.world.level.ServerLevelAccessor
 *  net.minecraft.world.level.block.BedBlock
 *  net.minecraft.world.level.block.Block
 *  net.minecraft.world.level.block.Blocks
 *  net.minecraft.world.level.block.state.BlockState
 *  net.minecraft.world.level.block.state.properties.BedPart
 *  net.minecraft.world.level.block.state.properties.Property
 *  net.minecraft.world.phys.AABB
 *  net.minecraft.world.phys.BlockHitResult
 *  net.minecraft.world.phys.HitResult
 *  net.minecraft.world.phys.HitResult$Type
 *  net.minecraft.world.phys.Vec3
 */
package com.craftagent.bridge;

import com.craftagent.bridge.AStar;
import com.craftagent.bridge.FakeClientConnection;
import com.google.gson.Gson;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.mojang.authlib.GameProfile;
import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.io.PrintWriter;
import java.io.Writer;
import java.net.HttpURLConnection;
import java.net.InetAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.net.URL;
import java.net.URLEncoder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.HashMap;
import java.util.Map;
import java.util.Optional;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.TimeUnit;
import java.util.function.Supplier;
import java.util.stream.Stream;
import net.fabricmc.api.ModInitializer;
import net.fabricmc.fabric.api.event.lifecycle.v1.ServerLifecycleEvents;
import net.fabricmc.fabric.api.event.lifecycle.v1.ServerTickEvents;
import net.minecraft.commands.arguments.EntityAnchorArgument;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.Holder;
import net.minecraft.core.Registry;
import net.minecraft.core.UUIDUtil;
import net.minecraft.core.Vec3i;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.Connection;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.PacketFlow;
import net.minecraft.network.protocol.game.ClientboundEntityPositionSyncPacket;
import net.minecraft.network.protocol.game.ClientboundPlayerInfoUpdatePacket;
import net.minecraft.network.protocol.game.ClientboundRotateHeadPacket;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ClientInformation;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.network.CommonListenerCookie;
import net.minecraft.server.players.NameAndId;
import net.minecraft.tags.FluidTags;
import net.minecraft.util.RandomSource;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.effect.MobEffect;
import net.minecraft.world.effect.MobEffectInstance;
import net.minecraft.world.effect.MobEffects;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntitySpawnReason;
import net.minecraft.world.entity.EntitySpawnRequest;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.Mob;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.entity.animal.equine.AbstractHorse;
import net.minecraft.world.entity.animal.pig.Pig;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.entity.monster.Monster;
import net.minecraft.world.entity.npc.villager.Villager;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.entity.vehicle.boat.Boat;
import net.minecraft.world.entity.vehicle.minecart.AbstractMinecart;
import net.minecraft.world.inventory.AbstractContainerMenu;
import net.minecraft.world.inventory.ContainerInput;
import net.minecraft.world.inventory.CraftingContainer;
import net.minecraft.world.inventory.Slot;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.item.enchantment.Enchantment;
import net.minecraft.world.item.enchantment.EnchantmentHelper;
import net.minecraft.world.item.trading.ItemCost;
import net.minecraft.world.item.trading.Merchant;
import net.minecraft.world.item.trading.MerchantOffer;
import net.minecraft.world.item.trading.MerchantOffers;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.ItemLike;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.LightLayer;
import net.minecraft.world.level.ServerLevelAccessor;
import net.minecraft.world.level.block.BedBlock;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.BedPart;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.HitResult;
import net.minecraft.world.phys.Vec3;

public class CraftAgentBridge
implements ModInitializer {
    public static final int PORT = 25567;
    private static final int SCAN_RADIUS = 16;
    private static final Gson GSON;
    private static volatile MinecraftServer serverInstance;
    private static volatile EntityPlayerMPFake fakePlayer;
    private static volatile double[] moveTarget;
    private static volatile int moveTicksLeft;
    private static volatile boolean moveReached;
    private static volatile double moveFinalDist;
    private static volatile boolean moveStuck;
    private static volatile int moveStuckCounter;
    private static volatile List<Vec3> moveWaypoints;
    private static volatile int moveCurrentWpIndex;
    private static volatile double[] lastPos;
    private static volatile int noProgressTicks;
    private static volatile boolean moveSprinting;
    private static volatile boolean shouldStop;
    private static volatile String currentGoal;
    private static volatile boolean fakePlayerSpawning;
    private static final Set<String> BLOCK_WHITELIST;
    private static int autoSurviveCooldown;
    private static int autoSurviveAttackCd;

    // ═════════════════════════════════════════════════════════════════════════
    // 命令分派表（重构 performAction 的巨型 switch 用）：
    // 每个命令 → 一个处理器方法，注册进 COMMAND_HANDLERS，performAction 优先查表。
    // 表中没有的命令才回退到 legacy 的 switch（逐步迁移、随时可回退）。
    // ═════════════════════════════════════════════════════════════════════════
    @FunctionalInterface
    private interface CommandHandler {
        JsonObject handle(ServerPlayer player, ServerLevel level, JsonObject req);
    }

    private static final Map<String, CommandHandler> COMMAND_HANDLERS = new HashMap<>();

    private void registerCommandHandlers() {
        COMMAND_HANDLERS.put("look", this::actLook);
        COMMAND_HANDLERS.put("look_abs", this::actLookAbs);
        COMMAND_HANDLERS.put("look_at", this::actLookAt);
        COMMAND_HANDLERS.put("dig_at", this::actDigAt);
        COMMAND_HANDLERS.put("place_at", this::actPlaceAt);
        COMMAND_HANDLERS.put("get_block", this::actGetBlock);
        COMMAND_HANDLERS.put("get_blocks", this::actGetBlocks);
        COMMAND_HANDLERS.put("clear_chat", this::actClearChat);
        COMMAND_HANDLERS.put("debug_spawn", this::actDebugSpawn);
        COMMAND_HANDLERS.put("debug_give", this::actDebugGive);
        COMMAND_HANDLERS.put("debug_damage", this::actDebugDamage);
        COMMAND_HANDLERS.put("debug_heal", this::actDebugHeal);
        COMMAND_HANDLERS.put("debug_clear", this::actDebugClear);
        COMMAND_HANDLERS.put("debug_place", this::actDebugPlace);
        COMMAND_HANDLERS.put("debug_xp", this::actDebugXp);
        COMMAND_HANDLERS.put("debug_food", this::actDebugFood);
        COMMAND_HANDLERS.put("debug_time", this::actDebugTime);
        COMMAND_HANDLERS.put("debug_teleport_player", this::actDebugTeleportPlayer);
        COMMAND_HANDLERS.put("debug_teleport_bot", this::actDebugTeleportBot);
    }

    public void onInitialize() {
        registerCommandHandlers();
        Thread serverThread = new Thread(this::runServer, "craft-agent-bridge");
        serverThread.setDaemon(true);
        serverThread.start();
        System.out.println("[craft-agent-bridge] \u670d\u52a1\u7aef TCP \u7ebf\u7a0b\u5df2\u542f\u52a8\uff0c\u76d1\u542c 127.0.0.1:25567");
        ServerLifecycleEvents.SERVER_STARTED.register(server -> {
            serverInstance = server;
            System.out.println("[craft-agent-bridge] MinecraftServer \u5df2\u7ed1\u5b9a\uff08ServerPlayer \u67b6\u6784\u5c31\u7eea\uff09");
            server.executeIfPossible(() -> {
                try {
                    Thread.sleep(100L);
                }
                catch (InterruptedException e) {
                    Thread.currentThread().interrupt();
                }
                CraftAgentBridge.createFakePlayer();
            });
        });
        ServerLifecycleEvents.SERVER_STOPPING.register(server -> {
            serverInstance = null;
        });
        ServerTickEvents.START_SERVER_TICK.register(this::onStartServerTick);
        ServerTickEvents.END_SERVER_TICK.register(this::onEndServerTick);
        System.out.println("[craft-agent-bridge] ServerTickEvents.START+END_SERVER_TICK \u5df2\u6ce8\u518c\uff08\u53cc tick + Carpet \u540c\u6b65\uff09");
    }

    private static boolean isInWater(ServerPlayer player) {
        return player.isInWater() || player.isEyeInFluid(FluidTags.WATER);
    }

    private void onStartServerTick(MinecraftServer server) {
        double ddz;
        if (moveWaypoints == null) {
            return;
        }
        ServerPlayer player = CraftAgentBridge.getFirstPlayer(server);
        if (player == null) {
            moveWaypoints = null;
            return;
        }
        if (moveCurrentWpIndex >= moveWaypoints.size()) {
            return;
        }
        Vec3 wp = moveWaypoints.get(moveCurrentWpIndex);
        double tx = wp.x;
        double tz = wp.z;
        double ddx = tx - player.getX();
        double horiz = Math.sqrt(ddx * ddx + (ddz = tz - player.getZ()) * ddz);
        if (horiz < 0.001) {
            return;
        }
        float yaw = (float)Math.toDegrees(Math.atan2(-ddx, ddz));
        player.setYRot(yaw);
        player.yHeadRot = yaw;
        boolean inWater = CraftAgentBridge.isInWater(player);
        if (inWater) {
            double ny = wp.y - player.getY();
            double vy = ny > 0.3 ? 0.35 : (ny < -0.3 ? -0.35 : 0.0);
            double nx = ddx / horiz;
            double nz = ddz / horiz;
            player.setDeltaMovement(nx * 0.25, vy, nz * 0.25);
            return;
        }
        if (player.onGround() && wp.y - player.getY() > 0.05) {
            player.setDeltaMovement(player.getDeltaMovement().x, 0.42, player.getDeltaMovement().z);
        }
        moveSprinting = !player.horizontalCollision && horiz > 2.0 && noProgressTicks < 5;
        player.setSprinting(moveSprinting);
        player.zza = moveSprinting ? 1.3f : 1.0f;
        player.xxa = 0.0f;
        double speed = 0.3;
        double nx = ddx / horiz;
        double nz = ddz / horiz;
        player.setDeltaMovement(nx * speed, player.getDeltaMovement().y, nz * speed);
    }

    private void onEndServerTick(MinecraftServer server) {
        ServerPlayer survPlayer = CraftAgentBridge.getFirstPlayer(server);
        if (survPlayer != null) {
            CraftAgentBridge.autoSurvive(survPlayer, server);
        }
        if (moveWaypoints == null) {
            return;
        }
        ServerPlayer player = CraftAgentBridge.getFirstPlayer(server);
        if (player == null) {
            moveWaypoints = null;
            return;
        }
        double pxBefore = player.getX();
        double pzBefore = player.getZ();
        if (moveCurrentWpIndex >= moveWaypoints.size()) {
            moveReached = true;
            moveFinalDist = 0.0;
            moveWaypoints = null;
            moveTarget = null;
            player.zza = 0.0f;
            player.xxa = 0.0f;
            player.setSprinting(false);
            player.setDeltaMovement(0.0, player.getDeltaMovement().y, 0.0);
            System.out.println("[cab-move] DONE all waypoints reached");
            return;
        }
        Vec3 wp = moveWaypoints.get(moveCurrentWpIndex);
        double tx = wp.x;
        double ty = wp.y;
        double tz = wp.z;
        double ddx = tx - player.getX();
        double ddy = ty - (player.getY() + (double)player.getEyeHeight());
        double ddz = tz - player.getZ();
        double horiz = Math.sqrt(ddx * ddx + ddz * ddz);
        float yaw = (float)Math.toDegrees(Math.atan2(-ddx, ddz));
        float pitch = (float)Math.toDegrees(-Math.atan2(ddy, horiz));
        player.setYRot(yaw);
        player.setXRot(CraftAgentBridge.clamp(pitch, -90.0f, 90.0f));
        player.yHeadRot = yaw;
        boolean inWater = CraftAgentBridge.isInWater(player);
        if (inWater) {
            player.zza = 1.0f;
            player.xxa = 0.0f;
            double dy = wp.y - player.getY();
            double vy = dy > 0.2 ? 0.35 : (dy < -0.2 ? -0.35 : 0.0);
            player.setDeltaMovement(player.getDeltaMovement().x, vy, player.getDeltaMovement().z);
        } else {
            player.zza = moveSprinting ? 1.3f : 1.0f;
            player.xxa = 0.0f;
        }
        if (moveTarget != null) {
            moveFinalDist = Math.sqrt((moveTarget[0] - player.getX()) * (moveTarget[0] - player.getX()) + (moveTarget[1] - player.getY()) * (moveTarget[1] - player.getY()) + (moveTarget[2] - player.getZ()) * (moveTarget[2] - player.getZ()));
        }
        --moveTicksLeft;
        try {
            player.connection.resetPosition();
            player.level().getChunkSource().move(player);
        }
        catch (Exception dy) {
            // empty catch block
        }
        if (fakePlayer != null && serverInstance != null && serverInstance.getTickCount() % 20 == 0) {
            try {
                ResourceKey dim = fakePlayer.level().dimension();
                serverInstance.getPlayerList().broadcastAll((Packet)new ClientboundPlayerInfoUpdatePacket(ClientboundPlayerInfoUpdatePacket.Action.ADD_PLAYER, (ServerPlayer)fakePlayer), dim);
                serverInstance.getPlayerList().broadcastAll((Packet)ClientboundEntityPositionSyncPacket.of((Entity)fakePlayer), dim);
                serverInstance.getPlayerList().broadcastAll((Packet)new ClientboundRotateHeadPacket((Entity)fakePlayer, (byte)(CraftAgentBridge.fakePlayer.yHeadRot * 256.0f / 360.0f)), dim);
            }
            catch (Exception dim) {
                // empty catch block
            }
        }
        double moved = Math.sqrt(Math.pow(player.getX() - pxBefore, 2.0) + Math.pow(player.getZ() - pzBefore, 2.0));
        double movedSinceLastCheck = 0.0;
        if (lastPos != null) {
            movedSinceLastCheck = Math.sqrt(Math.pow(player.getX() - lastPos[0], 2.0) + Math.pow(player.getZ() - lastPos[1], 2.0));
        }
        lastPos = new double[]{player.getX(), player.getZ()};
        noProgressTicks = movedSinceLastCheck < 0.01 && moveTicksLeft > 0 ? ++noProgressTicks : 0;
        if (horiz < 1.2 || moveTicksLeft <= 0) {
            if (horiz < 1.2) {
                ++moveCurrentWpIndex;
            }
            if (moveCurrentWpIndex >= moveWaypoints.size()) {
                moveReached = moveFinalDist < 2.0;
                moveWaypoints = null;
                moveTarget = null;
                player.zza = 0.0f;
                player.xxa = 0.0f;
                player.setSprinting(false);
                player.setDeltaMovement(0.0, player.getDeltaMovement().y, 0.0);
                System.out.println("[cab-move] DONE reached=" + moveReached + " dist=" + String.format("%.2f", moveFinalDist));
                return;
            }
        }
        if (player.horizontalCollision && player.onGround()) {
            player.setDeltaMovement(player.getDeltaMovement().x, 0.42, player.getDeltaMovement().z);
            if (++moveStuckCounter >= 20) {
                moveStuck = true;
            }
        } else {
            moveStuckCounter = 0;
            moveStuck = false;
        }
        if (noProgressTicks >= 15 && player.onGround()) {
            BlockPos below = player.blockPosition().below();
            ServerLevel level = player.level();
            BlockState bs = level.getBlockState(below);
            if (bs.isAir() || bs.canBeReplaced()) {
                for (int slot = 0; slot < 9; ++slot) {
                    String itemId;
                    ItemStack stack = player.getInventory().getItem(slot);
                    if (stack.isEmpty() || !(itemId = BuiltInRegistries.ITEM.getKey(stack.getItem()).toString()).contains("dirt") && !itemId.contains("cobblestone") && !itemId.contains("stone") && !itemId.contains("planks") && !itemId.contains("log") && !itemId.contains("sand") && !itemId.contains("gravel") && !itemId.contains("deepslate")) continue;
                    player.getInventory().setSelectedSlot(slot);
                    CraftAgentBridge.placeAt(player, level, below.getX(), below.getY(), below.getZ(), itemId);
                    player.setDeltaMovement(player.getDeltaMovement().x, 0.42, player.getDeltaMovement().z);
                    noProgressTicks = 0;
                    System.out.println("[cab-move] AUTO PILLAR at " + below.getX() + "," + below.getY() + "," + below.getZ());
                    break;
                }
            }
        }
        if (moveTicksLeft % 20 == 0) {
            System.out.println("[cab-move] tick ticksLeft=" + moveTicksLeft + " wp=" + moveCurrentWpIndex + "/" + moveWaypoints.size() + " pos=(" + String.format("%.2f", player.getX()) + "," + String.format("%.2f", player.getZ()) + ") dist=" + String.format("%.2f", moveFinalDist) + " moved=" + String.format("%.3f", moved) + " noProg=" + noProgressTicks + " stuck=" + moveStuck + " sprint=" + moveSprinting);
        }
    }

    private static void autoSurvive(ServerPlayer player, MinecraftServer server) {
        double dz;
        double dx;
        double len;
        if (--autoSurviveCooldown > 0) {
            if (autoSurviveAttackCd > 0) {
                --autoSurviveAttackCd;
            }
            return;
        }
        autoSurviveCooldown = 20;
        if (autoSurviveAttackCd > 0) {
            --autoSurviveAttackCd;
        }
        ServerLevel level = player.level();
        float hp = player.getHealth();
        LivingEntity threat = null;
        double minDist = Double.MAX_VALUE;
        AABB area = AABB.ofSize((Vec3)player.position(), (double)16.0, (double)16.0, (double)16.0);
        for (Entity e : level.getEntities((Entity)player, area)) {
            double d;
            if (!(e instanceof LivingEntity)) continue;
            LivingEntity le = (LivingEntity)e;
            String tn = BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).getPath();
            if (!CraftAgentBridge.isHostile(tn) || !((d = (double)e.distanceTo((Entity)player)) < minDist)) continue;
            minDist = d;
            threat = le;
        }
        float hunger = player.getFoodData().getFoodLevel();
        if (hunger < 10.0f) {
            for (int slot = 0; slot < player.getInventory().getContainerSize(); ++slot) {
                int useSlot;
                String id;
                ItemStack s = player.getInventory().getItem(slot);
                if (s.isEmpty() || !(id = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase()).contains("apple") && !id.contains("bread") && !id.contains("cooked") && !id.contains("beef") && !id.contains("pork") && !id.contains("chicken") && !id.contains("fish") && !id.contains("potato") && !id.contains("carrot") && !id.contains("mutton") && !id.contains("rabbit") && !id.contains("melon") && !id.contains("berry") && !id.contains("stew") && !id.contains("cookie") && !id.contains("golden") && !id.contains("honey") && !id.contains("pumpkin")) continue;
                int n = useSlot = slot < 9 ? slot : 0;
                if (slot >= 9) {
                    ItemStack tmp = player.getInventory().getItem(0);
                    player.getInventory().setItem(0, player.getInventory().getItem(slot));
                    player.getInventory().setItem(slot, tmp);
                }
                player.getInventory().setSelectedSlot(useSlot);
                player.startUsingItem(InteractionHand.MAIN_HAND);
                player.containerMenu.broadcastChanges();
                System.out.println("[cab-survive] AUTO EAT " + id + " (hunger=" + hunger + ")");
                return;
            }
        }
        if (hp <= 6.0f && threat != null && (len = Math.sqrt((dx = player.getX() - threat.getX()) * dx + (dz = player.getZ() - threat.getZ()) * dz)) > 0.01) {
            double fleeX = player.getX() + dx / len * 8.0;
            double fleeZ = player.getZ() + dz / len * 8.0;
            if (moveWaypoints == null) {
                moveReached = false;
                moveStuck = false;
                moveStuckCounter = 0;
                moveTicksLeft = 100;
                moveTarget = new double[]{fleeX, player.getY(), fleeZ};
                moveWaypoints = new ArrayList<Vec3>();
                moveWaypoints.add(Vec3.atCenterOf((Vec3i)BlockPos.containing((double)fleeX, (double)player.getY(), (double)fleeZ)));
                moveCurrentWpIndex = 0;
                System.out.println("[cab-survive] AUTO FLEE from threat at dist=" + String.format("%.1f", minDist));
            }
        }
        if (threat != null && hp > 6.0f) {
            CraftAgentBridge.equipBestWeapon(player);
            double dist = threat.distanceTo((Entity)player);
            player.lookAt(EntityAnchorArgument.Anchor.EYES, threat.position().add(0.0, 1.0, 0.0));
            if (dist <= 4.0 && autoSurviveAttackCd <= 0) {
                player.attack((Entity)threat);
                player.containerMenu.broadcastChanges();
                autoSurviveAttackCd = 10;
                System.out.println("[cab-survive] AUTO ATTACK " + BuiltInRegistries.ENTITY_TYPE.getKey(threat.getType()).getPath() + " dist=" + String.format("%.1f", dist) + " hp=" + String.format("%.1f", Float.valueOf(hp)));
            }
        }
    }

    private static ServerPlayer getFirstPlayer(MinecraftServer server) {
        if (fakePlayer != null && (fakePlayer.isDeadOrDying() || !fakePlayer.isAlive())) {
            System.out.println("[craft-agent-bridge] fakePlayer dead, reviving...");
            CraftAgentBridge.removeFakePlayer();
            CraftAgentBridge.createFakePlayer();
            if (fakePlayer != null) {
                fakePlayer.setHealth(20.0f);
                ServerPlayer real = null;
                for (ServerPlayer p : server.getPlayerList().getPlayers()) {
                    if (p == fakePlayer) continue;
                    real = p;
                    break;
                }
                if (real != null) {
                    int gy = (int)real.getY();
                    fakePlayer.teleportTo(real.level(), real.getX(), gy + 1, real.getZ() + 1.0, Set.of(), 0.0f, 0.0f, true);
                }
            }
        }
        if (fakePlayer != null) {
            return fakePlayer;
        }
        List players = server.getPlayerList().getPlayers();
        return players.isEmpty() ? null : (ServerPlayer)players.get(0);
    }

    private static EntityPlayerMPFake getFakePlayer() {
        return fakePlayer;
    }

    /*
     * WARNING - Removed try catching itself - possible behaviour change.
     */
    private static boolean createFakePlayer() {
        MinecraftServer server = serverInstance;
        if (server == null) {
            return false;
        }
        if (fakePlayer != null) {
            return true;
        }
        if (fakePlayerSpawning) {
            return false;
        }
        fakePlayerSpawning = true;
        try {
            ServerLevel level = server.getLevel(Level.OVERWORLD);
            if (level == null) {
                level = ((ServerPlayer)server.getPlayerList().getPlayers().get(0)).level();
            }
            String username = "CraftAgent";
            GameProfile profile = UUIDUtil.createOfflineProfile((String)username);
            ClientInformation clientInfo = ClientInformation.createDefault();
            EntityPlayerMPFake fake = new EntityPlayerMPFake(server, level, profile, clientInfo);
            server.getPlayerList().placeNewPlayer((Connection)new FakeClientConnection(PacketFlow.SERVERBOUND), (ServerPlayer)fake, new CommonListenerCookie(profile, 0, clientInfo, false));
            server.getPlayerList().op(new NameAndId(profile));
            fake.teleportTo(level, 0.5, 64.0, 0.5, Set.of(), 0.0f, 0.0f, true);
            fake.setHealth(20.0f);
            fake.unsetRemovedPublic();
            fake.getAttribute(Attributes.STEP_HEIGHT).setBaseValue((double)0.6f);
            fake.gameMode.changeGameModeForPlayer(GameType.SURVIVAL);
            server.getPlayerList().broadcastAll((Packet)new ClientboundRotateHeadPacket((Entity)fake, (byte)(fake.yHeadRot * 256.0f / 360.0f)), level.dimension());
            server.getPlayerList().broadcastAll((Packet)ClientboundEntityPositionSyncPacket.of((Entity)fake), level.dimension());
            server.getPlayerList().broadcastAll((Packet)new ClientboundPlayerInfoUpdatePacket(ClientboundPlayerInfoUpdatePacket.Action.ADD_PLAYER, (ServerPlayer)fake));
            fakePlayer = fake;
            System.out.println("[craft-agent-bridge] Fake player created: " + username + " at (0.5, 64.0, 0.5)");
            boolean bl = true;
            return bl;
        }
        catch (Exception e) {
            System.err.println("[craft-agent-bridge] Failed to create fake player: " + e.getMessage());
            e.printStackTrace();
            fakePlayer = null;
            boolean bl = false;
            return bl;
        }
        finally {
            fakePlayerSpawning = false;
        }
    }

    private static void removeFakePlayer() {
        if (fakePlayer == null) {
            return;
        }
        try {
            fakePlayer.kill(fakePlayer.level());
        }
        catch (Exception e) {
            System.err.println("[craft-agent-bridge] Error removing fake player: " + e.getMessage());
        }
        fakePlayer = null;
    }

    private static JsonObject runOnServerThread(Supplier<JsonObject> task) {
        MinecraftServer server = serverInstance;
        if (server == null) {
            JsonObject err = new JsonObject();
            err.addProperty("status", "fail");
            err.addProperty("detail", "\u670d\u52a1\u5668\u672a\u5c31\u7eea");
            return err;
        }
        CompletableFuture future = new CompletableFuture();
        server.executeIfPossible(() -> {
            try {
                future.complete((JsonObject)task.get());
            }
            catch (Exception e) {
                JsonObject err = new JsonObject();
                err.addProperty("status", "fail");
                err.addProperty("detail", e.getMessage());
                future.complete(err);
            }
        });
        try {
            return (JsonObject)future.get(30L, TimeUnit.SECONDS);
        }
        catch (Exception e) {
            JsonObject err = new JsonObject();
            err.addProperty("status", "fail");
            err.addProperty("detail", "\u670d\u52a1\u7aef\u7ebf\u7a0b\u8d85\u65f6: " + e.getMessage());
            return err;
        }
    }

    private static <T> T onServer(Supplier<T> task) {
        MinecraftServer server = serverInstance;
        if (server == null) {
            return null;
        }
        CompletableFuture<T> future = new CompletableFuture<>();
        server.executeIfPossible(() -> {
            try {
                future.complete(task.get());
            }
            catch (Exception e) {
                future.complete(null);
            }
        });
        try {
            return future.get(30L, TimeUnit.SECONDS);
        }
        catch (Exception e) {
            return null;
        }
    }

    private void runServer() {
        try (ServerSocket server = new ServerSocket(25567, 0, InetAddress.getByName("127.0.0.1"));){
            while (!Thread.interrupted()) {
                Socket sock = server.accept();
                Thread clientThread = new Thread(() -> this.handleClient(sock), "cab-client");
                clientThread.setDaemon(true);
                clientThread.start();
            }
        }
        catch (Exception e) {
            System.err.println("[craft-agent-bridge] \u670d\u52a1\u5f02\u5e38: " + String.valueOf(e));
        }
    }

    private void handleClient(Socket sock) {
        try (BufferedReader in = new BufferedReader(new InputStreamReader(sock.getInputStream(), StandardCharsets.UTF_8));
             PrintWriter out = new PrintWriter((Writer)new OutputStreamWriter(sock.getOutputStream(), StandardCharsets.UTF_8), true);){
            String line;
            while ((line = in.readLine()) != null) {
                JsonObject resp;
                try {
                    JsonObject req = (JsonObject)GSON.fromJson(line, JsonObject.class);
                    resp = this.dispatch(req);
                }
                catch (Exception e) {
                    resp = new JsonObject();
                    resp.addProperty("status", "fail");
                    resp.addProperty("detail", "\u89e3\u6790/\u6267\u884c\u5931\u8d25: " + e.getMessage());
                }
                out.println(GSON.toJson((JsonElement)resp));
                out.flush();
            }
        }
        catch (Exception e) {
            System.err.println("[craft-agent-bridge] \u5ba2\u6237\u7aef\u8fde\u63a5\u5f02\u5e38: " + String.valueOf(e));
        }
    }

    private JsonObject dispatch(JsonObject req) {
        String type;
        String string = type = req.has("type") ? req.get("type").getAsString() : "";
        if ("state".equals(type)) {
            return CraftAgentBridge.runOnServerThread(this::buildState);
        }
        if ("move_to".equals(type)) {
            return this.performMoveTo(req);
        }
        if ("go_to_player".equals(type)) {
            return this.performGoToPlayer(req);
        }
        if ("give_player".equals(type)) {
            return this.performGivePlayer(req);
        }
        if ("discard_smart".equals(type)) {
            return this.performDiscardSmart(req);
        }
        if ("collect_items".equals(type)) {
            return this.performCollectItems(req);
        }
        if ("attack_player".equals(type)) {
            return this.performAttackPlayer(req);
        }
        if ("follow_player".equals(type)) {
            return this.performFollowPlayer(req);
        }
        if ("combat".equals(type)) {
            return this.performCombat(req);
        }
        if ("use_item".equals(type)) {
            return this.performUseItem(req);
        }
        if ("eat_item".equals(type)) {
            return this.performEatItem(req);
        }
        if ("pillar_up".equals(type)) {
            return this.performPillarUp(req);
        }
        if ("wait".equals(type)) {
            return this.performWait(req);
        }
        return CraftAgentBridge.runOnServerThread(() -> {
            try {
                return this.performAction(type, req);
            }
            catch (Exception e) {
                JsonObject o = new JsonObject();
                o.addProperty("status", "fail");
                o.addProperty("detail", e.getMessage());
                return o;
            }
        });
    }

    private JsonObject performMoveTo(JsonObject req) {
        JsonObject o = new JsonObject();
        if (serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u670d\u52a1\u5668\u672a\u5c31\u7eea");
            return o;
        }
        double tx = req.get("x").getAsDouble();
        double ty = req.get("y").getAsDouble();
        double tz = req.get("z").getAsDouble();
        int maxTicks = req.has("max_ticks") ? req.get("max_ticks").getAsInt() : 200;
        moveReached = false;
        moveFinalDist = 999.0;
        moveStuck = false;
        moveTicksLeft = maxTicks;
        moveTarget = new double[]{tx, ty, tz};
        moveStuckCounter = 0;
        ServerLevel level = CraftAgentBridge.onServer(() -> {
            ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
            return p != null ? p.level() : null;
        });
        if (level == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u65e0\u6cd5\u83b7\u53d6\u4e16\u754c");
            return o;
        }
        BlockPos targetPos = BlockPos.containing((double)tx, (double)(ty + 1.0), (double)tz);
        BlockPos fromPos = CraftAgentBridge.onServer(() -> {
            ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
            return p != null ? p.blockPosition() : targetPos;
        });
        moveWaypoints = AStar.findPath(level, Vec3.atCenterOf((Vec3i)fromPos), Vec3.atCenterOf((Vec3i)targetPos));
        moveCurrentWpIndex = 0;
        if (moveWaypoints == null) {
            o.addProperty("status", "ok");
            o.addProperty("reached", Boolean.valueOf(false));
            o.addProperty("stuck", Boolean.valueOf(true));
            o.addProperty("detail", "no_path");
            moveTarget = null;
            return o;
        }
        if (moveWaypoints.isEmpty()) {
            o.addProperty("status", "ok");
            o.addProperty("reached", Boolean.valueOf(true));
            o.addProperty("stuck", Boolean.valueOf(false));
            o.addProperty("detail", "already_at_target");
            moveTarget = null;
            return o;
        }
        boolean hasWater = false;
        boolean hasFall = false;
        double prevY = fromPos.getY();
        for (Vec3 wp : moveWaypoints) {
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
        for (int waitMs = 0; waitMs < hardLimit && moveWaypoints != null; waitMs += 50) {
            try {
                Thread.sleep(50L);
                continue;
            }
            catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                break;
            }
        }
        if (moveWaypoints != null) {
            moveWaypoints = null;
            moveTarget = null;
            moveReached = false;
        }
        o.addProperty("status", "ok");
        o.addProperty("reached", Boolean.valueOf(moveReached));
        o.addProperty("final_dist", (Number)moveFinalDist);
        o.addProperty("stuck", Boolean.valueOf(moveStuck));
        o.addProperty("detail", "move_to " + tx + "," + ty + "," + tz + " (reached=" + moveReached + ", dist=" + String.format("%.1f", moveFinalDist) + "m)" + detailSuffix);
        return o;
    }

    private JsonObject performGoToPlayer(JsonObject req) {
        JsonObject o = new JsonObject();
        if (serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u670d\u52a1\u5668\u672a\u5c31\u7eea");
            return o;
        }
        String targetName = req.has("player_name") ? req.get("player_name").getAsString() : "";
        String targetId = CraftAgentBridge.onServer(() -> {
            for (ServerPlayer p : serverInstance.getPlayerList().getPlayers()) {
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
            for (ServerPlayer p : serverInstance.getPlayerList().getPlayers()) {
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
        moveReached = false;
        moveFinalDist = 999.0;
        moveStuck = false;
        moveTicksLeft = 400;
        moveTarget = targetPos;
        moveStuckCounter = 0;
        ServerLevel level = CraftAgentBridge.onServer(() -> {
            ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
            return p != null ? p.level() : null;
        });
        if (level == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u65e0\u6cd5\u83b7\u53d6\u4e16\u754c");
            return o;
        }
        BlockPos targetBlockPos = BlockPos.containing((double)targetPos[0], (double)targetPos[1], (double)targetPos[2]);
        BlockPos fromPos = CraftAgentBridge.onServer(() -> {
            ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
            return p != null ? p.blockPosition() : targetBlockPos;
        });
        moveWaypoints = AStar.findPath(level, Vec3.atCenterOf((Vec3i)fromPos), Vec3.atCenterOf((Vec3i)targetBlockPos));
        moveCurrentWpIndex = 0;
        if (moveWaypoints == null) {
            o.addProperty("status", "ok");
            o.addProperty("reached", Boolean.valueOf(false));
            o.addProperty("stuck", Boolean.valueOf(true));
            o.addProperty("detail", "go_to_player " + targetName + ": no_path");
            moveTarget = null;
            return o;
        }
        if (moveWaypoints.isEmpty()) {
            double dz;
            double dx = targetPos[0] - (double)fromPos.getX();
            double dist = Math.sqrt(dx * dx + (dz = targetPos[2] - (double)fromPos.getZ()) * dz);
            moveReached = dist <= closeness;
            moveFinalDist = dist;
            o.addProperty("status", "ok");
            o.addProperty("reached", Boolean.valueOf(moveReached));
            o.addProperty("final_dist", (Number)moveFinalDist);
            o.addProperty("detail", "go_to_player " + targetName + " reached=" + moveReached + " dist=" + String.format("%.1f", moveFinalDist));
            moveTarget = null;
            return o;
        }
        long start = System.currentTimeMillis();
        while (moveWaypoints != null && System.currentTimeMillis() - start < 20000L) {
            if (shouldStop) {
                shouldStop = false;
                break;
            }
            try {
                Thread.sleep(50L);
            }
            catch (InterruptedException e) {
                // empty catch block
                break;
            }
        }
        o.addProperty("status", "ok");
        o.addProperty("reached", Boolean.valueOf(moveReached));
        o.addProperty("final_dist", (Number)moveFinalDist);
        o.addProperty("detail", "go_to_player " + targetName + " reached=" + moveReached + " dist=" + String.format("%.1f", moveFinalDist));
        return o;
    }

    private JsonObject performGivePlayer(JsonObject req) {
        JsonObject o = new JsonObject();
        if (serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u670d\u52a1\u5668\u672a\u5c31\u7eea");
            return o;
        }
        String targetName = req.has("player_name") ? req.get("player_name").getAsString() : "";
        String giveItem = req.has("item") ? req.get("item").getAsString() : "";
        int giveNum = req.has("num") ? req.get("num").getAsInt() : 1;
        String targetId = CraftAgentBridge.onServer(() -> {
            for (ServerPlayer p : serverInstance.getPlayerList().getPlayers()) {
                if (!p.getName().getString().equalsIgnoreCase(targetName)) continue;
                return p.getUUID().toString();
            }
            return null;
        });
        if (targetId == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "give_player: player '" + targetName + "' not found");
            return o;
        }
        double[] targetPos = CraftAgentBridge.onServer(() -> {
            for (ServerPlayer p : serverInstance.getPlayerList().getPlayers()) {
                if (!p.getUUID().toString().equals(targetId)) continue;
                return new double[]{p.getX(), p.getY(), p.getZ()};
            }
            return null;
        });
        if (targetPos == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "give_player: player disappeared");
            return o;
        }
        double[] myPos = CraftAgentBridge.onServer(() -> {
            ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
            if (p == null) {
                return null;
            }
            return new double[]{p.getX(), p.getY(), p.getZ()};
        });
        double dist = Double.MAX_VALUE;
        if (myPos != null) {
            double dx = myPos[0] - targetPos[0];
            double dy = myPos[1] - targetPos[1];
            double dz = myPos[2] - targetPos[2];
            dist = Math.sqrt(dx * dx + dy * dy + dz * dz);
        }
        if (dist > 3.0) {
            moveTarget = targetPos;
            moveTicksLeft = 200;
            long start = System.currentTimeMillis();
            while (moveTarget != null && System.currentTimeMillis() - start < 10000L) {
                if (shouldStop) {
                    shouldStop = false;
                    break;
                }
                try {
                    Thread.sleep(200L);
                }
                catch (InterruptedException e) {
                    // empty catch block
                    break;
                }
            }
        }
        String search = giveItem.replace("minecraft:", "").toLowerCase();
        int dropped = CraftAgentBridge.onServer(() -> {
            ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
            if (p == null) {
                return 0;
            }
            Inventory inv = p.getInventory();
            int count = 0;
            for (int i = 0; i < inv.getContainerSize() && count < giveNum; ++i) {
                String key;
                ItemStack s = inv.getItem(i);
                if (s.isEmpty() || !(key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase()).contains(search)) continue;
                int take = Math.min(s.getCount(), giveNum - count);
                ItemStack toDrop = s.copy();
                toDrop.setCount(take);
                s.shrink(take);
                p.drop(toDrop, false);
                count += take;
            }
            p.containerMenu.broadcastChanges();
            return count;
        });
        o.addProperty("status", "ok");
        o.addProperty("dropped", (Number)dropped);
        o.addProperty("detail", "give_player " + giveItem + " x" + dropped + " to " + targetName);
        return o;
    }

    private JsonObject performDiscardSmart(JsonObject req) {
        JsonObject o = new JsonObject();
        if (serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u670d\u52a1\u5668\u672a\u5c31\u7eea");
            return o;
        }
        String itemName = req.has("item") ? req.get("item").getAsString() : "";
        int num = req.has("num") ? req.get("num").getAsInt() : 1;
        String search = itemName.replace("minecraft:", "").toLowerCase();
        double[] startData = CraftAgentBridge.onServer(() -> {
            ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
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
        moveTarget = new double[]{startX + awayDx, startY, startZ + awayDz};
        moveTicksLeft = 100;
        long moveStart = System.currentTimeMillis();
        while (moveTarget != null && System.currentTimeMillis() - moveStart < 5000L) {
            if (shouldStop) {
                shouldStop = false;
                break;
            }
            try {
                Thread.sleep(100L);
            }
            catch (InterruptedException e) {
                // empty catch block
                break;
            }
        }
        int dropped = CraftAgentBridge.onServer(() -> {
            ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
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
            // empty catch block
        }
        moveTarget = new double[]{startX, startY, startZ};
        moveTicksLeft = 100;
        long returnStart = System.currentTimeMillis();
        while (moveTarget != null && System.currentTimeMillis() - returnStart < 5000L) {
            if (shouldStop) {
                shouldStop = false;
                break;
            }
            try {
                Thread.sleep(100L);
            }
            catch (InterruptedException e) {
                // empty catch block
                break;
            }
        }
        o.addProperty("status", "ok");
        o.addProperty("dropped", (Number)dropped);
        o.addProperty("detail", "discard_smart " + itemName + " x" + dropped + " (moved away 5m, dropped, returned)");
        return o;
    }

    private JsonObject performCollectItems(JsonObject req) {
        JsonObject o = new JsonObject();
        if (serverInstance == null) {
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
            if (shouldStop) {
                shouldStop = false;
                break;
            }
            int[] collectedThisLoop = new int[]{0};
            CraftAgentBridge.onServer(() -> {
                ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
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
                ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
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
                ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
                if (p == null) {
                    return null;
                }
                return new double[]{p.getX(), p.getY(), p.getZ()};
            })) != null) {
                moveTarget = new double[]{nx, myPos[1], nz};
                moveTicksLeft = 100;
                long walkStart = System.currentTimeMillis();
                while (moveTarget != null && System.currentTimeMillis() - walkStart < 5000L) {
                    if (shouldStop) {
                        shouldStop = false;
                        break;
                    }
                    try {
                        Thread.sleep(100L);
                    }
                    catch (InterruptedException e) {
                        // empty catch block
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

    private JsonObject performAttackPlayer(JsonObject req) {
        JsonObject o = new JsonObject();
        if (serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u670d\u52a1\u5668\u672a\u5c31\u7eea");
            return o;
        }
        String targetName = req.has("player_name") ? req.get("player_name").getAsString() : "";
        int ticks = req.has("ticks") ? req.get("ticks").getAsInt() : 60;
        String targetId = CraftAgentBridge.onServer(() -> {
            for (ServerPlayer p : serverInstance.getPlayerList().getPlayers()) {
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
            if (shouldStop) {
                shouldStop = false;
                break;
            }
            double[] info = CraftAgentBridge.onServer(() -> {
                ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
                if (p == null) {
                    return null;
                }
                ServerPlayer target = null;
                for (ServerPlayer pp : serverInstance.getPlayerList().getPlayers()) {
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
                moveReached = false;
                moveFinalDist = 999.0;
                moveStuck = false;
                moveTicksLeft = 40;
                moveTarget = new double[]{tx, ty, tz};
                long moveStart = System.currentTimeMillis();
                while (moveTarget != null && System.currentTimeMillis() - moveStart < 2000L && !shouldStop) {
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
                    ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
                    if (p == null) {
                        return null;
                    }
                    ServerPlayer target = null;
                    for (ServerPlayer pp : serverInstance.getPlayerList().getPlayers()) {
                        if (!pp.getUUID().toString().equals(targetId)) continue;
                        target = pp;
                        break;
                    }
                    if (target != null && target.isAlive() && (double)p.distanceTo(target) <= 5.0) {
                        CraftAgentBridge.equipBestWeapon(p);
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
        moveTarget = null;
        o.addProperty("status", "ok");
        o.addProperty("hits", (Number)hitCount);
        o.addProperty("detail", "attack_player " + targetName + " hits=" + hitCount);
        return o;
    }

    private JsonObject performFollowPlayer(JsonObject req) {
        JsonObject o = new JsonObject();
        if (serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u670d\u52a1\u5668\u672a\u5c31\u7eea");
            return o;
        }
        String targetName = req.has("player_name") ? req.get("player_name").getAsString() : "";
        double followDist = req.has("follow_dist") ? req.get("follow_dist").getAsDouble() : 3.0;
        int totalTicks = req.has("ticks") ? req.get("ticks").getAsInt() : 600;
        String targetId = CraftAgentBridge.onServer(() -> {
            for (ServerPlayer p : serverInstance.getPlayerList().getPlayers()) {
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
            if (shouldStop) {
                shouldStop = false;
                break;
            }
            double[] targetPos = CraftAgentBridge.onServer(() -> {
                for (ServerPlayer p : serverInstance.getPlayerList().getPlayers()) {
                    if (!p.getUUID().toString().equals(targetId)) continue;
                    if (!p.isAlive()) {
                        return null;
                    }
                    return new double[]{p.getX(), p.getY(), p.getZ()};
                }
                return null;
            });
            if (targetPos == null || (myPos = CraftAgentBridge.onServer(() -> {
                ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
                if (p == null) {
                    return null;
                }
                return new double[]{p.getX(), p.getY(), p.getZ()};
            })) == null) break;
            double dx = targetPos[0] - myPos[0];
            double dz = targetPos[2] - myPos[2];
            double dist = Math.sqrt(dx * dx + dz * dz);
            if (dist > followDist + 0.5) {
                moveReached = false;
                moveFinalDist = 999.0;
                moveStuck = false;
                moveTicksLeft = 30;
                moveTarget = (double[])targetPos.clone();
                long moveStart = System.currentTimeMillis();
                while (moveTarget != null && System.currentTimeMillis() - moveStart < 1500L && !shouldStop) {
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
                moveReached = false;
                moveFinalDist = 999.0;
                moveStuck = false;
                moveTicksLeft = 20;
                moveTarget = new double[]{backX, myPos[1], backZ};
                long moveStart = System.currentTimeMillis();
                while (moveTarget != null && System.currentTimeMillis() - moveStart < 1000L && !shouldStop) {
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
        moveTarget = null;
        o.addProperty("status", "ok");
        o.addProperty("followed_ticks", (Number)followTicks);
        o.addProperty("detail", "follow_player " + targetName + " for " + followTicks + " ticks");
        return o;
    }

    private JsonObject performCombat(JsonObject req) {
        JsonObject o = new JsonObject();
        if (serverInstance == null) {
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
            if (shouldStop) {
                shouldStop = false;
                break;
            }
            String[] tType = new String[]{""};
            double[] info = CraftAgentBridge.onServer(() -> {
                ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
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
                    if (!CraftAgentBridge.isHostile(tn) || !((d = (double)e.distanceTo((Entity)p)) < minDist)) continue;
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
                    ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
                    if (p == null) {
                        return null;
                    }
                    return new double[]{p.getX(), p.getY(), p.getZ()};
                });
                if (myPos2 == null || !((len2 = Math.sqrt((dx2 = myPos2[0] - tx) * dx2 + (dz2 = myPos2[2] - tz) * dz2)) > 0.0)) break;
                moveReached = false;
                moveFinalDist = 999.0;
                moveTicksLeft = 100;
                moveTarget = new double[]{myPos2[0] + dx2 / len2 * 15.0, myPos2[1], myPos2[2] + dz2 / len2 * 15.0};
                long moveStart2 = System.currentTimeMillis();
                while (moveTarget != null && System.currentTimeMillis() - moveStart2 < 5000L && !shouldStop) {
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
                    ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
                    if (p == null) {
                        return null;
                    }
                    return new double[]{p.getX(), p.getY(), p.getZ()};
                });
                if (myPos3 == null || !((len = Math.sqrt((dx = myPos3[0] - tx) * dx + (dz = myPos3[2] - tz) * dz)) > 0.0)) continue;
                moveReached = false;
                moveFinalDist = 999.0;
                moveTicksLeft = 30;
                moveTarget = new double[]{myPos3[0] + dx / len * 8.0, myPos3[1], myPos3[2] + dz / len * 8.0};
                moveStart = System.currentTimeMillis();
                while (moveTarget != null && System.currentTimeMillis() - moveStart < 1500L) {
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
                    ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
                    if (p == null) {
                        return null;
                    }
                    return new double[]{p.getX(), p.getY(), p.getZ()};
                });
                if (myPos4 != null && (len = Math.sqrt((dx = myPos4[0] - tx) * dx + (dz = myPos4[2] - tz) * dz)) > 0.0 && dist < 15.0) {
                    moveReached = false;
                    moveFinalDist = 999.0;
                    moveTicksLeft = 50;
                    moveTarget = new double[]{myPos4[0] + dx / len * 18.0, myPos4[1], myPos4[2] + dz / len * 18.0};
                    moveStart = System.currentTimeMillis();
                    while (moveTarget != null && System.currentTimeMillis() - moveStart < 2500L && !shouldStop) {
                        try {
                            Thread.sleep(50L);
                        }
                        catch (InterruptedException e) {
                            // empty catch block
                            break;
                        }
                    }
                }
                if (!(dist > 15.0)) continue;
                result = "retreated";
                break;
            }
            if (dist > 4.0) {
                moveReached = false;
                moveFinalDist = 999.0;
                moveStuck = false;
                moveTicksLeft = 30;
                moveTarget = new double[]{tx, ty, tz};
                long moveStart3 = System.currentTimeMillis();
                while (moveTarget != null && System.currentTimeMillis() - moveStart3 < 1500L && !shouldStop) {
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
                    ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
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
                        if (!CraftAgentBridge.isHostile(tn) || !((d = (double)e.distanceTo((Entity)p)) < minDist)) continue;
                        minDist = d;
                        target = le;
                    }
                    if (target != null && minDist <= 5.0) {
                        CraftAgentBridge.equipBestWeapon(p);
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
                ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
                if (p == null) {
                    return null;
                }
                return new double[]{p.getX(), p.getY(), p.getZ()};
            })) != null && (len = Math.sqrt((dx = myPos[0] - tx) * dx + (dz = myPos[2] - tz) * dz)) > 0.0 && dist < 6.0) {
                moveReached = false;
                moveFinalDist = 999.0;
                moveTicksLeft = 15;
                moveTarget = new double[]{myPos[0] + dx / len * 8.0, myPos[1], myPos[2] + dz / len * 8.0};
                moveStart = System.currentTimeMillis();
                while (moveTarget != null && System.currentTimeMillis() - moveStart < 800L) {
                    try {
                        Thread.sleep(50L);
                    }
                    catch (InterruptedException e) {
                        // empty catch block
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
        moveTarget = null;
        if (result.equals("none")) {
            result = "timeout";
        }
        o.addProperty("status", "ok");
        o.addProperty("result", result);
        o.addProperty("target", targetType);
        o.addProperty("detail", "combat mode=" + mode + " -> " + result + " (target=" + targetType + ")");
        return o;
    }

    private JsonObject performUseItem(JsonObject req) {
        JsonObject o = new JsonObject();
        if (serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u670d\u52a1\u5668\u672a\u5c31\u7eea");
            return o;
        }
        int ticks = req.has("ticks") ? req.get("ticks").getAsInt() : 5;
        boolean[] consumed = new boolean[]{false};
        String[] itemId = new String[]{""};
        CraftAgentBridge.onServer(() -> {
            ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
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
                // empty catch block
            }
        }
        o.addProperty("status", "ok");
        o.addProperty("consumed", Boolean.valueOf(consumed[0]));
        o.addProperty("detail", "use_item " + itemId[0] + " (consumed=" + consumed[0] + ")");
        return o;
    }

    private JsonObject performEatItem(JsonObject req) {
        JsonObject o = new JsonObject();
        if (serverInstance == null) {
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
            ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
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
                // empty catch block
            }
            CraftAgentBridge.onServer(() -> {
                ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
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

    private JsonObject performPillarUp(JsonObject req) {
        Boolean ok;
        JsonObject o = new JsonObject();
        if (serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u670d\u52a1\u5668\u672a\u5c31\u7eea");
            return o;
        }
        int count = req.has("count") ? req.get("count").getAsInt() : 3;
        String item = req.has("item") ? req.get("item").getAsString() : "dirt";
        int placed = 0;
        for (int i = 0; i < count && (ok = CraftAgentBridge.onServer(() -> {
            BlockPos below;
            ServerPlayer p = CraftAgentBridge.getFirstPlayer(serverInstance);
            if (p == null) {
                return false;
            }
            if (shouldStop) {
                return false;
            }
            ServerLevel lvl = p.level();
            if (CraftAgentBridge.placeAt(p, lvl, (below = p.blockPosition().below()).getX(), below.getY(), below.getZ(), item)) {
                p.setDeltaMovement(p.getDeltaMovement().x, 0.42, p.getDeltaMovement().z);
                return true;
            }
            return false;
        })) != null && ok.booleanValue(); ++i) {
            ++placed;
            try {
                Thread.sleep(200L);
                continue;
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

    private JsonObject performWait(JsonObject req) {
        JsonObject o = new JsonObject();
        int seconds = req.has("seconds") ? req.get("seconds").getAsInt() : 1;
        try {
            Thread.sleep((long)seconds * 1000L);
        }
        catch (InterruptedException interruptedException) {
            // empty catch block
        }
        o.addProperty("status", "ok");
        o.addProperty("detail", "wait " + seconds + "s");
        return o;
    }

    private JsonObject buildState() {
        JsonObject o = new JsonObject();
        MinecraftServer server = serverInstance;
        if (server == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u670d\u52a1\u5668\u672a\u5c31\u7eea");
            return o;
        }
        ServerPlayer player = CraftAgentBridge.getFirstPlayer(server);
        if (player == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u6ca1\u6709\u5728\u7ebf\u73a9\u5bb6\uff08\u8bf7\u5148\u8fdb\u5165\u4e16\u754c\uff09");
            return o;
        }
        ServerLevel level = player.level();
        Vec3 pos = player.position();
        o.add("position", (JsonElement)CraftAgentBridge.arr(pos.x, pos.y, pos.z));
        o.addProperty("yaw", (Number)Float.valueOf(player.getYRot()));
        o.addProperty("pitch", (Number)Float.valueOf(player.getXRot()));
        o.addProperty("health", (Number)Float.valueOf(player.getHealth()));
        o.addProperty("hunger", (Number)player.getFoodData().getFoodLevel());
        o.addProperty("gamemode", player.gameMode.getGameModeForPlayer().getName());
        o.addProperty("time", (Number)level.getOverworldClockTime());
        o.addProperty("dimension", level.dimension().toString());
        o.addProperty("biome", level.getBiomeManager().getBiome(player.blockPosition()).unwrapKey().map(k -> k.identifier().toString()).orElse("?"));
        long time = level.getOverworldClockTime() % 24000L;
        int hour = (int)((time / 1000L + 6L) % 24L);
        int minute = (int)(time % 1000L * 60L / 1000L);
        boolean isDay = time < 12000L || time >= 23000L;
        o.addProperty("time_str", String.format("%02d:%02d (%s)", hour, minute, isDay ? "day" : "night"));
        Vec3 vel = player.getDeltaMovement();
        o.add("velocity", (JsonElement)CraftAgentBridge.arr(vel.x, vel.y, vel.z));
        JsonArray effects = new JsonArray();
        for (MobEffectInstance me : player.getActiveEffects()) {
            MobEffect effect = (MobEffect)me.getEffect().value();
            String id = BuiltInRegistries.MOB_EFFECT.getKey(effect).toString();
            JsonObject eo = new JsonObject();
            eo.addProperty("id", id);
            eo.addProperty("amplifier", (Number)me.getAmplifier());
            eo.addProperty("duration", (Number)me.getDuration());
            effects.add((JsonElement)eo);
        }
        o.add("effects", (JsonElement)effects);
        o.addProperty("experience_level", (Number)player.experienceLevel);
        o.addProperty("experience_progress", (Number)Float.valueOf(player.experienceProgress));
        o.addProperty("raining", Boolean.valueOf(level.isRaining()));
        o.addProperty("thundering", Boolean.valueOf(level.isThundering()));
        BlockPos pp = player.blockPosition();
        int skyLight = level.getLightEngine().getLayerListener(LightLayer.SKY).getLightValue(pp);
        int blockLight = level.getLightEngine().getLayerListener(LightLayer.BLOCK).getLightValue(pp);
        o.addProperty("sky_light", (Number)skyLight);
        o.addProperty("block_light", (Number)blockLight);
        JsonArray inv = new JsonArray();
        Inventory inventory = player.getInventory();
        int size = inventory.getContainerSize();
        for (int i = 0; i < size; ++i) {
            ItemStack stack = inventory.getItem(i);
            if (stack.isEmpty()) continue;
            String id = BuiltInRegistries.ITEM.getKey(stack.getItem()).toString();
            JsonObject s = new JsonObject();
            s.addProperty("slot", (Number)i);
            s.addProperty("id", id);
            s.addProperty("count", (Number)stack.getCount());
            inv.add((JsonElement)s);
        }
        o.add("inventory", (JsonElement)inv);
        ItemStack held = player.getMainHandItem();
        int selectedSlot = inventory.getSelectedSlot();
        o.addProperty("held_item", held.isEmpty() ? "minecraft:air" : BuiltInRegistries.ITEM.getKey(held.getItem()).toString());
        o.addProperty("selected_slot", (Number)selectedSlot);
        HitResult hit = player.pick(6.0, 0.0f, false);
        if (hit != null && hit.getType() == HitResult.Type.BLOCK) {
            BlockPos bp = ((BlockHitResult)hit).getBlockPos();
            BlockState bs = level.getBlockState(bp);
            String id = BuiltInRegistries.BLOCK.getKey(bs.getBlock()).toString();
            double dist = player.position().distanceTo(Vec3.atCenterOf((Vec3i)bp));
            JsonObject tb = new JsonObject();
            tb.addProperty("id", id);
            tb.addProperty("dist", (Number)dist);
            tb.addProperty("x", (Number)bp.getX());
            tb.addProperty("y", (Number)bp.getY());
            tb.addProperty("z", (Number)bp.getZ());
            o.add("targeted_block", (JsonElement)tb);
        } else {
            o.add("targeted_block", null);
        }
        JsonArray blocks = new JsonArray();
        BlockPos pc = player.blockPosition();
        for (BlockPos bp : BlockPos.betweenClosed((int)(pc.getX() - 16), (int)(pc.getY() - 16), (int)(pc.getZ() - 16), (int)(pc.getX() + 16), (int)(pc.getY() + 16), (int)(pc.getZ() + 16))) {
            String id;
            BlockState bs = level.getBlockState(bp);
            if (bs.isAir() || !CraftAgentBridge.matchesWhitelist(id = BuiltInRegistries.BLOCK.getKey(bs.getBlock()).toString())) continue;
            double dist = player.position().distanceTo(Vec3.atCenterOf((Vec3i)bp));
            JsonObject b = new JsonObject();
            b.addProperty("id", id);
            b.addProperty("x", (Number)bp.getX());
            b.addProperty("y", (Number)bp.getY());
            b.addProperty("z", (Number)bp.getZ());
            b.addProperty("dist", (Number)dist);
            b.addProperty("height_diff", (Number)(player.getY() - (double)bp.getY()));
            blocks.add((JsonElement)b);
        }
        o.add("nearby_blocks", (JsonElement)blocks);
        JsonArray ents = new JsonArray();
        AABB scanArea = AABB.ofSize((Vec3)player.position(), (double)32.0, (double)32.0, (double)32.0);
        for (Entity e : level.getEntities((Entity)player, scanArea)) {
            float f;
            if (e == player) continue;
            String tid = BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString();
            Vec3 ep = e.position();
            double dist = player.distanceTo(e);
            JsonObject en = new JsonObject();
            en.addProperty("type", tid);
            en.addProperty("x", (Number)ep.x);
            en.addProperty("y", (Number)ep.y);
            en.addProperty("z", (Number)ep.z);
            en.addProperty("dist", (Number)dist);
            if (e instanceof LivingEntity) {
                LivingEntity le = (LivingEntity)e;
                f = le.getHealth();
            } else {
                f = 0.0f;
            }
            float hp = f;
            en.addProperty("health", (Number)Float.valueOf(hp));
            ents.add((JsonElement)en);
        }
        o.add("entities", (JsonElement)ents);
        String nearestThreatType = null;
        double nearestThreatDist = Double.MAX_VALUE;
        for (Entity e : level.getEntities((Entity)player, scanArea)) {
            double d;
            if (e == player || !(e instanceof Mob)) continue;
            Mob mob = (Mob)e;
            boolean hostile = e instanceof Monster;
            if (!hostile && mob.getTarget() == player) {
                hostile = true;
            }
            if (!hostile || !((d = (double)player.distanceTo(e)) < nearestThreatDist)) continue;
            nearestThreatDist = d;
            nearestThreatType = BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString();
        }
        if (nearestThreatType != null) {
            JsonObject nt = new JsonObject();
            nt.addProperty("type", nearestThreatType);
            nt.addProperty("dist", (Number)nearestThreatDist);
            o.add("nearest_threat", (JsonElement)nt);
        } else {
            o.add("nearest_threat", null);
        }
        o.addProperty("status", "ok");
        return o;
    }

    /*
     * Enabled force condition propagation
     * Lifted jumps to return sites
     */
    private JsonObject performAction(String type, JsonObject req) {
        MinecraftServer server = serverInstance;
        if (server == null) {
            JsonObject o = new JsonObject();
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u670d\u52a1\u5668\u672a\u5c31\u7eea");
            return o;
        }
        ServerPlayer player = CraftAgentBridge.getFirstPlayer(server);
        if (player == null) {
            JsonObject o = new JsonObject();
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u6ca1\u6709\u5728\u7ebf\u73a9\u5bb6");
            return o;
        }
        ServerLevel level = player.level();
        // 优先走命令分派表；表中无此命令再回退 legacy switch（逐步迁移）。
        CommandHandler handler = COMMAND_HANDLERS.get(type);
        if (handler != null) {
            return handler.handle(player, level, req);
        }
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        switch (type) {
            case "attack": {
                LivingEntity target = null;
                double minDist = Double.MAX_VALUE;
                AABB scanArea = AABB.ofSize((Vec3)player.position(), (double)16.0, (double)16.0, (double)16.0);
                for (Entity e4 : level.getEntities((Entity)player, scanArea)) {
                    double d;
                    if (!(e4 instanceof LivingEntity)) continue;
                    LivingEntity le = (LivingEntity)e4;
                    String tn = BuiltInRegistries.ENTITY_TYPE.getKey(e4.getType()).getPath();
                    if (!CraftAgentBridge.isHostile(tn) || !((d = (double)e4.distanceTo((Entity)player)) < minDist)) continue;
                    minDist = d;
                    target = le;
                }
                if (target == null) {
                    o.addProperty("detail", "attack: no hostile entity nearby");
                    return o;
                }
                CraftAgentBridge.equipBestWeapon(player);
                player.lookAt(EntityAnchorArgument.Anchor.EYES, target.position().add(0.0, 1.0, 0.0));
                player.attack(target);
                player.containerMenu.broadcastChanges();
                o.addProperty("detail", "attack " + BuiltInRegistries.ENTITY_TYPE.getKey(target.getType()).getPath() + " dist=" + String.format("%.1f", minDist) + "m");
                return o;
            }
            case "enchant": {
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
            case "select_slot": {
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
            case "move_to_hotbar": {
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
            case "move_slot": {
                String toId;
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
                String string = toId = toStack.isEmpty() ? "minecraft:air" : BuiltInRegistries.ITEM.getKey(toStack.getItem()).toString();
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
            case "craft": {
                String item = req.get("item").getAsString();
                int want = req.has("count") ? req.get("count").getAsInt() : 1;
                int crafted = CraftAgentBridge.craftItem(player, item, want);
                player.containerMenu.broadcastChanges();
                o.addProperty("crafted", (Number)crafted);
                o.addProperty("detail", "craft " + item + " x" + crafted);
                return o;
            }
            case "discard": {
                String item = req.get("item").getAsString();
                int num = req.has("num") ? req.get("num").getAsInt() : 1;
                int discarded = CraftAgentBridge.discardItem(player, item, num);
                player.containerMenu.broadcastChanges();
                o.addProperty("detail", "discarded " + discarded + " x " + item);
                return o;
            }
            case "smelt": {
                String item = req.get("item").getAsString();
                int num = req.has("num") ? req.get("num").getAsInt() : 1;
                int smelted = CraftAgentBridge.smeltItem(player, item, num);
                player.containerMenu.broadcastChanges();
                o.addProperty("detail", "smelted " + smelted + " x " + item);
                return o;
            }
            case "inspect_gui": {
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
            case "close_gui": {
                if (player.containerMenu != player.inventoryMenu) {
                    player.closeContainer();
                    o.addProperty("detail", "close_gui: container closed");
                    return o;
                }
                o.addProperty("detail", "close_gui: no container open");
                return o;
            }
            case "transfer": {
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
                    int count;
                    JsonObject mv = moves.get(mi).getAsJsonObject();
                    int fromSlot = mv.get("from").getAsInt();
                    Integer toSlot = mv.has("to") && !mv.get("to").isJsonNull() ? Integer.valueOf(mv.get("to").getAsInt()) : null;
                    int n = count = mv.has("count") ? mv.get("count").getAsInt() : -1;
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
            case "equip_item": {
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
                            EquipmentSlot equipmentSlot = EquipmentSlot.MAINHAND;
                            break;
                        }
                        case "offhand": 
                        case "off_hand": {
                            EquipmentSlot equipmentSlot = EquipmentSlot.OFFHAND;
                            break;
                        }
                        case "head": 
                        case "helmet": {
                            EquipmentSlot equipmentSlot = EquipmentSlot.HEAD;
                            break;
                        }
                        case "chest": 
                        case "chestplate": {
                            EquipmentSlot equipmentSlot = EquipmentSlot.CHEST;
                            break;
                        }
                        case "legs": 
                        case "leggings": {
                            EquipmentSlot equipmentSlot = EquipmentSlot.LEGS;
                            break;
                        }
                        case "feet": 
                        case "boots": {
                            EquipmentSlot equipmentSlot = EquipmentSlot.FEET;
                            break;
                        }
                        default: {
                            EquipmentSlot equipmentSlot = equipSlot = null;
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
            case "drop_items": {
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
            case "list_players": {
                JsonArray players = new JsonArray();
                for (ServerPlayer p : server.getPlayerList().getPlayers()) {
                    JsonObject po = new JsonObject();
                    po.addProperty("name", p.getName().getString());
                    po.addProperty("uuid", p.getUUID().toString());
                    po.add("position", (JsonElement)CraftAgentBridge.arr(p.getX(), p.getY(), p.getZ()));
                    po.addProperty("dist", (Number)Math.sqrt(Math.pow(p.getX() - player.getX(), 2.0) + Math.pow(p.getY() - player.getY(), 2.0) + Math.pow(p.getZ() - player.getZ(), 2.0)));
                    players.add((JsonElement)po);
                }
                o.add("players", (JsonElement)players);
                o.addProperty("count", (Number)players.size());
                o.addProperty("detail", "list_players: " + players.size() + " online");
                return o;
            }
            case "stop": {
                shouldStop = true;
                moveTarget = null;
                o.addProperty("detail", "stop: all actions cancelled");
                return o;
            }
            case "set_goal": {
                String goal;
                String string = goal = req.has("goal") ? req.get("goal").getAsString() : "";
                if (goal.isEmpty()) {
                    currentGoal = null;
                    o.addProperty("detail", "set_goal: cleared");
                    return o;
                }
                currentGoal = goal;
                o.addProperty("detail", "set_goal: " + goal);
                return o;
            }
            case "get_goal": {
                o.addProperty("goal", currentGoal != null ? currentGoal : "(none)");
                o.addProperty("detail", "get_goal: " + (currentGoal != null ? currentGoal : "none"));
                return o;
            }
            case "search_wiki": {
                String query = req.has("query") ? req.get("query").getAsString() : "";
                try {
                    URL url = new URL("https://minecraft.wiki/w/" + URLEncoder.encode(query.replace(" ", "_"), "UTF-8"));
                    HttpURLConnection conn = (HttpURLConnection)url.openConnection();
                    conn.setRequestProperty("User-Agent", "Craft-Agent/1.0");
                    conn.setConnectTimeout(5000);
                    conn.setReadTimeout(10000);
                    if (conn.getResponseCode() == 404) {
                        o.addProperty("detail", "search_wiki: '" + query + "' not found on minecraft.wiki");
                        return o;
                    }
                    try (BufferedReader wr = new BufferedReader(new InputStreamReader(conn.getInputStream(), StandardCharsets.UTF_8));){
                        String line;
                        StringBuilder sb = new StringBuilder();
                        while ((line = wr.readLine()) != null) {
                            sb.append(line).append("\n");
                        }
                        String html = sb.toString();
                        Object text = html.replaceAll("<script[^>]*>[\\s\\S]*?</script>", "").replaceAll("<style[^>]*>[\\s\\S]*?</style>", "").replaceAll("<[^>]+>", " ").replaceAll("&amp;", "&").replaceAll("&lt;", "<").replaceAll("&gt;", ">").replaceAll("&quot;", "\"").replaceAll("&#39;", "'").replaceAll("\\s+", " ").trim();
                        if (((String)text).length() > 2000) {
                            text = ((String)text).substring(0, 2000) + "... [truncated]";
                        }
                        o.addProperty("content", (String)text);
                        o.addProperty("detail", "search_wiki: " + query + " (" + ((String)text).length() + " chars)");
                        return o;
                    }
                }
                catch (Exception e5) {
                    o.addProperty("status", "fail");
                    o.addProperty("detail", "search_wiki error: " + e5.getMessage());
                }
                return o;
            }
            case "villager_trades": {
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
            case "trade_with_villager": {
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
            case "look_at_player": {
                String targetName = req.has("player_name") ? req.get("player_name").getAsString() : "";
                ServerPlayer target = null;
                for (ServerPlayer p : server.getPlayerList().getPlayers()) {
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
            case "look_at_position": {
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
            case "activate_block": {
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
            case "use_on_entity": {
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
            case "fish": {
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
                    // empty catch block
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
            case "ride": {
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
            case "sleep": {
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
            case "wake": {
                player.stopSleeping();
                o.addProperty("detail", "wake (was sleeping=false)");
                return o;
            }
            case "activate_nearest_block": {
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
            case "get_crafting_plan": {
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
            case "build_portal": {
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
                    if (!existing.isAir() || !CraftAgentBridge.placeAt(player, level, bx, by, bz, search)) continue;
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
            case "teleport_to": {
                ServerLevel targetLevel;
                String dimension = req.has("dimension") ? req.get("dimension").getAsString() : "the_nether";
                switch (dimension.toLowerCase()) {
                    case "the_nether":
                    case "nether": {
                        targetLevel = server.getLevel(Level.NETHER);
                        break;
                    }
                    case "the_end":
                    case "end": {
                        targetLevel = server.getLevel(Level.END);
                        break;
                    }
                    default: {
                        targetLevel = server.getLevel(Level.OVERWORLD);
                    }
                }
                if (targetLevel == null) {
                    o.addProperty("status", "fail");
                    o.addProperty("detail", "teleport_to: dimension '" + dimension + "' not available");
                    return o;
                }
                double scale = 1.0;
                if (level.dimension() == Level.NETHER && targetLevel.dimension() == Level.OVERWORLD) {
                    scale = 8.0;
                } else if (level.dimension() == Level.OVERWORLD && targetLevel.dimension() == Level.NETHER) {
                    scale = 0.125;
                }
                double tx = player.getX() * scale;
                double tz = player.getZ() * scale;
                double ty = player.getY();
                if (targetLevel.dimension() == Level.NETHER) {
                    ty = Math.min(ty, 120.0);
                } else if (targetLevel.dimension() == Level.END) {
                    tx = 0.0;
                    ty = 65.0;
                    tz = 0.0;
                }
                player.teleportTo(targetLevel, tx, ty, tz, Set.of(), player.getYRot(), player.getXRot(), false);
                player.containerMenu.broadcastChanges();
                o.addProperty("detail", "teleport_to " + dimension + " at (" + String.format("%.1f", tx) + "," + String.format("%.1f", ty) + "," + String.format("%.1f", tz) + ")");
                return o;
            }
            default: {
                o.addProperty("status", "fail");
                o.addProperty("detail", "\u672a\u77e5\u547d\u4ee4: " + type);
            }
        }
        return o;
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 命令处理器：从 performAction 的 switch case 抽出，逐个迁移到此。
    // 约定：每个 act* 自建 o(status=ok)，返回 JsonObject；失败 addProperty("status","fail")。
    // ═════════════════════════════════════════════════════════════════════════

    private JsonObject actLook(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private JsonObject actLookAbs(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        float yaw = req.has("yaw") ? req.get("yaw").getAsFloat() : player.getYRot();
        float pitch = req.has("pitch") ? req.get("pitch").getAsFloat() : player.getXRot();
        player.setYRot(yaw);
        player.setXRot(CraftAgentBridge.clamp(pitch, -90.0f, 90.0f));
        o.addProperty("detail", "look_abs yaw=" + yaw + " pitch=" + pitch);
        return o;
    }

    private JsonObject actLookAt(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private JsonObject actDigAt(ServerPlayer player, ServerLevel level, JsonObject req) {
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
        CraftAgentBridge.equipBestTool(player, blockId);
        boolean ok = player.level().destroyBlock(pos, true);
        player.containerMenu.broadcastChanges();
        o.addProperty("broken", Boolean.valueOf(ok));
        o.addProperty("block_id", blockId);
        o.addProperty("detail", "dig_at " + tx + "," + ty + "," + tz + " (broken=" + ok + ", block=" + blockId + ")");
        return o;
    }

    private JsonObject actPlaceAt(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        int tx = req.get("x").getAsInt();
        int ty = req.get("y").getAsInt();
        int tz = req.get("z").getAsInt();
        String item = req.has("item") ? req.get("item").getAsString() : "dirt";
        boolean placed = CraftAgentBridge.placeAt(player, level, tx, ty, tz, item);
        player.containerMenu.broadcastChanges();
        o.addProperty("placed", Boolean.valueOf(placed));
        o.addProperty("detail", "place_at " + tx + "," + ty + "," + tz + " item=" + item + " (placed=" + placed + ")");
        return o;
    }

    private JsonObject actGetBlock(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private JsonObject actGetBlocks(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private JsonObject actClearChat(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        o.addProperty("detail", "clear_chat: mod side ack, Rust side should clear history");
        return o;
    }

    private JsonObject actDebugSpawn(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        EntitySpawnRequest spawnReq;
        String ent = req.has("entity") ? req.get("entity").getAsString().toLowerCase() : "zombie";
        double fx = player.getX() + player.getLookAngle().x * 3.0;
        double fz = player.getZ() + player.getLookAngle().z * 3.0;
        double fy = player.getY() + 1.0;
        if (ent.equals("item")) {
            String itemId = req.has("item") ? req.get("item").getAsString() : "minecraft:oak_log";
            int num = req.has("num") ? req.get("num").getAsInt() : 1;
            Optional holder = BuiltInRegistries.ITEM.get(Identifier.fromNamespaceAndPath((String)(itemId.contains(":") ? itemId.split(":")[0] : "minecraft"), (String)(itemId.contains(":") ? itemId.split(":")[1] : itemId)));
            if (holder.isEmpty()) {
                o.addProperty("detail", "debug_spawn unknown item: " + itemId);
                return o;
            }
            ItemStack stack = new ItemStack((ItemLike)((Holder.Reference)holder.get()).value(), num);
            ItemEntity ie = new ItemEntity((Level)level, fx, fy + 1.0, fz, stack);
            level.addFreshEntity((Entity)ie);
            o.addProperty("detail", "debug_spawn item " + itemId + " x" + num);
            return o;
        }
        Optional eth = BuiltInRegistries.ENTITY_TYPE.get(Identifier.fromNamespaceAndPath((String)"minecraft", (String)ent));
        if (eth.isEmpty()) {
            o.addProperty("detail", "debug_spawn unknown entity: " + ent);
            return o;
        }
        EntityType et = (EntityType)((Holder.Reference)eth.get()).value();
        Entity e2 = et.spawn(level, null, BlockPos.containing((double)fx, (double)fy, (double)fz), EntitySpawnReason.COMMAND, true, false);
        if (e2 == null && (e2 = et.create((Level)level, spawnReq = new EntitySpawnRequest(EntitySpawnReason.COMMAND, true))) != null) {
            e2.setPos(fx, fy, fz);
            level.addFreshEntity(e2);
        }
        if (e2 == null) {
            o.addProperty("detail", "debug_spawn failed to create: " + ent);
            return o;
        }
        if (e2 instanceof Mob) {
            Mob mob = (Mob)e2;
            mob.finalizeSpawn((ServerLevelAccessor)level, level.getCurrentDifficultyAt(BlockPos.containing((double)fx, (double)fy, (double)fz)), EntitySpawnReason.COMMAND, null);
            mob.setPersistenceRequired();
            mob.setNoAi(false);
            mob.addEffect(new MobEffectInstance(MobEffects.FIRE_RESISTANCE, 999999, 0, false, false));
        }
        if (e2 instanceof LivingEntity && !(e2 instanceof Villager)) {
            e2.startRiding((Entity)player, false, false);
        }
        if (e2 instanceof Villager) {
            String prof;
            Villager v = (Villager)e2;
            String string = prof = req.has("profession") ? req.get("profession").getAsString().toLowerCase() : "";
            if (!prof.isEmpty() && !prof.equals("none")) {
                MerchantOffers offers = new MerchantOffers();
                offers.add(new MerchantOffer(new ItemCost((ItemLike)Items.EMERALD, 1), new ItemStack((ItemLike)Items.BOOK, 1), 5, 5, 0.05f));
                offers.add(new MerchantOffer(new ItemCost((ItemLike)Items.EMERALD, 1), new ItemStack((ItemLike)Items.WHEAT, 4), 5, 5, 0.05f));
                v.setOffers(offers);
                player.getInventory().add(new ItemStack((ItemLike)Items.EMERALD, 16));
                player.containerMenu.broadcastChanges();
                o.addProperty("detail", "debug_spawn villager(profession=" + prof + ") with " + offers.size() + " injected offers");
            }
        }
        int nearbyCount = level.getEntities((Entity)player, AABB.ofSize((Vec3)player.position(), (double)32.0, (double)32.0, (double)32.0)).size();
        StringBuilder nearby = new StringBuilder();
        for (Entity ne : level.getEntities((Entity)player, AABB.ofSize((Vec3)player.position(), (double)32.0, (double)32.0, (double)32.0))) {
            if (ne == player) continue;
            nearby.append(BuiltInRegistries.ENTITY_TYPE.getKey(ne.getType()).toString()).append("(ride=").append(ne.isPassenger()).append(")@").append(String.format("%.1f,%.1f,%.1f", ne.getX(), ne.getY(), ne.getZ())).append("; ");
        }
        o.addProperty("detail", "debug_spawn " + ent + " at " + String.format("%.1f,%.1f,%.1f", e2.getX(), e2.getY(), e2.getZ()) + " | riding=" + e2.isPassenger() + " | nearbyEntities=" + nearbyCount + " | [" + String.valueOf(nearby) + "]");
        return o;
    }

    private JsonObject actDebugGive(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String itemId = req.has("item") ? req.get("item").getAsString() : "minecraft:oak_log";
        int num = req.has("num") ? req.get("num").getAsInt() : 1;
        Optional holder = BuiltInRegistries.ITEM.get(Identifier.fromNamespaceAndPath((String)(itemId.contains(":") ? itemId.split(":")[0] : "minecraft"), (String)(itemId.contains(":") ? itemId.split(":")[1] : itemId)));
        if (holder.isEmpty()) {
            o.addProperty("detail", "debug_give unknown item: " + itemId);
            return o;
        }
        player.getInventory().add(new ItemStack((ItemLike)((Holder.Reference)holder.get()).value(), num));
        player.containerMenu.broadcastChanges();
        o.addProperty("detail", "debug_give " + itemId + " x" + num);
        return o;
    }

    private JsonObject actDebugDamage(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        float amt = req.has("amount") ? req.get("amount").getAsFloat() : 5.0f;
        float newHp = Math.max(1.0f, player.getHealth() - amt);
        player.setHealth(newHp);
        o.addProperty("detail", "debug_damage " + amt + " -> hp=" + newHp);
        return o;
    }

    private JsonObject actDebugHeal(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        float amt = req.has("amount") ? req.get("amount").getAsFloat() : 20.0f;
        float newHp = Math.min(20.0f, player.getHealth() + amt);
        player.setHealth(newHp);
        o.addProperty("detail", "debug_heal " + amt + " -> hp=" + newHp);
        return o;
    }

    private JsonObject actDebugClear(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        player.getInventory().clearContent();
        player.containerMenu.broadcastChanges();
        AABB global = AABB.ofSize((Vec3)new Vec3(0.0, 64.0, 0.0), (double)100000.0, (double)100000.0, (double)100000.0);
        for (Entity e3 : level.getEntities(null, global)) {
            if (e3 instanceof ItemEntity) {
                e3.discard();
            }
            if (!(e3 instanceof Monster)) continue;
            e3.discard();
        }
        o.addProperty("detail", "debug_clear inventory + drops + hostiles");
        return o;
    }

    private JsonObject actDebugPlace(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String blockId = req.has("block") ? req.get("block").getAsString() : "minecraft:oak_log";
        int bx = req.has("x") ? req.get("x").getAsInt() : (int)Math.floor(player.getX()) + 1;
        int by = req.has("y") ? req.get("y").getAsInt() : (int)Math.floor(player.getY()) - 1;
        int bz = req.has("z") ? req.get("z").getAsInt() : (int)Math.floor(player.getZ());
        Optional bHolder = BuiltInRegistries.BLOCK.get(Identifier.fromNamespaceAndPath((String)(blockId.contains(":") ? blockId.split(":")[0] : "minecraft"), (String)(blockId.contains(":") ? blockId.split(":")[1] : blockId)));
        if (bHolder.isEmpty()) {
            o.addProperty("detail", "debug_place unknown block: " + blockId);
            return o;
        }
        BlockState bs = ((Block)((Holder.Reference)bHolder.get()).value()).defaultBlockState();
        level.setBlock(new BlockPos(bx, by, bz), bs, 3);
        BlockState after = level.getBlockState(new BlockPos(bx, by, bz));
        String afterId = BuiltInRegistries.BLOCK.getKey(after.getBlock()).toString();
        o.addProperty("detail", "debug_place " + blockId + " @ (" + bx + "," + by + "," + bz + ") -> actual=" + afterId + " air=" + after.isAir());
        return o;
    }

    private JsonObject actDebugXp(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        int levels = req.has("levels") ? req.get("levels").getAsInt() : 30;
        player.giveExperienceLevels(levels);
        o.addProperty("detail", "debug_xp +" + levels + " levels (now " + player.experienceLevel + ")");
        return o;
    }

    private JsonObject actDebugFood(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        int lvl = req.has("level") ? req.get("level").getAsInt() : 0;
        lvl = Math.max(0, Math.min(20, lvl));
        player.getFoodData().setFoodLevel(lvl);
        o.addProperty("detail", "debug_food level=" + lvl);
        return o;
    }

    private JsonObject actDebugTime(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String which;
        String timeArg = which = req.has("value") ? req.get("value").getAsString().toLowerCase() : "night";
        if (which.equals("night")) {
            timeArg = "night";
        } else if (which.equals("day")) {
            timeArg = "day";
        } else if (which.equals("noon")) {
            timeArg = "noon";
        } else if (which.equals("midnight")) {
            timeArg = "midnight";
        }
        try {
            serverInstance.getCommands().performPrefixedCommand(serverInstance.createCommandSourceStack(), "time set " + timeArg);
            serverInstance.getCommands().performPrefixedCommand(serverInstance.createCommandSourceStack(), "gamerule doDaylightCycle false");
            o.addProperty("detail", "debug_time -> " + which + " (doDaylightCycle=false)");
            return o;
        }
        catch (Exception ex) {
            o.addProperty("detail", "debug_time failed: " + ex.getMessage());
        }
        return o;
    }

    private JsonObject actDebugTeleportPlayer(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String name = req.has("name") ? req.get("name").getAsString() : "";
        double dist = req.has("dist") ? req.get("dist").getAsDouble() : 3.0;
        ServerPlayer target = null;
        for (ServerPlayer p : serverInstance.getPlayerList().getPlayers()) {
            if (!p.getName().getString().equalsIgnoreCase(name) || p == player) continue;
            target = p;
            break;
        }
        if (target == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "debug_teleport_player: player '" + name + "' not found (or is the bot)");
            return o;
        }
        ServerPlayer tgt = target;
        double lx = player.getLookAngle().x;
        double lz = player.getLookAngle().z;
        double len = Math.sqrt(lx * lx + lz * lz);
        if (len < 1.0E-6) {
            lx = 0.0;
            lz = 1.0;
            len = 1.0;
        }
        double tx = player.getX() + (lx /= len) * dist;
        double tz = player.getZ() + (lz /= len) * dist;
        int groundY = (int)player.getY();
        int y = 320;
        while ((double)y > player.getY() - 20.0) {
            BlockPos bp = BlockPos.containing((double)tx, (double)y, (double)tz);
            if (!level.getBlockState(bp).isAir() && level.getBlockState(bp.above()).isAir()) {
                groundY = y + 1;
                break;
            }
            --y;
        }
        double ty = (double)groundY + 0.0;
        tgt.teleportTo(tx, ty, tz);
        tgt.setYRot((float)Math.toDegrees(Math.atan2(-lx, lz)));
        tgt.setXRot(0.0f);
        System.out.println("[TP-DIAG] " + name + " -> (" + String.format("%.1f,%.1f,%.1f", tx, ty, tz) + ") botAt=(" + String.format("%.1f,%.1f,%.1f", player.getX(), player.getY(), player.getZ()) + ")");
        o.addProperty("detail", "debug_teleport_player " + name + " -> (" + String.format("%.1f,%.1f,%.1f", tx, ty, tz) + ") near bot");
        return o;
    }

    private JsonObject actDebugTeleportBot(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        double tz;
        double tx;
        ServerPlayer real = null;
        for (ServerPlayer p : serverInstance.getPlayerList().getPlayers()) {
            if (p == player) continue;
            real = p;
            break;
        }
        moveTarget = null;
        moveWaypoints = null;
        moveTicksLeft = 0;
        moveReached = false;
        if (req.has("x") && req.has("z")) {
            tx = req.get("x").getAsDouble();
            tz = req.get("z").getAsDouble();
        } else if (real != null) {
            tx = real.getX();
            tz = real.getZ() + 1.0;
        } else {
            tx = player.getX();
            tz = player.getZ();
        }
        int groundY = (int)player.getY();
        int y = 320;
        while ((double)y > player.getY() - 40.0) {
            BlockPos bp = BlockPos.containing((double)tx, (double)y, (double)tz);
            if (!level.getBlockState(bp).isAir() && level.getBlockState(bp.above()).isAir() && level.getBlockState(bp.above(2)).isAir()) {
                groundY = y + 1;
                break;
            }
            --y;
        }
        double ftx = tx;
        double fty = groundY;
        double ftz = tz;
        player.teleportTo(level, ftx, fty, ftz, Set.of(), 0.0f, 0.0f, true);
        player.setYRot(0.0f);
        player.setXRot(0.0f);
        o.addProperty("detail", "debug_teleport_bot -> (" + String.format("%.1f,%.1f,%.1f", tx, (double)groundY, tz) + ")");
        return o;
    }

    private static boolean placeAt(ServerPlayer player, ServerLevel level, int x, int y, int z, String itemName) {
        Block b;
        Direction[] dirOrder;
        String key;
        ItemStack s;
        int i;
        double dist = player.position().distanceTo(Vec3.atCenterOf((Vec3i)new BlockPos(x, y, z)));
        if (dist > 5.5) {
            return false;
        }
        Inventory inv = player.getInventory();
        int slot = -1;
        String search = itemName.replace("minecraft:", "").toLowerCase();
        for (i = 0; i < 9; ++i) {
            s = inv.getItem(i);
            if (s.isEmpty() || !(key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase()).contains(search)) continue;
            slot = i;
            break;
        }
        if (slot == -1) {
            for (i = 9; i < inv.getContainerSize(); ++i) {
                s = inv.getItem(i);
                if (s.isEmpty() || !(key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase()).contains(search)) continue;
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
        for (Direction dir : dirOrder = new Direction[]{Direction.UP, Direction.NORTH, Direction.SOUTH, Direction.EAST, Direction.WEST, Direction.DOWN}) {
            BlockPos neighbor = pos.relative(dir);
            BlockState ns = level.getBlockState(neighbor);
            boolean isPlayerAnchor = neighbor.equals(playerPos);
            if ((ns.isAir() || !ns.isSolid()) && !isPlayerAnchor) continue;
            BlockHitResult hit = new BlockHitResult(Vec3.atCenterOf((Vec3i)pos), dir.getOpposite(), neighbor, false);
            if (!player.gameMode.useItemOn(player, (Level)level, player.getMainHandItem(), InteractionHand.MAIN_HAND, hit).consumesAction()) continue;
            return true;
        }
        ItemStack held = player.getMainHandItem();
        if (!held.isEmpty() && (b = Block.byItem((Item)held.getItem())) != null && b != Blocks.AIR) {
            level.setBlock(pos, b.defaultBlockState(), 3);
            held.shrink(1);
            if (held.isEmpty()) {
                inv.setItem(slot, ItemStack.EMPTY);
            }
            player.containerMenu.broadcastChanges();
            return true;
        }
        return false;
    }

    private static boolean isHostile(String typeName) {
        String[] hostile;
        for (String h : hostile = new String[]{"zombie", "skeleton", "creeper", "spider", "phantom", "witch", "enderman", "blaze", "ghast", "slime", "magma_cube", "pillager", "vindicator", "evoker", "ravager", "hoglin", "piglin", "zoglin", "warden", "wither", "dragon"}) {
            if (!typeName.contains(h)) continue;
            return true;
        }
        return false;
    }

    private static CombatResult combat(ServerPlayer player, ServerLevel level, String mode, int maxTicks) {
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
                if (!CraftAgentBridge.isHostile(tn) || !((d = (double)e.distanceTo((Entity)player)) < minDist)) continue;
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
            CraftAgentBridge.equipBestWeapon(player);
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

    private static void equipBestWeapon(ServerPlayer player) {
        Inventory inv = player.getInventory();
        int best = -1;
        double bestDmg = -1.0;
        for (int i = 0; i < 9; ++i) {
            String key;
            ItemStack s = inv.getItem(i);
            if (s.isEmpty() || !(key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString()).contains("sword") && (!key.contains("axe") || key.contains("pickaxe"))) continue;
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

    private static void equipBestTool(ServerPlayer player, String blockId) {
        int tier;
        int i;
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
        for (i = 0; i < 9; ++i) {
            tier = CraftAgentBridge.toolTier(inv.getItem(i), toolType);
            if (tier <= bestTier) continue;
            bestTier = tier;
            best = i;
        }
        if (bestTier <= 0) {
            for (i = 9; i < inv.getContainerSize(); ++i) {
                tier = CraftAgentBridge.toolTier(inv.getItem(i), toolType);
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

    private static int toolTier(ItemStack stack, String toolType) {
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

    private static int craftItem(ServerPlayer player, String targetId, int want) {
        Inventory inv = player.getInventory();
        int crafted = 0;
        String t = targetId.toLowerCase();
        if (t.contains("planks") && CraftAgentBridge.countItem(inv, "log") > 0) {
            for (String log : new String[]{"oak_log", "birch_log", "spruce_log", "jungle_log", "acacia_log", "dark_oak_log", "mangrove_log", "cherry_log"}) {
                while (crafted < want && CraftAgentBridge.countItem(inv, log) > 0) {
                    CraftAgentBridge.removeItem(inv, log, 1);
                    String plank = log.replace("_log", "_planks");
                    CraftAgentBridge.addItem(inv, plank, 4);
                    crafted += 4;
                }
            }
        }
        if (t.contains("stick")) {
            while (crafted < want && CraftAgentBridge.countItem(inv, "planks") >= 2) {
                CraftAgentBridge.removeItem(inv, "planks", 2);
                CraftAgentBridge.addItem(inv, "stick", 4);
                crafted += 4;
            }
        }
        if (t.contains("crafting_table")) {
            while (crafted < want && CraftAgentBridge.countItem(inv, "planks") >= 4) {
                CraftAgentBridge.removeItem(inv, "planks", 4);
                CraftAgentBridge.addItem(inv, "crafting_table", 1);
                ++crafted;
            }
        }
        if (t.contains("wooden_pickaxe") || t.contains("wooden_axe")) {
            while (crafted < want && CraftAgentBridge.countItem(inv, "planks") >= 3 && CraftAgentBridge.countItem(inv, "stick") >= 2) {
                CraftAgentBridge.removeItem(inv, "planks", 3);
                CraftAgentBridge.removeItem(inv, "stick", 2);
                CraftAgentBridge.addItem(inv, t.contains("pickaxe") ? "wooden_pickaxe" : "wooden_axe", 1);
                ++crafted;
            }
        }
        if (t.contains("wooden_sword")) {
            while (crafted < want && CraftAgentBridge.countItem(inv, "planks") >= 2 && CraftAgentBridge.countItem(inv, "stick") >= 1) {
                CraftAgentBridge.removeItem(inv, "planks", 2);
                CraftAgentBridge.removeItem(inv, "stick", 1);
                CraftAgentBridge.addItem(inv, "wooden_sword", 1);
                ++crafted;
            }
        }
        if (t.contains("wooden_shovel")) {
            while (crafted < want && CraftAgentBridge.countItem(inv, "planks") >= 1 && CraftAgentBridge.countItem(inv, "stick") >= 2) {
                CraftAgentBridge.removeItem(inv, "planks", 1);
                CraftAgentBridge.removeItem(inv, "stick", 2);
                CraftAgentBridge.addItem(inv, "wooden_shovel", 1);
                ++crafted;
            }
        }
        if (t.contains("stone_pickaxe") || t.contains("stone_axe")) {
            while (crafted < want && CraftAgentBridge.countItem(inv, "cobblestone") >= 3 && CraftAgentBridge.countItem(inv, "stick") >= 2) {
                CraftAgentBridge.removeItem(inv, "cobblestone", 3);
                CraftAgentBridge.removeItem(inv, "stick", 2);
                CraftAgentBridge.addItem(inv, t.contains("pickaxe") ? "stone_pickaxe" : "stone_axe", 1);
                ++crafted;
            }
        }
        if (t.contains("stone_sword")) {
            while (crafted < want && CraftAgentBridge.countItem(inv, "cobblestone") >= 2 && CraftAgentBridge.countItem(inv, "stick") >= 1) {
                CraftAgentBridge.removeItem(inv, "cobblestone", 2);
                CraftAgentBridge.removeItem(inv, "stick", 1);
                CraftAgentBridge.addItem(inv, "stone_sword", 1);
                ++crafted;
            }
        }
        if (t.contains("torch")) {
            while (crafted < want && CraftAgentBridge.countItem(inv, "stick") >= 1 && CraftAgentBridge.countItem(inv, "coal") >= 1) {
                CraftAgentBridge.removeItem(inv, "stick", 1);
                CraftAgentBridge.removeItem(inv, "coal", 1);
                CraftAgentBridge.addItem(inv, "torch", 4);
                crafted += 4;
            }
        }
        if (t.contains("furnace")) {
            while (crafted < want && CraftAgentBridge.countItem(inv, "cobblestone") >= 8) {
                CraftAgentBridge.removeItem(inv, "cobblestone", 8);
                CraftAgentBridge.addItem(inv, "furnace", 1);
                ++crafted;
            }
        }
        if (t.contains("chest")) {
            while (crafted < want && CraftAgentBridge.countItem(inv, "planks") >= 8) {
                CraftAgentBridge.removeItem(inv, "planks", 8);
                CraftAgentBridge.addItem(inv, "chest", 1);
                ++crafted;
            }
        }
        if (t.contains("iron_pickaxe") || t.contains("iron_axe")) {
            while (crafted < want && CraftAgentBridge.countItem(inv, "iron_ingot") >= 3 && CraftAgentBridge.countItem(inv, "stick") >= 2) {
                CraftAgentBridge.removeItem(inv, "iron_ingot", 3);
                CraftAgentBridge.removeItem(inv, "stick", 2);
                CraftAgentBridge.addItem(inv, t.contains("pickaxe") ? "iron_pickaxe" : "iron_axe", 1);
                ++crafted;
            }
        }
        if (t.contains("iron_sword")) {
            while (crafted < want && CraftAgentBridge.countItem(inv, "iron_ingot") >= 2 && CraftAgentBridge.countItem(inv, "stick") >= 1) {
                CraftAgentBridge.removeItem(inv, "iron_ingot", 2);
                CraftAgentBridge.removeItem(inv, "stick", 1);
                CraftAgentBridge.addItem(inv, "iron_sword", 1);
                ++crafted;
            }
        }
        if (t.contains("diamond_pickaxe") || t.contains("diamond_axe")) {
            while (crafted < want && CraftAgentBridge.countItem(inv, "diamond") >= 3 && CraftAgentBridge.countItem(inv, "stick") >= 2) {
                CraftAgentBridge.removeItem(inv, "diamond", 3);
                CraftAgentBridge.removeItem(inv, "stick", 2);
                CraftAgentBridge.addItem(inv, t.contains("pickaxe") ? "diamond_pickaxe" : "diamond_axe", 1);
                ++crafted;
            }
        }
        if (t.contains("diamond_sword")) {
            while (crafted < want && CraftAgentBridge.countItem(inv, "diamond") >= 2 && CraftAgentBridge.countItem(inv, "stick") >= 1) {
                CraftAgentBridge.removeItem(inv, "diamond", 2);
                CraftAgentBridge.removeItem(inv, "stick", 1);
                CraftAgentBridge.addItem(inv, "diamond_sword", 1);
                ++crafted;
            }
        }
        if (t.contains("iron_helmet")) {
            while (crafted < want && CraftAgentBridge.countItem(inv, "iron_ingot") >= 5) {
                CraftAgentBridge.removeItem(inv, "iron_ingot", 5);
                CraftAgentBridge.addItem(inv, "iron_helmet", 1);
                ++crafted;
            }
        }
        if (t.contains("iron_chestplate")) {
            while (crafted < want && CraftAgentBridge.countItem(inv, "iron_ingot") >= 8) {
                CraftAgentBridge.removeItem(inv, "iron_ingot", 8);
                CraftAgentBridge.addItem(inv, "iron_chestplate", 1);
                ++crafted;
            }
        }
        if (t.contains("iron_leggings")) {
            while (crafted < want && CraftAgentBridge.countItem(inv, "iron_ingot") >= 7) {
                CraftAgentBridge.removeItem(inv, "iron_ingot", 7);
                CraftAgentBridge.addItem(inv, "iron_leggings", 1);
                ++crafted;
            }
        }
        if (t.contains("iron_boots")) {
            while (crafted < want && CraftAgentBridge.countItem(inv, "iron_ingot") >= 4) {
                CraftAgentBridge.removeItem(inv, "iron_ingot", 4);
                CraftAgentBridge.addItem(inv, "iron_boots", 1);
                ++crafted;
            }
        }
        if (t.contains("diamond_helmet")) {
            while (crafted < want && CraftAgentBridge.countItem(inv, "diamond") >= 5) {
                CraftAgentBridge.removeItem(inv, "diamond", 5);
                CraftAgentBridge.addItem(inv, "diamond_helmet", 1);
                ++crafted;
            }
        }
        if (t.contains("diamond_chestplate")) {
            while (crafted < want && CraftAgentBridge.countItem(inv, "diamond") >= 8) {
                CraftAgentBridge.removeItem(inv, "diamond", 8);
                CraftAgentBridge.addItem(inv, "diamond_chestplate", 1);
                ++crafted;
            }
        }
        if (t.contains("diamond_leggings")) {
            while (crafted < want && CraftAgentBridge.countItem(inv, "diamond") >= 7) {
                CraftAgentBridge.removeItem(inv, "diamond", 7);
                CraftAgentBridge.addItem(inv, "diamond_leggings", 1);
                ++crafted;
            }
        }
        if (t.contains("diamond_boots")) {
            while (crafted < want && CraftAgentBridge.countItem(inv, "diamond") >= 4) {
                CraftAgentBridge.removeItem(inv, "diamond", 4);
                CraftAgentBridge.addItem(inv, "diamond_boots", 1);
                ++crafted;
            }
        }
        if (t.contains("shield")) {
            while (crafted < want && CraftAgentBridge.countItem(inv, "planks") >= 6 && CraftAgentBridge.countItem(inv, "iron_ingot") >= 1) {
                CraftAgentBridge.removeItem(inv, "planks", 6);
                CraftAgentBridge.removeItem(inv, "iron_ingot", 1);
                CraftAgentBridge.addItem(inv, "shield", 1);
                ++crafted;
            }
        }
        return crafted;
    }

    private static int discardItem(ServerPlayer player, String itemId, int num) {
        Inventory inv = player.getInventory();
        int discarded = 0;
        String search = itemId.toLowerCase();
        for (int i = 0; i < inv.getContainerSize() && discarded < num; ++i) {
            String key;
            ItemStack s = inv.getItem(i);
            if (s.isEmpty() || !(key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase()).contains(search)) continue;
            int take = Math.min(s.getCount(), num - discarded);
            s.shrink(take);
            discarded += take;
        }
        return discarded;
    }

    private static int smeltItem(ServerPlayer player, String itemId, int num) {
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
        while (smelted < num && CraftAgentBridge.countItem(inv, input) >= 1 && CraftAgentBridge.countItem(inv, "coal") >= 1) {
            CraftAgentBridge.removeItem(inv, input, 1);
            CraftAgentBridge.removeItem(inv, "coal", 1);
            CraftAgentBridge.addItem(inv, output, 1);
            ++smelted;
        }
        return smelted;
    }

    private static boolean matchesWhitelist(String id) {
        String lower = id.toLowerCase();
        for (String k : BLOCK_WHITELIST) {
            if (!lower.contains(k)) continue;
            return true;
        }
        return false;
    }

    private static JsonArray arr(double x, double y, double z) {
        JsonArray a = new JsonArray();
        a.add((Number)x);
        a.add((Number)y);
        a.add((Number)z);
        return a;
    }

    private static float clamp(float v, float lo, float hi) {
        return Math.max(lo, Math.min(hi, v));
    }

    private static double clamp(double v, double lo, double hi) {
        return Math.max(lo, Math.min(hi, v));
    }

    private static int countItem(Inventory inv, String id) {
        String search = id.toLowerCase();
        int n = 0;
        for (int i = 0; i < inv.getContainerSize(); ++i) {
            String key;
            ItemStack s = inv.getItem(i);
            if (s.isEmpty() || !(key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase()).endsWith(":" + search) && (search.contains(":") || !key.contains(search))) continue;
            n += s.getCount();
        }
        return n;
    }

    private static void addItem(Inventory inv, String id, int count) {
        Item target;
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
        Item item = target = exact != null ? exact : fallback;
        if (target != null) {
            inv.add(new ItemStack(target, count));
        }
    }

    private static void removeItem(Inventory inv, String id, int count) {
        String search = id.toLowerCase();
        for (int i = 0; i < inv.getContainerSize() && count > 0; ++i) {
            String key;
            ItemStack s = inv.getItem(i);
            if (s.isEmpty() || !(key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase()).endsWith(":" + search) && !key.contains(search)) continue;
            int take = Math.min(s.getCount(), count);
            s.shrink(take);
            count -= take;
        }
    }

    static {
        String[] keys;
        GSON = new Gson();
        fakePlayer = null;
        moveTarget = null;
        moveTicksLeft = 0;
        moveReached = false;
        moveFinalDist = 0.0;
        moveStuck = false;
        moveStuckCounter = 0;
        moveWaypoints = null;
        moveCurrentWpIndex = 0;
        lastPos = null;
        noProgressTicks = 0;
        moveSprinting = false;
        shouldStop = false;
        currentGoal = null;
        fakePlayerSpawning = false;
        BLOCK_WHITELIST = new HashSet<String>();
        for (String k : keys = new String[]{"log", "planks", "crafting_table", "chest", "furnace", "smoker", "blast_furnace", "stone", "cobblestone", "ore", "coal", "iron", "gold", "diamond", "dirt", "grass", "sand", "gravel", "sandstone", "nether", "end_", "amethyst", "copper", "lapis", "emerald", "redstone", "deepslate", "oak", "birch", "spruce", "jungle", "acacia", "dark_oak", "mangrove", "bamboo", "obsidian", "glowstone", "ice", "clay", "wart", "water", "lava", "magma", "bedrock", "terracotta", "concrete", "bricks", "netherrack", "end_stone", "snow_block", "snow", "podzol", "mycelium", "coarse_dirt", "rooted_dirt", "moss_block", "tuff", "calcite", "dripstone", "basalt", "blackstone", "nylium", "shroomlight", "packed_ice", "blue_ice", "mud", "soul_sand", "soul_soil", "glass", "wool", "carpet", "bookshelf", "lectern", "lantern", "torch", "wall", "stairs", "slab", "fence", "door", "trapdoor", "bed", "banner", "flower_pot", "anvil", "grindstone", "stonecutter", "loom", "barrel", "composter", "beehive", "beacon", "conduit", "enchanting_table", "jukebox", "note_block", "observer", "piston", "dispenser", "dropper", "hopper"}) {
            BLOCK_WHITELIST.add(k);
        }
        autoSurviveCooldown = 0;
        autoSurviveAttackCd = 0;
    }

    public static class EntityPlayerMPFake
    extends ServerPlayer {
        public EntityPlayerMPFake(MinecraftServer server, ServerLevel worldIn, GameProfile profile, ClientInformation cli) {
            super(server, worldIn, profile, cli);
        }

        public void tick() {
            if (this.level().getServer().getTickCount() % 10 == 0) {
                this.connection.resetPosition();
                this.level().getChunkSource().move((ServerPlayer)this);
            }
            try {
                super.tick();
                this.doTick();
            }
            catch (NullPointerException nullPointerException) {
                // empty catch block
            }
        }

        public void unsetRemovedPublic() {
            super.unsetRemoved();
        }
    }

    private static class CombatResult {
        String result = "none";
        String targetType = "";

        private CombatResult() {
        }
    }
}
