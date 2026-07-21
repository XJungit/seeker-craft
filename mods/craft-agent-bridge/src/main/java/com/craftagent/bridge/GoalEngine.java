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
                tickCooldown = 5;
                gatherMaterial(player, matName, need - haveMat);
                return;
            }
        }

        tickCooldown = 20;
        CraftAgentBridge.serverInstance.executeIfPossible(() -> {
            try {
                ContainerController.actCraft(player, player.level(), buildReq(goalParam, goalCount));
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

        gatherMaterial(player, goalParam, goalCount - have);
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

    private void gatherMaterial(ServerPlayer player, String material, int need) {
        if (PlayerNavManager.get().isActive()) return;
        ServerLevel level = player.level();
        BlockPos found = findNearestBlock(level, player, material, 30);
        if (found == null) {
            finish("no " + material + " nearby");
            return;
        }
        PlayerNavManager.get().navigateTo(found.getX() + 0.5, found.getY(), found.getZ() + 0.5);
        System.out.println("[goal] GATHER " + material + " at " + found.toShortString());
    }

    private BlockPos findNearestBlock(ServerLevel level, ServerPlayer player, String blockType, int range) {
        BlockPos center = player.blockPosition();
        BlockPos best = null;
        double bestDist = range * range;
        for (int dx = -range; dx <= range; dx++) {
            for (int dz = -range; dz <= range; dz++) {
                for (int dy = -5; dy <= 5; dy++) {
                    BlockPos bp = center.offset(dx, dy, dz);
                    String id = BuiltInRegistries.BLOCK.getKey(level.getBlockState(bp).getBlock()).toString();
                    if (id.contains(blockType) && bp.distSqr(center) < bestDist) {
                        bestDist = bp.distSqr(center);
                        best = bp;
                    }
                }
            }
        }
        return best;
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
            case "iron_pickaxe" -> new String[]{"iron_ingot:3", "stick:2"};
            case "iron_sword" -> new String[]{"iron_ingot:2", "stick:1"};
            case "stone_pickaxe" -> new String[]{"cobblestone:3", "stick:2"};
            case "stone_sword" -> new String[]{"cobblestone:2", "stick:1"};
            case "furnace" -> new String[]{"cobblestone:8"};
            case "chest" -> new String[]{"planks:8"};
            case "oak_planks" -> new String[]{"oak_log:1"};
            case "stick" -> new String[]{"planks:2"};
            case "crafting_table" -> new String[]{"planks:4"};
            case "torch" -> new String[]{"stick:1", "coal:1"};
            case "iron_helmet" -> new String[]{"iron_ingot:5"};
            case "iron_chestplate" -> new String[]{"iron_ingot:8"};
            case "iron_leggings" -> new String[]{"iron_ingot:7"};
            case "iron_boots" -> new String[]{"iron_ingot:4"};
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