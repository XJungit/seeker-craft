package com.craftagent.bridge.pathing;

import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.Container;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
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
    private final Map<Block, Double> toolCache = new HashMap<>();

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
        return new NavContext(level, level, inventory, hasScaff, 3, 30000);
    }

    public static NavContext forExecution(ServerLevel level, Container inventory) {
        return new NavContext(level, level, inventory, hasAnyScaffold(inventory), 3, 0);
    }

    private static boolean hasAnyScaffold(Container inv) {
        for (int i = 0; i < inv.getContainerSize(); i++) {
            if (!inv.getItem(i).isEmpty() && isScaffold(inv.getItem(i))) return true;
        }
        return false;
    }

    public static boolean isScaffold(ItemStack stack) {
        String id = BuiltInRegistries.ITEM.getKey(stack.getItem()).toString().toLowerCase();
        for (String s : SCAFFOLD_ITEMS) if (id.contains(s)) return true;
        return false;
    }

    public BlockClass classify(BlockPos pos) {
        if (pos.getY() < level.getMinY() || pos.getY() > level.getMaxY()) return BlockClass.SOLID;
        BlockState bs = view.getBlockState(pos);
        if (bs.isAir()) return BlockClass.PASSABLE;
        String id = BuiltInRegistries.BLOCK.getKey(bs.getBlock()).toString();
        boolean airOrReplaceable = bs.isAir() || bs.canBeReplaced();
        return classifyId(id, airOrReplaceable);
    }

    public static BlockClass classifyId(String id, boolean airOrReplaceable) {
        if (id.contains("lava")) return BlockClass.LAVA;
        if (id.contains("water")) return BlockClass.WATER;
        if (airOrReplaceable) return BlockClass.PASSABLE;
        String lowId = id.toLowerCase();
        if (lowId.contains("_leaves") || lowId.contains("leaves")
            || lowId.endsWith("grass") || lowId.contains("_sapling")
            || lowId.contains("flower") || lowId.contains("mushroom")
            || lowId.contains("carpet") || lowId.contains("glass")
            || lowId.contains("pane") || lowId.contains("fence")
            || lowId.contains("iron_bars") || lowId.contains("door")
            || lowId.contains("trapdoor") || lowId.contains("vine")
            || lowId.contains("ladder") || lowId.contains("snow")
            || lowId.contains("seagrass") || lowId.contains("kelp")
            || lowId.contains("lily_pad") || lowId.contains("torch")
            || lowId.contains("lantern") || lowId.contains("cobweb")
            || lowId.contains("bamboo") || lowId.contains("reeds")
            || lowId.contains("coral") || lowId.contains("banner")
            || lowId.contains("pressure_plate") || lowId.contains("button")
            || lowId.contains("rail") || lowId.contains("_sign")
            || lowId.contains("candle") || lowId.contains("amethyst_cluster")
            || lowId.contains("bud") || lowId.contains("hang")
            || lowId.contains("air") || lowId.contains("structure_void")
            || lowId.contains("light") || lowId.contains("sculk_vein")
        ) return BlockClass.PASSABLE;
        return BlockClass.SOLID;
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
        Block b = bs.getBlock();
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
        String id = BuiltInRegistries.BLOCK.getKey(bs.getBlock()).toString().toLowerCase();
        return !id.contains("leaves") && !id.contains("glass")
            && !id.contains("pane") && !id.contains("cobweb")
            && !id.contains("tall_grass") && !id.contains("fern");
    }
}
