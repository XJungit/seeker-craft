package com.craftagent.bridge;

import com.google.gson.Gson;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import net.fabricmc.api.DedicatedServerModInitializer;
import net.fabricmc.fabric.api.event.lifecycle.v1.ServerTickEvents;
import net.fabricmc.fabric.api.event.lifecycle.v1.ServerLifecycleEvents;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.commands.arguments.EntityAnchorArgument;
import net.minecraft.world.effect.MobEffect;
import net.minecraft.world.effect.MobEffectInstance;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.level.LightLayer;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.HitResult;
import net.minecraft.world.phys.Vec3;

import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.io.PrintWriter;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.util.HashSet;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.TimeUnit;
import java.util.function.Supplier;

/**
 * Craft-Agent Bridge —— 服务端 Fabric mod（ServerPlayer 架构）。
 *
 * 核心改变：从客户端 LocalPlayer + KeyMapping 模拟按键，改为服务端 ServerPlayer 原生 API。
 * 优势：
 *   - 所有操作走服务端原生代码路径，天然同步，无需发网络包给服务端
 *   - destroyBlock/useItemOn/attack/setSelectedSlot 都是服务端直接执行
 *   - 移动用 setDeltaMovement + ServerTickEvents，服务端 tick 处理移动和同步
 *   - 无焦点/KeyMapping/releaseAll 问题
 *
 * TCP 协议不变（127.0.0.1:25567），新增 get_block/get_blocks 供 Rust 侧 A* 寻路。
 */
public class CraftAgentBridge implements DedicatedServerModInitializer {
    public static final int PORT = 25567;
    private static final int SCAN_RADIUS = 16;
    private static final Gson GSON = new Gson();

    /** 全局 MinecraftServer 引用（onInitializeServer 时保存）。 */
    private static volatile MinecraftServer serverInstance;

    // ═══ 移动状态（TCP 线程设置，服务端 tick 读取+清除）═══
    private static volatile double[] moveTarget = null;
    private static volatile int moveTicksLeft = 0;
    private static volatile boolean moveReached = false;
    private static volatile double moveFinalDist = 0;
    private static volatile boolean moveStuck = false;

    // ═══ 全局停止标志（stop 命令设置，长耗时动作检查）═══
    private static volatile boolean shouldStop = false;

    // ═══ 持续目标（set_goal 命令设置，SelfPrompter 读取）═══
    private static volatile String currentGoal = null;

    private static final Set<String> BLOCK_WHITELIST = new HashSet<>();

    static {
        String[] keys = {
            "log", "planks", "crafting_table", "chest", "furnace", "smoker", "blast_furnace",
            "stone", "cobblestone", "ore", "coal", "iron", "gold", "diamond", "dirt", "grass",
            "sand", "gravel", "sandstone", "nether", "end_", "amethyst", "copper", "lapis",
            "emerald", "redstone", "deepslate", "oak", "birch", "spruce", "jungle", "acacia",
            "dark_oak", "mangrove", "bamboo", "obsidian", "glowstone", "ice", "clay", "wart",
            "water", "lava", "magma",
            "bedrock", "terracotta", "concrete", "bricks", "netherrack", "end_stone",
            "snow_block", "snow", "podzol", "mycelium", "coarse_dirt", "rooted_dirt",
            "moss_block", "tuff", "calcite", "dripstone", "basalt", "blackstone", "nylium",
            "shroomlight", "packed_ice", "blue_ice", "mud", "soul_sand", "soul_soil",
            "glass", "wool", "carpet", "bookshelf", "lectern", "lantern", "torch",
            "wall", "stairs", "slab", "fence", "door", "trapdoor", "bed", "banner",
            "flower_pot", "anvil", "grindstone", "stonecutter", "loom", "barrel",
            "composter", "beehive", "beacon", "conduit", "enchanting_table", "jukebox",
            "note_block", "observer", "piston", "dispenser", "dropper", "hopper"
        };
        for (String k : keys) BLOCK_WHITELIST.add(k);
    }

    @Override
    public void onInitializeServer() {
        Thread serverThread = new Thread(this::runServer, "craft-agent-bridge");
        serverThread.setDaemon(true);
        serverThread.start();
        System.out.println("[craft-agent-bridge] 服务端 TCP 线程已启动，监听 127.0.0.1:" + PORT);

        // 服务器启动时保存 serverInstance（onInitializeServer 没有 server 参数）
        ServerLifecycleEvents.SERVER_STARTED.register(server -> {
            serverInstance = server;
            System.out.println("[craft-agent-bridge] MinecraftServer 已绑定（ServerPlayer 架构就绪）");
        });
        ServerLifecycleEvents.SERVER_STOPPING.register(server -> {
            serverInstance = null;
        });

        // 服务端 tick：处理移动（setDeltaMovement + 朝向）
        ServerTickEvents.START_SERVER_TICK.register(this::onServerTick);
        System.out.println("[craft-agent-bridge] ServerTickEvents 已注册");
    }

    /** 服务端每 tick 调用：处理移动目标。
     *  ServerPlayer 的 tick() 会在本 tick 后执行，应用 setDeltaMovement 移动玩家，
     *  然后 ServerGamePacketListenerImpl 自动同步位置给客户端。 */
    private void onServerTick(MinecraftServer server) {
        if (moveTarget == null) return;
        ServerPlayer player = getFirstPlayer(server);
        if (player == null) { moveTarget = null; return; }

        double tx = moveTarget[0], tz = moveTarget[2];
        double ddx = tx - player.getX(), ddz = tz - player.getZ();
        double horiz = Math.sqrt(ddx * ddx + ddz * ddz);
        moveFinalDist = horiz;
        moveTicksLeft--;

        if (horiz < 1.5 || moveTicksLeft <= 0) {
            moveReached = horiz < 1.5;
            moveTarget = null;
            // 停止移动
            player.setDeltaMovement(0, player.getDeltaMovement().y, 0);
            return;
        }

        // 设置朝向
        float yaw = (float) Math.toDegrees(Math.atan2(-ddx, ddz));
        player.setYRot(yaw);

        // 直接设置速度（服务端原生，不需要 KeyMapping）
        double speed = 0.28;
        double nx = ddx / horiz;
        double nz = ddz / horiz;
        player.setDeltaMovement(nx * speed, player.getDeltaMovement().y, nz * speed);

        // 障碍检测：水平碰撞且在地面时跳跃
        if (player.horizontalCollision && player.onGround()) {
            player.setDeltaMovement(player.getDeltaMovement().x, 0.42, player.getDeltaMovement().z);
            moveStuck = true;
        } else {
            moveStuck = false;
        }
    }

    /** 获取第一个在线玩家（单人游戏中只有一个）。 */
    private static ServerPlayer getFirstPlayer(MinecraftServer server) {
        var players = server.getPlayerList().getPlayers();
        return players.isEmpty() ? null : players.get(0);
    }

    /** 在服务端主线程执行任务，同步等待结果（线程安全）。 */
    private static JsonObject runOnServerThread(Supplier<JsonObject> task) {
        MinecraftServer server = serverInstance;
        if (server == null) {
            JsonObject err = new JsonObject();
            err.addProperty("status", "fail");
            err.addProperty("detail", "服务器未就绪");
            return err;
        }
        CompletableFuture<JsonObject> future = new CompletableFuture<>();
        server.executeIfPossible(() -> {
            try {
                future.complete(task.get());
            } catch (Exception e) {
                JsonObject err = new JsonObject();
                err.addProperty("status", "fail");
                err.addProperty("detail", e.getMessage());
                future.complete(err);
            }
        });
        try {
            return future.get(30, TimeUnit.SECONDS);
        } catch (Exception e) {
            JsonObject err = new JsonObject();
            err.addProperty("status", "fail");
            err.addProperty("detail", "服务端线程超时: " + e.getMessage());
            return err;
        }
    }

    private void runServer() {
        try (ServerSocket server = new ServerSocket(PORT, 0, java.net.InetAddress.getByName("127.0.0.1"))) {
            while (!Thread.interrupted()) {
                Socket sock = server.accept();
                Thread clientThread = new Thread(() -> handleClient(sock), "cab-client");
                clientThread.setDaemon(true);
                clientThread.start();
            }
        } catch (Exception e) {
            System.err.println("[craft-agent-bridge] 服务异常: " + e);
        }
    }

    private void handleClient(Socket sock) {
        try (BufferedReader in = new BufferedReader(new InputStreamReader(sock.getInputStream(), StandardCharsets.UTF_8));
             PrintWriter out = new PrintWriter(new java.io.OutputStreamWriter(sock.getOutputStream(), StandardCharsets.UTF_8), true)) {
            String line;
            while ((line = in.readLine()) != null) {
                JsonObject resp;
                try {
                    JsonObject req = GSON.fromJson(line, JsonObject.class);
                    resp = dispatch(req);
                } catch (Exception e) {
                    resp = new JsonObject();
                    resp.addProperty("status", "fail");
                    resp.addProperty("detail", "解析/执行失败: " + e.getMessage());
                }
                out.println(GSON.toJson(resp));
                out.flush();
            }
        } catch (Exception e) {
            System.err.println("[craft-agent-bridge] 客户端连接异常: " + e);
        }
    }

    private JsonObject dispatch(JsonObject req) {
        String type = req.has("type") ? req.get("type").getAsString() : "";
        if ("state".equals(type)) {
            return runOnServerThread(this::buildState);
        }
        return runOnServerThread(() -> {
            try {
                return performAction(type, req);
            } catch (Exception e) {
                JsonObject o = new JsonObject();
                o.addProperty("status", "fail");
                o.addProperty("detail", e.getMessage());
                return o;
            }
        });
    }

    // ══════════════════════════════════════════════════════════════
    // 状态查询
    // ══════════════════════════════════════════════════════════════

