package com.craftagent.bridge.pathing;

import net.minecraft.core.BlockPos;

public class PathNode implements Comparable<PathNode> {
    public final int x, y, z;
    public final double g, h;
    public final PathNode parent;
    public final Movement movement;

    public PathNode(int x, int y, int z, double g, double h, PathNode parent, Movement movement) {
        this.x = x; this.y = y; this.z = z;
        this.g = g; this.h = h; this.parent = parent;
        this.movement = movement;
    }

    public double f() { return g + h; }

    public int compareTo(PathNode o) { return Double.compare(f(), o.f()); }

    public BlockPos pos() { return new BlockPos(x, y, z); }

    public String key() { return x + "," + y + "," + z; }
}
