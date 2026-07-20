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
        COMMAND_HANDLERS.put("look", this::actLook);
        COMMAND_HANDLERS.put("look_abs", this::actLookAbs);
        COMMAND_HANDLERS.put("look_at", this::actLookAt);
        COMMAND_HANDLERS.put("dig_at", this::actDigAt);
        COMMAND_HANDLERS.put("place_at", this::actPlaceAt);
        COMMAND_HANDLERS.put("get_block", this::actGetBlock);
        COMMAND_HANDLERS.put("get_blocks", this::actGetBlocks);
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
        COMMAND_HANDLERS.put("attack", this::actAttack);
        COMMAND_HANDLERS.put("enchant", this::actEnchant);
        COMMAND_HANDLERS.put("select_slot", this::actSelectSlot);
        COMMAND_HANDLERS.put("move_to_hotbar", this::actMoveToHotbar);
        COMMAND_HANDLERS.put("move_slot", this::actMoveSlot);
        COMMAND_HANDLERS.put("craft", this::actCraft);
        COMMAND_HANDLERS.put("discard", this::actDiscard);
        COMMAND_HANDLERS.put("smelt", this::actSmelt);
        COMMAND_HANDLERS.put("inspect_gui", this::actInspectGui);
        COMMAND_HANDLERS.put("close_gui", this::actCloseGui);
        COMMAND_HANDLERS.put("transfer", this::actTransfer);
        COMMAND_HANDLERS.put("equip_item", this::actEquipItem);
        COMMAND_HANDLERS.put("drop_items", this::actDropItems);
        COMMAND_HANDLERS.put("list_players", this::actListPlayers);
        COMMAND_HANDLERS.put("stop", this::actStop);
        COMMAND_HANDLERS.put("set_goal", this::actSetGoal);
        COMMAND_HANDLERS.put("get_goal", this::actGetGoal);
        COMMAND_HANDLERS.put("search_wiki", this::actSearchWiki);
        COMMAND_HANDLERS.put("look_at_player", this::actLookAtPlayer);
        COMMAND_HANDLERS.put("look_at_position", this::actLookAtPosition);
        COMMAND_HANDLERS.put("get_crafting_plan", this::actGetCraftingPlan);
        COMMAND_HANDLERS.put("villager_trades", this::actVillagerTrades);
        COMMAND_HANDLERS.put("trade_with_villager", this::actTradeWithVillager);
        COMMAND_HANDLERS.put("activate_block", this::actActivateBlock);
        COMMAND_HANDLERS.put("use_on_entity", this::actUseOnEntity);
        COMMAND_HANDLERS.put("fish", this::actFish);
        COMMAND_HANDLERS.put("ride", this::actRide);
        COMMAND_HANDLERS.put("sleep", this::actSleep);
        COMMAND_HANDLERS.put("wake", this::actWake);
        COMMAND_HANDLERS.put("activate_nearest_block", this::actActivateNearestBlock);
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
        InventoryHelper.equipBestTool(player, blockId);
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
        boolean placed = InventoryHelper.placeAt(player, level, tx, ty, tz, item);
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

    private JsonObject actAttack(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private JsonObject actEnchant(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private JsonObject actSelectSlot(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private JsonObject actMoveToHotbar(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private JsonObject actMoveSlot(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
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

    private JsonObject actCraft(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private JsonObject actDiscard(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String item = req.get("item").getAsString();
        int num = req.has("num") ? req.get("num").getAsInt() : 1;
        int discarded = InventoryHelper.discardItem(player, item, num);
        player.containerMenu.broadcastChanges();
        o.addProperty("detail", "discarded " + discarded + " x " + item);
        return o;
    }

    private JsonObject actSmelt(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String item = req.get("item").getAsString();
        int num = req.has("num") ? req.get("num").getAsInt() : 1;
        int smelted = CraftingHelper.smeltItem(player, item, num);
        player.containerMenu.broadcastChanges();
        o.addProperty("detail", "smelted " + smelted + " x " + item);
        return o;
    }

    private JsonObject actInspectGui(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private JsonObject actCloseGui(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private JsonObject actTransfer(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private JsonObject actEquipItem(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private JsonObject actDropItems(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private JsonObject actLookAtPlayer(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        String targetName = req.has("player_name") ? req.get("player_name").getAsString() : "";
        ServerPlayer target = null;
        for (ServerPlayer p : serverInstance.getPlayerList().getPlayers()) {
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

    private JsonObject actLookAtPosition(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private JsonObject actGetCraftingPlan(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private JsonObject actActivateBlock(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private JsonObject actUseOnEntity(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private JsonObject actActivateNearestBlock(ServerPlayer player, ServerLevel level, JsonObject req) {
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

    private static float clamp(float v, float lo, float hi) {
        return Math.max(lo, Math.min(hi, v));
    }

    private static double clamp(double v, double lo, double hi) {
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
