package com.craftagent.bridge;

import com.google.gson.JsonObject;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.block.Blocks;
import com.craftagent.bridge.pathing.PlayerNavManager;
import java.util.HashMap;
import java.util.Map;

public class GoalEngine {

    public enum Status { IDLE, RUNNING, DONE, FAILED }
    public enum GoalType { CRAFT, GET, HUNT_FOOD, BUILD, SMELT, ENCHANT, NONE }

    private static GoalEngine instance;
    private GoalType currentGoal = GoalType.NONE;
    private String goalParam;
    private int goalCount;
    private int goalHave;
    private Status status = Status.IDLE;
    private String result = "";
    private int tickCooldown;

    private GoalEngine() {}

    public static synchronized GoalEngine get() {
        if (instance == null) instance = new GoalEngine();
        return instance;
    }

    public void start(String goalType, String param, int count) {
        stop();
        switch (goalType) {
            case "craft" -> { currentGoal = GoalType.CRAFT; goalParam = param; goalCount = count; }
            case "get"   -> { currentGoal = GoalType.GET;   goalParam = param; goalCount = count; }
            case "hunt"  -> { currentGoal = GoalType.HUNT_FOOD; goalParam = "food"; goalCount = 1; }
            case "build" -> { currentGoal = GoalType.BUILD;  goalParam = param; goalCount = 1; }
            case "smelt" -> { currentGoal = GoalType.SMELT;  goalParam = param; goalCount = count; }
            case "enchant" -> { currentGoal = GoalType.ENCHANT; goalParam = param; goalCount = count; }
            default -> { status = Status.FAILED; result = "unknown goal type: " + goalType; return; }
        }
        status = Status.RUNNING;
        tickCooldown = 0;
        result = "";
        goalHave = countInInventory(param);
        System.out.println("[goal] START " + goalType + " " + param + " x" + count + " (have=" + goalHave + ")");
    }

    public void stop() {
        currentGoal = GoalType.NONE;
        status = Status.IDLE;
        result = "";
    }

    public void tick() {
        if (status != Status.RUNNING) return;
        if (--tickCooldown > 0) return;
        tickCooldown = 10;

        ServerPlayer player = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
        if (player == null) return;

        switch (currentGoal) {
            case CRAFT   -> tickCraft(player);
            case GET     -> tickGet(player);
            case HUNT_FOOD -> tickHunt(player);
            case BUILD  -> tickBuild(player);
            case SMELT  -> tickSmelt(player);
            case ENCHANT -> tickEnchant(player);
            default -> {}
        }
    }

    private void tickCraft(ServerPlayer player) {
        int have = countInInventory(goalParam);
        if (have >= goalCount) {
            finish("crafted " + goalParam + " x" + goalCount + " (have=" + have + ")");
            return;
        }

        String[] materials = getRecipeMaterials(goalParam);
        if (materials == null) {
            finish("no recipe for " + goalParam);
            return;
        }

        for (String mat : materials) {
            String[] parts = mat.split(":");
            String matName = parts[0];
            int need = Integer.parseInt(parts[1]);
            int haveMat = countInInventory(matName);
            if (haveMat < need) {
                int missing = need - haveMat;
                System.out.println("[goal] need " + matName + " x" + missing + " for " + goalParam);
                // 递归：如果目标本身可合成，先合成
                if (getRecipeMaterials(matName) != null) {
                    start("craft", matName, missing);
                } else {
                    // 否则用 CollectController 采集
                    CollectController.get().start(matName, missing);
                    tickCooldown = 20;
                }
                return;
            }
        }

        // 所有材料齐了，合成
        tickCooldown = 20;
        CraftAgentBridge.serverInstance.executeIfPossible(() -> {
            try {
                var req = new JsonObject();
                req.addProperty("item", goalParam);
                req.addProperty("count", goalCount);
                ContainerController.actCraft(player, player.level(), req);
            } catch (Exception e) {
                System.out.println("[goal] craft failed: " + e.getMessage());
            }
        });
    }

    private void tickGet(ServerPlayer player) {
        int have = countInInventory(goalParam);
        if (have >= goalCount) {
            finish("got " + goalParam + " x" + goalCount);
            return;
        }

        // 检查是否有可合成的中间产物
        String[] subRecipe = getRecipeMaterials(goalParam);
        if (subRecipe != null) {
            // 这个物品可以合成，用 craft 目标
            start("craft", goalParam, goalCount - (int) have);
            return;
        }

        // 否则用 CollectController 自动采集
        CollectController.get().start(goalParam, goalCount - (int) have);
        tickCooldown = 10;
    }

    private void tickHunt(ServerPlayer player) {
        ServerLevel level = player.level();
        var animals = level.getEntities(player, player.getBoundingBox().inflate(16),
            e -> e instanceof net.minecraft.world.entity.animal.Animal);
        if (animals.isEmpty()) {
            finish("no animals nearby to hunt");
            return;
        }
        var target = animals.get(0);
        if (target.distanceTo(player) > 3.0) {
            if (!PlayerNavManager.get().isActive()) {
                PlayerNavManager.get().navigateTo(target.getX(), target.getY(), target.getZ());
            }
            return;
        }
        player.attack(target);
        System.out.println("[goal] HUNT attack " + BuiltInRegistries.ENTITY_TYPE.getKey(target.getType()).getPath());
    }

