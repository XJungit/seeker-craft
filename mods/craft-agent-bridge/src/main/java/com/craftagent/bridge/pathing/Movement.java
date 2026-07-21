package com.craftagent.bridge.pathing;

import net.minecraft.core.BlockPos;
import java.util.List;
import java.util.ArrayList;
import java.util.Collections;

public class Movement {
    public enum Kind {
        TRAVERSE, ASCEND, DESCEND, FALL, DIAGONAL, PILLAR, DIG_DOWN, PARKOUR
    }

    public final Kind kind;
    public final BlockPos src;
    public final BlockPos dest;
    public final double cost;
    public final List<BlockPos> toBreak;
    public final BlockPos toPlace;

    public Movement(Kind kind, BlockPos src, BlockPos dest, double cost,
                    List<BlockPos> toBreak, BlockPos toPlace) {
        this.kind = kind;
        this.src = src;
        this.dest = dest;
        this.cost = cost;
        this.toBreak = toBreak != null ? Collections.unmodifiableList(new ArrayList<>(toBreak)) : List.of();
        this.toPlace = toPlace;
    }

    public Movement(Kind kind, BlockPos src, BlockPos dest, double cost) {
        this(kind, src, dest, cost, List.of(), null);
    }

    public String toString() {
        return kind + " (" + src.toShortString() + " -> " + dest.toShortString() + " cost=" + String.format("%.1f", cost) + ")";
    }
}
