package com.craftagent.bridge.pathing;

import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.Vec3;
import com.craftagent.bridge.CraftAgentBridge;
import com.craftagent.bridge.InventoryHelper;
import java.util.List;
import java.util.function.Supplier;

public class PlayerPathExecutor {
    public enum Status { RUNNING, ARRIVED, STUCK, FAILED }

    private final ServerPlayer player;
    private final Path path;
    private final Supplier<NavContext> ctxSupplier;
    private int index;
    private int ticksOnCurrent;
    private int ticksSinceProgress;
    private Vec3 lastProgressPos;
    private boolean stuckWarned;
    private boolean jumpedThisTick;
    private static final int STUCK_WARN_TICKS = 40;
    private static final int STUCK_REPLAN_TICKS = 100;
    private static final double PROGRESS_THRESHOLD_SQ = 0.5;
    private static final double ARRIVAL_HORIZ_SQ = 0.8 * 0.8;
    private static final double ARRIVAL_VERT = 1.2;
    private static final double SWIM_UP_SPEED = 0.04;
    private static final int FORCE_FINISH_TICKS = 40;

    public PlayerPathExecutor(ServerPlayer player, Path path, Supplier<NavContext> ctxSupplier) {
        this.player = player;
        this.path = path;
        this.ctxSupplier = ctxSupplier;
        this.index = 0;
        this.ticksOnCurrent = 0;
        this.ticksSinceProgress = 0;
        this.lastProgressPos = player.position();
    }

    public Status tick() {
        if (path == null || path.movements.isEmpty()) return Status.ARRIVED;
        if (index >= path.movements.size()) return Status.ARRIVED;

        NavContext ctx = ctxSupplier.get();

        Movement movement = path.movements.get(index);
        Vec3 pos = player.position();
        jumpedThisTick = false;

        // Progress tracking using actual position
        double distSq = pos.distanceToSqr(lastProgressPos);
        if (distSq > PROGRESS_THRESHOLD_SQ) {
            ticksSinceProgress = 0;
            lastProgressPos = pos;
        } else {
            ticksSinceProgress++;
        }

        // Auto-dig obstructing blocks
        if (!movement.toBreak.isEmpty()) {
            for (BlockPos bp : movement.toBreak) {
                BlockState bs = ctx.view.getBlockState(bp);
                if (!bs.isAir() && !bs.canBeReplaced()) {
                    InventoryHelper.equipBestTool(player,
                        BuiltInRegistries.BLOCK.getKey(bs.getBlock()).toString());
                    player.level().destroyBlock(bp, true);
                    player.containerMenu.broadcastChanges();
                }
            }
        }

        // Auto-place for PILLAR
        if (movement.kind == Movement.Kind.PILLAR && movement.toPlace != null) {
            BlockState below = ctx.view.getBlockState(movement.toPlace.below());
            if ((below.isAir() || below.canBeReplaced()) && !touchesLava(ctx, movement.toPlace.below())) {
                autoPlace(movement.toPlace);
            }
        }

        Vec3 target = Vec3.atCenterOf(movement.dest);
        double hDistSq = Math.pow(pos.x - target.x, 2) + Math.pow(pos.z - target.z, 2);
        double vDist = Math.abs(pos.y - target.y);
        boolean nearHoriz = hDistSq < ARRIVAL_HORIZ_SQ;
        boolean nearVert = vDist < ARRIVAL_VERT;

        switch (movement.kind) {
            case TRAVERSE:
            case DIAGONAL:
                driveToward(target, true);
                if (player.isInWater() && player.horizontalCollision && player.onGround()) {
                    player.jumpFromGround();
                    jumpedThisTick = true;
                }
                break;
            case ASCEND:
                driveToward(target, true);
                if (player.horizontalCollision && player.onGround()) {
                    player.jumpFromGround();
                    jumpedThisTick = true;
                }
                if (nearHoriz && nearVert) {
                    advance();
                    return Status.RUNNING;
                }
                ticksOnCurrent++;
                return Status.RUNNING;
            case DESCEND:
                driveToward(target, true);
                break;
            case FALL:
                break;
            case PILLAR:
                driveToward(target, false);
                if (player.onGround()) {
                    player.jumpFromGround();
                    jumpedThisTick = true;
                    autoPlace(movement.toPlace);
                }
                break;
            case DIG_DOWN:
                for (BlockPos bp : movement.toBreak) {
                    InventoryHelper.equipBestTool(player,
                        BuiltInRegistries.BLOCK.getKey(ctx.view.getBlockState(bp).getBlock()).toString());
                    player.level().destroyBlock(bp, true);
                }
                break;
            case PARKOUR:
                driveToward(target, true);
                if (player.onGround()) {
                    player.jumpFromGround();
                    jumpedThisTick = true;
                }
                player.setSprinting(true);
                break;
        }

        ticksOnCurrent++;

        // Arrival check: near horizontally AND near vertically
        if (nearHoriz && nearVert) {
            advance();
            return Status.RUNNING;
        }

        // Force-finish if only 1 movement left and no progress for a while
        if (remainingMovements() <= 1 && ticksSinceProgress > FORCE_FINISH_TICKS) {
            System.out.println("[nav] force-finish last movement " + index + " (" + movement.kind
                + ") hDist=" + String.format("%.2f", Math.sqrt(hDistSq)) + " vDist=" + String.format("%.2f", vDist));
            advance();
            return Status.RUNNING;
        }

        // Stuck detection
        if (ticksSinceProgress > STUCK_REPLAN_TICKS) {
            if (!stuckWarned) {
                stuckWarned = true;
                System.out.println("[nav] STUCK at movement " + index + " (" + movement.kind + "), ticks=" + ticksSinceProgress);
            }
            return Status.STUCK;
        }

        if (ticksSinceProgress > STUCK_WARN_TICKS && !stuckWarned) {
            stuckWarned = true;
            System.out.println("[nav] slow progress at movement " + index + " (" + movement.kind + ") after " + ticksSinceProgress + " ticks");
        }

        return Status.RUNNING;
    }

