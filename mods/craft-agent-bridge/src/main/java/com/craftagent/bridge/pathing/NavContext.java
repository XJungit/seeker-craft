package com.craftagent.bridge.pathing;

import net.minecraft.core.BlockPos;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.Container;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.pathfinder.PathType;
import com.craftagent.bridge.mixin.PathEvaluatorInvoker;
import java.util.HashMap;
import java.util.Map;

public class NavContext {
    public enum BlockClass { SOLID, PASSABLE, WATER, LAVA }

    public final ServerLevel level;
    public final BlockGetter view;
    public final boolean hasScaffold;
    public final int maxFallHeight;
    public final int maxNodes;
    private final Container inventory;
    private final Map<net.minecraft.world.level.block.Block, Double> toolCache = new HashMap<>();

    private static final String[] SCAFFOLD_ITEMS = {
        "dirt", "cobblestone", "stone", "planks", "log", "sand",
        "gravel", "deepslate", "netherrack", "granite", "diorite", "andesite",
        "grass_block", "podzol", "mycelium", "moss_block", "calcite",
        "tuff", "dripstone_block", "smooth_basalt"
    };

    public NavContext(ServerLevel level, BlockGetter view, Container inventory,
                      boolean hasScaffold, int maxFallHeight, int maxNodes) {
        this.level = level;
        this.view = view;
        this.inventory = inventory;
        this.hasScaffold = hasScaffold;
        this.maxFallHeight = maxFallHeight;
        this.maxNodes = maxNodes;
    }

    public static NavContext forSearch(ServerLevel level, Container inventory) {
        boolean hasScaff = false;
        for (int i = 0; i < inventory.getContainerSize(); i++) {
            ItemStack s = inventory.getItem(i);
            if (!s.isEmpty() && isScaffold(s)) { hasScaff = true; break; }
        }
        return new NavContext(level, level, inventory, hasScaff, 10, 50000);
    }

    public static NavContext forExecution(ServerLevel level, Container inventory) {
        return new NavContext(level, level, inventory, hasAnyScaffold(inventory), 10, 0);
    }

    private static boolean hasAnyScaffold(Container inv) {
        for (int i = 0; i < inv.getContainerSize(); i++) {
            if (!inv.getItem(i).isEmpty() && isScaffold(inv.getItem(i))) return true;
        }
        return false;
    }

    public static boolean isScaffold(ItemStack stack) {
        String id = net.minecraft.core.registries.BuiltInRegistries.ITEM.getKey(stack.getItem()).toString().toLowerCase();
        for (String s : SCAFFOLD_ITEMS) if (id.contains(s)) return true;
        return false;
    }

    public PathType getPathType(BlockPos pos) {
        if (pos.getY() < level.getMinY() || pos.getY() > level.getMaxY()) return PathType.BLOCKED;
        try {
            return PathEvaluatorInvoker.invokeGetPathTypeFromState(view, pos);
        } catch (Exception e) {
            return pathTypeFallback(pos);
        }
    }

    public BlockClass classify(BlockPos pos) {
        return pathTypeToBlockClass(getPathType(pos));
    }

    private BlockClass pathTypeToBlockClass(PathType pt) {
        if (pt == null) return BlockClass.SOLID;
        if (pt == PathType.LAVA) return BlockClass.LAVA;
        if (pt == PathType.WATER || pt == PathType.WATER_BORDER) return BlockClass.WATER;
        if (isBodyPassable(pt)) return BlockClass.PASSABLE;
        return BlockClass.SOLID;
    }

    private boolean isBodyPassable(PathType pt) {
        if (pt == null) return false;
        return pt != PathType.BLOCKED
            && pt != PathType.FENCE
            && pt != PathType.UNPASSABLE_RAIL
            && pt != PathType.DOOR_WOOD_CLOSED
            && pt != PathType.DOOR_IRON_CLOSED
            && pt != PathType.STICKY_HONEY;
    }

    private PathType pathTypeFallback(BlockPos pos) {
        BlockState bs = view.getBlockState(pos);
        if (bs.isAir()) return PathType.OPEN;
        String id = net.minecraft.core.registries.BuiltInRegistries.BLOCK.getKey(bs.getBlock()).toString();
        if (id.contains("lava")) return PathType.LAVA;
        if (id.contains("water")) return PathType.WATER;
        if (bs.canBeReplaced()) return PathType.OPEN;
        return PathType.BLOCKED;
    }

