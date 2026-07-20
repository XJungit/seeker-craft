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
import com.google.gson.Gson;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
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
import net.minecraft.core.Vec3i;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ClientboundEntityPositionSyncPacket;
import net.minecraft.network.protocol.game.ClientboundPlayerInfoUpdatePacket;
import net.minecraft.network.protocol.game.ClientboundRotateHeadPacket;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
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
    static volatile MinecraftServer serverInstance;
    static volatile FakePlayerManager.EntityPlayerMPFake fakePlayer;
    static volatile double[] moveTarget;
    static volatile int moveTicksLeft;
    static volatile boolean moveReached;
    static volatile double moveFinalDist;
    static volatile boolean moveStuck;
    static volatile int moveStuckCounter;
    static volatile List<Vec3> moveWaypoints;
    static volatile int moveCurrentWpIndex;
    static volatile double[] lastPos;
    static volatile int noProgressTicks;
    static volatile boolean moveSprinting;
    static volatile boolean shouldStop;
    static volatile String currentGoal;
    static volatile boolean fakePlayerSpawning;
    static final Set<String> BLOCK_WHITELIST;
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
        COMMAND_HANDLERS.put("look", InteractionController::actLook);
        COMMAND_HANDLERS.put("look_abs", InteractionController::actLookAbs);
        COMMAND_HANDLERS.put("look_at", InteractionController::actLookAt);
        COMMAND_HANDLERS.put("dig_at", InteractionController::actDigAt);
        COMMAND_HANDLERS.put("place_at", InteractionController::actPlaceAt);
        COMMAND_HANDLERS.put("get_block", InteractionController::actGetBlock);
        COMMAND_HANDLERS.put("get_blocks", InteractionController::actGetBlocks);
        COMMAND_HANDLERS.put("clear_chat", this::actClearChat);
        COMMAND_HANDLERS.put("debug_spawn", DebugController::actDebugSpawn);
        COMMAND_HANDLERS.put("debug_give", DebugController::actDebugGive);
        COMMAND_HANDLERS.put("debug_damage", DebugController::actDebugDamage);
        COMMAND_HANDLERS.put("debug_heal", DebugController::actDebugHeal);
        COMMAND_HANDLERS.put("debug_clear", DebugController::actDebugClear);
        COMMAND_HANDLERS.put("debug_place", DebugController::actDebugPlace);
        COMMAND_HANDLERS.put("debug_xp", DebugController::actDebugXp);
        COMMAND_HANDLERS.put("debug_food", DebugController::actDebugFood);
        COMMAND_HANDLERS.put("debug_time", DebugController::actDebugTime);
        COMMAND_HANDLERS.put("debug_teleport_player", DebugController::actDebugTeleportPlayer);
        COMMAND_HANDLERS.put("debug_teleport_bot", DebugController::actDebugTeleportBot);
        COMMAND_HANDLERS.put("attack", InteractionController::actAttack);
        COMMAND_HANDLERS.put("enchant", ContainerController::actEnchant);
        COMMAND_HANDLERS.put("select_slot", ContainerController::actSelectSlot);
        COMMAND_HANDLERS.put("move_to_hotbar", ContainerController::actMoveToHotbar);
        COMMAND_HANDLERS.put("move_slot", ContainerController::actMoveSlot);
        COMMAND_HANDLERS.put("craft", ContainerController::actCraft);
        COMMAND_HANDLERS.put("discard", ContainerController::actDiscard);
        COMMAND_HANDLERS.put("smelt", ContainerController::actSmelt);
        COMMAND_HANDLERS.put("inspect_gui", ContainerController::actInspectGui);
        COMMAND_HANDLERS.put("close_gui", ContainerController::actCloseGui);
        COMMAND_HANDLERS.put("transfer", ContainerController::actTransfer);
        COMMAND_HANDLERS.put("equip_item", ContainerController::actEquipItem);
        COMMAND_HANDLERS.put("drop_items", ContainerController::actDropItems);
        COMMAND_HANDLERS.put("list_players", this::actListPlayers);
        COMMAND_HANDLERS.put("stop", this::actStop);
        COMMAND_HANDLERS.put("set_goal", this::actSetGoal);
        COMMAND_HANDLERS.put("get_goal", this::actGetGoal);
        COMMAND_HANDLERS.put("search_wiki", this::actSearchWiki);
        COMMAND_HANDLERS.put("look_at_player", InteractionController::actLookAtPlayer);
        COMMAND_HANDLERS.put("look_at_position", InteractionController::actLookAtPosition);
        COMMAND_HANDLERS.put("get_crafting_plan", ContainerController::actGetCraftingPlan);
        COMMAND_HANDLERS.put("villager_trades", this::actVillagerTrades);
        COMMAND_HANDLERS.put("trade_with_villager", this::actTradeWithVillager);
        COMMAND_HANDLERS.put("activate_block", InteractionController::actActivateBlock);
        COMMAND_HANDLERS.put("use_on_entity", InteractionController::actUseOnEntity);
        COMMAND_HANDLERS.put("fish", this::actFish);
        COMMAND_HANDLERS.put("ride", this::actRide);
        COMMAND_HANDLERS.put("sleep", this::actSleep);
        COMMAND_HANDLERS.put("wake", this::actWake);
        COMMAND_HANDLERS.put("activate_nearest_block", InteractionController::actActivateNearestBlock);
        COMMAND_HANDLERS.put("build_portal", this::actBuildPortal);
        COMMAND_HANDLERS.put("teleport_to", this::actTeleportTo);
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
                FakePlayerManager.createFakePlayer();
            });
        });
        ServerLifecycleEvents.SERVER_STOPPING.register(server -> {
            serverInstance = null;
        });
        ServerTickEvents.START_SERVER_TICK.register(this::onStartServerTick);
        ServerTickEvents.END_SERVER_TICK.register(this::onEndServerTick);
        System.out.println("[craft-agent-bridge] ServerTickEvents.START+END_SERVER_TICK \u5df2\u6ce8\u518c\uff08\u53cc tick + Carpet \u540c\u6b65\uff09");
    }

    private void onStartServerTick(MinecraftServer server) {
        double ddz;
        if (moveWaypoints == null) {
            return;
        }
        ServerPlayer player = FakePlayerManager.getFirstPlayer(server);
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
        boolean inWater = MovementController.isInWater(player);
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
        ServerPlayer survPlayer = FakePlayerManager.getFirstPlayer(server);
        if (survPlayer != null) {
            CraftAgentBridge.autoSurvive(survPlayer, server);
        }
        if (moveWaypoints == null) {
            return;
        }
        ServerPlayer player = FakePlayerManager.getFirstPlayer(server);
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
        boolean inWater = MovementController.isInWater(player);
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
                    InventoryHelper.placeAt(player, level, below.getX(), below.getY(), below.getZ(), itemId);
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
            if (!InventoryHelper.isHostile(tn) || !((d = (double)e.distanceTo((Entity)player)) < minDist)) continue;
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
            InventoryHelper.equipBestWeapon(player);
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

    static ServerPlayer getFirstPlayer(MinecraftServer server) {
        return FakePlayerManager.getFirstPlayer(server);
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

    static <T> T onServer(Supplier<T> task) {
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
            return CraftAgentBridge.runOnServerThread(StateBuilder::buildState);
        }
        if ("move_to".equals(type)) {
            return MovementController.performMoveTo(req);
        }
        if ("go_to_player".equals(type)) {
            return MovementController.performGoToPlayer(req);
        }
        if ("give_player".equals(type)) {
            return this.performGivePlayer(req);
        }
        if ("discard_smart".equals(type)) {
            return MovementController.performDiscardSmart(req);
        }
        if ("collect_items".equals(type)) {
            return MovementController.performCollectItems(req);
        }
        if ("attack_player".equals(type)) {
            return MovementController.performAttackPlayer(req);
        }
        if ("follow_player".equals(type)) {
            return MovementController.performFollowPlayer(req);
        }
        if ("combat".equals(type)) {
            return MovementController.performCombat(req);
        }
        if ("use_item".equals(type)) {
            return MovementController.performUseItem(req);
        }
        if ("eat_item".equals(type)) {
            return MovementController.performEatItem(req);
        }
        if ("pillar_up".equals(type)) {
            return MovementController.performPillarUp(req);
        }
        if ("wait".equals(type)) {
            return MovementController.performWait(req);
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
            ServerPlayer p = FakePlayerManager.getFirstPlayer(serverInstance);
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
            ServerPlayer p = FakePlayerManager.getFirstPlayer(serverInstance);
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

    private JsonObject performAction(String type, JsonObject req) {
        MinecraftServer server = serverInstance;
        if (server == null) {
            JsonObject o = new JsonObject();
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u670d\u52a1\u5668\u672a\u5c31\u7eea");
            return o;
        }
        ServerPlayer player = FakePlayerManager.getFirstPlayer(server);
        if (player == null) {
            JsonObject o = new JsonObject();
            o.addProperty("status", "fail");
            o.addProperty("detail", "\u6ca1\u6709\u5728\u7ebf\u73a9\u5bb6");
            return o;
        }
        ServerLevel level = player.level();
        // 命令分派表：每个命令由 COMMAND_HANDLERS 中的 act* 方法处理。
        CommandHandler handler = COMMAND_HANDLERS.get(type);
        if (handler != null) {
            return handler.handle(player, level, req);
        }
        JsonObject o = new JsonObject();
        o.addProperty("status", "fail");
        o.addProperty("detail", "\u672a\u77e5\u547d\u4ee4: " + type);
        return o;
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 命令处理器：从 performAction 的 switch case 抽出，逐个迁移到此。
    // 约定：每个 act* 自建 o(status=ok)，返回 JsonObject；失败 addProperty("status","fail")。
    // ═════════════════════════════════════════════════════════════════════════

    private JsonObject actClearChat(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        o.addProperty("detail", "clear_chat: mod side ack, Rust side should clear history");
        return o;
    }

    private JsonObject actListPlayers(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        JsonArray players = new JsonArray();
        for (ServerPlayer p : serverInstance.getPlayerList().getPlayers()) {
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

    private JsonObject actStop(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        shouldStop = true;
        moveTarget = null;
        o.addProperty("detail", "stop: all actions cancelled");
        return o;
    }

    private JsonObject actSetGoal(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
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

    private JsonObject actGetGoal(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        o.addProperty("goal", currentGoal != null ? currentGoal : "(none)");
        o.addProperty("detail", "get_goal: " + (currentGoal != null ? currentGoal : "none"));
        return o;
    }

    private JsonObject actSearchWiki(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
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

    private JsonObject actVillagerTrades(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private JsonObject actTradeWithVillager(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private JsonObject actFish(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private JsonObject actRide(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private JsonObject actSleep(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private JsonObject actWake(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        player.stopSleeping();
        o.addProperty("detail", "wake (was sleeping=false)");
        return o;
    }

    private JsonObject actBuildPortal(ServerPlayer player, ServerLevel level, JsonObject req) {
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
            if (!existing.isAir() || !InventoryHelper.placeAt(player, level, bx, by, bz, search)) continue;
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

    private JsonObject actTeleportTo(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        ServerLevel targetLevel;
        String dimension = req.has("dimension") ? req.get("dimension").getAsString() : "the_nether";
        switch (dimension.toLowerCase()) {
            case "the_nether":
            case "nether": {
                targetLevel = serverInstance.getLevel(Level.NETHER);
                break;
            }
            case "the_end":
            case "end": {
                targetLevel = serverInstance.getLevel(Level.END);
                break;
            }
            default: {
                targetLevel = serverInstance.getLevel(Level.OVERWORLD);
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




    static JsonArray arr(double x, double y, double z) {
        JsonArray a = new JsonArray();
        a.add((Number)x);
        a.add((Number)y);
        a.add((Number)z);
        return a;
    }

    static float clamp(float v, float lo, float hi) {
        return Math.max(lo, Math.min(hi, v));
    }

    static double clamp(double v, double lo, double hi) {
        return Math.max(lo, Math.min(hi, v));
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

}
