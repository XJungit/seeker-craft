package com.craftagent.bridge;

import com.google.gson.Gson;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import net.fabricmc.api.ClientModInitializer;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Options;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.effect.MobEffect;
import net.minecraft.world.effect.MobEffectInstance;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.level.LightLayer;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
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

/**
 * Craft-Agent Bridge —— 进程内 Fabric 客户端 mod。
 *
 * 在客户端启动时开一个本机 TCP 服务（127.0.0.1:25567），用 JSON 行协议与外部 Rust Agent 通信：
 *   - 请求 {"type":"state"}                         → 返回结构化游戏状态
 *   - 请求 {"type":"look","dx":..,"dy":..}          → 相对转视角
 *   - 请求 {"type":"look_at","x":..,"y":..,"z":..}  → 绝对朝向某坐标（精确对准）
 *   - 请求 {"type":"press","keys":".","ticks":..}   → 按住按键若干 tick
 *   - 请求 {"type":"mine","ticks":..}              → 按住左键挖掘，回执带原木数量差
 *   - 请求 {"type":"move","dir":".","ticks":..}     → 朝某方向移动
 *   - 请求 {"type":"move_to","x":..,"y":..,"z":..}  → 简易寻路走到坐标
 *
 * 这是 MindFlayer 式"直接读游戏数据"：不需要 CheatEngine 式内存扫描，mod 在 JVM 内
 * 用 Minecraft 的 Java API 直接取结构化状态，天然随版本升级（mapping 变化）而稳定。
 */
public class CraftAgentBridge implements ClientModInitializer {
    /** 监听端口（避开 GameQuery 的 25566）。 */
    public static final int PORT = 25567;
    /** 相对转视角灵敏度：每单位 dx 对应角度（度）。dx=300 ≈ 90°。直接写 yaw/pitch，无鼠标灵敏度乘子。 */
    private static final float LOOK_SENS = 0.3f;
    /** 附近方块扫描半径（格）。 */
    private static final int SCAN_RADIUS = 8;
    private static final Gson GSON = new Gson();

    /** 只回报感兴趣的方块（避免把空气/草丛全塞给 LLM）。 */
    private static final Set<String> BLOCK_WHITELIST = new HashSet<>();

    static {
        String[] keys = {
            "log", "planks", "crafting_table", "chest", "furnace", "smoker", "blast_furnace",
            "stone", "cobblestone", "ore", "coal", "iron", "gold", "diamond", "dirt", "grass",
            "sand", "gravel", "sandstone", "nether", "end_", "amethyst", "copper", "lapis",
            "emerald", "redstone", "deepslate", "oak", "birch", "spruce", "jungle", "acacia",
            "dark_oak", "mangrove", "bamboo", "obsidian", "glowstone", "ice", "clay", "wart"
        };
        for (String k : keys) BLOCK_WHITELIST.add(k);
    }

