package com.craftagent.bridge.pathing;

import net.minecraft.core.BlockPos;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.player.Inventory;
import com.craftagent.bridge.CraftAgentBridge;
import java.util.function.Supplier;

public class PlayerNav {
    public enum Status { RUNNING, ARRIVED, FAILED, IDLE }

    private final ServerPlayer player;
    private final Supplier<NavGoal> goalSupplier;
    private final double speed;
    private NavContext searchContext;
    private PlayerPathExecutor executor;
    private AStarSearch currentSearch;
    private AStarSearch precomputeSearch;
    private Path precomputedPath;
    private Status status = Status.IDLE;
    private int replans;
    private String failReason;
    private static final int MAX_REPLANS = 5;
    private static final int NODES_PER_TICK = 2000;
    private static final double GOAL_MOVED_SQR = 16.0;
    private BlockPos lastGoalCenter;

    public PlayerNav(ServerPlayer player, Supplier<NavGoal> goalSupplier, double speed) {
        this.player = player;
        this.goalSupplier = goalSupplier;
        this.speed = speed;
    }

    public Status tick() {
        if (status == Status.IDLE) return Status.IDLE;

        NavGoal goal = goalSupplier.get();

        // Check if goal moved
        if (lastGoalCenter != null && goal.center() != null
            && goal.center().distSqr(lastGoalCenter) > GOAL_MOVED_SQR) {
            replan("goal_moved");
            lastGoalCenter = goal.center();
            return Status.RUNNING;
        }

        // Tick current executor
        if (executor != null) {
            PlayerPathExecutor.Status execStatus = executor.tick();
            if (execStatus == PlayerPathExecutor.Status.ARRIVED) {
                if (goal.isAt(player.blockPosition()) || executor.remainingMovements() == 0) {
                    status = Status.ARRIVED;
                    stop();
                    return status;
                }
                replan("arrived_midway");
                return Status.RUNNING;
            }
            if (execStatus == PlayerPathExecutor.Status.STUCK) {
                if (replans < MAX_REPLANS) {
                    replan("stuck");
                    return Status.RUNNING;
                }
                status = Status.FAILED;
                failReason = "stuck_after_" + replans + "_replans";
                return status;
            }
        } else {
            // No executor — start fresh search
            startFreshSearch();
            return Status.RUNNING;
        }

        // Advance current search
        if (currentSearch != null) {
            AStarSearch.State s = currentSearch.step(NODES_PER_TICK);
            if (s == AStarSearch.State.FOUND) {
                Path path = currentSearch.result();
                if (path != null && !path.movements.isEmpty()) {
                    NavContext execCtx = NavContext.forExecution(
                        (ServerLevel) player.level(), player.getInventory());
                    executor = new PlayerPathExecutor(player, path, () -> execCtx);
                    currentSearch = null;
                }
            } else if (s == AStarSearch.State.FAILED) {
                failReason = "no_path";
                status = Status.FAILED;
                return status;
            }
        }

        // Precompute next path if near end of current
        if (executor != null && executor.remainingMovements() <= 3 && precomputedPath == null) {
            maybePrecompute();
        }
        if (precomputeSearch != null) {
            AStarSearch.State s = precomputeSearch.step(NODES_PER_TICK);
            if (s == AStarSearch.State.FOUND) {
                precomputedPath = precomputeSearch.result();
                precomputeSearch = null;
            }
        }

        return Status.RUNNING;
    }

    private void startFreshSearch() {
        cancelSearch();
        NavGoal goal = goalSupplier.get();
        lastGoalCenter = goal.center();
        ServerLevel level = (ServerLevel) player.level();
        BlockPos feet = player.blockPosition();

        BlockPos startPos = findValidStart(level, feet);
        if (startPos == null) startPos = feet;

        searchContext = NavContext.forSearch(level, player.getInventory());
        currentSearch = new AStarSearch(searchContext, startPos, goal, 30000);
    }

    private BlockPos findValidStart(ServerLevel level, BlockPos pos) {
        if (searchContext != null && searchContext.isStandable(pos)) return pos;
        for (int dy = 0; dy >= -5; dy--) {
            BlockPos p = new BlockPos(pos.getX(), pos.getY() + dy, pos.getZ());
            if (searchContext != null && searchContext.isStandable(p)) return p;
        }
        return null;
    }

    private void maybePrecompute() {
        if (executor == null || executor.path() == null || executor.path().movements.isEmpty()) return;
        Movement lastMove = executor.path().movements.get(executor.path().movements.size() - 1);
        if (lastMove == null) return;
        BlockPos from = lastMove.dest;
        NavGoal goal = goalSupplier.get();
        if (goal.isAt(from)) return;
        ServerLevel level = (ServerLevel) player.level();
        NavContext preCtx = NavContext.forSearch(level, player.getInventory());
        precomputeSearch = new AStarSearch(preCtx, from, goal, 30000);
    }

    private void replan(String reason) {
        System.out.println("[nav] replan #" + (replans + 1) + " reason=" + reason);
        replans++;
        if (executor != null) executor.stop();
        executor = null;
        precomputedPath = null;
        precomputeSearch = null;
        startFreshSearch();
    }

    private void cancelSearch() {
        if (currentSearch != null) {
            currentSearch.cancel();
            currentSearch = null;
        }
        if (precomputeSearch != null) {
            precomputeSearch.cancel();
            precomputeSearch = null;
        }
    }

    public void start() {
        this.status = Status.RUNNING;
        this.replans = 0;
        this.failReason = null;
        this.executor = null;
        startFreshSearch();
    }

    public void stop() {
        cancelSearch();
        if (executor != null) executor.stop();
        executor = null;
        status = Status.IDLE;
        player.zza = 0;
        player.xxa = 0;
        player.setSprinting(false);
        player.setDeltaMovement(0, player.getDeltaMovement().y, 0);
    }

    public Status status() { return status; }
    public String failReason() { return failReason; }
    public PlayerPathExecutor executor() { return executor; }
    public int replans() { return replans; }
}