    private void driveToward(Vec3 target, boolean sprint) {
        double ddx = target.x - player.getX();
        double ddz = target.z - player.getZ();
        double horiz = Math.sqrt(ddx * ddx + ddz * ddz);
        if (horiz < 0.01) {
            player.zza = 0;
            player.xxa = 0;
            return;
        }
        float yaw = (float) Math.toDegrees(Math.atan2(-ddx, ddz));
        player.setYRot(yaw);
        player.yHeadRot = yaw;

        player.zza = sprint ? 1.3f : 1.0f;
        player.setSprinting(sprint);

        double nx = ddx / horiz;
        double nz = ddz / horiz;
        double vy = player.getDeltaMovement().y;

        if (player.isInWater()) {
            player.setSwimming(true);
            double waterSpeed = sprint ? 0.25 : 0.18;
            double swimUp = player.isUnderWater() ? SWIM_UP_SPEED : 0.0;
            player.setDeltaMovement(nx * waterSpeed, swimUp, nz * waterSpeed);
        } else {
            double speed = sprint ? 0.35 : 0.25;
            if (!jumpedThisTick) vy = player.getDeltaMovement().y;
            player.setDeltaMovement(nx * speed, vy, nz * speed);
        }
    }

    private void autoPlace(BlockPos target) {
        if (target == null) return;
        int slot = findScaffoldSlot();
        if (slot < 0) return;
        player.getInventory().setSelectedSlot(slot);
        String itemId = BuiltInRegistries.ITEM.getKey(
            player.getInventory().getItem(slot).getItem()).toString();
        InventoryHelper.placeAt(player, player.level(),
            target.getX(), target.getY(), target.getZ(), itemId);
    }

    private int findScaffoldSlot() {
        Inventory inv = player.getInventory();
        for (int s = 0; s < 9; s++) {
            ItemStack stack = inv.getItem(s);
            if (!stack.isEmpty() && NavContext.isScaffold(stack)) return s;
        }
        for (int s = 9; s < inv.getContainerSize(); s++) {
            ItemStack stack = inv.getItem(s);
            if (!stack.isEmpty() && NavContext.isScaffold(stack)) {
                inv.setSelectedSlot(s);
                return s;
            }
        }
        return -1;
    }

    private boolean touchesLava(NavContext ctx, BlockPos pos) {
        for (int dx = -1; dx <= 1; dx++) {
            for (int dz = -1; dz <= 1; dz++) {
                for (int dy = -1; dy <= 1; dy++) {
                    if (ctx.classify(pos.offset(dx, dy, dz)) == NavContext.BlockClass.LAVA)
                        return true;
                }
            }
        }
        return false;
    }

    private void advance() {
        index++;
        ticksOnCurrent = 0;
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

    public int remainingMovements() { return Math.max(0, path.movements.size() - index); }
    public int currentIndex() { return index; }
    public Path path() { return path; }
}