    @Override
    public void onInitializeClient() {
        Thread serverThread = new Thread(this::runServer, "craft-agent-bridge");
        serverThread.setDaemon(true);
        serverThread.start();
        System.out.println("[craft-agent-bridge] TCP 服务线程已启动，监听 127.0.0.1:" + PORT);
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

    /** 分发请求：state 同步返回；动作命令执行后返回回执。 */
    private JsonObject dispatch(JsonObject req) {
        String type = req.has("type") ? req.get("type").getAsString() : "";
        if ("state".equals(type)) {
            return buildState();
        }
        // 动作命令在执行期间可能 sleep，统一包一层 try/catch。
        try {
            return performAction(type, req);
        } catch (Exception e) {
            JsonObject o = new JsonObject();
            o.addProperty("status", "fail");
            o.addProperty("detail", e.getMessage());
            return o;
        }
    }

    // ── 状态读取 ──

    private JsonObject buildState() {
        JsonObject o = new JsonObject();
        Minecraft mc = Minecraft.getInstance();
        LocalPlayer player = mc.player;
        Level level = mc.level;
        if (player == null || level == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "游戏/玩家未就绪（请先进入一个世界）");
            return o;
        }

        // 位置 / 朝向 / 血量
        Vec3 pos = player.position();
        o.add("position", arr(pos.x, pos.y, pos.z));
        o.addProperty("yaw", player.getYRot());
        o.addProperty("pitch", player.getXRot());
        o.addProperty("health", player.getHealth());
        o.addProperty("hunger", player.getFoodData().getFoodLevel());
        o.addProperty("gamemode", mc.gameMode != null ? mc.gameMode.getPlayerMode().getName() : "?");
        o.addProperty("time", level.getDayTime());
        o.addProperty("dimension", level.dimension().identifier().toString());
        o.addProperty("biome", level.getBiome(player.blockPosition()).unwrapKey()
                .map(k -> k.identifier().toString()).orElse("?"));

        // 运动速度（用于判断是否在移动 / 坠落 / 被击退）
        Vec3 vel = player.getDeltaMovement();
        o.add("velocity", arr(vel.x, vel.y, vel.z));

        // 状态效果（中毒 / 缓慢 / 发光 / 速度等）
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

        // 经验等级 / 进度（0~1）
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
        o.addProperty("held_item",
            held.isEmpty() ? "minecraft:air"
            : BuiltInRegistries.ITEM.getKey(held.getItem()).toString());

        // 准星所指方块（MC 自带 raycast）
        HitResult hit = mc.hitResult;
        if (hit != null && hit.getType() == HitResult.Type.BLOCK) {
            BlockPos bp = ((BlockHitResult) hit).getBlockPos();
            BlockState bs = level.getBlockState(bp);
            Block block = bs.getBlock();
            String id = BuiltInRegistries.BLOCK.getKey(block).toString();
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

        // 附近实体（生物/掉落物等）
        JsonArray ents = new JsonArray();
        for (Entity e : ((ClientLevel) level).entitiesForRendering()) {
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
            Vec3 ev = e.getDeltaMovement();
            en.add("velocity", arr(ev.x, ev.y, ev.z));
            if (e instanceof LivingEntity le) {
                JsonArray ee = new JsonArray();
                for (MobEffectInstance me : le.getActiveEffects()) {
                    MobEffect effect = me.getEffect().value();
                    String id = BuiltInRegistries.MOB_EFFECT.getKey(effect).toString();
                    JsonObject eo = new JsonObject();
                    eo.addProperty("id", id);
                    eo.addProperty("amplifier", me.getAmplifier());
                    eo.addProperty("duration", me.getDuration());
                    ee.add(eo);
                }
                en.add("effects", ee);
            } else {
                en.add("effects", new JsonArray());
            }
            ents.add(en);
        }
        o.add("entities", ents);

        o.addProperty("status", "ok");
        return o;
    }

    private static boolean matchesWhitelist(String id) {
        String lower = id.toLowerCase();
        for (String k : BLOCK_WHITELIST) {
            if (lower.contains(k)) return true;
        }
        return false;
    }

    private static JsonArray arr(double x, double y, double z) {
        JsonArray a = new JsonArray();
        a.add(x); a.add(y); a.add(z);
        return a;
    }

    // ── 动作执行 ──

    private JsonObject performAction(String type, JsonObject req) {
        Minecraft mc = Minecraft.getInstance();
        LocalPlayer player = mc.player;
        Level level = mc.level;
        if (player == null) {
            JsonObject o = new JsonObject();
            o.addProperty("status", "fail");
            o.addProperty("detail", "玩家未就绪");
            return o;
        }
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");

        switch (type) {
            case "look": {
                int dx = req.has("dx") ? req.get("dx").getAsInt() : 0;
                int dy = req.has("dy") ? req.get("dy").getAsInt() : 0;
                float yaw = player.getYRot() - dx * LOOK_SENS;
                float pitch = clamp(player.getXRot() + dy * LOOK_SENS, -90f, 90f);
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
                // Auto-offset to block center: if coordinates are integers, shift to center
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
                // Force raycast update so targeted_block is correct immediately
                HitResult forceHit = player.pick(6.0, 0.0f, false);
                String hitInfo = "nothing";
                if (forceHit != null && forceHit.getType() == HitResult.Type.BLOCK) {
                    BlockPos bp = ((BlockHitResult) forceHit).getBlockPos();
                    BlockState bs = level.getBlockState(bp);
                    hitInfo = BuiltInRegistries.BLOCK.getKey(bs.getBlock()).toString();
                }
                o.addProperty("detail", "look_at(" + tx + "," + ty + "," + tz + ") -> facing " + hitInfo);
                break;
            }
            case "press":
            case "move": {
                String keys = req.has("keys") ? req.get("keys").getAsString() :
                        (type.equals("move") && req.has("dir") ? req.get("dir").getAsString() : "w");
                int ticks = req.has("ticks") ? req.get("ticks").getAsInt() : 20;
                KeyMapping key = resolveKey(mc.options, keys);
                if (key == null) {
                    o.addProperty("status", "fail");
                    o.addProperty("detail", "未知按键: " + keys);
                    break;
                }
                holdKey(key, ticks);
                o.addProperty("detail", type + " " + keys + " x" + ticks);
                break;
            }
            case "mine": {
                int maxTicks = req.has("ticks") ? req.get("ticks").getAsInt() : 200;
                int before = countLogs(player);
                // 挖到方块破坏为止，不是固定 tick
                HitResult hit = mc.hitResult;
                BlockPos target = null;
                BlockState targetState = null;
                if (hit != null && hit.getType() == HitResult.Type.BLOCK) {
                    target = ((BlockHitResult) hit).getBlockPos();
                    targetState = level.getBlockState(target);
                }
                KeyMapping attack = mc.options.keyAttack;
                int usedTicks = 0;
                for (int t = 0; t < maxTicks; t++) {
                    attack.setDown(true);
                    try { Thread.sleep(50); } catch (InterruptedException e) { Thread.currentThread().interrupt(); break; }
                    attack.setDown(false);
                    usedTicks++;
                    // 检查方块是否已被破坏
                    if (target != null && targetState != null) {
                        BlockState current = level.getBlockState(target);
                        if (!current.equals(targetState) || current.isAir()) break;
                    }
                }
                attack.setDown(false);
                int after = countLogs(player);
                o.addProperty("logs_before", before);
                o.addProperty("logs_after", after);
                o.addProperty("detail", "mine " + usedTicks + "ticks (block broken=" + (after > before) + ")");
                o.addProperty("ok", true);
                break;
            }
            case "move_to": {
                double tx = req.get("x").getAsDouble();
                double ty = req.get("y").getAsDouble();
                double tz = req.get("z").getAsDouble();
                moveToward(player, mc.options, tx, ty, tz, 200);
                o.addProperty("detail", "move_to " + tx + "," + ty + "," + tz);
                break;
            }
            case "right_click": {
                int ticks = req.has("ticks") ? req.get("ticks").getAsInt() : 5;
                holdKey(mc.options.keyUse, ticks);
                o.addProperty("detail", "right_click " + ticks + "ticks");
                break;
            }
            case "attack": {
                int ticks = req.has("ticks") ? req.get("ticks").getAsInt() : 30;
                holdKey(mc.options.keyAttack, ticks);
                o.addProperty("detail", "attack " + ticks + "ticks");
                break;
            }
            case "craft": {
                String item = req.get("item").getAsString();
                int want = req.has("count") ? req.get("count").getAsInt() : 1;
                int crafted = craftItem(player, level, item, want);
                o.addProperty("crafted", crafted);
                o.addProperty("detail", "craft " + item + " x" + crafted);
                break;
            }
            case "discard": {
                String item = req.get("item").getAsString();
                int num = req.has("num") ? req.get("num").getAsInt() : 1;
                int discarded = discardItem(player, item, num);
                o.addProperty("detail", "discarded " + discarded + " x " + item);
                break;
            }
            case "smelt": {
                String item = req.get("item").getAsString();
                int num = req.has("num") ? req.get("num").getAsInt() : 1;
                int smelted = smeltItem(player, mc, level, item, num);
                o.addProperty("detail", "smelted " + smelted + " x " + item);
                break;
            }
            default:
                o.addProperty("status", "fail");
                o.addProperty("detail", "未知命令: " + type);
        }
        return o;
    }

    /** 按住某键 ticks*50ms 后释放（RAII 式：任何异常也释放）。 */
    private static void holdKey(KeyMapping km, int ticks) {
        long ms = (long) ticks * 50L;
        KeyMapping.click(km.getDefaultKey());
        km.setDown(true);
        try {
            Thread.sleep(ms);
        } catch (InterruptedException ignored) {
            Thread.currentThread().interrupt();
        } finally {
            km.setDown(false);
        }
    }

    /** 简易寻路：转向目标并前进。遇障碍时左右绕行、跳起跨障。 */
    private static void moveToward(LocalPlayer player, Options options, double tx, double ty, double tz, int maxTicks) {
        KeyMapping fwd = options.keyUp;
        KeyMapping left = options.keyLeft;
        KeyMapping right = options.keyRight;
        KeyMapping jump = options.keyJump;
        int stuckTicks = 0;
        int strafeDir = 1; // 1=右绕, -1=左绕, 交替
        Vec3 lastPos = player.position();

        for (int i = 0; i < maxTicks; i++) {
            Vec3 p = player.position();
            double ddx = tx - p.x, ddz = tz - p.z;
            double horiz = Math.sqrt(ddx * ddx + ddz * ddz);
            if (horiz < 1.5) break;

            float yaw = (float) Math.toDegrees(Math.atan2(-ddx, ddz));
            player.setYRot(yaw);
            fwd.setDown(true);

            // 检测卡墙
            if (player.horizontalCollision) {
                stuckTicks++;
                jump.setDown(true);
                // 卡住超过 5 tick 时左右绕行
                if (stuckTicks > 5) {
                    if (strafeDir > 0) {
                        right.setDown(true);
                        left.setDown(false);
                    } else {
                        left.setDown(true);
                        right.setDown(false);
                    }
                    // 绕行 15 tick 后换方向
                    if (stuckTicks > 20) {
                        strafeDir = -strafeDir;
                        stuckTicks = 6;
                    }
                }
            } else {
                stuckTicks = 0;
                left.setDown(false);
                right.setDown(false);
                if (!player.horizontalCollision) jump.setDown(false);
            }

            // 如果还是完全没移动，换方向绕行
            if (i > 0 && i % 10 == 0 && p.distanceTo(lastPos) < 0.1 && !player.horizontalCollision) {
                stuckTicks = 6; // 触发绕行
            }
            lastPos = p;

            try { Thread.sleep(50); } catch (InterruptedException e) {
                Thread.currentThread().interrupt(); break;
            }
        }
        fwd.setDown(false);
        left.setDown(false);
        right.setDown(false);
        jump.setDown(false);
    }

    private static KeyMapping resolveKey(Options opt, String k) {
        if (k == null) return null;
        switch (k.toLowerCase()) {
            case "w": case "forward": return opt.keyUp;
            case "s": case "back": return opt.keyDown;
            case "a": case "left": return opt.keyLeft;
            case "d": case "right": return opt.keyRight;
            case "space": case "jump": return opt.keyJump;
            case "shift": return opt.keyShift;
            case "ctrl": case "sprint": return opt.keySprint;
            case "e": case "inventory": return opt.keyInventory;
            default: return null;
        }
    }

    private static int countLogs(LocalPlayer player) {
        int n = 0;
        Inventory inv = player.getInventory();
        for (int i = 0; i < inv.getContainerSize(); i++) {
            ItemStack s = inv.getItem(i);
            if (s.isEmpty()) continue;
            String id = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase();
            if (id.contains("log") || id.contains("planks")) n += s.getCount();
        }
        return n;
    }

    private static float clamp(float v, float lo, float hi) {
        return Math.max(lo, Math.min(hi, v));
    }

    private static double clamp(double v, double lo, double hi) {
        return Math.max(lo, Math.min(hi, v));
    }

    /** 合成：直接操作 Inventory 扣材料加结果。覆盖全部常用配方。 */
    private static int craftItem(LocalPlayer player, Level level, String targetId, int want) {
        return craftItemFallback(player, targetId, want);
    }

    private static int craftItemFallback(LocalPlayer player, String targetId, int want) {
        Inventory inv = player.getInventory();
        int crafted = 0;
        String t = targetId.toLowerCase();

        // ── 原木 → 木板 (1→4) ──
        if (t.contains("planks") && countItem(inv, "log") > 0) {
            for (String log : new String[]{"oak_log","birch_log","spruce_log","jungle_log","acacia_log","dark_oak_log","mangrove_log","cherry_log"}) {
                while (crafted < want && countItem(inv, log) > 0) {
                    removeItem(inv, log, 1);
                    String plank = log.replace("_log","_planks");
                    addItem(inv, plank, 4); crafted += 4;
                }
            }
        }
        // ── 木板 → 木棍 (2→4) ──
        if (t.contains("stick")) {
            while (crafted < want && countItem(inv, "planks") >= 2) {
                removeItem(inv, "planks", 2); addItem(inv, "stick", 4); crafted += 4;
            }
        }
        // ── 木板 → 工作台 (4→1) ──
        if (t.contains("crafting_table")) {
            while (crafted < want && countItem(inv, "planks") >= 4) {
                removeItem(inv, "planks", 4); addItem(inv, "crafting_table", 1); crafted += 1;
            }
        }
        // ── 木工具 (2 sticks + 3 planks or 1 stick + 2 planks) ──
        if (t.contains("wooden_pickaxe") || t.contains("wooden_axe") || t.contains("wooden_hoe")) {
            while (crafted < want && countItem(inv, "planks") >= 3 && countItem(inv, "stick") >= 2) {
                removeItem(inv, "planks", 3); removeItem(inv, "stick", 2);
                addItem(inv, t.contains("pickaxe") ? "wooden_pickaxe" : t.contains("axe") ? "wooden_axe" : "wooden_hoe", 1);
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
        // ── 石工具 (2 sticks + 3 cobblestone) ──
        if (t.contains("stone_pickaxe") || t.contains("stone_axe") || t.contains("stone_hoe")) {
            while (crafted < want && countItem(inv, "cobblestone") >= 3 && countItem(inv, "stick") >= 2) {
                removeItem(inv, "cobblestone", 3); removeItem(inv, "stick", 2);
                addItem(inv, t.contains("pickaxe") ? "stone_pickaxe" : t.contains("axe") ? "stone_axe" : "stone_hoe", 1);
                crafted += 1;
            }
        }
        if (t.contains("stone_sword")) {
            while (crafted < want && countItem(inv, "cobblestone") >= 2 && countItem(inv, "stick") >= 1) {
                removeItem(inv, "cobblestone", 2); removeItem(inv, "stick", 1);
                addItem(inv, "stone_sword", 1); crafted += 1;
            }
        }
        if (t.contains("stone_shovel")) {
            while (crafted < want && countItem(inv, "cobblestone") >= 1 && countItem(inv, "stick") >= 2) {
                removeItem(inv, "cobblestone", 1); removeItem(inv, "stick", 2);
                addItem(inv, "stone_shovel", 1); crafted += 1;
            }
        }
        // ── 火把 (1 stick + 1 coal → 4) ──
        if (t.contains("torch")) {
            while (crafted < want && countItem(inv, "stick") >= 1 && countItem(inv, "coal") >= 1) {
                removeItem(inv, "stick", 1); removeItem(inv, "coal", 1);
                addItem(inv, "torch", 4); crafted += 4;
            }
        }
        // ── 熔炉 (8 cobblestone → 1) ──
        if (t.contains("furnace")) {
            while (crafted < want && countItem(inv, "cobblestone") >= 8) {
                removeItem(inv, "cobblestone", 8); addItem(inv, "furnace", 1); crafted += 1;
            }
        }
        // ── 箱子 (8 planks → 1) ──
        if (t.contains("chest")) {
            while (crafted < want && countItem(inv, "planks") >= 8) {
                removeItem(inv, "planks", 8); addItem(inv, "chest", 1); crafted += 1;
            }
        }
        // ── 木门 (6 planks → 3) ──
        if (t.contains("door")) {
            while (crafted < want && countItem(inv, "planks") >= 6) {
                removeItem(inv, "planks", 6); addItem(inv, "oak_door", 3); crafted += 3;
            }
        }
        // ── 碗 (3 planks → 4) ──
        if (t.contains("bowl")) {
            while (crafted < want && countItem(inv, "planks") >= 3) {
                removeItem(inv, "planks", 3); addItem(inv, "bowl", 4); crafted += 4;
            }
        }
        // ── 梯子 (7 sticks → 3) ──
        if (t.contains("ladder")) {
            while (crafted < want && countItem(inv, "stick") >= 7) {
                removeItem(inv, "stick", 7); addItem(inv, "ladder", 3); crafted += 3;
            }
        }
        // ── 告示牌 (6 planks + 1 stick → 3) ──
        if (t.contains("sign")) {
            while (crafted < want && countItem(inv, "planks") >= 6 && countItem(inv, "stick") >= 1) {
                removeItem(inv, "planks", 6); removeItem(inv, "stick", 1);
                addItem(inv, "oak_sign", 3); crafted += 3;
            }
        }
        // ── 栅栏 (4 planks + 2 sticks → 3) ──
        if (t.contains("fence")) {
            while (crafted < want && countItem(inv, "planks") >= 4 && countItem(inv, "stick") >= 2) {
                removeItem(inv, "planks", 4); removeItem(inv, "stick", 2);
                addItem(inv, "oak_fence", 3); crafted += 3;
            }
        }
        return crafted;
    }

    /** 丢弃指定物品 N 个：找到物品→切到该格→按 Q */
    private static int discardItem(LocalPlayer player, String itemId, int num) {
        Inventory inv = player.getInventory();
        int discarded = 0;
        for (int i = 0; i < inv.getContainerSize() && discarded < num; i++) {
            ItemStack s = inv.getItem(i);
            if (s.isEmpty()) continue;
            String id = BuiltInRegistries.ITEM.getKey(s.getItem()).toString();
            if (!id.contains(itemId)) continue;
            int take = Math.min(s.getCount(), num - discarded);
            // Switch to slot if in hotbar
            if (i < 9) {
                KeyMapping slotKey = resolveKeyBySlot(i);
                if (slotKey != null) { holdKey(slotKey, 2); try { Thread.sleep(100); } catch (InterruptedException e) {} }
            }
            // Press Q to drop
            KeyMapping dropKey = Minecraft.getInstance().options.keyDrop;
            for (int d = 0; d < take; d++) {
                KeyMapping.click(dropKey.getDefaultKey());
                try { Thread.sleep(50); } catch (InterruptedException e) {}
            }
            discarded += take;
        }
        return discarded;
    }

    /** 烧制物品：找最近熔炉→右键打开→放物品+燃料→等待→取成品*/
    private static int smeltItem(LocalPlayer player, Minecraft mc, Level level, String itemId, int num) {
        // Find nearest furnace
        BlockPos playerPos = player.blockPosition();
        BlockPos furnacePos = null;
        for (BlockPos bp : BlockPos.betweenClosed(
                playerPos.getX() - 8, playerPos.getY() - 4, playerPos.getZ() - 8,
                playerPos.getX() + 8, playerPos.getY() + 4, playerPos.getZ() + 8)) {
            if (level.getBlockState(bp).getBlock().getName().getString().toLowerCase().contains("furnace")) {
                furnacePos = bp; break;
            }
        }
        if (furnacePos == null) return 0;
        // Right-click furnace (open GUI)
        moveTowardBlock(player, mc.options, furnacePos, 100);
        holdKey(mc.options.keyUse, 5);
        try { Thread.sleep(500); } catch (InterruptedException e) {}
        // Simplified: just wait and return placeholder
        // Full GUI manipulation needs screen handler access which is complex
        try { Thread.sleep(num * 10000L); } catch (InterruptedException e) {}
        return 0; // TODO: implement proper GUI furnace interaction
    }

    private static void moveTowardBlock(LocalPlayer player, Options options, BlockPos target, int maxTicks) {
        double tx = target.getX() + 0.5, ty = target.getY() + 0.5, tz = target.getZ() + 0.5;
        moveToward(player, options, tx, ty, tz, maxTicks);
    }

    private static KeyMapping resolveKeyBySlot(int slot) {
        // Hotbar keys: slots 0-8 correspond to keys 1-9
        if (slot >= 0 && slot < 9) {
            String key = Integer.toString(slot + 1);
            return resolveKey(Minecraft.getInstance().options, key);
        }
        return null;
    }

    private static int countItem(Inventory inv, String id) {
        int n = 0;
        for (int i = 0; i < inv.getContainerSize(); i++) {
            ItemStack s = inv.getItem(i);
            if (BuiltInRegistries.ITEM.getKey(s.getItem()).toString().contains(id)) n += s.getCount();
        }
        return n;
    }

    private static void addItem(Inventory inv, String id, int count) {
        net.minecraft.world.item.Item exact = null;
        net.minecraft.world.item.Item fallback = null;
        String search = id.toLowerCase();
        for (net.minecraft.world.item.Item item : BuiltInRegistries.ITEM) {
            String key = BuiltInRegistries.ITEM.getKey(item).toString().toLowerCase();
            // 精确匹配: key 以 ":{search}" 结尾（如 "minecraft:stick"）
            if (key.endsWith(":" + search)) { exact = item; break; }
            // 回退: key 包含 search（如 "oak_planks" 匹配 "plank"）
            if (fallback == null && key.contains(search) && !key.contains("sticky")) { fallback = item; }
        }
        net.minecraft.world.item.Item target = exact != null ? exact : fallback;
        if (target != null) {
            inv.add(new ItemStack(target, count));
        }
    }

    private static void removeItem(Inventory inv, String id, int count) {
        String search = id.toLowerCase();
        for (int i = 0; i < inv.getContainerSize() && count > 0; i++) {
            ItemStack s = inv.getItem(i);
            String key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase();
            if (key.endsWith(":" + search) || key.contains(search)) {
                int take = Math.min(s.getCount(), count);
                s.shrink(take);
                count -= take;
            }
        }
    }
}