    private void tickBuild(ServerPlayer player) {
        finish("use build() tool directly for blueprints (GoalEngine build WIP)");
    }

    private void tickSmelt(ServerPlayer player) {
        String resultName = getSmeltResult(goalParam);
        if (resultName == null) { finish("cannot smelt " + goalParam); return; }
        int have = countInInventory(resultName);
        if (have >= goalCount) {
            finish("smelted " + goalParam + " -> " + resultName + " x" + goalCount);
            return;
        }
        int rawHave = countInInventory(goalParam);
        if (rawHave == 0) { finish("no " + goalParam + " to smelt"); return; }
        CraftAgentBridge.serverInstance.executeIfPossible(() -> {
            try {
                ContainerController.actSmelt(player, player.level(), buildReq(goalParam, 1));
            } catch (Exception e) {
                System.out.println("[goal] smelt failed: " + e.getMessage());
            }
        });
        tickCooldown = 40;
    }

    private void tickEnchant(ServerPlayer player) {
        if (player.experienceLevel < (int) goalCount) {
            finish("not enough XP levels (need " + goalCount + ", have " + player.experienceLevel + ")");
            return;
        }
        CraftAgentBridge.serverInstance.executeIfPossible(() -> {
            try {
                ContainerController.actEnchant(player, player.level(), buildReq(goalParam, goalCount));
            } catch (Exception e) {
                System.out.println("[goal] enchant failed: " + e.getMessage());
            }
        });
        finish("enchanted " + goalParam);
    }

    private int countInInventory(String item) {
        ServerPlayer player = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
        if (player == null) return 0;
        int count = 0;
        for (int i = 0; i < player.getInventory().getContainerSize(); i++) {
            ItemStack s = player.getInventory().getItem(i);
            if (!s.isEmpty() && BuiltInRegistries.ITEM.getKey(s.getItem()).toString().contains(item))
                count += s.getCount();
        }
        return count;
    }

    private String[] getRecipeMaterials(String item) {
        return switch (item) {
            case "wooden_pickaxe" -> new String[]{"planks:3", "stick:2"};
            case "wooden_axe" -> new String[]{"planks:3", "stick:2"};
            case "wooden_sword" -> new String[]{"planks:2", "stick:1"};
            case "stone_pickaxe" -> new String[]{"cobblestone:3", "stick:2"};
            case "stone_axe" -> new String[]{"cobblestone:3", "stick:2"};
            case "stone_sword" -> new String[]{"cobblestone:2", "stick:1"};
            case "iron_pickaxe" -> new String[]{"iron_ingot:3", "stick:2"};
            case "iron_axe" -> new String[]{"iron_ingot:3", "stick:2"};
            case "iron_sword" -> new String[]{"iron_ingot:2", "stick:1"};
            case "iron_shovel" -> new String[]{"iron_ingot:1", "stick:2"};
            case "diamond_pickaxe" -> new String[]{"diamond:3", "stick:2"};
            case "diamond_sword" -> new String[]{"diamond:2", "stick:1"};
            case "oak_planks" -> new String[]{"oak_log:1"};
            case "stick" -> new String[]{"planks:2"};
            case "crafting_table" -> new String[]{"planks:4"};
            case "furnace" -> new String[]{"cobblestone:8"};
            case "chest" -> new String[]{"planks:8"};
            case "torch" -> new String[]{"stick:1", "coal:1"};
            case "shield" -> new String[]{"planks:6", "iron_ingot:1"};
            case "oak_door" -> new String[]{"planks:6"};
            case "iron_helmet" -> new String[]{"iron_ingot:5"};
            case "iron_chestplate" -> new String[]{"iron_ingot:8"};
            case "iron_leggings" -> new String[]{"iron_ingot:7"};
            case "iron_boots" -> new String[]{"iron_ingot:4"};
            case "diamond_helmet" -> new String[]{"diamond:5"};
            case "diamond_chestplate" -> new String[]{"diamond:8"};
            case "diamond_leggings" -> new String[]{"diamond:7"};
            case "diamond_boots" -> new String[]{"diamond:4"};
            case "bow" -> new String[]{"stick:3", "string:3"};
            case "arrow" -> new String[]{"flint:1", "stick:1", "feather:1"};
            case "bucket" -> new String[]{"iron_ingot:3"};
            case "shears" -> new String[]{"iron_ingot:2"};
            case "flint_and_steel" -> new String[]{"iron_ingot:1", "flint:1"};
            case "fishing_rod" -> new String[]{"stick:3", "string:2"};
            case "compass" -> new String[]{"iron_ingot:4", "redstone:1"};
            case "clock" -> new String[]{"gold_ingot:4", "redstone:1"};
            default -> null;
        };
    }

    private String getSmeltResult(String raw) {
        return switch (raw) {
            case "raw_iron" -> "iron_ingot";
            case "raw_gold" -> "gold_ingot";
            case "raw_copper" -> "copper_ingot";
            case "sand" -> "glass";
            case "cobblestone" -> "stone";
            case "oak_log" -> "charcoal";
            default -> null;
        };
    }

    private void finish(String msg) {
        status = Status.DONE;
        result = msg;
        System.out.println("[goal] DONE: " + msg);
    }

    public Status status() { return status; }
    public String result() { return result; }
    public String statusString() {
        return switch (status) {
            case IDLE -> "idle";
            case RUNNING -> "running: " + currentGoal + " " + goalParam + " x" + goalCount;
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