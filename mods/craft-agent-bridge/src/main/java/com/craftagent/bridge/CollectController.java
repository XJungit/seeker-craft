package com.craftagent.bridge;

import com.google.gson.JsonObject;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import com.craftagent.bridge.pathing.PlayerNavManager;
import java.util.HashSet;
import java.util.Set;

public class CollectController {

    private static CollectController instance;
    private String targetBlock;
    private int targetCount;
    private int collected;
    private int totalAttempts;
    private String status = "idle";
    private String result = "";
    private Set<String> searchedTypes;

    private CollectController() {}

    public static synchronized CollectController get() {
        if (instance == null) instance = new CollectController();
        return instance;
    }

    public JsonObject start(String target, int count) {
        JsonObject o = new JsonObject();
        this.targetBlock = target;
        this.targetCount = count;
        this.collected = 0;
        this.totalAttempts = 0;
        this.searchedTypes = new HashSet<>();
        this.status = "running";
        this.result = "";
        System.out.println("[collect] START " + target + " x" + count);
        o.addProperty("status", "ok");
        o.addProperty("detail", "collect " + target + " x" + count + " started");
        return o;
    }

    public void tick() {
        if (!"running".equals(status)) return;

        ServerPlayer player = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
        if (player == null) return;
        if (PlayerNavManager.get().isActive()) return;

        ServerLevel level = (ServerLevel) player.level();

        // Check if we already have enough
        int have = countInInventory(player, targetBlock);
        if (have >= targetCount) {
            finish("collected " + targetBlock + ": 0→" + have + " (+" + have + ", wanted " + targetCount + ")");
            return;
        }

        // Find nearest block
        BlockPos found = findBlock(level, player, targetBlock, 30);
        if (found == null) {
            finish("collected " + targetBlock + ": no more " + targetBlock + " nearby");
            return;
        }

        // Navigate to it
        totalAttempts++;
        if (player.blockPosition().distSqr(found) > 4) {
            PlayerNavManager.get().navigateTo(found.getX() + 0.5, found.getY(), found.getZ() + 0.5);
            return;
        }

        // Destroy it
        BlockState bs = level.getBlockState(found);
        if (bs.isAir() || bs.canBeReplaced() || bs.getBlock() == Blocks.BEDROCK) {
            searchedTypes.add(found.toShortString());
            return;
        }
        String blockId = BuiltInRegistries.BLOCK.getKey(bs.getBlock()).toString();
        InventoryHelper.equipBestTool(player, blockId);
        if (level.destroyBlock(found, true)) {
            int after = countInInventory(player, targetBlock);
            int gained = after - have;
            collected += gained;
            System.out.println("[collect] DUG " + found.toShortString() + " (" + blockId + ") gained=" + gained);
        }
    }

    private BlockPos findBlock(ServerLevel level, ServerPlayer player, String target, int range) {
        BlockPos center = player.blockPosition();
        BlockPos best = null;
        double bestDist = range * range;
        String targetClean = target.replace("minecraft:", "");
        for (int dx = -range; dx <= range; dx++) {
            for (int dz = -range; dz <= range; dz++) {
                for (int dy = -5; dy <= 5; dy++) {
                    BlockPos bp = center.offset(dx, dy, dz);
                    String key = bp.toShortString();
                    if (searchedTypes.contains(key)) continue;
                    String id = BuiltInRegistries.BLOCK.getKey(level.getBlockState(bp).getBlock()).toString();
                    if (id.contains(targetClean) && bp.distSqr(center) < bestDist) {
                        bestDist = bp.distSqr(center);
                        best = bp;
                    }
                }
            }
        }
        return best;
    }

    private int countInInventory(ServerPlayer player, String item) {
        int count = 0;
        String clean = item.replace("minecraft:", "");
        for (int i = 0; i < player.getInventory().getContainerSize(); i++) {
            var s = player.getInventory().getItem(i);
            if (!s.isEmpty() && BuiltInRegistries.ITEM.getKey(s.getItem()).toString().contains(clean))
                count += s.getCount();
        }
        return count;
    }

    private void finish(String msg) {
        status = "done";
        result = msg;
        System.out.println("[collect] DONE: " + msg);
    }

    public void stop() {
        status = "idle";
        result = "";
    }

    public String statusString() {
        return status + ": " + result;
    }

    public static JsonObject actCollect(ServerPlayer player, ServerLevel level, JsonObject req) {
        String target = req.get("target").getAsString();
        int count = req.has("count") ? req.get("count").getAsInt() : 1;
        return CollectController.get().start(target, count);
    }

    public static JsonObject actCollectStatus(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        o.addProperty("detail", CollectController.get().statusString());
        return o;
    }
}