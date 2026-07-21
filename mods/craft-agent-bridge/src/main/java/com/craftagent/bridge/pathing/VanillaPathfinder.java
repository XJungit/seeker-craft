package com.craftagent.bridge.pathing;

import net.minecraft.core.BlockPos;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.ai.navigation.PathNavigation;
import net.minecraft.world.entity.ai.navigation.AmphibiousPathNavigation;
import net.minecraft.world.entity.ai.navigation.GroundPathNavigation;
import net.minecraft.world.entity.monster.zombie.Zombie;
import net.minecraft.world.level.pathfinder.Path;
import java.util.ArrayList;
import java.util.List;

public class VanillaPathfinder {
    private static final int MAX_NODES = 50000;
    private static final int SEARCH_RANGE = 48;

    public static List<BlockPos> findPath(ServerLevel level, ServerPlayer player, BlockPos target) {
        Zombie dummy = new Zombie(level);
        try {
            dummy.setPos(player.getX(), player.getY(), player.getZ());
            dummy.setYRot(player.getYRot());
            dummy.yHeadRot = player.yHeadRot;

            boolean aquatic = isAquatic(level, player.blockPosition(), target);
            PathNavigation nav;
            if (aquatic) {
                nav = new AmphibiousPathNavigation(dummy, level);
            } else {
                nav = new GroundPathNavigation(dummy, level);
            }
            nav.setSpeedModifier(1.0);
            nav.setCanFloat(true);

            Path path = nav.createPath(target, SEARCH_RANGE);
            if (path == null || path.isDone() || path.getNodeCount() == 0) return null;

            List<BlockPos> waypoints = new ArrayList<>();
            for (int i = path.getNextNodeIndex(); i < path.getNodeCount(); i++) {
                waypoints.add(path.getNodePos(i));
            }
            return waypoints;
        } finally {
            dummy.kill(level);
            dummy.remove(Entity.RemovalReason.DISCARDED);
        }
    }

    private static boolean isAquatic(ServerLevel level, BlockPos from, BlockPos to) {
        return containsWater(level, from) || containsWater(level, to);
    }

    private static boolean containsWater(ServerLevel level, BlockPos pos) {
        for (int dx = -2; dx <= 2; dx++) {
            for (int dz = -2; dz <= 2; dz++) {
                for (int dy = -2; dy <= 2; dy++) {
                    if (level.getBlockState(pos.offset(dx, dy, dz)).getFluidState().is(
                        net.minecraft.tags.FluidTags.WATER)) return true;
                }
            }
        }
        return false;
    }
}
