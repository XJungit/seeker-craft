package com.craftagent.bridge.pathing;

import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.Blocks;
import java.util.ArrayList;
import java.util.List;

public class Moves {
    private static final Direction[] HORIZONTAL = {
        Direction.NORTH, Direction.EAST, Direction.SOUTH, Direction.WEST
    };
    private static final Direction[][] DIAGONALS = {
        {Direction.NORTH, Direction.EAST},
        {Direction.NORTH, Direction.WEST},
        {Direction.SOUTH, Direction.EAST},
        {Direction.SOUTH, Direction.WEST}
    };
    private static final int MAX_PARKOUR = 3;
    private static final double SQRT2 = Math.sqrt(2);

    public static List<Movement> generate(NavContext ctx, BlockPos src) {
        List<Movement> moves = new ArrayList<>();
        if (ctx.classify(src) == NavContext.BlockClass.LAVA) return moves;
        for (Direction dir : HORIZONTAL) {
            addIfNotNull(moves, traverse(ctx, src, dir));
            addIfNotNull(moves, ascend(ctx, src, dir));
            addIfNotNull(moves, descend(ctx, src, dir));
            addIfNotNull(moves, parkour(ctx, src, dir));
        }
        for (Direction[] pair : DIAGONALS) {
            addIfNotNull(moves, diagonal(ctx, src, pair[0], pair[1]));
        }
        addIfNotNull(moves, pillar(ctx, src));
        addIfNotNull(moves, digDown(ctx, src));
        return moves;
    }

    private static void addIfNotNull(List<Movement> list, Movement m) {
        if (m != null) list.add(m);
    }

    static Movement traverse(NavContext ctx, BlockPos src, Direction dir) {
        BlockPos dest = src.relative(dir);
        if (!ctx.isStandable(dest)) return null;
        List<BlockPos> toBreak = new ArrayList<>();
        checkBreak(ctx, dest, toBreak);
        checkBreak(ctx, dest.above(), toBreak);
        double cost = toBreak.isEmpty() ? 1.0 : 1.0 + toBreak.stream().mapToDouble(ctx::costOfBreaking).sum();
        return new Movement(Movement.Kind.TRAVERSE, src, dest, cost, toBreak, null);
    }

    static Movement ascend(NavContext ctx, BlockPos src, Direction dir) {
        BlockPos dest = src.relative(dir).above();
        if (!ctx.isStandable(dest)) return null;
        if (!ctx.bodyPassable(src.relative(dir).above())) return null;
        List<BlockPos> toBreak = new ArrayList<>();
        checkBreak(ctx, src.relative(dir), toBreak);
        double cost = 1.5 + toBreak.stream().mapToDouble(ctx::costOfBreaking).sum();
        return new Movement(Movement.Kind.ASCEND, src, dest, cost, toBreak, null);
    }

    static Movement descend(NavContext ctx, BlockPos src, Direction dir) {
        BlockPos feetPos = src.relative(dir);
        int landingY = ctx.findLandingY(feetPos.getX(), feetPos.getZ(), src.getY());
        if (landingY == Integer.MIN_VALUE) return null;
        int drop = src.getY() - landingY;
        if (drop > ctx.maxFallHeight) return null;
        if (drop <= 0) return null;
        BlockPos dest = new BlockPos(feetPos.getX(), landingY, feetPos.getZ());
        List<BlockPos> toBreak = new ArrayList<>();
        for (int y = src.getY(); y > landingY; y--) {
            checkBreak(ctx, new BlockPos(dest.getX(), y, dest.getZ()), toBreak);
        }
        double cost = 1.0 + drop * 0.15 + toBreak.stream().mapToDouble(ctx::costOfBreaking).sum();
        return new Movement(Movement.Kind.DESCEND, src, dest, cost, toBreak, null);
    }

    static Movement diagonal(NavContext ctx, BlockPos src, Direction dir1, Direction dir2) {
        BlockPos mid1 = src.relative(dir1);
        BlockPos mid2 = src.relative(dir2);
        BlockPos dest = src.relative(dir1).relative(dir2);
        if (!ctx.isStandable(dest)) return null;
        if (!ctx.bodyPassable(mid1) && !ctx.bodyPassable(mid1.above())) return null;
        if (!ctx.bodyPassable(mid2) && !ctx.bodyPassable(mid2.above())) return null;
        if (!ctx.bodyPassable(dest)) return null;
        if (!ctx.bodyPassable(dest.above())) return null;
        List<BlockPos> toBreak = new ArrayList<>();
        checkBreak(ctx, dest, toBreak);
        checkBreak(ctx, dest.above(), toBreak);
        double cost = SQRT2 + toBreak.stream().mapToDouble(ctx::costOfBreaking).sum();
        return new Movement(Movement.Kind.DIAGONAL, src, dest, cost, toBreak, null);
    }

    static Movement pillar(NavContext ctx, BlockPos src) {
        if (!ctx.hasScaffold) return null;
        BlockPos dest = src.above();
        if (!ctx.bodyPassable(dest) || !ctx.bodyPassable(dest.above())) return null;
        if (!ctx.isPlacementSafe(src)) return null;
        double cost = 2.5 + ctx.costOfPlacing(src);
        return new Movement(Movement.Kind.PILLAR, src, dest, cost, List.of(), src);
    }

    static Movement digDown(NavContext ctx, BlockPos src) {
        BlockPos below = src.below();
        NavContext.BlockClass bc = ctx.classify(below);
        if (bc == NavContext.BlockClass.LAVA) return null;
        if (bc == NavContext.BlockClass.PASSABLE) return null;
        if (bc == NavContext.BlockClass.WATER) return null;
        BlockPos dest = src.below(2);
        if (!ctx.bodyPassable(dest) || !ctx.bodyPassable(dest.above())) return null;
        double cost = 2.0 + ctx.costOfBreaking(below);
        return new Movement(Movement.Kind.DIG_DOWN, src, dest, cost, List.of(below), null);
    }

    static Movement parkour(NavContext ctx, BlockPos src, Direction dir) {
        if (!ctx.hasScaffold) return null;
        for (int gap = 2; gap <= MAX_PARKOUR; gap++) {
            BlockPos far = src.relative(dir, gap);
            int landingY = ctx.findLandingY(far.getX(), far.getZ(), src.getY());
            if (landingY == Integer.MIN_VALUE) continue;
            int drop = src.getY() - landingY;
            if (drop > ctx.maxFallHeight) continue;
            BlockPos dest = new BlockPos(far.getX(), landingY, far.getZ());
            boolean clear = true;
            for (int g = 1; g < gap; g++) {
                BlockPos mid = src.relative(dir, g);
                if (ctx.classify(mid) == NavContext.BlockClass.SOLID
                    || ctx.classify(mid.above()) == NavContext.BlockClass.SOLID) {
                    clear = false;
                    break;
                }
            }
            if (!clear) continue;
            if (!ctx.isStandable(dest)) continue;
            if (!ctx.isPlacementSafe(dest)) continue;
            double cost = gap * 1.5;
            return new Movement(Movement.Kind.PARKOUR, src, dest, cost, List.of(), null);
        }
        return null;
    }

    private static void checkBreak(NavContext ctx, BlockPos pos, List<BlockPos> acc) {
        NavContext.BlockClass c = ctx.classify(pos);
        if (c == NavContext.BlockClass.SOLID) acc.add(pos);
    }
}
