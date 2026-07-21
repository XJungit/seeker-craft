package com.craftagent.bridge.pathing;

import net.minecraft.core.BlockPos;
import java.util.List;
import java.util.Collections;

public class Path {
    public final BlockPos start;
    public final BlockPos end;
    public final List<Movement> movements;
    public final boolean partial;

    public Path(BlockPos start, BlockPos end, List<Movement> movements, boolean partial) {
        this.start = start;
        this.end = end;
        this.movements = movements != null ? Collections.unmodifiableList(movements) : List.of();
        this.partial = partial;
    }

    public boolean isEmpty() { return movements.isEmpty(); }

    public int length() { return movements.size(); }

    public String toString() {
        return "Path(" + start.toShortString() + " -> " + end.toShortString()
            + ", " + movements.size() + " moves, partial=" + partial + ")";
    }
}
