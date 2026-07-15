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
                double ddx = tx - eye.x, ddy = ty - eye.y, ddz = tz - eye.z;
                double len = Math.sqrt(ddx * ddx + ddy * ddy + ddz * ddz);
                // MC 前向向量 = (-sin(yaw)cos(pitch), sin(pitch), cos(yaw)cos(pitch))
                // 反解：yaw = atan2(-dx, dz)，pitch = asin(dy/len)
                float yaw = (float) Math.toDegrees(Math.atan2(-ddx, ddz));
                float pitch = (float) Math.toDegrees(Math.asin(clamp(ddy / len, -1.0, 1.0)));
                player.setYRot(yaw);
                player.setXRot(clamp(pitch, -90f, 90f));
                o.addProperty("detail", "look_at " + tx + "," + ty + "," + tz);
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
                int ticks = req.has("ticks") ? req.get("ticks").getAsInt() : 60;
                int before = countLogs(player);
                KeyMapping attack = mc.options.keyAttack;
                holdKey(attack, ticks);
                int after = countLogs(player);
                o.addProperty("logs_before", before);
                o.addProperty("logs_after", after);
                o.addProperty("detail", "mine " + ticks + "ticks");
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
            default:
                o.addProperty("status", "fail");
                o.addProperty("detail", "未知命令: " + type);
        }
        return o;
    }

    /** 按住某键 ticks*50ms 后释放（RAII 式：任何异常也释放）。 */
    private void holdKey(KeyMapping km, int ticks) {
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

    /** 简易寻路：转向目标并前进，水平距离 < 1.5 或超时停止；卡墙则跳一下。 */
    private void moveToward(LocalPlayer player, Options options, double tx, double ty, double tz, int maxTicks) {
        KeyMapping fwd = options.keyUp; // 前进
        KeyMapping jump = options.keyJump;
        for (int i = 0; i < maxTicks; i++) {
            Vec3 p = player.position();
            double ddx = tx - p.x, ddz = tz - p.z;
            double horiz = Math.sqrt(ddx * ddx + ddz * ddz);
            if (horiz < 1.5) break;
            // 水平朝目标设 yaw（忽略 y 以稳定前进）
            float yaw = (float) Math.toDegrees(Math.atan2(-ddx, ddz));
            player.setYRot(yaw);
            fwd.setDown(true);
            if (player.horizontalCollision) {
                jump.setDown(true);
            }
            try {
                Thread.sleep(50);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                break;
            } finally {
                if (player.horizontalCollision) jump.setDown(false);
            }
        }
        fwd.setDown(false);
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

    /** 从玩家背包扣除材料并添加合成结果。覆盖核心早期配方。 */
    private static int craftItem(LocalPlayer player, Level level, String targetId, int want) {
        Inventory inv = player.getInventory();
        int crafted = 0;

        // 橡木原木 → 橡木木板 (1→4)
        if (targetId.contains("planks") || targetId.contains("plank")) {
            for (String log : new String[]{"oak_log", "birch_log", "spruce_log", "jungle_log", "acacia_log", "dark_oak_log", "mangrove_log", "cherry_log"}) {
                if (!targetId.contains(log.replace("_log", ""))) continue;
                while (crafted < want && countItem(inv, log) > 0) {
                    removeItem(inv, log, 1);
                    addItem(inv, targetId, 4);
                    crafted += 4;
                }
            }
        }
        // 木板 → 木棍 (2→4)
        if (targetId.contains("stick")) {
            while (crafted < want) {
                boolean found = false;
                for (String plank : new String[]{"oak_planks", "birch_planks", "spruce_planks", "jungle_planks", "acacia_planks", "dark_oak_planks", "mangrove_planks", "cherry_planks"}) {
                    if (countItem(inv, plank) >= 2) {
                        removeItem(inv, plank, 2);
                        addItem(inv, "stick", 4);
                        crafted += 4;
                        found = true;
                        break;
                    }
                }
                if (!found) break;
            }
        }
        // 木板 → 工作台 (4→1)
        if (targetId.contains("crafting_table")) {
            while (crafted < want) {
                boolean found = false;
                for (String plank : new String[]{"oak_planks", "birch_planks", "spruce_planks", "jungle_planks", "acacia_planks", "dark_oak_planks", "mangrove_planks", "cherry_planks"}) {
                    if (countItem(inv, plank) >= 4) {
                        removeItem(inv, plank, 4);
                        addItem(inv, "crafting_table", 1);
                        crafted += 1;
                        found = true;
                        break;
                    }
                }
                if (!found) break;
            }
        }
        // 木棍+木板 → 木镐 (3+2→1)
        if (targetId.contains("wooden_pickaxe")) {
            while (crafted < want) {
                boolean found = false;
                for (String plank : new String[]{"oak_planks", "birch_planks", "spruce_planks", "jungle_planks", "acacia_planks", "dark_oak_planks", "mangrove_planks", "cherry_planks"}) {
                    if (countItem(inv, plank) >= 3 && countItem(inv, "stick") >= 2) {
                        removeItem(inv, plank, 3);
                        removeItem(inv, "stick", 2);
                        addItem(inv, "wooden_pickaxe", 1);
                        crafted += 1;
                        found = true;
                        break;
                    }
                }
                if (!found) break;
            }
        }
        // 木棍+木板 → 木斧 (3+2→1)
        if (targetId.contains("wooden_axe")) {
            while (crafted < want) {
                boolean found = false;
                for (String plank : new String[]{"oak_planks", "birch_planks", "spruce_planks", "jungle_planks", "acacia_planks", "dark_oak_planks", "mangrove_planks", "cherry_planks"}) {
                    if (countItem(inv, plank) >= 3 && countItem(inv, "stick") >= 2) {
                        removeItem(inv, plank, 3);
                        removeItem(inv, "stick", 2);
                        addItem(inv, "wooden_axe", 1);
                        crafted += 1;
                        found = true;
                        break;
                    }
                }
                if (!found) break;
            }
        }
        // 木棍+木板 → 木剑 (2+1→1)
        if (targetId.contains("wooden_sword")) {
            while (crafted < want) {
                boolean found = false;
                for (String plank : new String[]{"oak_planks", "birch_planks", "spruce_planks", "jungle_planks", "acacia_planks", "dark_oak_planks", "mangrove_planks", "cherry_planks"}) {
                    if (countItem(inv, plank) >= 2 && countItem(inv, "stick") >= 1) {
                        removeItem(inv, plank, 2);
                        removeItem(inv, "stick", 1);
                        addItem(inv, "wooden_sword", 1);
                        crafted += 1;
                        found = true;
                        break;
                    }
                }
                if (!found) break;
            }
        }
        return crafted;
    }

    private static int countItem(Inventory inv, String id) {
        int n = 0;
        for (int i = 0; i < inv.getContainerSize(); i++) {
            ItemStack s = inv.getItem(i);
            if (BuiltInRegistries.ITEM.getKey(s.getItem()).toString().contains(id)) n += s.getCount();
        }
        return n;
    }
    private static void removeItem(Inventory inv, String id, int count) {
        for (int i = 0; i < inv.getContainerSize() && count > 0; i++) {
            ItemStack s = inv.getItem(i);
            if (BuiltInRegistries.ITEM.getKey(s.getItem()).toString().contains(id)) {
                int take = Math.min(s.getCount(), count);
                s.shrink(take);
                count -= take;
            }
        }
    }
    private static void addItem(Inventory inv, String id, int count) {
        // 通过注册表名映射到 Item 实例
        net.minecraft.world.item.Item target = null;
        for (net.minecraft.world.item.Item item : BuiltInRegistries.ITEM) {
            if (BuiltInRegistries.ITEM.getKey(item).toString().contains(id)) {
                target = item;
                break;
            }
        }
        if (target != null) {
            inv.add(new ItemStack(target, count));
        }
    }
}
