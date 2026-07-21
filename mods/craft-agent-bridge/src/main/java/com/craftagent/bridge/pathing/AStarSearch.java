package com.craftagent.bridge.pathing;

import net.minecraft.core.BlockPos;
import java.util.*;

public class AStarSearch {
    public enum State { SEARCHING, FOUND, FAILED, CANCELLED }

    private final NavContext ctx;
    private final BlockPos start;
    private final NavGoal goal;
    private final int maxNodes;
    private final int originX, originY, originZ;
    private final long originLong;
    private final Map<String, PathNode> nodes = new HashMap<>();
    private final BinaryHeapOpenSet open;
    private State state = State.SEARCHING;
    private Path result;
    private int expansions;
    private boolean cancelled;
    private PathNode bestSoFar;
    private double bestHeuristic = Double.MAX_VALUE;
    private static final int SEARCH_RADIUS = 48;

    public AStarSearch(NavContext ctx, BlockPos start, NavGoal goal, int maxNodes) {
        this.ctx = ctx;
        this.start = start;
        this.goal = goal;
        this.maxNodes = maxNodes;
        this.originX = start.getX();
        this.originY = start.getY();
        this.originZ = start.getZ();
        this.originLong = start.asLong();
        this.open = new BinaryHeapOpenSet(4096);
        double h = goal.heuristic(start);
        open.push(new PathNode(0, 0, 0, 0, h, null, null));
    }

    public State state() { return state; }
    public Path result() { return result; }
    public int expansions() { return expansions; }
    public void cancel() { this.cancelled = true; state = State.CANCELLED; }

    public State step(int maxIterations) {
        if (state != State.SEARCHING) return state;
        BlockPos origin = new BlockPos(originX, originY, originZ);
        for (int iter = 0; iter < maxIterations && !open.isEmpty() && nodes.size() < maxNodes && !cancelled; iter++) {
            PathNode cur = open.pop();
            if (cur == null) break;
            String key = cur.key();
            if (nodes.containsKey(key)) continue;
            nodes.put(key, cur);

            int wx = originX + cur.x;
            int wy = originY + cur.y;
            int wz = originZ + cur.z;

            expansions++;

            BlockPos curPos = new BlockPos(wx, wy, wz);
            if (goal.isAt(curPos)) {
                result = reconstruct(cur, true);
                state = State.FOUND;
                return state;
            }

            double h = goal.heuristic(curPos);
            if (h < bestHeuristic) {
                bestHeuristic = h;
                bestSoFar = cur;
            }

            List<Movement> moves = Moves.generate(ctx, curPos);
            for (Movement m : moves) {
                int nx = m.dest.getX() - originX;
                int ny = m.dest.getY() - originY;
                int nz = m.dest.getZ() - originZ;

                if (Math.abs(nx) > SEARCH_RADIUS || Math.abs(ny) > 32 || Math.abs(nz) > SEARCH_RADIUS) continue;

                String nk = nx + "," + ny + "," + nz;
                if (nodes.containsKey(nk)) continue;

                double ng = cur.g + m.cost;
                double nh = goal.heuristic(m.dest);
                open.push(new PathNode(nx, ny, nz, ng, nh, cur, m));
            }
        }

        if (open.isEmpty() || nodes.size() >= maxNodes || cancelled) {
            if (bestSoFar != null) {
                result = reconstruct(bestSoFar, false);
                state = State.FOUND;
            } else {
                state = State.FAILED;
            }
        }
        return state;
    }

    private Path reconstruct(PathNode end, boolean reachedGoal) {
        BlockPos origin = new BlockPos(originX, originY, originZ);
        LinkedList<Movement> moves = new LinkedList<>();
        PathNode cur = end;
        while (cur != null && cur.movement != null) {
            moves.addFirst(cur.movement);
            cur = cur.parent;
        }
        BlockPos endPos = end != null ? new BlockPos(originX + end.x, originY + end.y, originZ + end.z) : start;
        return new Path(start, endPos, moves, !reachedGoal);
    }
}
