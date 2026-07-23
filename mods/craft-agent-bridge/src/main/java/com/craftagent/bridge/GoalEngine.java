package com.craftagent.bridge;

import com.google.gson.JsonObject;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.item.ItemStack;
import com.craftagent.bridge.pathing.PlayerNavManager;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Deque;
import java.util.List;

/**
 * GoalEngine — LLM 只发目标（craft/get/hunt/smelt/enchant/build），Mod 自动分解→执行→容错。
 * 使用目标栈实现递归子目标：父目标缺材料时把子目标压栈，子目标完成后再回到父目标继续。
 */
public class GoalEngine {

    public enum Status { IDLE, RUNNING, PAUSED, DONE, FAILED }

    // 单个目标节点
    private static class Goal {
        String type;     // craft / get / smelt / enchant / hunt / build
        String param;    // 物品名
        int count;
        Goal(String t, String p, int c) { type = t; param = p; count = c; }
    }

    private static GoalEngine instance;
    private Deque<Goal> stack = new ArrayDeque<>();
    private Status status = Status.IDLE;
    private String result = "";
    private int tickCooldown;
    private int rootTypeHash; // 用于 statusString 显示根目标

    private GoalEngine() {}

    public static synchronized GoalEngine get() {
        if (instance == null) instance = new GoalEngine();
        return instance;
    }

    public void start(String goalType, String param, int count) {
        stack.clear();
        if ("explore".equals(goalType)) {
            stack.push(new Goal("explore", param.isEmpty() ? "cave" : param, count));
        } else if ("defend".equals(goalType)) {
            stack.push(new Goal("defend", param, count));
        } else {
            stack.push(new Goal(goalType, param, count));
        }
        status = Status.RUNNING;
        tickCooldown = 0;
        result = "";
        System.out.println("[goal] START " + goalType + " " + param + " x" + count);
    }

    public void stop() {
        stack.clear();
        status = Status.IDLE;
        result = "";
    }

    public void tick() {
        if (status != Status.RUNNING && status != Status.PAUSED) return;

        // ── 容错：血量/饥饿过低暂停，等 autoSurvive 处理 ──
        ServerPlayer p0 = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
        if (p0 != null) {
            float hp = p0.getHealth();
            float hunger = p0.getFoodData().getFoodLevel();
            if (hp < 6.0f || hunger < 4.0f) {
                if (status != Status.PAUSED) {
                    status = Status.PAUSED;
                    System.out.println("[goal] PAUSED (hp=" + hp + " hunger=" + hunger + ") — waiting for autoSurvive");
                }
                return;
            } else if (status == Status.PAUSED) {
                status = Status.RUNNING;
                System.out.println("[goal] RESUMED");
            }
        }

        if (--tickCooldown > 0) return;
        tickCooldown = 8;

        ServerPlayer player = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
        if (player == null) return;

        if (stack.isEmpty()) {
            status = Status.FAILED;
            result = "goal stack empty (unexpected)";
            return;
        }

        Goal g = stack.peek();
        switch (g.type) {
            case "craft"   -> tickCraft(player, g);
            case "get"     -> tickGet(player, g);
            case "smelt"   -> tickSmelt(player, g);
            case "enchant" -> tickEnchant(player, g);
            case "hunt"    -> tickHunt(player, g);
            case "build"   -> tickBuild(player, g);
            case "explore" -> tickExplore(player, g);
            case "defend"  -> tickDefend(player, g);
            default -> { popDone("unknown goal type: " + g.type); }
        }
    }

