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
        COMMAND_HANDLERS.put("clear_chat", MetaController::actClearChat);
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
        COMMAND_HANDLERS.put("list_players", MetaController::actListPlayers);
        COMMAND_HANDLERS.put("stop", MetaController::actStop);
        COMMAND_HANDLERS.put("set_goal", MetaController::actSetGoal);
        COMMAND_HANDLERS.put("get_goal", MetaController::actGetGoal);
        COMMAND_HANDLERS.put("search_wiki", MetaController::actSearchWiki);
        COMMAND_HANDLERS.put("look_at_player", InteractionController::actLookAtPlayer);
        COMMAND_HANDLERS.put("look_at_position", InteractionController::actLookAtPosition);
        COMMAND_HANDLERS.put("get_crafting_plan", ContainerController::actGetCraftingPlan);
        COMMAND_HANDLERS.put("villager_trades", EntityInteractionController::actVillagerTrades);
        COMMAND_HANDLERS.put("trade_with_villager", EntityInteractionController::actTradeWithVillager);
        COMMAND_HANDLERS.put("activate_block", InteractionController::actActivateBlock);
        COMMAND_HANDLERS.put("use_on_entity", InteractionController::actUseOnEntity);
        COMMAND_HANDLERS.put("fish", EntityInteractionController::actFish);
        COMMAND_HANDLERS.put("ride", EntityInteractionController::actRide);
        COMMAND_HANDLERS.put("sleep", EntityInteractionController::actSleep);
        COMMAND_HANDLERS.put("wake", EntityInteractionController::actWake);
        COMMAND_HANDLERS.put("activate_nearest_block", InteractionController::actActivateNearestBlock);
        COMMAND_HANDLERS.put("build_portal", EntityInteractionController::actBuildPortal);
        COMMAND_HANDLERS.put("teleport_to", MetaController::actTeleportTo);
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
