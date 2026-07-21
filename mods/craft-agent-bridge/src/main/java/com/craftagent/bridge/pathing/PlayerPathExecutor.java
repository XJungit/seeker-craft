package com.craftagent.bridge.pathing;

import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.Vec3;
import com.craftagent.bridge.InventoryHelper;
import java.util.List;

public class PlayerPathExecutor {
    public enum Status { RUNNING, ARRIVED, STUCK, FAILED }

    private final ServerPlayer player;
    private final List<BlockPos> waypoints;
    private int index;
    private int ticksSinceProgress;
    private Vec3 lastProgressPos;
    private boolean stuckWarned;
    private boolean jumpedThisTick;

    private static final int PROGRESS_TIMEOUT = 100;
    private static final int FORCE_FINISH_TICKS = 40;
    private static final double PROGRESS_THRESHOLD_SQ = 0.5;
    private static final double ARRIVAL_HORIZ_SQ = 0.8 * 0.8;
    private static final double ARRIVAL_VERT = 1.2;
    private static final double SWIM_UP_SPEED = 0.04;
    private static final double AUTO_DIG_RANGE = 2.5;

    public PlayerPathExecutor(ServerPlayer player, List<BlockPos> waypoints) {
        this.player = player;
        this.waypoints = waypoints;
        this.index = 0;
        this.ticksSinceProgress = 0;
        this.lastProgressPos = player.position();
    }

    public Status tick() {
        if (waypoints == null || waypoints.isEmpty()) return Status.ARRIVED;
        if (index >= waypoints.size()) return Status.ARRIVED;

        Vec3 pos = player.position();
        jumpedThisTick = false;

        double distSq = pos.distanceToSqr(lastProgressPos);
        if (distSq > PROGRESS_THRESHOLD_SQ) {
            ticksSinceProgress = 0;
            lastProgressPos = pos;
        } else {
            ticksSinceProgress++;
        }

        BlockPos targetWp = waypoints.get(index);
        Vec3 target = Vec3.atCenterOf(targetWp);
        double hDistSq = Math.pow(pos.x - target.x, 2) + Math.pow(pos.z - target.z, 2);
        double vDist = Math.abs(pos.y - target.y);
        boolean nearHoriz = hDistSq < ARRIVAL_HORIZ_SQ;
        boolean nearVert = vDist < ARRIVAL_VERT;

        autoDig();

        if (nearHoriz && nearVert) {
            advance();
            return Status.RUNNING;
        }

        driveToward(target);

        if (nearHoriz && nearVert) {
            advance();
            return Status.RUNNING;
        }

        if (remaining() <= 1 && ticksSinceProgress > FORCE_FINISH_TICKS) {
            System.out.println("[nav] force-finish last waypoint " + index
                + " hDist=" + String.format("%.2f", Math.sqrt(hDistSq))
                + " vDist=" + String.format("%.2f", vDist));
            advance();
            return Status.RUNNING;
        }

        if (ticksSinceProgress > PROGRESS_TIMEOUT) {
            if (!stuckWarned) {
                stuckWarned = true;
                System.out.println("[nav] STUCK at waypoint " + index + " ticks=" + ticksSinceProgress);
            }
            return Status.STUCK;
        }

        if (ticksSinceProgress > 40 && !stuckWarned) {
            stuckWarned = true;
            System.out.println("[nav] slow progress at waypoint " + index + " ticks=" + ticksSinceProgress);
        }

        return Status.RUNNING;
    }

    private void driveToward(Vec3 target) {
        double ddx = target.x - player.getX();
        double ddz = target.z - player.getZ();
        double horiz = Math.sqrt(ddx * ddx + ddz * ddz);
        if (horiz < 0.01) return;

        float yaw = (float) Math.toDegrees(Math.atan2(-ddx, ddz));
        player.setYRot(yaw);
        player.yHeadRot = yaw;
        player.zza = 1.3f;
        player.setSprinting(true);

        double nx = ddx / horiz;
        double nz = ddz / horiz;

        if (player.isInWater()) {
            player.setSwimming(true);
            double swimUp = player.isUnderWater() ? SWIM_UP_SPEED : 0.0;
            if (!jumpedThisTick) {
                if (player.horizontalCollision && player.onGround()) {
                    player.jumpFromGround();
                    jumpedThisTick = true;
                }
            }
            double vy = jumpedThisTick ? player.getDeltaMovement().y : swimUp;
            player.setDeltaMovement(nx * 0.25, vy, nz * 0.25);
        } else {
            double vy = jumpedThisTick ? player.getDeltaMovement().y : player.getDeltaMovement().y;
            player.setDeltaMovement(nx * 0.35, vy, nz * 0.35);
        }

        if (player.isInWater() && player.horizontalCollision && player.onGround() && !jumpedThisTick) {
            player.jumpFromGround();
            jumpedThisTick = true;
        }
    }

    private void autoDig() {
        BlockPos front = player.blockPosition().offset(
            (int) Math.round(-Math.sin(Math.toRadians(player.getYRot()))),
            0,
            (int) Math.round(Math.cos(Math.toRadians(player.getYRot())))
        );
        ServerLevel level = (ServerLevel) player.level();
        for (int dy = -1; dy <= 5; dy++) {
            BlockPos bp = front.offset(0, dy, 0);
            if (bp.distSqr(player.blockPosition()) > AUTO_DIG_RANGE * AUTO_DIG_RANGE) break;
            BlockState bs = level.getBlockState(bp);
            if (bs.isAir() || bs.canBeReplaced() || bs.getBlock() == net.minecraft.world.level.block.Blocks.BEDROCK)
                continue;
            InventoryHelper.equipBestTool(player,
                BuiltInRegistries.BLOCK.getKey(bs.getBlock()).toString());
            if (level.destroyBlock(bp, true)) {
                ticksSinceProgress = 0;
                System.out.println("[nav] AUTO DIG " + bp.toShortString());
            }
        }
    }

    private void advance() {
        index++;
        ticksSinceProgress = 0;
        stuckWarned = false;
        lastProgressPos = player.position();
    }

    public void stop() {
        player.zza = 0;
        player.xxa = 0;
        player.setSprinting(false);
        player.setSwimming(false);
        player.setDeltaMovement(0, player.getDeltaMovement().y, 0);
    }

    public int remaining() { return Math.max(0, waypoints.size() - index); }
    public int currentIndex() { return index; }
}