    private JsonObject buildState() {
        JsonObject o = new JsonObject();
        MinecraftServer server = serverInstance;
        if (server == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "服务器未就绪");
            return o;
        }
        ServerPlayer player = getFirstPlayer(server);
        if (player == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "没有在线玩家（请先进入世界）");
            return o;
        }
        ServerLevel level = player.level();

        // 位置 / 朝向 / 血量
        Vec3 pos = player.position();
        o.add("position", arr(pos.x, pos.y, pos.z));
        o.addProperty("yaw", player.getYRot());
        o.addProperty("pitch", player.getXRot());
        o.addProperty("health", player.getHealth());
        o.addProperty("hunger", player.getFoodData().getFoodLevel());
        o.addProperty("gamemode", player.gameMode.getGameModeForPlayer().getName());
        o.addProperty("time", level.getOverworldClockTime());
        o.addProperty("dimension", level.dimension().toString());
        o.addProperty("biome", level.getBiomeManager().getBiome(player.blockPosition()).unwrapKey()
                .map(k -> k.identifier().toString()).orElse("?"));

        // 时间格式化（tick 0=6:00, 6000=12:00, 12000=18:00, 18000=0:00）
        long time = level.getOverworldClockTime() % 24000;
        int hour = (int) ((time / 1000 + 6) % 24);
        int minute = (int) ((time % 1000) * 60 / 1000);
        boolean isDay = time < 12000 || time >= 23000;
        o.addProperty("time_str", String.format("%02d:%02d (%s)", hour, minute, isDay ? "day" : "night"));

        // 运动速度
        Vec3 vel = player.getDeltaMovement();
        o.add("velocity", arr(vel.x, vel.y, vel.z));

        // 状态效果
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

        // 经验
        o.addProperty("experience_level", player.experienceLevel);
        o.addProperty("experience_progress", player.experienceProgress);

        // 天气 / 光照
        o.addProperty("raining", level.isRaining());
        o.addProperty("thundering", level.isThundering());
        BlockPos pp = player.blockPosition();
        int skyLight = level.getLightEngine().getLayerListener(LightLayer.SKY).getLightValue(pp);
        int blockLight = level.getLightEngine().getLayerListener(LightLayer.BLOCK).getLightValue(pp);
        o.addProperty("sky_light", skyLight);
        o.addProperty("block_light", blockLight);

        // 物品栏
        JsonArray inv = new JsonArray();
        Inventory inventory = player.getInventory();
        int size = inventory.getContainerSize();
        for (int i = 0; i < size; i++) {
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

        // 手持物品
        ItemStack held = player.getMainHandItem();
        int selectedSlot = inventory.getSelectedSlot();
        o.addProperty("held_item",
            held.isEmpty() ? "minecraft:air"
            : BuiltInRegistries.ITEM.getKey(held.getItem()).toString());
        o.addProperty("selected_slot", selectedSlot);

        // 准星所指方块（服务端主动 raycast）
        HitResult hit = player.pick(6.0, 0.0f, false);
        if (hit != null && hit.getType() == HitResult.Type.BLOCK) {
            BlockPos bp = ((BlockHitResult) hit).getBlockPos();
            BlockState bs = level.getBlockState(bp);
            String id = BuiltInRegistries.BLOCK.getKey(bs.getBlock()).toString();
            double dist = player.position().distanceTo(Vec3.atCenterOf(bp));
            JsonObject tb = new JsonObject();
            tb.addProperty("id", id);
            tb.addProperty("dist", dist);
            o.add("targeted_block", tb);
        } else {
            o.add("targeted_block", null);
        }

        // 附近方块（白名单扫描）
        JsonArray blocks = new JsonArray();
        BlockPos pc = player.blockPosition();
        for (BlockPos bp : BlockPos.betweenClosed(
                pc.getX() - SCAN_RADIUS, pc.getY() - SCAN_RADIUS, pc.getZ() - SCAN_RADIUS,
                pc.getX() + SCAN_RADIUS, pc.getY() + SCAN_RADIUS, pc.getZ() + SCAN_RADIUS)) {
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
            blocks.add(b);
        }
        o.add("nearby_blocks", blocks);

        // 附近实体（服务端用 getEntities + AABB）
        JsonArray ents = new JsonArray();
        AABB scanArea = AABB.ofSize(player.position(), 32, 32, 32);
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
            float hp = (e instanceof LivingEntity le) ? le.getHealth() : 0f;
            en.addProperty("health", hp);
            ents.add(en);
        }
        o.add("entities", ents);

        o.addProperty("status", "ok");
        return o;
    }

    // ══════════════════════════════════════════════════════════════
    // 动作执行（全部在服务端主线程，天然同步）
    // ══════════════════════════════════════════════════════════════

    private JsonObject performAction(String type, JsonObject req) {
        MinecraftServer server = serverInstance;
        if (server == null) {
            JsonObject o = new JsonObject();
            o.addProperty("status", "fail");
            o.addProperty("detail", "服务器未就绪");
            return o;
        }
        ServerPlayer player = getFirstPlayer(server);
        if (player == null) {
            JsonObject o = new JsonObject();
            o.addProperty("status", "fail");
            o.addProperty("detail", "没有在线玩家");
            return o;
        }
        ServerLevel level = player.level();
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");

        switch (type) {
            case "look": {
                int dx = req.has("dx") ? req.get("dx").getAsInt() : 0;
                int dy = req.has("dy") ? req.get("dy").getAsInt() : 0;
                float yaw = player.getYRot() - dx * 0.3f;
                float pitch = clamp(player.getXRot() + dy * 0.3f, -90f, 90f);
                player.setYRot(yaw);
                player.setXRot(pitch);
                o.addProperty("detail", "look dx=" + dx + " dy=" + dy);
                break;
            }
            case "look_at": {
                double tx = req.get("x").getAsDouble();
                double ty = req.get("y").getAsDouble();
                double tz = req.get("z").getAsDouble();
                Vec3 eye = player.getEyePosition();
                double bx = (Math.abs(tx % 1.0) < 0.01) ? tx + 0.5 : tx;
                double by = (Math.abs(ty % 1.0) < 0.01) ? ty + 0.5 : ty;
                double bz = (Math.abs(tz % 1.0) < 0.01) ? tz + 0.5 : tz;
                double ddx = bx - eye.x, ddy = by - eye.y, ddz = bz - eye.z;
                double len = Math.sqrt(ddx * ddx + ddy * ddy + ddz * ddz);
                if (len < 0.001) break;
                float yaw = (float) Math.toDegrees(Math.atan2(-ddx, ddz));
                float pitch = (float) Math.toDegrees(Math.asin(clamp(ddy / len, -1.0, 1.0)));
                player.setYRot(yaw);
                player.setXRot(clamp(pitch, -90f, 90f));
                o.addProperty("detail", "look_at(" + tx + "," + ty + "," + tz + ")");
                break;
            }
            case "move_to": {
                double tx = req.get("x").getAsDouble();
                double ty = req.get("y").getAsDouble();
                double tz = req.get("z").getAsDouble();
                int maxTicks = req.has("max_ticks") ? req.get("max_ticks").getAsInt() : 200;
                moveReached = false;
                moveFinalDist = 999;
                moveStuck = false;
                moveTicksLeft = maxTicks;
                moveTarget = new double[]{tx, ty, tz};
                // 等待服务端 tick 完成移动
                int waitMs = 0;
                while (moveTarget != null && waitMs < maxTicks * 50 + 2000) {
                    try { Thread.sleep(50); } catch (InterruptedException e) { Thread.currentThread().interrupt(); break; }
                    waitMs += 50;
                }
                o.addProperty("reached", moveReached);
                o.addProperty("final_dist", moveFinalDist);
                o.addProperty("stuck", moveStuck);
                o.addProperty("detail", "move_to " + tx + "," + ty + "," + tz + " (reached=" + moveReached + ", dist=" + String.format("%.1f", moveFinalDist) + "m)");
                break;
            }
            case "dig_at": {
                int tx = req.get("x").getAsInt();
                int ty = req.get("y").getAsInt();
                int tz = req.get("z").getAsInt();
                BlockPos pos = new BlockPos(tx, ty, tz);
                BlockState state = level.getBlockState(pos);
                if (state.isAir()) {
                    o.addProperty("broken", false);
                    o.addProperty("detail", "dig_at: block is air");
                    break;
                }
                double dist = player.position().distanceTo(Vec3.atCenterOf(pos));
                if (dist > 5.5) {
                    o.addProperty("broken", false);
                    o.addProperty("detail", "dig_at: too far (" + String.format("%.1f", dist) + "m)");
                    break;
                }
                String blockId = BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString();
                // 自动装备最佳工具
                equipBestTool(player, blockId);
                // 服务端原生 destroyBlock：检查 canHarvestBlock → 有掉落
                boolean ok = player.gameMode.destroyBlock(pos);
                // 同步物品栏
                player.containerMenu.broadcastChanges();
                o.addProperty("broken", ok);
                o.addProperty("block_id", blockId);
                o.addProperty("detail", "dig_at " + tx + "," + ty + "," + tz + " (broken=" + ok + ", block=" + blockId + ")");
                break;
            }
            case "place_at": {
                int tx = req.get("x").getAsInt();
                int ty = req.get("y").getAsInt();
                int tz = req.get("z").getAsInt();
                String item = req.has("item") ? req.get("item").getAsString() : "dirt";
                boolean placed = placeAt(player, level, tx, ty, tz, item);
                player.containerMenu.broadcastChanges();
                o.addProperty("placed", placed);
                o.addProperty("detail", "place_at " + tx + "," + ty + "," + tz + " item=" + item + " (placed=" + placed + ")");
                break;
            }
            case "attack": {
                // 攻击最近实体
                LivingEntity target = null;
                double minDist = Double.MAX_VALUE;
                AABB scanArea = AABB.ofSize(player.position(), 16, 16, 16);
                for (Entity e : level.getEntities(player, scanArea)) {
                    if (!(e instanceof LivingEntity le)) continue;
                    String tn = BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).getPath();
                    if (!isHostile(tn)) continue;
                    double d = e.distanceTo(player);
                    if (d < minDist) { minDist = d; target = le; }
                }
                if (target == null) {
                    o.addProperty("detail", "attack: no hostile entity nearby");
                    break;
                }
                equipBestWeapon(player);
                player.lookAt(EntityAnchorArgument.Anchor.EYES, target.position().add(0, 1.0, 0));
                player.attack(target);
                player.containerMenu.broadcastChanges();
                o.addProperty("detail", "attack " + BuiltInRegistries.ENTITY_TYPE.getKey(target.getType()).getPath() + " dist=" + String.format("%.1f", minDist) + "m");
                break;
            }
            case "combat": {
                String mode = req.has("mode") ? req.get("mode").getAsString() : "melee";
                int ticks = req.has("ticks") ? req.get("ticks").getAsInt() : 200;
                CombatResult cr = combat(player, level, mode, ticks);
                o.addProperty("result", cr.result);
                o.addProperty("target", cr.targetType);
                o.addProperty("detail", "combat mode=" + mode + " -> " + cr.result + " (target=" + cr.targetType + ")");
                break;
            }
            case "select_slot": {
                int slot = req.get("slot").getAsInt();
                player.getInventory().setSelectedSlot(slot);
                player.containerMenu.broadcastChanges();
                int actual = player.getInventory().getSelectedSlot();
                ItemStack held = player.getMainHandItem();
                String heldId = held.isEmpty() ? "minecraft:air" : BuiltInRegistries.ITEM.getKey(held.getItem()).toString();
                o.addProperty("slot", actual);
                o.addProperty("held_item", heldId);
                o.addProperty("detail", "select_slot " + slot + " (actual=" + actual + ", held=" + heldId + ")");
                break;
            }
            case "move_to_hotbar": {
                String item = req.has("item") ? req.get("item").getAsString() : "";
                String search = item.replace("minecraft:", "").toLowerCase();
                Inventory inv = player.getInventory();
                int srcSlot = -1;
                for (int i = 9; i < inv.getContainerSize(); i++) {
                    ItemStack s = inv.getItem(i);
                    if (s.isEmpty()) continue;
                    String key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase();
                    if (key.contains(search)) { srcSlot = i; break; }
                }
                if (srcSlot == -1) {
                    o.addProperty("moved", false);
                    o.addProperty("detail", "move_to_hotbar: " + item + " not found in main inventory");
                    break;
                }
                int dstSlot = -1;
                for (int i = 0; i < 9; i++) {
                    if (inv.getItem(i).isEmpty()) { dstSlot = i; break; }
                }
                if (dstSlot < 0) dstSlot = 0;
                // 服务端直接交换（不需要 handleInventoryMouseClick）
                ItemStack tmp = inv.getItem(dstSlot);
                inv.setItem(dstSlot, inv.getItem(srcSlot));
                inv.setItem(srcSlot, tmp);
                player.containerMenu.broadcastChanges();
                o.addProperty("moved", true);
                o.addProperty("hotbar_slot", dstSlot);
                o.addProperty("detail", "move_to_hotbar " + item + " -> slot " + dstSlot);
                break;
            }
            case "craft": {
                String item = req.get("item").getAsString();
                int want = req.has("count") ? req.get("count").getAsInt() : 1;
                int crafted = craftItem(player, item, want);
                player.containerMenu.broadcastChanges();
                o.addProperty("crafted", crafted);
                o.addProperty("detail", "craft " + item + " x" + crafted);
                break;
            }
            case "discard": {
                String item = req.get("item").getAsString();
                int num = req.has("num") ? req.get("num").getAsInt() : 1;
                int discarded = discardItem(player, item, num);
                player.containerMenu.broadcastChanges();
                o.addProperty("detail", "discarded " + discarded + " x " + item);
                break;
            }
            case "smelt": {
                String item = req.get("item").getAsString();
                int num = req.has("num") ? req.get("num").getAsInt() : 1;
                int smelted = smeltItem(player, item, num);
                player.containerMenu.broadcastChanges();
                o.addProperty("detail", "smelted " + smelted + " x " + item);
                break;
            }
            case "use_item": {
                // 右键使用主手物品（吃东西/用桶/扔末影珍珠等）
                // ticks 模拟长按时长（吃东西 32 tick ≈ 1.6s）
                int ticks = req.has("ticks") ? req.get("ticks").getAsInt() : 5;
                ItemStack held = player.getMainHandItem();
                if (held.isEmpty()) {
                    o.addProperty("detail", "use_item: main hand empty");
                    break;
                }
                // 触发 useItem，服务端开始使用物品（吃东西会自动 tick 累积）
                var result = player.gameMode.useItem(player, level, held, InteractionHand.MAIN_HAND);
                boolean consumed = result.consumesAction();
                // 等待使用完成（吃东西按 ticks 等待，简单物品立即完成）
                if (consumed && ticks > 1) {
                    try { Thread.sleep(ticks * 50L); } catch (InterruptedException e) { /* ignore */ }
                }
                player.containerMenu.broadcastChanges();
                o.addProperty("consumed", consumed);
                o.addProperty("detail", "use_item " + BuiltInRegistries.ITEM.getKey(held.getItem()).getPath() + " (consumed=" + consumed + ")");
                break;
            }
            case "get_block": {
                int x = req.get("x").getAsInt();
                int y = req.get("y").getAsInt();
                int z = req.get("z").getAsInt();
                BlockState state = level.getBlockState(new BlockPos(x, y, z));
                String id = BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString();
                o.addProperty("id", id);
                o.addProperty("solid", !state.isAir() && !state.canBeReplaced());
                o.addProperty("air", state.isAir());
                break;
            }
            case "get_blocks": {
                int x1 = req.get("x1").getAsInt(), y1 = req.get("y1").getAsInt(), z1 = req.get("z1").getAsInt();
                int x2 = req.get("x2").getAsInt(), y2 = req.get("y2").getAsInt(), z2 = req.get("z2").getAsInt();
                JsonArray blocks = new JsonArray();
                for (BlockPos bp : BlockPos.betweenClosed(x1, y1, z1, x2, y2, z2)) {
                    BlockState state = level.getBlockState(bp);
                    if (state.isAir()) continue;
                    String id = BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString();
                    JsonObject b = new JsonObject();
                    b.addProperty("x", bp.getX());
                    b.addProperty("y", bp.getY());
                    b.addProperty("z", bp.getZ());
                    b.addProperty("id", id);
                    b.addProperty("solid", !state.canBeReplaced());
                    blocks.add(b);
                }
                o.add("blocks", blocks);
                o.addProperty("count", blocks.size());
                break;
            }
            // ══════════════════════════════════════════════════════════════
            // 容器/GUI 交互（参考 Numen 的 ContainerTools + GuiTools）
            // ══════════════════════════════════════════════════════════════
            case "inspect_gui": {
                var menu = player.containerMenu;
                boolean hasGui = menu != player.inventoryMenu;
                o.addProperty("has_gui", hasGui);
                if (!hasGui) {
                    o.addProperty("detail", "inspect_gui: no container open");
                    break;
                }
                JsonArray slots = new JsonArray();
                JsonArray craftingGrid = new JsonArray();
                boolean hasCrafting = false;
                for (int i = 0; i < menu.slots.size(); i++) {
                    var slot = menu.getSlot(i);
                    ItemStack stack = slot.getItem();
                    JsonObject so = new JsonObject();
                    so.addProperty("slot_index", i);
                    so.addProperty("id", stack.isEmpty() ? "minecraft:air" : BuiltInRegistries.ITEM.getKey(stack.getItem()).toString());
                    so.addProperty("count", stack.getCount());
                    // 区分容器侧 vs 玩家背包侧
                    boolean isPlayerInv = slot.container == player.getInventory();
                    so.addProperty("side", isPlayerInv ? "player" : "container");
                    // 识别合成网格
                    if (slot.container instanceof net.minecraft.world.inventory.CraftingContainer) {
                        hasCrafting = true;
                        JsonObject co = new JsonObject();
                        co.addProperty("slot_index", i);
                        co.addProperty("id", stack.isEmpty() ? "minecraft:air" : BuiltInRegistries.ITEM.getKey(stack.getItem()).toString());
                        co.addProperty("count", stack.getCount());
                        craftingGrid.add(co);
                    }
                    slots.add(so);
                }
                o.add("slots", slots);
                if (hasCrafting) o.add("crafting_grid", craftingGrid);
                // 光标上的物品
                ItemStack carried = menu.getCarried();
                o.addProperty("carried_item", carried.isEmpty() ? "minecraft:air" : BuiltInRegistries.ITEM.getKey(carried.getItem()).toString());
                o.addProperty("carried_count", carried.getCount());
                o.addProperty("detail", "inspect_gui: " + menu.slots.size() + " slots");
                break;
            }
            case "close_gui": {
                if (player.containerMenu != player.inventoryMenu) {
                    player.closeContainer();
                    o.addProperty("detail", "close_gui: container closed");
                } else {
                    o.addProperty("detail", "close_gui: no container open");
                }
                break;
            }
            case "transfer": {
                // 在打开的容器中进行物品转移（参考 Numen ContainerTools）
                if (player.containerMenu == player.inventoryMenu) {
                    o.addProperty("status", "fail");
                    o.addProperty("detail", "transfer: no container open");
                    break;
                }
                var menu = player.containerMenu;
                if (!req.has("moves") || !req.get("moves").isJsonArray()) {
                    o.addProperty("status", "fail");
                    o.addProperty("detail", "transfer: moves array required");
                    break;
                }
                JsonArray moves = req.get("moves").getAsJsonArray();
                int movedTotal = 0;
                for (int mi = 0; mi < moves.size(); mi++) {
                    JsonObject mv = moves.get(mi).getAsJsonObject();
                    int fromSlot = mv.get("from").getAsInt();
                    Integer toSlot = mv.has("to") && !mv.get("to").isJsonNull() ? mv.get("to").getAsInt() : null;
                    int count = mv.has("count") ? mv.get("count").getAsInt() : -1;
                    if (fromSlot < 0 || fromSlot >= menu.slots.size()) continue;
                    if (toSlot != null && (toSlot < 0 || toSlot >= menu.slots.size())) continue;
                    if (toSlot == null) {
                        // Shift+点击路由（QUICK_MOVE）
                        menu.clicked(fromSlot, 0, net.minecraft.world.inventory.ContainerInput.QUICK_MOVE, player);
                        movedTotal++;
                    } else {
                        // 精确槽位移动：PICKUP + 按钮0（整组）
                        menu.clicked(fromSlot, 0, net.minecraft.world.inventory.ContainerInput.PICKUP, player);
                        menu.clicked(toSlot, 0, net.minecraft.world.inventory.ContainerInput.PICKUP, player);
                        movedTotal++;
                    }
                }
                player.containerMenu.broadcastChanges();
                o.addProperty("moved_count", movedTotal);
                o.addProperty("detail", "transfer: " + movedTotal + " moves executed");
                break;
            }
            // ══════════════════════════════════════════════════════════════
            // 装备/进食/丢弃/等待
            // ══════════════════════════════════════════════════════════════
            case "equip_item": {
                String itemName = req.has("item") ? req.get("item").getAsString() : "";
                String slotName = req.has("slot") ? req.get("slot").getAsString() : "auto";
                String search = itemName.replace("minecraft:", "").toLowerCase();
                Inventory inv = player.getInventory();
                // 在背包中查找物品
                ItemStack targetStack = ItemStack.EMPTY;
                int foundSlot = -1;
                for (int i = 0; i < inv.getContainerSize(); i++) {
                    ItemStack s = inv.getItem(i);
                    if (s.isEmpty()) continue;
                    String key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase();
                    if (key.contains(search)) { targetStack = s.copy(); foundSlot = i; break; }
                }
                if (targetStack.isEmpty()) {
                    o.addProperty("status", "fail");
                    o.addProperty("detail", "equip_item: " + itemName + " not found");
                    break;
                }
                net.minecraft.world.entity.EquipmentSlot equipSlot = null;
                if (!slotName.equals("auto")) {
                    equipSlot = switch (slotName.toLowerCase()) {
                        case "mainhand", "main_hand" -> net.minecraft.world.entity.EquipmentSlot.MAINHAND;
                        case "offhand", "off_hand" -> net.minecraft.world.entity.EquipmentSlot.OFFHAND;
                        case "head", "helmet" -> net.minecraft.world.entity.EquipmentSlot.HEAD;
                        case "chest", "chestplate" -> net.minecraft.world.entity.EquipmentSlot.CHEST;
                        case "legs", "leggings" -> net.minecraft.world.entity.EquipmentSlot.LEGS;
                        case "feet", "boots" -> net.minecraft.world.entity.EquipmentSlot.FEET;
                        default -> null;
                    };
                }
                // auto 模式：根据物品类型推断槽位
                if (equipSlot == null) {
                    String key = BuiltInRegistries.ITEM.getKey(targetStack.getItem()).toString().toLowerCase();
                    if (key.contains("helmet") || key.contains("cap")) equipSlot = net.minecraft.world.entity.EquipmentSlot.HEAD;
                    else if (key.contains("chestplate") || key.contains("jacket")) equipSlot = net.minecraft.world.entity.EquipmentSlot.CHEST;
                    else if (key.contains("leggings") || key.contains("pants")) equipSlot = net.minecraft.world.entity.EquipmentSlot.LEGS;
                    else if (key.contains("boots")) equipSlot = net.minecraft.world.entity.EquipmentSlot.FEET;
                    else if (key.contains("shield")) equipSlot = net.minecraft.world.entity.EquipmentSlot.OFFHAND;
                    else equipSlot = net.minecraft.world.entity.EquipmentSlot.MAINHAND;
                }
                // 先尝试右键装备（useItem，兼容 Equippable）
                boolean equipped = false;
                if (equipSlot == net.minecraft.world.entity.EquipmentSlot.MAINHAND) {
                    // 切到快捷栏对应物品
                    if (foundSlot < 9) {
                        inv.setSelectedSlot(foundSlot);
                    } else {
                        // 从主背包移到快捷栏
                        int dst = 0;
                        for (int i = 0; i < 9; i++) { if (inv.getItem(i).isEmpty()) { dst = i; break; } }
                        ItemStack tmp = inv.getItem(dst);
                        inv.setItem(dst, inv.getItem(foundSlot));
                        inv.setItem(foundSlot, tmp);
                        inv.setSelectedSlot(dst);
                    }
                    equipped = true;
                } else {
                    // 尝试右键装备
                    if (foundSlot < 9) inv.setSelectedSlot(foundSlot);
                    var result = player.gameMode.useItem(player, level, player.getMainHandItem(), InteractionHand.MAIN_HAND);
                    if (result.consumesAction()) {
                        equipped = true;
                    } else {
                        // 回退：直接 setItemSlot
                        ItemStack current = player.getItemBySlot(equipSlot);
                        player.setItemSlot(equipSlot, targetStack.copy());
                        // 原装备退回背包或掉落
                        if (!current.isEmpty()) {
                            if (!inv.add(current)) player.drop(current, false);
                        }
                        // 从原位置移除
                        inv.getItem(foundSlot).shrink(targetStack.getCount());
                        equipped = true;
                    }
                }
                player.containerMenu.broadcastChanges();
                o.addProperty("equipped", equipped);
                o.addProperty("slot", equipSlot.getName());
                o.addProperty("detail", "equip_item " + itemName + " -> " + equipSlot.getName() + " (equipped=" + equipped + ")");
                break;
            }
            case "eat_item": {
                String itemName = req.has("item") ? req.get("item").getAsString() : "";
                String search = itemName.replace("minecraft:", "").toLowerCase();
                Inventory inv = player.getInventory();
                int eatSlot = -1;
                for (int i = 0; i < inv.getContainerSize(); i++) {
                    ItemStack s = inv.getItem(i);
                    if (s.isEmpty()) continue;
                    String key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase();
                    if (key.contains(search)) { eatSlot = i; break; }
                }
                if (eatSlot < 0) {
                    o.addProperty("status", "fail");
                    o.addProperty("detail", "eat_item: " + itemName + " not found");
                    break;
                }
                // 切到该物品
                if (eatSlot < 9) {
                    inv.setSelectedSlot(eatSlot);
                } else {
                    int dst = 0;
                    for (int i = 0; i < 9; i++) { if (inv.getItem(i).isEmpty()) { dst = i; break; } }
                    ItemStack tmp = inv.getItem(dst);
                    inv.setItem(dst, inv.getItem(eatSlot));
                    inv.setItem(eatSlot, tmp);
                    inv.setSelectedSlot(dst);
                }
                player.containerMenu.broadcastChanges();
                // 开始吃（useItem 触发 FoodData 恢复）
                var result = player.gameMode.useItem(player, level, player.getMainHandItem(), InteractionHand.MAIN_HAND);
                boolean consumed = result.consumesAction();
                if (consumed) {
                    // 等待吃完（默认 32 tick ≈ 1.6s）
                    int eatTicks = req.has("ticks") ? req.get("ticks").getAsInt() : 32;
                    try { Thread.sleep(eatTicks * 50L); } catch (InterruptedException e) { /* ignore */ }
                }
                player.containerMenu.broadcastChanges();
                o.addProperty("consumed", consumed);
                o.addProperty("detail", "eat_item " + itemName + " (consumed=" + consumed + ")");
                break;
            }
            case "drop_items": {
                String itemName = req.has("item") ? req.get("item").getAsString() : "";
                int num = req.has("num") ? req.get("num").getAsInt() : 1;
                String search = itemName.replace("minecraft:", "").toLowerCase();
                Inventory inv = player.getInventory();
                int dropped = 0;
                for (int i = 0; i < inv.getContainerSize() && dropped < num; i++) {
                    ItemStack s = inv.getItem(i);
                    if (s.isEmpty()) continue;
                    String key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase();
                    if (!key.contains(search)) continue;
                    int take = Math.min(s.getCount(), num - dropped);
                    ItemStack toDrop = s.copy();
                    toDrop.setCount(take);
                    s.shrink(take);
                    // 真正生成地面掉落物（带拾取冷却）
                    player.drop(toDrop, false);
                    dropped += take;
                }
                player.containerMenu.broadcastChanges();
                o.addProperty("dropped", dropped);
                o.addProperty("detail", "drop_items " + itemName + " x" + dropped + " (ItemEntity spawned)");
                break;
            }
            case "wait": {
                int seconds = req.has("seconds") ? req.get("seconds").getAsInt() : 1;
                try { Thread.sleep(seconds * 1000L); } catch (InterruptedException e) { /* ignore */ }
                o.addProperty("detail", "wait " + seconds + "s");
                break;
            }
            // ══════════════════════════════════════════════════════════════
            // 玩家交互（参考 mindcraft: goToPlayer, attackPlayer, givePlayer）
            // ══════════════════════════════════════════════════════════════
            case "list_players": {
                JsonArray players = new JsonArray();
                for (ServerPlayer p : server.getPlayerList().getPlayers()) {
                    JsonObject po = new JsonObject();
                    po.addProperty("name", p.getName().getString());
                    po.addProperty("uuid", p.getUUID().toString());
                    po.add("position", arr(p.getX(), p.getY(), p.getZ()));
                    po.addProperty("dist", Math.sqrt(
                        Math.pow(p.getX() - player.getX(), 2) +
                        Math.pow(p.getY() - player.getY(), 2) +
                        Math.pow(p.getZ() - player.getZ(), 2)));
                    players.add(po);
                }
                o.add("players", players);
                o.addProperty("count", players.size());
                o.addProperty("detail", "list_players: " + players.size() + " online");
                break;
            }
            case "go_to_player": {
                String targetName = req.has("player_name") ? req.get("player_name").getAsString() : "";
                ServerPlayer target = null;
                for (ServerPlayer p : server.getPlayerList().getPlayers()) {
                    if (p.getName().getString().equalsIgnoreCase(targetName)) { target = p; break; }
                }
                if (target == null) {
                    o.addProperty("status", "fail");
                    o.addProperty("detail", "go_to_player: player '" + targetName + "' not found");
                    break;
                }
                double closeness = req.has("closeness") ? req.get("closeness").getAsDouble() : 2.0;
                // 设置移动目标
                moveTarget = new double[]{target.getX(), target.getY(), target.getZ()};
                moveTicksLeft = 400; // 20s timeout
                moveReached = false;
                moveStuck = false;
                // 等待到达或超时（轮询检查）
                long start = System.currentTimeMillis();
                while (moveTarget != null && System.currentTimeMillis() - start < 20000) {
                    if (shouldStop) { shouldStop = false; break; }
                    try { Thread.sleep(200); } catch (InterruptedException e) { break; }
                }
                o.addProperty("reached", moveReached);
                o.addProperty("final_dist", moveFinalDist);
                o.addProperty("detail", "go_to_player " + targetName + " reached=" + moveReached + " dist=" + String.format("%.1f", moveFinalDist));
                break;
            }
            case "attack_player": {
                String targetName = req.has("player_name") ? req.get("player_name").getAsString() : "";
                ServerPlayer target = null;
                for (ServerPlayer p : server.getPlayerList().getPlayers()) {
                    if (p.getName().getString().equalsIgnoreCase(targetName)) { target = p; break; }
                }
                if (target == null) {
                    o.addProperty("status", "fail");
                    o.addProperty("detail", "attack_player: player '" + targetName + "' not found");
                    break;
                }
                int ticks = req.has("ticks") ? req.get("ticks").getAsInt() : 60;
                int hitCount = 0;
                for (int i = 0; i < ticks; i++) {
                    if (shouldStop) { shouldStop = false; break; }
                    if (target.isRemoved() || target.getHealth() <= 0) break;
                    double dist = player.distanceTo(target);
                    if (dist > 4.0) {
                        // 距离太远，朝向目标移动
                        double dx = target.getX() - player.getX();
                        double dz = target.getZ() - player.getZ();
                        float yaw = (float) Math.toDegrees(Math.atan2(-dx, dz));
                        player.setYRot(yaw);
                        player.setDeltaMovement(dx / dist * 0.28, player.getDeltaMovement().y, dz / dist * 0.28);
                    } else {
                        // 在范围内，攻击
                        player.setYRot((float) Math.toDegrees(Math.atan2(-(target.getX() - player.getX()), target.getZ() - player.getZ())));
                        player.attack(target);
                        hitCount++;
                    }
                    try { Thread.sleep(50); } catch (InterruptedException e) { break; }
                }
                player.setDeltaMovement(0, player.getDeltaMovement().y, 0);
                o.addProperty("hits", hitCount);
                o.addProperty("detail", "attack_player " + targetName + " hits=" + hitCount);
                break;
            }
            case "give_player": {
                String targetName = req.has("player_name") ? req.get("player_name").getAsString() : "";
                String giveItem = req.has("item") ? req.get("item").getAsString() : "";
                int giveNum = req.has("num") ? req.get("num").getAsInt() : 1;
                ServerPlayer target = null;
                for (ServerPlayer p : server.getPlayerList().getPlayers()) {
                    if (p.getName().getString().equalsIgnoreCase(targetName)) { target = p; break; }
                }
                if (target == null) {
                    o.addProperty("status", "fail");
                    o.addProperty("detail", "give_player: player '" + targetName + "' not found");
                    break;
                }
                // 走到玩家附近
                double dist = player.distanceTo(target);
                if (dist > 3.0) {
                    moveTarget = new double[]{target.getX(), target.getY(), target.getZ()};
                    moveTicksLeft = 200;
                    long start = System.currentTimeMillis();
                    while (moveTarget != null && System.currentTimeMillis() - start < 10000) {
                        if (shouldStop) { shouldStop = false; break; }
                        try { Thread.sleep(200); } catch (InterruptedException e) { break; }
                    }
                }
                // 查找物品并丢弃到玩家附近
                String search = giveItem.replace("minecraft:", "").toLowerCase();
                Inventory inv = player.getInventory();
                int dropped = 0;
                for (int i = 0; i < inv.getContainerSize() && dropped < giveNum; i++) {
                    ItemStack s = inv.getItem(i);
                    if (s.isEmpty()) continue;
                    String key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase();
                    if (!key.contains(search)) continue;
                    int take = Math.min(s.getCount(), giveNum - dropped);
                    ItemStack toDrop = s.copy();
                    toDrop.setCount(take);
                    s.shrink(take);
                    player.drop(toDrop, false);
                    dropped += take;
                }
                player.containerMenu.broadcastChanges();
                o.addProperty("dropped", dropped);
                o.addProperty("detail", "give_player " + giveItem + " x" + dropped + " to " + targetName);
                break;
            }
            // ══════════════════════════════════════════════════════════════
            // 自动拾取（参考 Numen collect_items）
            // ══════════════════════════════════════════════════════════════
            case "collect_items": {
                JsonArray itemFilters = req.has("item_ids") ? req.get("item_ids").getAsJsonArray() : new JsonArray();
                double radius = req.has("radius") ? req.get("radius").getAsDouble() : 16.0;
                int maxCount = req.has("max_count") ? req.get("max_count").getAsInt() : 64;
                Set<String> filters = new HashSet<>();
                for (int i = 0; i < itemFilters.size(); i++) filters.add(itemFilters.get(i).getAsString().toLowerCase());
                int collected = 0;
                long start = System.currentTimeMillis();
                while (collected < maxCount && System.currentTimeMillis() - start < 30000) {
                    if (shouldStop) { shouldStop = false; break; }
                    // 扫描附近 ItemEntity
                    ItemEntity nearest = null;
                    double minDist = Double.MAX_VALUE;
                    for (Entity e : level.getEntities(player, AABB.ofSize(player.position(), radius * 2, radius * 2, radius * 2))) {
                        if (!(e instanceof ItemEntity)) continue;
                        ItemEntity ie = (ItemEntity) e;
                        String itemId = BuiltInRegistries.ITEM.getKey(ie.getItem().getItem()).toString().toLowerCase();
                        if (!filters.isEmpty()) {
                            boolean match = false;
                            for (String f : filters) { if (itemId.contains(f)) { match = true; break; } }
                            if (!match) continue;
                        }
                        double d = player.distanceTo(ie);
                        if (d < minDist) { minDist = d; nearest = ie; }
                    }
                    if (nearest == null) break;
                    if (minDist > 1.5) {
                        // 走向物品
                        moveTarget = new double[]{nearest.getX(), nearest.getY(), nearest.getZ()};
                        moveTicksLeft = 100;
                        long walkStart = System.currentTimeMillis();
                        while (moveTarget != null && System.currentTimeMillis() - walkStart < 5000) {
                            if (shouldStop) { shouldStop = false; break; }
                            try { Thread.sleep(100); } catch (InterruptedException e) { break; }
                        }
                    }
                    // 等待拾取（原版 1 格内自动吸物）
                    try { Thread.sleep(300); } catch (InterruptedException e) { break; }
                    collected++;
                }
                o.addProperty("collected", collected);
                o.addProperty("detail", "collect_items: collected " + collected + " items");
                break;
            }
            // ══════════════════════════════════════════════════════════════
            // 控制命令（参考 mindcraft stop + goal）
            // ══════════════════════════════════════════════════════════════
            case "stop": {
                shouldStop = true;
                moveTarget = null;
                o.addProperty("detail", "stop: all actions cancelled");
                break;
            }
            case "set_goal": {
                String goal = req.has("goal") ? req.get("goal").getAsString() : "";
                if (goal.isEmpty()) {
                    currentGoal = null;
                    o.addProperty("detail", "set_goal: cleared");
                } else {
                    currentGoal = goal;
                    o.addProperty("detail", "set_goal: " + goal);
                }
                break;
            }
            case "get_goal": {
                o.addProperty("goal", currentGoal != null ? currentGoal : "(none)");
                o.addProperty("detail", "get_goal: " + (currentGoal != null ? currentGoal : "none"));
                break;
            }
            // ══════════════════════════════════════════════════════════════
            // 持续跟随（参考 mindcraft followPlayer resume=true）
            // ══════════════════════════════════════════════════════════════
            case "follow_player": {
                String targetName = req.has("player_name") ? req.get("player_name").getAsString() : "";
                ServerPlayer target = null;
                for (ServerPlayer p : server.getPlayerList().getPlayers()) {
                    if (p.getName().getString().equalsIgnoreCase(targetName)) { target = p; break; }
                }
                if (target == null) {
                    o.addProperty("status", "fail");
                    o.addProperty("detail", "follow_player: player '" + targetName + "' not found");
                    break;
                }
                double followDist = req.has("follow_dist") ? req.get("follow_dist").getAsDouble() : 3.0;
                int totalTicks = req.has("ticks") ? req.get("ticks").getAsInt() : 600; // 30s default
                long start = System.currentTimeMillis();
                int followTicks = 0;
                while (followTicks < totalTicks && System.currentTimeMillis() - start < totalTicks * 50L) {
                    if (shouldStop) { shouldStop = false; break; }
                    if (target.isRemoved() || !target.isAlive()) break;
                    double dx = target.getX() - player.getX();
                    double dz = target.getZ() - player.getZ();
                    double dist = Math.sqrt(dx * dx + dz * dz);
                    if (dist > followDist) {
                        float yaw = (float) Math.toDegrees(Math.atan2(-dx, dz));
                        player.setYRot(yaw);
                        double speed = 0.28;
                        player.setDeltaMovement(dx / dist * speed, player.getDeltaMovement().y, dz / dist * speed);
                        if (player.horizontalCollision && player.onGround()) {
                            player.setDeltaMovement(player.getDeltaMovement().x, 0.42, player.getDeltaMovement().z);
                        }
                    } else {
                        player.setDeltaMovement(0, player.getDeltaMovement().y, 0);
                    }
                    try { Thread.sleep(50); } catch (InterruptedException e) { break; }
                    followTicks++;
                }
                player.setDeltaMovement(0, player.getDeltaMovement().y, 0);
                o.addProperty("followed_ticks", followTicks);
                o.addProperty("detail", "follow_player " + targetName + " for " + followTicks + " ticks");
                break;
            }
            // ══════════════════════════════════════════════════════════════
            // 维基搜索（参考 mindcraft searchWiki）
            // ══════════════════════════════════════════════════════════════
            case "search_wiki": {
                String query = req.has("query") ? req.get("query").getAsString() : "";
                try {
                    java.net.URL url = new java.net.URL("https://minecraft.wiki/w/" + java.net.URLEncoder.encode(query.replace(" ", "_"), "UTF-8"));
                    java.net.HttpURLConnection conn = (java.net.HttpURLConnection) url.openConnection();
                    conn.setRequestProperty("User-Agent", "Craft-Agent/1.0");
                    conn.setConnectTimeout(5000);
                    conn.setReadTimeout(10000);
                    if (conn.getResponseCode() == 404) {
                        o.addProperty("detail", "search_wiki: '" + query + "' not found on minecraft.wiki");
                        break;
                    }
                    try (BufferedReader wr = new BufferedReader(new InputStreamReader(conn.getInputStream(), StandardCharsets.UTF_8))) {
                        StringBuilder sb = new StringBuilder();
                        String line;
                        while ((line = wr.readLine()) != null) sb.append(line).append("\n");
                        String html = sb.toString();
                        // 简单提取正文（去 HTML 标签）
                        String text = html.replaceAll("<script[^>]*>[\\s\\S]*?</script>", "")
                                .replaceAll("<style[^>]*>[\\s\\S]*?</style>", "")
                                .replaceAll("<[^>]+>", " ")
                                .replaceAll("&amp;", "&").replaceAll("&lt;", "<").replaceAll("&gt;", ">")
                                .replaceAll("&quot;", "\"").replaceAll("&#39;", "'")
                                .replaceAll("\\s+", " ").trim();
                        if (text.length() > 2000) text = text.substring(0, 2000) + "... [truncated]";
                        o.addProperty("content", text);
                        o.addProperty("detail", "search_wiki: " + query + " (" + text.length() + " chars)");
                    }
                } catch (Exception e) {
                    o.addProperty("status", "fail");
                    o.addProperty("detail", "search_wiki error: " + e.getMessage());
                }
                break;
            }
            // ══════════════════════════════════════════════════════════════
            // 村民交易（参考 mindcraft showVillagerTrades/tradeWithVillager）
            // ══════════════════════════════════════════════════════════════
            case "villager_trades": {
                double radius = req.has("radius") ? req.get("radius").getAsDouble() : 8.0;
                net.minecraft.world.item.trading.Merchant nearest = null;
                double minDist = Double.MAX_VALUE;
                for (Entity e : level.getEntities(player, AABB.ofSize(player.position(), radius * 2, radius * 2, radius * 2))) {
                    if (!(e instanceof net.minecraft.world.item.trading.Merchant)) continue;
                    double d = player.distanceTo(e);
                    if (d < minDist) { minDist = d; nearest = (net.minecraft.world.item.trading.Merchant) e; }
                }
                if (nearest == null) {
                    o.addProperty("status", "fail");
                    o.addProperty("detail", "villager_trades: no villager within " + radius + "m");
                    break;
                }
                Entity villagerEntity = (Entity) nearest;
                o.addProperty("villager_id", villagerEntity.getId());
                o.addProperty("villager_type", BuiltInRegistries.ENTITY_TYPE.getKey(villagerEntity.getType()).toString());
                o.addProperty("villager_profession", "merchant");
                JsonArray trades = new JsonArray();
                var merchantOffers = nearest.getOffers();
                for (int i = 0; i < merchantOffers.size(); i++) {
                    var offer = merchantOffers.get(i);
                    JsonObject to = new JsonObject();
                    to.addProperty("index", i + 1); // 1-indexed
                    to.addProperty("input_a", offer.getCostA().isEmpty() ? "air" : BuiltInRegistries.ITEM.getKey(offer.getCostA().getItem()).toString());
                    to.addProperty("input_a_count", offer.getCostA().getCount());
                    if (!offer.getCostB().isEmpty()) {
                        to.addProperty("input_b", BuiltInRegistries.ITEM.getKey(offer.getCostB().getItem()).toString());
                        to.addProperty("input_b_count", offer.getCostB().getCount());
                    }
                    to.addProperty("output", BuiltInRegistries.ITEM.getKey(offer.getResult().getItem()).toString());
                    to.addProperty("output_count", offer.getResult().getCount());
                    trades.add(to);
                }
                o.add("trades", trades);
                o.addProperty("detail", "villager_trades: " + trades.size() + " trades from " + BuiltInRegistries.ENTITY_TYPE.getKey(villagerEntity.getType()));
                break;
            }
            case "trade_with_villager": {
                double radius = req.has("radius") ? req.get("radius").getAsDouble() : 8.0;
                int tradeIndex = req.has("index") ? req.get("index").getAsInt() : 1; // 1-indexed
                int count = req.has("count") ? req.get("count").getAsInt() : 1;
                net.minecraft.world.item.trading.Merchant nearest = null;
                double minDist = Double.MAX_VALUE;
                for (Entity e : level.getEntities(player, AABB.ofSize(player.position(), radius * 2, radius * 2, radius * 2))) {
                    if (!(e instanceof net.minecraft.world.item.trading.Merchant)) continue;
                    double d = player.distanceTo(e);
                    if (d < minDist) { minDist = d; nearest = (net.minecraft.world.item.trading.Merchant) e; }
                }
                if (nearest == null) {
                    o.addProperty("status", "fail");
                    o.addProperty("detail", "trade_with_villager: no villager within " + radius + "m");
                    break;
                }
                var offers = nearest.getOffers();
                if (tradeIndex < 1 || tradeIndex > offers.size()) {
                    o.addProperty("status", "fail");
                    o.addProperty("detail", "trade_with_villager: invalid trade index " + tradeIndex + " (1-" + offers.size() + ")");
                    break;
                }
                int traded = 0;
                for (int i = 0; i < count; i++) {
                    var offer = offers.get(tradeIndex - 1);
                    if (offer.isOutOfStock()) break;
                    // 检查玩家是否有足够物品
                    var costA = offer.getCostA();
                    var costB = offer.getCostB();
                    int haveA = 0, haveB = 0;
                    Inventory inv = player.getInventory();
                    for (int j = 0; j < inv.getContainerSize(); j++) {
                        ItemStack s = inv.getItem(j);
                        if (s.isEmpty()) continue;
                        if (s.getItem() == costA.getItem()) haveA += s.getCount();
                        if (!costB.isEmpty() && s.getItem() == costB.getItem()) haveB += s.getCount();
                    }
                    if (haveA < costA.getCount() || (!costB.isEmpty() && haveB < costB.getCount())) break;
                    // 扣除输入物品
                    int needA = costA.getCount(), needB = costB.isEmpty() ? 0 : costB.getCount();
                    for (int j = 0; j < inv.getContainerSize() && needA > 0; j++) {
                        ItemStack s = inv.getItem(j);
                        if (s.isEmpty() || s.getItem() != costA.getItem()) continue;
                        int take = Math.min(s.getCount(), needA);
                        s.shrink(take); needA -= take;
                    }
                    for (int j = 0; j < inv.getContainerSize() && needB > 0; j++) {
                        ItemStack s = inv.getItem(j);
                        if (s.isEmpty() || s.getItem() != costB.getItem()) continue;
                        int take = Math.min(s.getCount(), needB);
                        s.shrink(take); needB -= take;
                    }
                    // 给输出物品
                    ItemStack result = offer.getResult().copy();
                    if (!inv.add(result)) player.drop(result, false);
                    offer.increaseUses();
                    traded++;
                }
                player.containerMenu.broadcastChanges();
                o.addProperty("traded", traded);
                o.addProperty("detail", "trade_with_villager: " + traded + " trades of index " + tradeIndex);
                break;
            }
            // ══════════════════════════════════════════════════════════════
            // 视觉/看向（参考 mindcraft lookAtPlayer/lookAtPosition）
            // ══════════════════════════════════════════════════════════════
            case "look_at_player": {
                String targetName = req.has("player_name") ? req.get("player_name").getAsString() : "";
                ServerPlayer target = null;
                for (ServerPlayer p : server.getPlayerList().getPlayers()) {
                    if (p.getName().getString().equalsIgnoreCase(targetName)) { target = p; break; }
                }
                if (target == null) {
                    o.addProperty("status", "fail");
                    o.addProperty("detail", "look_at_player: player '" + targetName + "' not found");
                    break;
                }
                double dx = target.getX() - player.getX();
                double dy = (target.getY() + target.getEyeHeight()) - (player.getY() + player.getEyeHeight());
                double dz = target.getZ() - player.getZ();
                double horiz = Math.sqrt(dx * dx + dz * dz);
                player.setYRot((float) Math.toDegrees(Math.atan2(-dx, dz)));
                player.setXRot((float) Math.toDegrees(-Math.atan2(dy, horiz)));
                o.addProperty("detail", "look_at_player: looking at " + targetName);
                break;
            }
            case "look_at_position": {
                double tx = req.has("x") ? req.get("x").getAsDouble() : player.getX();
                double ty = req.has("y") ? req.get("y").getAsDouble() : player.getY();
                double tz = req.has("z") ? req.get("z").getAsDouble() : player.getZ();
                double dx = tx - player.getX();
                double dy = ty - (player.getY() + player.getEyeHeight());
                double dz = tz - player.getZ();
                double horiz = Math.sqrt(dx * dx + dz * dz);
                player.setYRot((float) Math.toDegrees(Math.atan2(-dx, dz)));
                player.setXRot((float) Math.toDegrees(-Math.atan2(dy, horiz)));
                o.addProperty("detail", "look_at_position: looking at (" + tx + "," + ty + "," + tz + ")");
                break;
            }
            // ══════════════════════════════════════════════════════════════
            // 右键激活方块/对实体使用物品（参考 mindcraft activateNearestBlock/useOn）
            // ══════════════════════════════════════════════════════════════
            case "activate_block": {
                int x = req.get("x").getAsInt();
                int y = req.get("y").getAsInt();
                int z = req.get("z").getAsInt();
                BlockPos bp = new BlockPos(x, y, z);
                double dist = player.position().distanceTo(Vec3.atCenterOf(bp));
                if (dist > 5.5) {
                    o.addProperty("status", "fail");
                    o.addProperty("detail", "activate_block: too far (" + String.format("%.1f", dist) + "m)");
                    break;
                }
                BlockState state = level.getBlockState(bp);
                if (state.isAir()) {
                    o.addProperty("status", "fail");
                    o.addProperty("detail", "activate_block: air at (" + x + "," + y + "," + z + ")");
                    break;
                }
                // 朝向方块
                double dx = x + 0.5 - player.getX();
                double dy = y + 0.5 - (player.getY() + player.getEyeHeight());
                double dz = z + 0.5 - player.getZ();
                double horiz = Math.sqrt(dx * dx + dz * dz);
                player.setYRot((float) Math.toDegrees(Math.atan2(-dx, dz)));
                player.setXRot((float) Math.toDegrees(-Math.atan2(dy, horiz)));
                // useItemOn（26.2 Direction.getNearest 需要 4 参数）
                var hit = new BlockHitResult(Vec3.atCenterOf(bp), Direction.getNearest((int)Math.round(dx), (int)Math.round(dy), (int)Math.round(dz), Direction.UP), bp, false);
                var result = player.gameMode.useItemOn(player, level, player.getMainHandItem(), InteractionHand.MAIN_HAND, hit);
                player.containerMenu.broadcastChanges();
                o.addProperty("activated", result.consumesAction());
                o.addProperty("detail", "activate_block (" + x + "," + y + "," + z + ") consumed=" + result.consumesAction());
                break;
            }
            case "use_on_entity": {
                String entityType = req.has("entity_type") ? req.get("entity_type").getAsString() : "";
                double radius = req.has("radius") ? req.get("radius").getAsDouble() : 8.0;
                Entity nearest = null;
                double minDist = Double.MAX_VALUE;
                for (Entity e : level.getEntities(player, AABB.ofSize(player.position(), radius * 2, radius * 2, radius * 2))) {
                    if (e instanceof ServerPlayer) continue;
                    String eName = BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString().toLowerCase();
                    if (!eName.contains(entityType.toLowerCase())) continue;
                    double d = player.distanceTo(e);
                    if (d < minDist) { minDist = d; nearest = e; }
                }
                if (nearest == null) {
                    o.addProperty("status", "fail");
                    o.addProperty("detail", "use_on_entity: no '" + entityType + "' within " + radius + "m");
                    break;
                }
                // 朝向实体
                double dx = nearest.getX() - player.getX();
                double dy = (nearest.getY() + nearest.getEyeHeight()) - (player.getY() + player.getEyeHeight());
                double dz = nearest.getZ() - player.getZ();
                double horiz = Math.sqrt(dx * dx + dz * dz);
                player.setYRot((float) Math.toDegrees(Math.atan2(-dx, dz)));
                player.setXRot((float) Math.toDegrees(-Math.atan2(dy, horiz)));
                // 对实体使用物品（26.2 interactOn 需要 Entity, InteractionHand, Vec3 点击位置）
                var result = player.interactOn(nearest, InteractionHand.MAIN_HAND, nearest.position());
                player.containerMenu.broadcastChanges();
                o.addProperty("interacted", result.consumesAction());
                o.addProperty("detail", "use_on_entity " + entityType + " consumed=" + result.consumesAction());
                break;
            }
            // ══════════════════════════════════════════════════════════════
            // 清空对话历史 + 激活最近方块（参考 mindcraft clearChat/activateNearestBlock）
            // ══════════════════════════════════════════════════════════════
            case "clear_chat": {
                // mod 侧无对话历史，仅通知 Rust 侧清空
                o.addProperty("detail", "clear_chat: mod side ack, Rust side should clear history");
                break;
            }
            case "activate_nearest_block": {
                double radius = req.has("radius") ? req.get("radius").getAsDouble() : 5.0;
                String blockType = req.has("block_type") ? req.get("block_type").getAsString() : "";
                BlockPos nearest = null;
                double minDist = Double.MAX_VALUE;
                BlockPos pp = player.blockPosition();
                for (BlockPos bp : BlockPos.betweenClosed(pp.offset(-(int)radius, -2, -(int)radius), pp.offset((int)radius, 2, (int)radius))) {
                    BlockState s = level.getBlockState(bp);
                    if (s.isAir()) continue;
                    String id = BuiltInRegistries.BLOCK.getKey(s.getBlock()).toString().toLowerCase();
                    if (!blockType.isEmpty() && !id.contains(blockType.toLowerCase())) continue;
                    double d = player.position().distanceTo(Vec3.atCenterOf(bp));
                    if (d < minDist && d < 5.5) { minDist = d; nearest = bp.immutable(); }
                }
                if (nearest == null) {
                    o.addProperty("status", "fail");
                    o.addProperty("detail", "activate_nearest_block: no '" + blockType + "' within " + radius + "m");
                    break;
                }
                double dx = nearest.getX() + 0.5 - player.getX();
                double dy = nearest.getY() + 0.5 - (player.getY() + player.getEyeHeight());
                double dz = nearest.getZ() + 0.5 - player.getZ();
                double horiz = Math.sqrt(dx * dx + dz * dz);
                player.setYRot((float) Math.toDegrees(Math.atan2(-dx, dz)));
                player.setXRot((float) Math.toDegrees(-Math.atan2(dy, horiz)));
                var hit = new BlockHitResult(Vec3.atCenterOf(nearest), Direction.getNearest((int)Math.round(dx), (int)Math.round(dy), (int)Math.round(dz), Direction.UP), nearest, false);
                var result = player.gameMode.useItemOn(player, level, player.getMainHandItem(), InteractionHand.MAIN_HAND, hit);
                player.containerMenu.broadcastChanges();
                o.addProperty("activated", result.consumesAction());
                o.addProperty("x", nearest.getX());
                o.addProperty("y", nearest.getY());
                o.addProperty("z", nearest.getZ());
                o.addProperty("detail", "activate_nearest_block (" + nearest.getX() + "," + nearest.getY() + "," + nearest.getZ() + ") consumed=" + result.consumesAction());
                break;
            }
            // ══════════════════════════════════════════════════════════════
            // 获取详细合成计划（参考 mindcraft getCraftingPlan）
            // ══════════════════════════════════════════════════════════════
            case "get_crafting_plan": {
                String targetItem = req.has("item") ? req.get("item").getAsString() : "";
                int quantity = req.has("quantity") ? req.get("quantity").getAsInt() : 1;
                // 简单实现：返回当前库存中目标物品数量 + 提示
                Inventory inv = player.getInventory();
                int have = 0;
                for (int i = 0; i < inv.getContainerSize(); i++) {
                    ItemStack s = inv.getItem(i);
                    if (s.isEmpty()) continue;
                    String key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase();
                    if (key.contains(targetItem.toLowerCase())) have += s.getCount();
                }
                if (have >= quantity) {
                    o.addProperty("detail", "get_crafting_plan: already have " + have + " " + targetItem + " (need " + quantity + ")");
                } else {
                    o.addProperty("detail", "get_crafting_plan: have " + have + " " + targetItem + ", need " + quantity + " more. Use craft tool to make them.");
                }
                o.addProperty("have", have);
                o.addProperty("need", quantity);
                o.addProperty("missing", Math.max(0, quantity - have));
                break;
            }
            // ══════════════════════════════════════════════════════════════
            // 改进的 discard：三点组合（参考 mindcraft !discard）
            // moveAway 5米 + drop + goBack 原点
            // ══════════════════════════════════════════════════════════════
            case "discard_smart": {
                String itemName = req.has("item") ? req.get("item").getAsString() : "";
                int num = req.has("num") ? req.get("num").getAsInt() : 1;
                String search = itemName.replace("minecraft:", "").toLowerCase();
                Inventory inv = player.getInventory();
                // 记录原点
                double startX = player.getX(), startY = player.getY(), startZ = player.getZ();
                // 1. 远离 5 米（反向移动）
                float awayYaw = player.getYRot() + 180; // 反向
                double awayDx = -Math.sin(Math.toRadians(awayYaw)) * 5.0;
                double awayDz = Math.cos(Math.toRadians(awayYaw)) * 5.0;
                moveTarget = new double[]{startX + awayDx, startY, startZ + awayDz};
                moveTicksLeft = 100;
                long moveStart = System.currentTimeMillis();
                while (moveTarget != null && System.currentTimeMillis() - moveStart < 5000) {
                    if (shouldStop) { shouldStop = false; break; }
                    try { Thread.sleep(100); } catch (InterruptedException e) { break; }
                }
                // 2. 丢弃物品
                int dropped = 0;
                for (int i = 0; i < inv.getContainerSize() && dropped < num; i++) {
                    ItemStack s = inv.getItem(i);
                    if (s.isEmpty()) continue;
                    String key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase();
                    if (!key.contains(search)) continue;
                    int take = Math.min(s.getCount(), num - dropped);
                    ItemStack toDrop = s.copy();
                    toDrop.setCount(take);
                    s.shrink(take);
                    player.drop(toDrop, false);
                    dropped += take;
                }
                player.containerMenu.broadcastChanges();
                // 3. 返回原点
                try { Thread.sleep(500); } catch (InterruptedException e) { /* 等待物品落地 */ }
                moveTarget = new double[]{startX, startY, startZ};
                moveTicksLeft = 100;
                long returnStart = System.currentTimeMillis();
                while (moveTarget != null && System.currentTimeMillis() - returnStart < 5000) {
                    if (shouldStop) { shouldStop = false; break; }
                    try { Thread.sleep(100); } catch (InterruptedException e) { break; }
                }
                o.addProperty("dropped", dropped);
                o.addProperty("detail", "discard_smart " + itemName + " x" + dropped + " (moved away 5m, dropped, returned)");
                break;
            }
            default:
                o.addProperty("status", "fail");
                o.addProperty("detail", "未知命令: " + type);
        }
        return o;
    }

    // ══════════════════════════════════════════════════════════════
    // 放置/破坏/战斗
    // ══════════════════════════════════════════════════════════════

    private static boolean placeAt(ServerPlayer player, ServerLevel level, int x, int y, int z, String itemName) {
        double dist = player.position().distanceTo(Vec3.atCenterOf(new BlockPos(x, y, z)));
        if (dist > 5.5) return false;
        Inventory inv = player.getInventory();
        int slot = -1;
        String search = itemName.replace("minecraft:", "").toLowerCase();
        // 先找快捷栏
        for (int i = 0; i < 9; i++) {
            ItemStack s = inv.getItem(i);
            if (s.isEmpty()) continue;
            String key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase();
            if (key.contains(search)) { slot = i; break; }
        }
        // 再找主背包
        if (slot == -1) {
            for (int i = 9; i < inv.getContainerSize(); i++) {
                ItemStack s = inv.getItem(i);
                if (s.isEmpty()) continue;
                String key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase();
                if (key.contains(search)) { slot = i; break; }
            }
            if (slot == -1) return false;
            // 移到快捷栏
            int dstSlot = 0;
            for (int i = 0; i < 9; i++) {
                if (inv.getItem(i).isEmpty()) { dstSlot = i; break; }
            }
            ItemStack tmp = inv.getItem(dstSlot);
            inv.setItem(dstSlot, inv.getItem(slot));
            inv.setItem(slot, tmp);
            slot = dstSlot;
        }
        // 切到该栏位
        inv.setSelectedSlot(slot);
        player.containerMenu.broadcastChanges();

        BlockPos pos = new BlockPos(x, y, z);
        Direction[] dirOrder = {Direction.UP, Direction.NORTH, Direction.SOUTH, Direction.EAST, Direction.WEST, Direction.DOWN};
        for (Direction dir : dirOrder) {
            BlockPos neighbor = pos.relative(dir);
            BlockState ns = level.getBlockState(neighbor);
            if (!ns.isAir() && ns.isSolid()) {
                BlockHitResult hit = new BlockHitResult(Vec3.atCenterOf(pos), dir.getOpposite(), neighbor, false);
                if (player.gameMode.useItemOn(player, level, player.getMainHandItem(), InteractionHand.MAIN_HAND, hit).consumesAction()) {
                    return true;
                }
            }
        }
        return false;
    }

    private static boolean isHostile(String typeName) {
        String[] hostile = {"zombie", "skeleton", "creeper", "spider", "phantom", "witch", "enderman", "blaze", "ghast", "slime", "magma_cube", "pillager", "vindicator", "evoker", "ravager", "hoglin", "piglin", "zoglin", "warden", "wither", "dragon"};
        for (String h : hostile) { if (typeName.contains(h)) return true; }
        return false;
    }

    private static class CombatResult {
        String result = "none";
        String targetType = "";
    }

    private static CombatResult combat(ServerPlayer player, ServerLevel level, String mode, int maxTicks) {
        CombatResult cr = new CombatResult();
        long start = System.currentTimeMillis();
        long timeout = maxTicks * 50L;

        while (System.currentTimeMillis() - start < timeout) {
            LivingEntity target = null;
            double minDist = Double.MAX_VALUE;
            AABB scanArea = AABB.ofSize(player.position(), 32, 32, 32);
            for (Entity e : level.getEntities(player, scanArea)) {
                if (!(e instanceof LivingEntity le)) continue;
                String tn = BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).getPath();
                if (!isHostile(tn)) continue;
                double d = e.distanceTo(player);
                if (d < minDist) { minDist = d; target = le; }
            }
            if (target == null) { cr.result = "no_target"; break; }

            cr.targetType = BuiltInRegistries.ENTITY_TYPE.getKey(target.getType()).getPath();
            double dist = target.distanceTo(player);

            // 濒死撤退
            if (player.getHealth() < 5.0f) {
                cr.result = "retreated";
                float yaw = (float) Math.toDegrees(Math.atan2(-(player.getX() - target.getX()), player.getZ() - target.getZ()));
                player.setYRot(yaw);
                player.setDeltaMovement(-Math.cos(Math.toRadians(yaw)) * 0.28, 0, Math.sin(Math.toRadians(yaw)) * 0.28);
                break;
            }

            // 苦力怕后撤
            if (cr.targetType.contains("creeper") && dist < 6.0) {
                float yaw = (float) Math.toDegrees(Math.atan2(-(player.getX() - target.getX()), player.getZ() - target.getZ()));
                player.setYRot(yaw);
                player.setDeltaMovement(-Math.cos(Math.toRadians(yaw)) * 0.28, 0, Math.sin(Math.toRadians(yaw)) * 0.28);
                continue;
            }

            if (mode.equals("retreat")) {
                float yaw = (float) Math.toDegrees(Math.atan2(-(player.getX() - target.getX()), player.getZ() - target.getZ()));
                player.setYRot(yaw);
                player.setDeltaMovement(-Math.cos(Math.toRadians(yaw)) * 0.28, 0, Math.sin(Math.toRadians(yaw)) * 0.28);
                if (dist > 15.0) { cr.result = "retreated"; break; }
                continue;
            }

            equipBestWeapon(player);
            player.lookAt(EntityAnchorArgument.Anchor.EYES, target.position().add(0, 1.0, 0));

            if (dist > 4.0) {
                // 靠近
                float yaw = (float) Math.toDegrees(Math.atan2(-(target.getX() - player.getX()), target.getZ() - player.getZ()));
                player.setYRot(yaw);
                double nx = (target.getX() - player.getX()) / dist;
                double nz = (target.getZ() - player.getZ()) / dist;
                player.setDeltaMovement(nx * 0.28, player.getDeltaMovement().y, nz * 0.28);
            } else {
                // 攻击
                player.attack(target);
                player.containerMenu.broadcastChanges();
                if (mode.equals("kite")) {
                    // 风筝：攻击后后撤
                    float yaw = (float) Math.toDegrees(Math.atan2(-(player.getX() - target.getX()), player.getZ() - target.getZ()));
                    player.setYRot(yaw);
                    player.setDeltaMovement(-Math.cos(Math.toRadians(yaw)) * 0.28, 0, Math.sin(Math.toRadians(yaw)) * 0.28);
                }
            }

            if (!target.isAlive()) { cr.result = "killed"; break; }

            try { Thread.sleep(200); } catch (InterruptedException e) { Thread.currentThread().interrupt(); break; }
        }
        player.setDeltaMovement(0, player.getDeltaMovement().y, 0);
        if (cr.result.equals("none")) cr.result = "timeout";
        return cr;
    }

    private static void equipBestWeapon(ServerPlayer player) {
        Inventory inv = player.getInventory();
        int best = -1; double bestDmg = -1;
        for (int i = 0; i < 9; i++) {
            ItemStack s = inv.getItem(i);
            if (s.isEmpty()) continue;
            String key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString();
            if (key.contains("sword") || (key.contains("axe") && !key.contains("pickaxe"))) {
                double dmg = 4.0;
                if (key.contains("diamond")) dmg = 8.0;
                else if (key.contains("iron")) dmg = 6.0;
                else if (key.contains("stone")) dmg = 5.0;
                if (key.contains("sword")) dmg += 1.0;
                if (dmg > bestDmg) { bestDmg = dmg; best = i; }
            }
        }
        if (best >= 0) {
            inv.setSelectedSlot(best);
            player.containerMenu.broadcastChanges();
        }
    }

    private static void equipBestTool(ServerPlayer player, String blockId) {
        String b = blockId.toLowerCase();
        String toolType;
        if (b.contains("stone") || b.contains("cobble") || b.contains("ore") || b.contains("obsidian")
                || b.contains("granite") || b.contains("diorite") || b.contains("andesite")
                || b.contains("basalt") || b.contains("bricks") || b.contains("netherrack")) {
            toolType = "pickaxe";
        } else if (b.contains("log") || b.contains("planks") || b.contains("wood") || b.contains("leaves")
                || b.contains("crafting_table") || b.contains("chest") || b.contains("bookshelf")) {
            toolType = "axe";
        } else if (b.contains("dirt") || b.contains("grass") || b.contains("sand") || b.contains("gravel")
                || b.contains("snow") || b.contains("clay") || b.contains("podzol") || b.contains("mycelium")) {
            toolType = "shovel";
        } else {
            return;
        }

        Inventory inv = player.getInventory();
        int best = -1; int bestTier = -1;
        for (int i = 0; i < 9; i++) {
            int tier = toolTier(inv.getItem(i), toolType);
            if (tier > bestTier) { bestTier = tier; best = i; }
        }
        // 快捷栏没有，从主背包找
        if (bestTier <= 0) {
            for (int i = 9; i < inv.getContainerSize(); i++) {
                int tier = toolTier(inv.getItem(i), toolType);
                if (tier > bestTier) {
                    int dstSlot = 0;
                    for (int j = 0; j < 9; j++) {
                        if (inv.getItem(j).isEmpty()) { dstSlot = j; break; }
                    }
                    // 服务端直接交换（无需 handleInventoryMouseClick）
                    ItemStack tmp = inv.getItem(dstSlot);
                    inv.setItem(dstSlot, inv.getItem(i));
                    inv.setItem(i, tmp);
                    best = dstSlot; bestTier = tier;
                    break;
                }
            }
        }
        if (best >= 0 && bestTier > 0) {
            inv.setSelectedSlot(best);
            player.containerMenu.broadcastChanges();
        }
    }

    private static int toolTier(ItemStack stack, String toolType) {
        if (stack.isEmpty()) return 0;
        String key = BuiltInRegistries.ITEM.getKey(stack.getItem()).toString().toLowerCase();
        if (!key.contains(toolType)) return 0;
        if (key.contains("diamond")) return 4;
        if (key.contains("iron")) return 3;
        if (key.contains("stone")) return 2;
        if (key.contains("wooden") || key.contains("wood")) return 1;
        return 0;
    }

    // ══════════════════════════════════════════════════════════════
    // 合成/丢弃/烧制（服务端直接操作 Inventory）
    // ══════════════════════════════════════════════════════════════

    private static int craftItem(ServerPlayer player, String targetId, int want) {
        Inventory inv = player.getInventory();
        int crafted = 0;
        String t = targetId.toLowerCase();

        if (t.contains("planks") && countItem(inv, "log") > 0) {
            for (String log : new String[]{"oak_log","birch_log","spruce_log","jungle_log","acacia_log","dark_oak_log","mangrove_log","cherry_log"}) {
                while (crafted < want && countItem(inv, log) > 0) {
                    removeItem(inv, log, 1);
                    String plank = log.replace("_log","_planks");
                    addItem(inv, plank, 4); crafted += 4;
                }
            }
        }
        if (t.contains("stick")) {
            while (crafted < want && countItem(inv, "planks") >= 2) {
                removeItem(inv, "planks", 2); addItem(inv, "stick", 4); crafted += 4;
            }
        }
        if (t.contains("crafting_table")) {
            while (crafted < want && countItem(inv, "planks") >= 4) {
                removeItem(inv, "planks", 4); addItem(inv, "crafting_table", 1); crafted += 1;
            }
        }
        if (t.contains("wooden_pickaxe") || t.contains("wooden_axe")) {
            while (crafted < want && countItem(inv, "planks") >= 3 && countItem(inv, "stick") >= 2) {
                removeItem(inv, "planks", 3); removeItem(inv, "stick", 2);
                addItem(inv, t.contains("pickaxe") ? "wooden_pickaxe" : "wooden_axe", 1);
                crafted += 1;
            }
        }
        if (t.contains("wooden_sword")) {
            while (crafted < want && countItem(inv, "planks") >= 2 && countItem(inv, "stick") >= 1) {
                removeItem(inv, "planks", 2); removeItem(inv, "stick", 1);
                addItem(inv, "wooden_sword", 1); crafted += 1;
            }
        }
        if (t.contains("wooden_shovel")) {
            while (crafted < want && countItem(inv, "planks") >= 1 && countItem(inv, "stick") >= 2) {
                removeItem(inv, "planks", 1); removeItem(inv, "stick", 2);
                addItem(inv, "wooden_shovel", 1); crafted += 1;
            }
        }
        if (t.contains("stone_pickaxe") || t.contains("stone_axe")) {
            while (crafted < want && countItem(inv, "cobblestone") >= 3 && countItem(inv, "stick") >= 2) {
                removeItem(inv, "cobblestone", 3); removeItem(inv, "stick", 2);
                addItem(inv, t.contains("pickaxe") ? "stone_pickaxe" : "stone_axe", 1);
                crafted += 1;
            }
        }
        if (t.contains("stone_sword")) {
            while (crafted < want && countItem(inv, "cobblestone") >= 2 && countItem(inv, "stick") >= 1) {
                removeItem(inv, "cobblestone", 2); removeItem(inv, "stick", 1);
                addItem(inv, "stone_sword", 1); crafted += 1;
            }
        }
        if (t.contains("torch")) {
            while (crafted < want && countItem(inv, "stick") >= 1 && countItem(inv, "coal") >= 1) {
                removeItem(inv, "stick", 1); removeItem(inv, "coal", 1);
                addItem(inv, "torch", 4); crafted += 4;
            }
        }
        if (t.contains("furnace")) {
            while (crafted < want && countItem(inv, "cobblestone") >= 8) {
                removeItem(inv, "cobblestone", 8); addItem(inv, "furnace", 1); crafted += 1;
            }
        }
        if (t.contains("chest")) {
            while (crafted < want && countItem(inv, "planks") >= 8) {
                removeItem(inv, "planks", 8); addItem(inv, "chest", 1); crafted += 1;
            }
        }
        if (t.contains("iron_pickaxe") || t.contains("iron_axe")) {
            while (crafted < want && countItem(inv, "iron_ingot") >= 3 && countItem(inv, "stick") >= 2) {
                removeItem(inv, "iron_ingot", 3); removeItem(inv, "stick", 2);
                addItem(inv, t.contains("pickaxe") ? "iron_pickaxe" : "iron_axe", 1); crafted += 1;
            }
        }
        if (t.contains("iron_sword")) {
            while (crafted < want && countItem(inv, "iron_ingot") >= 2 && countItem(inv, "stick") >= 1) {
                removeItem(inv, "iron_ingot", 2); removeItem(inv, "stick", 1);
                addItem(inv, "iron_sword", 1); crafted += 1;
            }
        }
        if (t.contains("diamond_pickaxe") || t.contains("diamond_axe")) {
            while (crafted < want && countItem(inv, "diamond") >= 3 && countItem(inv, "stick") >= 2) {
                removeItem(inv, "diamond", 3); removeItem(inv, "stick", 2);
                addItem(inv, t.contains("pickaxe") ? "diamond_pickaxe" : "diamond_axe", 1); crafted += 1;
            }
        }
        return crafted;
    }

    private static int discardItem(ServerPlayer player, String itemId, int num) {
        Inventory inv = player.getInventory();
        int discarded = 0;
        String search = itemId.toLowerCase();
        for (int i = 0; i < inv.getContainerSize() && discarded < num; i++) {
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

    private static int smeltItem(ServerPlayer player, String itemId, int num) {
        Inventory inv = player.getInventory();
        int smelted = 0;
        String input = itemId.toLowerCase();
        String output = null;
        if (input.contains("raw_iron")) output = "iron_ingot";
        else if (input.contains("raw_copper")) output = "copper_ingot";
        else if (input.contains("raw_gold")) output = "gold_ingot";
        else if (input.contains("oak_log")) output = "charcoal";
        else if (input.contains("sand")) output = "glass";
        else if (input.contains("cobblestone")) output = "stone";
        if (output == null) return 0;
        while (smelted < num && countItem(inv, input) >= 1 && countItem(inv, "coal") >= 1) {
            removeItem(inv, input, 1);
            removeItem(inv, "coal", 1);
            addItem(inv, output, 1);
            smelted++;
        }
        return smelted;
    }

    // ══════════════════════════════════════════════════════════════
    // 辅助方法
    // ══════════════════════════════════════════════════════════════

    private static boolean matchesWhitelist(String id) {
        String lower = id.toLowerCase();
        for (String k : BLOCK_WHITELIST) { if (lower.contains(k)) return true; }
        return false;
    }

    private static JsonArray arr(double x, double y, double z) {
        JsonArray a = new JsonArray();
        a.add(x); a.add(y); a.add(z);
        return a;
    }

    private static float clamp(float v, float lo, float hi) { return Math.max(lo, Math.min(hi, v)); }
    private static double clamp(double v, double lo, double hi) { return Math.max(lo, Math.min(hi, v)); }

    private static int countItem(Inventory inv, String id) {
        String search = id.toLowerCase();
        int n = 0;
        for (int i = 0; i < inv.getContainerSize(); i++) {
            ItemStack s = inv.getItem(i);
            if (s.isEmpty()) continue;
            String key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase();
            if (key.endsWith(":" + search) || (!search.contains(":") && key.contains(search))) n += s.getCount();
        }
        return n;
    }

    private static void addItem(Inventory inv, String id, int count) {
        net.minecraft.world.item.Item exact = null;
        net.minecraft.world.item.Item fallback = null;
        String search = id.toLowerCase();
        for (net.minecraft.world.item.Item item : BuiltInRegistries.ITEM) {
            String key = BuiltInRegistries.ITEM.getKey(item).toString().toLowerCase();
            if (key.endsWith(":" + search)) { exact = item; break; }
            if (fallback == null && key.contains(search) && !key.contains("sticky")) { fallback = item; }
        }
        net.minecraft.world.item.Item target = exact != null ? exact : fallback;
        if (target != null) inv.add(new ItemStack(target, count));
    }

    private static void removeItem(Inventory inv, String id, int count) {
        String search = id.toLowerCase();
        for (int i = 0; i < inv.getContainerSize() && count > 0; i++) {
            ItemStack s = inv.getItem(i);
            if (s.isEmpty()) continue;
            String key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase();
            if (key.endsWith(":" + search) || key.contains(search)) {
                int take = Math.min(s.getCount(), count);
                s.shrink(take);
                count -= take;
            }
        }
    }
}
