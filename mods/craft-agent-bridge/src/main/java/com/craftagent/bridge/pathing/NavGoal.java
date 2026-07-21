package com.craftagent.bridge.pathing;

import net.minecraft.core.BlockPos;
import java.util.List;
import java.util.ArrayList;
import java.util.function.Supplier;

public interface NavGoal {
    boolean isAt(BlockPos pos);
    double heuristic(BlockPos pos);
    BlockPos center();

    static NavGoal exact(BlockPos pos) {
        return new NavGoal() {
            public boolean isAt(BlockPos p) { return p.equals(pos); }
            public double heuristic(BlockPos p) { return pointBound(p, pos); }
            public BlockPos center() { return pos; }
        };
    }

    static NavGoal near(BlockPos pos, double radius) {
        double rSq = radius * radius;
        return new NavGoal() {
            public boolean isAt(BlockPos p) {
                return p.distSqr(pos) <= rSq;
            }
            public double heuristic(BlockPos p) {
                double d = Math.sqrt(p.distSqr(pos)) - radius;
                return Math.max(0, d);
            }
            public BlockPos center() { return pos; }
        };
    }

    static NavGoal adjacent(BlockPos pos) {
        return new NavGoal() {
            public boolean isAt(BlockPos p) {
                int dx = Math.abs(p.getX() - pos.getX());
                int dy = Math.abs(p.getY() - pos.getY());
                int dz = Math.abs(p.getZ() - pos.getZ());
                return dx + dz <= 1 && dy == 0;
            }
            public double heuristic(BlockPos p) {
                double dx = Math.abs(p.getX() - pos.getX());
                double dy = Math.abs(p.getY() - pos.getY());
                double dz = Math.abs(p.getZ() - pos.getZ());
                return Math.max(0, dx + dz - 1) + dy;
            }
            public BlockPos center() { return pos; }
        };
    }

    static NavGoal column(int x, int z) {
        return new NavGoal() {
            public boolean isAt(BlockPos p) { return p.getX() == x && p.getZ() == z; }
            public double heuristic(BlockPos p) {
                double dx = Math.abs(p.getX() - x);
                double dz = Math.abs(p.getZ() - z);
                return dx + dz;
            }
            public BlockPos center() { return new BlockPos(x, 64, z); }
        };
    }

    static NavGoal yLevel(int y) {
        return new NavGoal() {
            public boolean isAt(BlockPos p) { return p.getY() == y; }
            public double heuristic(BlockPos p) { return Math.abs(p.getY() - y); }
            public BlockPos center() { return new BlockPos(0, y, 0); }
        };
    }

    static NavGoal runAway(BlockPos from, int dist) {
        return new NavGoal() {
            public boolean isAt(BlockPos p) { return false; }
            public double heuristic(BlockPos p) {
                double d = Math.sqrt(p.distSqr(from));
                return -d;
            }
            public BlockPos center() {
                int dx = from.getX() - 64;
                int dz = from.getZ() - 64;
                double len = Math.sqrt(dx * dx + dz * dz);
                if (len < 1) return new BlockPos(-64, 64, -64);
                int tx = from.getX() - (int)(dx / len * dist);
                int tz = from.getZ() - (int)(dz / len * dist);
                return new BlockPos(tx, from.getY(), tz);
            }
        };
    }

    static NavGoal composite(List<NavGoal> goals) {
        return new NavGoal() {
            public boolean isAt(BlockPos p) {
                for (NavGoal g : goals) if (g.isAt(p)) return true;
                return false;
            }
            public double heuristic(BlockPos p) {
                double best = Double.MAX_VALUE;
                for (NavGoal g : goals) best = Math.min(best, g.heuristic(p));
                return best;
            }
            public BlockPos center() { return goals.isEmpty() ? BlockPos.ZERO : goals.get(0).center(); }
        };
    }

    static double pointBound(BlockPos a, BlockPos b) {
        double dx = Math.abs(a.getX() - b.getX());
        double dy = Math.abs(a.getY() - b.getY());
        double dz = Math.abs(a.getZ() - b.getZ());
        double h = Math.min(dx, dz);
        return h * Math.sqrt(2) + (dx + dz - 2 * h) + dy;
    }
}
