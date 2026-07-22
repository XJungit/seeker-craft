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
        stack.push(new Goal(goalType, param, count));
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

        // 解析材料：返回子目标列表（craft/get/smelt/collect）
        List<Goal> mats = resolveMaterials(g.param, g.count - have);
        if (mats == null) {
            popDone("no recipe for " + g.param);
            return;
        }

        // 检查是否所有材料都已齐（直接尝试合成，craftItem 内部会校验）
        CraftAgentBridge.serverInstance.executeIfPossible(() -> {
            try {
                var req = new JsonObject();
                req.addProperty("item", g.param);
                req.addProperty("count", g.count);
                ContainerController.actCraft(player, player.level(), req);
            } catch (Exception e) {
                System.out.println("[goal] craft failed: " + e.getMessage());
            }
        });
        tickCooldown = 16;

        // 合成后再检查，若仍缺材料则展开第一个缺失子目标压栈
        int after = countInInventory(g.param);
        if (after < g.count) {
            for (Goal m : mats) {
                if (countInInventory(m.param) < m.count) {
                    System.out.println("[goal] need " + m.param + " x" + m.count + " -> push subgoal");
                    stack.push(m);
                    return;
                }
            }
            // 材料够但合成没出（可能配方不支持），结束并提示
            if (after >= 1) { popDone("crafted " + g.param + " x" + after); }
            else { popDone("cannot craft " + g.param + " (missing materials or unsupported recipe)"); }
        }
    }

    // ───────────────────────── get ─────────────────────────
    private void tickGet(ServerPlayer player, Goal g) {
        int have = countInInventory(g.param);
        if (have >= g.count) {
            popDone("got " + g.param + " x" + g.count);
            return;
        }
        // 若能合成则转 craft，否则采集
        if (resolveMaterials(g.param, 1) != null) {
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
        if (raw == 0) { popDone("no " + g.param + " to smelt"); return; }
        int coal = countInInventory("coal");
        if (coal == 0) {
            // 需要燃料：先采集煤
            CollectController.get().start("coal_ore", 1);
            tickCooldown = 16;
            if (countInInventory("coal") == 0) { popDone("no coal to smelt (fuel missing)"); return; }
        }
        CraftAgentBridge.serverInstance.executeIfPossible(() -> {
            try {
                ContainerController.actSmelt(player, player.level(), buildReq(g.param, g.count));
            } catch (Exception e) {
                System.out.println("[goal] smelt failed: " + e.getMessage());
            }
        });
        tickCooldown = 24;
    }

    // ───────────────────────── hunt ─────────────────────────
    private void tickHunt(ServerPlayer player, Goal g) {
        ServerLevel level = player.level();
        var animals = level.getEntities(player, player.getBoundingBox().inflate(20),
            e -> e instanceof net.minecraft.world.entity.animal.Animal);
        if (animals.isEmpty()) {
            popDone("no animals nearby to hunt");
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
        CraftAgentBridge.serverInstance.executeIfPossible(() -> {
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
        });
        popDone("built " + g.param + " (basic row)");
    }

    // ───────────────────────── enchant ─────────────────────────
    private void tickEnchant(ServerPlayer player, Goal g) {
        if (player.experienceLevel < g.count) {
            popDone("not enough XP levels (need " + g.count + ", have " + player.experienceLevel + ")");
            return;
        }
        CraftAgentBridge.serverInstance.executeIfPossible(() -> {
            try {
                ContainerController.actEnchant(player, player.level(), buildReq(g.param, g.count));
            } catch (Exception e) {
                System.out.println("[goal] enchant failed: " + e.getMessage());
            }
        });
        popDone("enchanted " + g.param);
    }

    // ───────────────────────── 材料解析 ─────────────────────────
    /** 返回把 param 合成出来所需的子目标（已计算数量）。null = 无配方（应采集/已知基础物）。 */
    private List<Goal> resolveMaterials(String item, int count) {
        return switch (item) {
            case "wooden_pickaxe" -> List.of(new Goal("craft","planks",3*count), new Goal("craft","stick",2*count));
            case "wooden_axe"     -> List.of(new Goal("craft","planks",3*count), new Goal("craft","stick",2*count));
            case "wooden_sword"   -> List.of(new Goal("craft","planks",2*count), new Goal("craft","stick",1*count));
            case "wooden_shovel"  -> List.of(new Goal("craft","planks",1*count), new Goal("craft","stick",2*count));
            case "stone_pickaxe","stone_axe" -> List.of(new Goal("craft","cobblestone",3*count), new Goal("craft","stick",2*count));
            case "stone_sword"    -> List.of(new Goal("craft","cobblestone",2*count), new Goal("craft","stick",1*count));
            case "iron_pickaxe","iron_axe" -> List.of(new Goal("smelt","raw_iron",3*count), new Goal("craft","stick",2*count));
            case "iron_sword"     -> List.of(new Goal("smelt","raw_iron",2*count), new Goal("craft","stick",1*count));
            case "iron_shovel"    -> List.of(new Goal("smelt","raw_iron",1*count), new Goal("craft","stick",2*count));
            case "diamond_pickaxe","diamond_axe" -> List.of(new Goal("craft","diamond",3*count), new Goal("craft","stick",2*count));
            case "diamond_sword"  -> List.of(new Goal("craft","diamond",2*count), new Goal("craft","stick",1*count));
            case "oak_planks","birch_planks","spruce_planks","jungle_planks","acacia_planks","dark_oak_planks" -> List.of(new Goal("get","log",1*count));
            case "stick"          -> List.of(new Goal("craft","planks",2*count));
            case "crafting_table" -> List.of(new Goal("craft","planks",4*count));
            case "furnace"        -> List.of(new Goal("get","cobblestone",8*count));
            case "chest"          -> List.of(new Goal("craft","planks",8*count));
            case "torch"          -> List.of(new Goal("craft","stick",1*count), new Goal("get","coal",1*count));
            case "shield"         -> List.of(new Goal("craft","planks",6*count), new Goal("smelt","raw_iron",1*count));
            case "iron_helmet"    -> List.of(new Goal("smelt","raw_iron",5*count));
            case "iron_chestplate"-> List.of(new Goal("smelt","raw_iron",8*count));
            case "iron_leggings"  -> List.of(new Goal("smelt","raw_iron",7*count));
            case "iron_boots"     -> List.of(new Goal("smelt","raw_iron",4*count));
            case "diamond_helmet" -> List.of(new Goal("craft","diamond",5*count));
            case "diamond_chestplate" -> List.of(new Goal("craft","diamond",8*count));
            case "diamond_leggings"   -> List.of(new Goal("craft","diamond",7*count));
            case "diamond_boots"      -> List.of(new Goal("craft","diamond",4*count));
            case "bow"      -> List.of(new Goal("craft","stick",3*count), new Goal("get","string",3*count));
            case "arrow"    -> List.of(new Goal("get","flint",1*count), new Goal("craft","stick",1*count), new Goal("get","feather",1*count));
            case "bucket","shears","flint_and_steel" -> List.of(new Goal("smelt","raw_iron", Math.max(1,(item.equals("bucket")?3:item.equals("shears")?2:1))*count));
            case "oak_door" -> List.of(new Goal("craft","planks",6*count));
            default -> null;
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
    public String result() { return result; }
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