    // ───────────────────────── craft ─────────────────────────
    private void tickCraft(ServerPlayer player, Goal g) {
        int have = countInInventory(g.param);
        if (have >= g.count) {
            popDone("crafted " + g.param + " x" + g.count + " (have=" + have + ")");
            return;
        }

        // 成品配方材料（与 CraftingHelper 一致）
        List<String> ings = requiredIngredients(g.param);
        if (ings == null) {
            popDone("no recipe for " + g.param);
            return;
        }

        // 先尝试合成（材料够则直接出）。tick() 已在服务端线程执行，直接同步调用即可，
        // 避免 executeIfPossible 延迟导致下方同步检查结果看到旧状态而误判材料缺失。
        try {
            var req = new JsonObject();
            req.addProperty("item", g.param);
            req.addProperty("count", g.count);
            var r = ContainerController.actCraft(player, player.level(), req);
            System.out.println("[goal] actCraft(" + g.param + ") -> " + r.get("crafted") + " | log=" + countInInventory("log") + " planks=" + countInInventory("planks"));
        } catch (Exception e) {
            System.out.println("[goal] craft failed: " + e.getMessage());
        }
        tickCooldown = 16;

        int after = countInInventory(g.param);
        if (after >= g.count) {
            popDone("crafted " + g.param + " x" + after);
            return;
        }

        // 找第一个仍缺失的材料，压入对应获取子目标
        for (String ing : ings) {
            String[] parts = ing.split(":");
            String matName = parts[0];
            int need = Integer.parseInt(parts[1]) * g.count;
            int haveMat = countInInventory(matName);
            if (haveMat < need) {
                Goal sub = obtainGoal(matName, need);
                System.out.println("[goal] need " + matName + " x" + need + " (have=" + haveMat + ") -> push " + sub.type + " " + sub.param);
                stack.push(sub);
                return;
            }
        }
        popDone("cannot craft " + g.param + " (missing materials or unsupported recipe)");
    }

    // ───────────────────────── get ─────────────────────────
    private void tickGet(ServerPlayer player, Goal g) {
        int have = countInInventory(g.param);
        if (have >= g.count) {
            popDone("got " + g.param + " x" + g.count);
            return;
        }
        // 若能合成（有成品配方）则转 craft，否则采集
        if (requiredIngredients(g.param) != null) {
            stack.push(new Goal("craft", g.param, g.count - have));
            return;
        }
        String block = collectTargetFor(g.param);
        if (block == null) { popDone("don't know how to get " + g.param); return; }
        CollectController.get().start(block, g.count - have);
        tickCooldown = 16;
        int after = countInInventory(g.param);
        if (after >= g.count) popDone("got " + g.param + " x" + g.count);
    }

    // ───────────────────────── smelt ─────────────────────────
    private void tickSmelt(ServerPlayer player, Goal g) {
        String out = smeltOutput(g.param);
        if (out == null) { popDone("cannot smelt " + g.param); return; }
        int have = countInInventory(out);
        if (have >= g.count) { popDone("smelted -> " + out + " x" + g.count); return; }
        int raw = countInInventory(g.param);
        if (raw == 0) {
            // 没有原料：先采集对应矿石（iron_ore→raw_iron 等）
            String ore = switch (g.param) {
                case "raw_iron" -> "iron_ore";
                case "raw_copper" -> "copper_ore";
                case "raw_gold" -> "gold_ore";
                default -> null;
            };
            if (ore != null) {
                // 只在 CollectController 空闲时才启动，避免每 tick 重置导致采矿永远推不动
                if (!CollectController.get().statusString().startsWith("running")) {
                    CollectController.get().start(ore, g.count);
                }
                tickCooldown = 16;
                if (countInInventory(g.param) == 0) return;
            } else {
                popDone("no " + g.param + " to smelt");
                return;
            }
        }
        int coal = countInInventory("coal");
        if (coal == 0) {
            // 需要燃料：先采集煤（同样仅在空闲时启动）
            if (!CollectController.get().statusString().startsWith("running")) {
                CollectController.get().start("coal_ore", 1);
            }
            tickCooldown = 16;
            if (countInInventory("coal") == 0) { popDone("no coal to smelt (fuel missing)"); return; }
        }
        try {
            ContainerController.actSmelt(player, player.level(), buildReq(g.param, g.count));
        } catch (Exception e) {
            System.out.println("[goal] smelt failed: " + e.getMessage());
        }
        tickCooldown = 24;
    }

