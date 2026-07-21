package com.craftagent.bridge.pathing;

import net.minecraft.core.BlockPos;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.LevelChunk;
import java.util.*;
import java.util.concurrent.ConcurrentHashMap;

public class PathCaches {
    private static final int RADIUS_CHUNKS = 4;
    private static final Map<ResourceKey<Level>, LoadedSnapshot> SNAPSHOTS = new ConcurrentHashMap<>();

    public static class LoadedSnapshot {
        public final ServerLevel level;
        public final Map<Long, BlockState> blocks = new HashMap<>();
        private final Set<Long> loadedChunks = new HashSet<>();

        LoadedSnapshot(ServerLevel level, List<BlockPos> centers) {
            this.level = level;
            for (BlockPos center : centers) {
                int cx = center.getX() >> 4;
                int cz = center.getZ() >> 4;
                for (int dx = -RADIUS_CHUNKS; dx <= RADIUS_CHUNKS; dx++) {
                    for (int dz = -RADIUS_CHUNKS; dz <= RADIUS_CHUNKS; dz++) {
                        long cp = chunkKey(cx + dx, cz + dz);
                        if (loadedChunks.add(cp)) {
                            LevelChunk chunk = level.getChunk(cx + dx, cz + dz);
                            int bx = (cx + dx) << 4;
                            int bz = (cz + dz) << 4;
                            for (int x = bx; x < bx + 16; x++) {
                                for (int z = bz; z < bz + 16; z++) {
                                    for (int y = level.getMinY(); y <= level.getMaxY(); y++) {
                                        long bp = BlockPos.asLong(x, y, z);
                                        blocks.put(bp, chunk.getBlockState(new BlockPos(x, y, z)));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        public BlockState getBlockState(BlockPos pos) {
            return blocks.getOrDefault(pos.asLong(), net.minecraft.world.level.block.Blocks.AIR.defaultBlockState());
        }

        private static long chunkKey(int cx, int cz) {
            return ((long) cx << 32) | (cz & 0xFFFFFFFFL);
        }
    }

    public static LoadedSnapshot ensureSnapshot(ServerLevel level, BlockPos center) {
        ResourceKey<Level> key = level.dimension();
        LoadedSnapshot snap = SNAPSHOTS.get(key);
        if (snap == null || snap.level != level) {
            snap = new LoadedSnapshot(level, List.of(center));
            SNAPSHOTS.put(key, snap);
        }
        return snap;
    }

    public static void dropAll() {
        SNAPSHOTS.clear();
    }
}
