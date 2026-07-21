package com.craftagent.bridge.pathing;

import net.minecraft.core.BlockPos;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import java.util.List;
import java.util.function.Supplier;

public class PlayerNav {
    public enum Status { RUNNING, ARRIVED, FAILED, IDLE }

    private final ServerPlayer player;
    private final Supplier<NavGoal> goalSupplier;
    private PlayerPathExecutor executor;
    private Status status = Status.IDLE;
    private int replans;
    private String failReason;
    private BlockPos targetPos;
    private static final int MAX_REPLANS = 5;

    public PlayerNav(ServerPlayer player, Supplier<NavGoal> goalSupplier, double speed) {
        this.player = player;
        this.goalSupplier = goalSupplier;
    }

    public Status tick() {
        if (status == Status.IDLE) return Status.IDLE;

        if (executor != null) {
            PlayerPathExecutor.Status execStatus = executor.tick();
            if (execStatus == PlayerPathExecutor.Status.ARRIVED) {
                if (executor.remaining() == 0 || goalSupplier.get().isAt(player.blockPosition())) {
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
        }

        if (executor == null) {
            startFreshSearch();
        }

        return Status.RUNNING;
    }

    private void startFreshSearch() {
        if (executor != null) executor.stop();
        executor = null;

        NavGoal goal = goalSupplier.get();
        targetPos = goal.center();
        if (targetPos == null) {
            status = Status.FAILED;
            failReason = "no_goal";
            return;
        }

        ServerLevel level = (ServerLevel) player.level();
        List<BlockPos> waypoints = VanillaPathfinder.findPath(level, player, targetPos);
        if (waypoints == null || waypoints.isEmpty()) {
            if (goal.isAt(player.blockPosition())) {
                status = Status.ARRIVED;
                return;
            }
            status = Status.FAILED;
            failReason = "no_path";
            return;
        }

        executor = new PlayerPathExecutor(player, waypoints);
    }

    private void replan(String reason) {
        System.out.println("[nav] replan #" + (replans + 1) + " reason=" + reason);
        replans++;
        if (executor != null) executor.stop();
        executor = null;
        startFreshSearch();
    }

    public void start() {
        this.status = Status.RUNNING;
        this.replans = 0;
        this.failReason = null;
        this.executor = null;
        startFreshSearch();
    }

    public void stop() {
        if (executor != null) executor.stop();
        executor = null;
        status = Status.IDLE;
        player.zza = 0;
        player.xxa = 0;
        player.setSprinting(false);
        player.setSwimming(false);
        player.setDeltaMovement(0, player.getDeltaMovement().y, 0);
    }

    public Status status() { return status; }
    public String failReason() { return failReason; }
    public PlayerPathExecutor executor() { return executor; }
    public int replans() { return replans; }
}