    // ───────────────────────── hunt ─────────────────────────
    private void tickHunt(ServerPlayer player, Goal g) {
        ServerLevel level = player.level();
        var animals = level.getEntities(player, player.getBoundingBox().inflate(20),
            e -> e instanceof net.minecraft.world.entity.animal.Animal);
        if (animals.isEmpty()) {
            // 动物已清完：把生肉烤成熟肉（烧肉）
            int raw = countInInventory("raw") + countInInventory("meat");
            if (raw > 0 && countInInventory("coal") > 0) {
                try {
                    // 烤所有可能的生肉
                    for (String r : new String[]{"raw_beef","raw_porkchop","raw_chicken","raw_mutton","raw_rabbit","raw_fish","salmon","cod"}) {
                        if (countInInventory(r) > 0) ContainerController.actSmelt(player, level, buildReq(r, countInInventory(r)));
                    }
                } catch (Exception e) { System.out.println("[goal] cook failed: " + e.getMessage()); }
                tickCooldown = 20;
                if (countInInventory("raw") + countInInventory("meat") == 0) {
                    popDone("hunted and cooked meat");
                }
                return;
            }
            popDone("hunted (no meat to cook)");
            return;
        }
        var target = animals.get(0);
        if (target.distanceTo(player) > 2.5) {
            if (!PlayerNavManager.get().isActive()) {
                PlayerNavManager.get().navigateTo(target.getX(), target.getY(), target.getZ());
            }
            return;
        }
        player.attack(target);
        System.out.println("[goal] HUNT attack " + BuiltInRegistries.ENTITY_TYPE.getKey(target.getType()).getPath());
        tickCooldown = 12;
    }

    // ───────────────────────── build ─────────────────────────
    private void tickBuild(ServerPlayer player, Goal g) {
        // 简化版建造：收集指定方块 N 个，在脚下沿 X 轴铺一排（placeholder 蓝图）。
        String block = collectTargetFor(g.param);
        if (block == null) { popDone("unknown build target: " + g.param); return; }
        int have = countInInventory(g.param);
        int need = 8;
        if (have < need) {
            CollectController.get().start(block, need - have);
            tickCooldown = 16;
            if (countInInventory(g.param) < need) return;
        }
        // 放置一排
        try {
            int placed = 0;
            for (int dx = 1; dx <= 8 && placed < need; dx++) {
                var req = new JsonObject();
                req.addProperty("item", g.param);
                req.addProperty("x", (int) player.getX() + dx);
                req.addProperty("y", (int) player.getY() - 1);
                req.addProperty("z", (int) player.getZ());
                InteractionController.actPlaceAt(player, player.level(), req);
                placed++;
            }
            System.out.println("[goal] BUILD placed " + placed + " x " + g.param);
        } catch (Exception e) {
            System.out.println("[goal] build failed: " + e.getMessage());
        }
        popDone("built " + g.param + " (basic row)");
    }

    // ───────────────────────── explore ─────────────────────────
    // 简化探索：朝一个方向走一段距离，沿途收集遇到的矿物/木头，直到到达或超时。
    private int exploreTicks = 0;
    private void tickExplore(ServerPlayer player, Goal g) {
        exploreTicks += 8;
        // 顺路采集附近的矿/木（非阻塞，让 CollectController 推进）
        if (!CollectController.get().statusString().startsWith("running")) {
            CollectController.get().start("log", 4);
        }
        if (!PlayerNavManager.get().isActive()) {
            double dir = Math.toRadians(player.getYRot());
            double tx = player.getX() + Math.sin(dir) * 24.0;
            double tz = player.getZ() + Math.cos(dir) * 24.0;
            PlayerNavManager.get().navigateTo(tx, player.getY(), tz);
        }
        if (exploreTicks > 600) { // 约 30 秒
            exploreTicks = 0;
            popDone("explored around (" + String.format("%.0f,%.0f", player.getX(), player.getZ()) + ")");
        }
    }