    public boolean bodyPassable(BlockPos pos) {
        BlockClass c = classify(pos);
        return c == BlockClass.PASSABLE || c == BlockClass.WATER;
    }

    public boolean isStandable(BlockPos pos) {
        BlockClass feet = classify(pos);
        BlockClass head = classify(pos.above());
        BlockClass floor = classify(pos.below());
        if (feet == BlockClass.LAVA || head == BlockClass.LAVA || floor == BlockClass.LAVA) return false;
        if (feet == BlockClass.WATER || head == BlockClass.WATER || floor == BlockClass.WATER) return true;
        if (!bodyPassable(pos)) return false;
        if (!bodyPassable(pos.above())) return false;
        return floor == BlockClass.SOLID;
    }

    public int findLandingY(int bx, int bz, int startY) {
        for (int y = startY; y >= startY - maxFallHeight; y--) {
            if (isStandable(new BlockPos(bx, y, bz))) return y;
        }
        return Integer.MIN_VALUE;
    }

    public BlockPos findStandingPos(BlockPos pos, int radius) {
        int x = pos.getX(), z = pos.getZ();
        for (int dy = 0; dy >= -8; dy--) {
            int y = pos.getY() + dy;
            if (y <= level.getMinY()) break;
            if (isStandable(new BlockPos(x, y, z))) return new BlockPos(x, y, z);
        }
        for (int r = 1; r <= radius; r++) {
            for (int dx = -r; dx <= r; dx++) {
                for (int dz = -r; dz <= r; dz++) {
                    if (Math.abs(dx) != r && Math.abs(dz) != r) continue;
                    for (int dy = 0; dy >= -6; dy--) {
                        int y = pos.getY() + dy;
                        if (y <= level.getMinY()) break;
                        if (isStandable(new BlockPos(x + dx, y, z + dz)))
                            return new BlockPos(x + dx, y, z + dz);
                    }
                }
            }
        }
        return null;
    }

    public double costOfBreaking(BlockPos pos) {
        BlockState bs = view.getBlockState(pos);
        if (bs.isAir() || bs.canBeReplaced()) return 0;
        net.minecraft.world.level.block.Block b = bs.getBlock();
        Double cached = toolCache.get(b);
        if (cached != null) return cached;
        double ticks = miningTicks(pos);
        toolCache.put(b, ticks);
        return ticks;
    }

    public double miningTicks(BlockPos pos) {
        BlockState bs = view.getBlockState(pos);
        float hardness = bs.getDestroySpeed(view, pos);
        if (hardness < 0) return 9999;
        if (hardness == 0) return 1;
        boolean canHarvest = !bs.requiresCorrectToolForDrops() || hasCorrectTool(bs);
        double speed = canHarvest ? 1.5 : 3.0;
        double ticks = Math.ceil(hardness * speed * 20.0);
        return Math.max(1, ticks);
    }

    private boolean hasCorrectTool(BlockState bs) {
        for (int i = 0; i < inventory.getContainerSize(); i++) {
            ItemStack stack = inventory.getItem(i);
            if (!stack.isEmpty() && stack.isCorrectToolForDrops(bs)) return true;
        }
        return false;
    }

    public double costOfPlacing(BlockPos pos) {
        if (hasScaffold && isPlacementSafe(pos)) return 1.0;
        return 9999;
    }

    public boolean isPlacementSafe(BlockPos pos) {
        BlockState bs = view.getBlockState(pos);
        return bs.isAir() || bs.canBeReplaced();
    }

    public boolean canPlaceAgainst(BlockPos pos) {
        BlockState bs = view.getBlockState(pos);
        if (bs.isAir()) return false;
        String id = net.minecraft.core.registries.BuiltInRegistries.BLOCK.getKey(bs.getBlock()).toString().toLowerCase();
        return !id.contains("leaves") && !id.contains("glass")
            && !id.contains("pane") && !id.contains("cobweb")
            && !id.contains("tall_grass") && !id.contains("fern");
    }
}
