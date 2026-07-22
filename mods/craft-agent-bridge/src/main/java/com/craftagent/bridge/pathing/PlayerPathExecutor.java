package com.craftagent.bridge.pathing;

import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.level.block.Blocks;
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
    private int pillarTries;

    private static final int PROGRESS_TIMEOUT = 120;
    private static final int FORCE_FINISH_TICKS = 40;
    private static final double PROGRESS_THRESHOLD_SQ = 0.5;
    private static final double ARRIVAL_HORIZ_SQ = 0.8 * 0.8;
    private static final double ARRIVAL_VERT = 1.2;
    private static final double SWIM_UP_SPEED = 0.12;
    private static final double AUTO_DIG_RANGE = 2.5;
    private static final double FALL_THRESHOLD = 3.0;
    private static final double CLIFF_SAFE_DIST = 2.0;

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
        double vdy = target.y - player.getY();
        double horiz = Math.sqrt(ddx * ddx + ddz * ddz);

        float yaw = (float) Math.toDegrees(Math.atan2(-ddx, ddz));
        player.setYRot(yaw);
        player.yHeadRot = yaw;

        if (horiz < 0.01 && Math.abs(vdy) < 0.01) return;

        double nx = horiz > 0.001 ? ddx / horiz : 0;
        double nz = horiz > 0.001 ? ddz / horiz : 0;

        ServerLevel level = (ServerLevel) player.level();
        boolean cliff = isCliffEdge(level, player.blockPosition(), yaw);
        double fallDist = checkFallDistance(level, target);

        double speed = 0.32;
        if (cliff || fallDist > FALL_THRESHOLD) speed = 0.12;

        double stepX = nx * speed;
        double stepZ = nz * speed;
        double stepY = 0.0;

        if (Math.abs(vdy) > 0.05) {
            stepY = Math.signum(vdy) * Math.min(Math.abs(vdy), 0.3);
        } else {
            BlockPos feet = player.blockPosition().below();
            if (level.getBlockState(feet).isAir() || level.getBlockState(feet).canBeReplaced()) {
                stepY = -0.25;
            }
        }

        double newX = player.getX() + stepX;
        double newZ = player.getZ() + stepZ;
        double newY = player.getY() + stepY;

        if (player.isInWater() && !player.isUnderWater()) {
            newY += 0.2;
        }

        player.setPos(newX, newY, newZ);
        player.setYRot(yaw);
        player.setXRot(player.getXRot());
        player.setDeltaMovement(0.0, player.getDeltaMovement().y, 0.0);
    }

    private boolean isCliffEdge(ServerLevel level, BlockPos pos, float yaw) {
        BlockPos front = pos.offset(
            (int) Math.round(-Math.sin(Math.toRadians(yaw))),
            0,
            (int) Math.round(Math.cos(Math.toRadians(yaw)))
        );
        BlockState frontBlock = level.getBlockState(front);
        if (frontBlock.isAir() || frontBlock.canBeReplaced()) {
            BlockState below = level.getBlockState(front.below());
            if (below.isAir() || below.canBeReplaced()) {
                int drop = 0;
                for (int dy = -1; dy >= -5; dy--) {
                    BlockState b = level.getBlockState(front.offset(0, dy, 0));
                    if (!b.isAir() && !b.canBeReplaced()) {
                        drop = -dy - 1;
                        break;
                    }
                }
                return drop >= 3;
            }
        }
        return false;
    }

    private double checkFallDistance(ServerLevel level, Vec3 target) {
        BlockPos targetBelow = BlockPos.containing(target.x, target.y - 1, target.z);
        int drop = 0;
        for (int dy = 0; dy >= -10; dy--) {
            BlockState b = level.getBlockState(targetBelow.offset(0, dy, 0));
            if (!b.isAir() && !b.canBeReplaced()) {
                drop = -dy;
                break;
            }
        }
        return drop;
    }

    private void autoDig() {
        BlockPos feet = player.blockPosition();
        ServerLevel level = (ServerLevel) player.level();
        // 只挖身体/头部高度的障碍（dy>=0 相对于脚下），绝不挖脚下地板（dy<0），
        // 否则会把自己站的地板挖穿、掉进坑里。
        for (int dy = 0; dy <= 5; dy++) {
            BlockPos bp = feet.offset(0, dy, 0);
            if (bp.distSqr(feet) > AUTO_DIG_RANGE * AUTO_DIG_RANGE) break;
            BlockState bs = level.getBlockState(bp);
            if (bs.isAir() || bs.canBeReplaced() || bs.getBlock() == Blocks.BEDROCK)
                continue;
            // 不挖玩家自己站立/头部所在的方块（避免把自己埋了）
            if (dy == 0 && bp.equals(feet)) continue;
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
        pillarTries = 0;
        player.setShiftKeyDown(false);
        lastProgressPos = player.position();
    }

    public void stop() {
        player.zza = 0;
        player.xxa = 0;
        player.setSprinting(false);
        player.setSwimming(false);
        player.setShiftKeyDown(false);
        player.setDeltaMovement(0, player.getDeltaMovement().y, 0);
    }

    public int remaining() { return Math.max(0, waypoints.size() - index); }
    public int currentIndex() { return index; }
}