    // ───────────────────────── defend ─────────────────────────
    // 站桩防御：清除周围敌对生物，无威胁则结束。
    private void tickDefend(ServerPlayer player, Goal g) {
        ServerLevel level = player.level();
        var hostiles = level.getEntities(player, player.getBoundingBox().inflate(16),
            e -> e instanceof net.minecraft.world.entity.Mob
                 && InventoryHelper.isHostile(BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).getPath()));
        if (hostiles.isEmpty()) {
            popDone("area secure (no hostiles)");
            return;
        }
        var target = hostiles.get(0);
        if (target.distanceTo(player) > 3.0) {
            if (!PlayerNavManager.get().isActive()) {
                PlayerNavManager.get().navigateTo(target.getX(), target.getY(), target.getZ());
            }
            return;
        }
        InventoryHelper.equipBestWeapon(player);
        player.attack(target);
        tickCooldown = 10;
    }

    // ───────────────────────── enchant ─────────────────────────
    private void tickEnchant(ServerPlayer player, Goal g) {
        if (player.experienceLevel < g.count) {
            popDone("not enough XP levels (need " + g.count + ", have " + player.experienceLevel + ")");
            return;
        }
        try {
            ContainerController.actEnchant(player, player.level(), buildReq(g.param, g.count));
        } catch (Exception e) {
            System.out.println("[goal] enchant failed: " + e.getMessage());
        }
        popDone("enchanted " + g.param);
    }

    // ───────────────────────── 材料解析 ─────────────────────────
    /** 返回把 param 合成出来所需的【成品配方材料名】（与 CraftingHelper 一致）。null = 无配方。 */
    private List<String> requiredIngredients(String item) {
        return switch (item) {
            case "wooden_pickaxe", "wooden_axe" -> List.of("planks:3", "stick:2");
            case "wooden_sword"   -> List.of("planks:2", "stick:1");
            case "wooden_shovel"  -> List.of("planks:1", "stick:2");
            case "stone_pickaxe", "stone_axe" -> List.of("cobblestone:3", "stick:2");
            case "stone_sword"    -> List.of("cobblestone:2", "stick:1");
            case "iron_pickaxe", "iron_axe" -> List.of("iron_ingot:3", "stick:2");
            case "iron_sword"     -> List.of("iron_ingot:2", "stick:1");
            case "iron_shovel"    -> List.of("iron_ingot:1", "stick:2");
            case "diamond_pickaxe", "diamond_axe" -> List.of("diamond:3", "stick:2");
            case "diamond_sword"  -> List.of("diamond:2", "stick:1");
            case "oak_planks","birch_planks","spruce_planks","jungle_planks","acacia_planks","dark_oak_planks","planks" -> List.of("log:1");
            case "stick"          -> List.of("planks:2");
            case "crafting_table" -> List.of("planks:4");
            case "furnace"        -> List.of("cobblestone:8");
            case "chest"          -> List.of("planks:8");
            case "torch"          -> List.of("stick:1", "coal:1");
            case "shield"         -> List.of("planks:6", "iron_ingot:1");
            case "iron_helmet"    -> List.of("iron_ingot:5");
            case "iron_chestplate"-> List.of("iron_ingot:8");
            case "iron_leggings"  -> List.of("iron_ingot:7");
            case "iron_boots"     -> List.of("iron_ingot:4");
            case "diamond_helmet" -> List.of("diamond:5");
            case "diamond_chestplate" -> List.of("diamond:8");
            case "diamond_leggings"   -> List.of("diamond:7");
            case "diamond_boots"      -> List.of("diamond:4");
            case "oak_door" -> List.of("planks:6");
            case "bucket" -> List.of("iron_ingot:3");
            case "shears" -> List.of("iron_ingot:2");
            case "flint_and_steel" -> List.of("iron_ingot:1");
            default -> null;
        };
    }

    /** 给定一种成品配方材料，返回如何获取它的子目标（自动处理 矿→raw→ingot 链）。 */
    private Goal obtainGoal(String ingredient, int count) {
        return switch (ingredient) {
            case "planks"   -> new Goal("craft", "planks", count);
            case "stick"    -> new Goal("craft", "stick", count);
            case "cobblestone" -> new Goal("get", "stone", count);
            case "iron_ingot"  -> new Goal("smelt", "raw_iron", count);
            case "copper_ingot" -> new Goal("smelt", "raw_copper", count);
            case "gold_ingot"  -> new Goal("smelt", "raw_gold", count);
            case "diamond"  -> new Goal("get", "diamond_ore", count);
            case "coal"     -> new Goal("get", "coal_ore", count);
            case "raw_iron" -> new Goal("get", "iron_ore", count);
            case "raw_copper" -> new Goal("get", "copper_ore", count);
            case "raw_gold" -> new Goal("get", "gold_ore", count);
            case "log"      -> new Goal("get", "log", count);
            default         -> new Goal("get", ingredient, count);
        };
    }

    /** 给定物品名，返回应采集的方块 id 子串（用于 CollectController / get）。 */
    private String collectTargetFor(String item) {
        String i = item.toLowerCase();
        if (i.contains("log") || i.contains("planks") || i.contains("wood")) return "log";
        if (i.contains("cobblestone") || i.contains("stone")) return "stone";
        if (i.contains("iron_ingot") || i.contains("raw_iron")) return "iron_ore";
        if (i.contains("coal")) return "coal_ore";
        if (i.contains("diamond")) return "diamond_ore";
        if (i.contains("gold")) return "gold_ore";
        if (i.contains("copper")) return "copper_ore";
        if (i.contains("sand")) return "sand";
        if (i.contains("glass")) return "glass";
        if (i.contains("dirt")) return "dirt";
        if (i.contains("sandstone")) return "sandstone";
        if (i.contains("oak") || i.contains("birch") || i.contains("spruce") || i.contains("jungle")
            || i.contains("acacia") || i.contains("dark_oak")) return i.contains("_log") ? i : i + "_log";
        // 兜底：直接用原名当方块子串
        return item;
    }

    private String smeltOutput(String raw) {
        return switch (raw) {
            case "raw_iron" -> "iron_ingot";
            case "raw_copper" -> "copper_ingot";
            case "raw_gold" -> "gold_ingot";
            case "oak_log","birch_log","spruce_log","jungle_log","acacia_log","dark_oak_log" -> "charcoal";
            case "sand" -> "glass";
            case "cobblestone" -> "stone";
            case "stone" -> "smooth_stone";
            default -> null;
        };
    }

    // ───────────────────────── 工具方法 ─────────────────────────
    private void popDone(String msg) {
        stack.pop();
        if (stack.isEmpty()) {
            status = Status.DONE;
            result = msg;
            System.out.println("[goal] DONE: " + msg);
        } else {
            System.out.println("[goal] subgoal done: " + msg + " (remaining stack=" + stack.size() + ")");
        }
    }

    private int countInInventory(String item) {
        ServerPlayer player = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
        if (player == null) return 0;
        int count = 0;
        String search = item.replace("minecraft:", "").toLowerCase();
        for (int i = 0; i < player.getInventory().getContainerSize(); i++) {
            ItemStack s = player.getInventory().getItem(i);
            if (!s.isEmpty() && BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase().contains(search))
                count += s.getCount();
        }
        return count;
    }

    public Status status() { return status; }
    /** #3/#5 仲裁：GoalEngine 是否正在执行 LLM 委托的复合目标（占用移动/战斗控制）。 */
    public boolean isRunning() { return status == Status.RUNNING; }
    public String result() { return result; }
    /** #4 可观测性：返回当前目标栈（从根到栈顶），供 LLM 感知 GoalEngine 正在做什么。 */
    public java.util.List<String> progressStack() {
        java.util.List<String> out = new java.util.ArrayList<>();
        int idx = 1;
        for (Goal g : stack) {
            out.add((idx++) + "/" + stack.size() + " " + g.type + " " + g.param + " x" + g.count);
        }
        return out;
    }
    public String statusString() {
        return switch (status) {
            case IDLE -> "idle";
            case RUNNING -> "running: stack=" + stack.size() + " top=" + (stack.peek() == null ? "?" : stack.peek().type + " " + stack.peek().param + " x" + stack.peek().count);
            case PAUSED -> "paused (low hp/hunger): " + (stack.peek() == null ? "?" : stack.peek().type + " " + stack.peek().param);
            case DONE -> "done: " + result;
            case FAILED -> "failed: " + result;
        };
    }

    private static JsonObject buildReq(String item, int count) {
        JsonObject req = new JsonObject();
        req.addProperty("item", item);
        req.addProperty("count", count);
        req.addProperty("levels", count);
        return req;
    }
}
