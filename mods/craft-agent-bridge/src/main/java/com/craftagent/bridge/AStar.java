package com.craftagent.bridge;

import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.Vec3;
import java.util.*;

/**
 * 增强 A* 寻路：
 * - 8 方向水平 + 上下共 10 方向
 * - 带重力下落/1 格跨步
 * - 2 格人身高碰撞箱检测
 * - 路径平滑（去共线中间点）
 * - 搜索范围 48×32×48，30000 节点上限
 * - 目标/起点自适应水平扫描
 * - A* 失败时返回单向 fallback 确保能移动
 */
public class AStar {
    static final int MAX_X = 48, MAX_Y = 32, MAX_Z = 48;
    static final int MAX_NODES = 30000;
    static final double SQRT2 = Math.sqrt(2);
    static final int MAX_STEP_UP = 1;
    // 硬坠落保护：落差 > 3 格必摔伤（MC 4 格起掉血），A* 直接拒绝该节点，
    // 强制绕缓坡/楼梯，避免 bot 沿悬崖直坠摔死。
    static final int MAX_FALL = 3;

    static final int[][] DIRS = {
        {1,0,0}, {-1,0,0}, {0,0,1}, {0,0,-1},
        {1,0,1}, {1,0,-1}, {-1,0,1}, {-1,0,-1},
        {0,1,0}, {0,-1,0}
    };

    static class Node implements Comparable<Node> {
        int x, y, z;
        double g, h;
        Node parent;
        Node(int x, int y, int z, double g, double h, Node parent) {
            this.x = x; this.y = y; this.z = z;
            this.g = g; this.h = h; this.parent = parent;
        }
        double f() { return g + h; }
        public int compareTo(Node o) { return Double.compare(f(), o.f()); }
    }

    static List<Vec3> findPath(ServerLevel level, Vec3 from, Vec3 to) {
        BlockPos origin = BlockPos.containing(from);
        BlockPos target = BlockPos.containing(to);

        BlockPos startPos = findStandingPos(level, origin, 2);
        if (startPos == null) startPos = origin;

        BlockPos finalTarget = findStandingPos(level, target, 4);
        if (finalTarget == null) {
            System.out.println("[AStar] WARN no standing pos near target " + target + ", using direct");
            List<Vec3> fallback = new ArrayList<>();
            fallback.add(Vec3.atCenterOf(target));
            return fallback;
        }

        // 如果起终点在同一格，直接返回
        if (startPos.distSqr(finalTarget) < 0.1) {
            List<Vec3> direct = new ArrayList<>();
            direct.add(Vec3.atCenterOf(finalTarget));
            return direct;
        }

        int originY = origin.getY();
        int sX = startPos.getX() - origin.getX();
        int sY = startPos.getY() - originY;
        int sZ = startPos.getZ() - origin.getZ();

        PriorityQueue<Node> open = new PriorityQueue<>();
        HashSet<String> closed = new HashSet<>();
        open.add(new Node(sX, sY, sZ, 0,
            octileDist(sX, sY, sZ, finalTarget, origin), null));

        int expandedNodes = 0;
        while (!open.isEmpty() && closed.size() < MAX_NODES) {
            Node cur = open.poll();
            String key = cur.x + "," + cur.y + "," + cur.z;
            if (!closed.add(key)) continue;
            expandedNodes++;

            int wx = origin.getX() + cur.x;
            int wy = originY + cur.y;
            int wz = origin.getZ() + cur.z;

            if (Math.abs(wx - finalTarget.getX()) <= 1
                && Math.abs(wz - finalTarget.getZ()) <= 1
                && wy == finalTarget.getY()) {
                System.out.println("[AStar] found path expanded=" + expandedNodes + " nodes=" + closed.size());
                return smoothPath(buildPath(cur, origin));
            }

            int fromY = wy;
            for (int[] d : DIRS) {
                int nx = cur.x + d[0];
                int ny = cur.y + d[1];
                int nz = cur.z + d[2];
                if (Math.abs(nx) > MAX_X / 2 || Math.abs(ny) > MAX_Y / 2 || Math.abs(nz) > MAX_Z / 2)
                    continue;

                int adjY = ny;
                double moveCost;

                if (d[1] == 0) {
                    int bx = origin.getX() + nx;
                    int bz = origin.getZ() + nz;
                    int searchStart = fromY + MAX_STEP_UP;
                    int landingY = findLandingY(level, bx, bz, searchStart);
                    if (landingY == Integer.MIN_VALUE) continue;
                    adjY = landingY - originY;

                    int heightDiff = landingY - fromY;
                    if (heightDiff > MAX_STEP_UP) continue;

                    // 跨步时中间方块 (bx, fromY, bz) 就是被跨的方块本身（实心），无需检查
                    // 目标的可站性由 isStandable 在 findLandingY 中保证

                    int drop = fromY - landingY;
                    if (drop < 0) drop = 0;
                    if (drop > MAX_FALL) continue;

                    moveCost = 0.0;
                    boolean diagonal = d[0] != 0 && d[2] != 0;
                    moveCost += diagonal ? SQRT2 : 1.0;
                    if (heightDiff == 1) moveCost += 0.5;
                    if (drop > 0) moveCost += drop * 0.15 + 0.3;
                } else if (d[1] == 1) {
                    int bx = origin.getX() + nx;
                    int bz = origin.getZ() + nz;
                    int by = originY + ny;
                    if (!isStandable(level, bx, by, bz)) continue;
                    BlockState belowJump = level.getBlockState(new BlockPos(bx, by - 1, bz));
                    if (classify(belowJump) != BlockClass.SOLID) continue;
                    moveCost = 2.5;
                } else {
                    int bx = origin.getX() + nx;
                    int bz = origin.getZ() + nz;
                    int by = originY + ny;
                    int landingY = findLandingY(level, bx, bz, by);
                    if (landingY == Integer.MIN_VALUE) continue;
                    adjY = landingY - originY;
                    int drop = (originY + ny) - landingY;
                    if (drop > MAX_FALL) continue;
                    moveCost = 1.0 + drop * 0.15;
                }

                String nk = nx + "," + adjY + "," + nz;
                if (closed.contains(nk)) continue;

                double ng = cur.g + moveCost;
                open.add(new Node(nx, adjY, nz, ng,
                    octileDist(nx, adjY, nz, finalTarget, origin), cur));
            }
        }

        // A* 搜索失败 → fallback：直接朝目标走（至少能尝试移动）
        System.out.println("[AStar] A* FAILED expanded=" + expandedNodes + " closed=" + closed.size()
            + " from=" + origin + " to=" + finalTarget + " — using direct fallback");
        List<Vec3> fallback = new ArrayList<>();
        fallback.add(Vec3.atCenterOf(finalTarget));
        return fallback;
    }

    // ── 方块通行分类（类似 Numen blocks.js 的 walkable/passable 表）──
    // SOLID    : 可踩踏、挡身体（常规实心方块：stone/dirt/log/planks/...）
    // PASSABLE : 可穿过身体、不可踩（air/树叶/草/花/地毯/玻璃/门/栅栏/藤蔓/雪层/水草...）
    // WATER    : 流体，水下可游
    // LAVA     : 流体，致命不可入
    enum BlockClass { SOLID, PASSABLE, WATER, LAVA }

    static BlockClass classify(BlockState bs) {
        String id = BuiltInRegistries.BLOCK.getKey(bs.getBlock()).toString();
        // air 与可替换（稀疏植被/雪层等）直接可穿过
        boolean airOrReplaceable = bs.isAir() || bs.canBeReplaced();
        return classifyId(id, airOrReplaceable);
    }

    /**
     * 纯函数方块分类（不依赖 Minecraft 运行时，便于单元测试）。
     * 把方块 id 字符串 + 是否 air/可替换 映射为通行分类。
     */
    static BlockClass classifyId(String id, boolean airOrReplaceable) {
        // 流体优先
        if (id.contains("lava")) return BlockClass.LAVA;
        if (id.contains("water")) return BlockClass.WATER;
        // air 与可替换（稀疏植被/雪层等）直接可穿过
        if (airOrReplaceable) return BlockClass.PASSABLE;

        // 非固体 / 可穿过方块（站进去、头穿过去都合法）
        // 用尾部/关键字匹配，覆盖 MC 常见非固体方块
        if (id.contains("_leaves")           // 各类树叶
            || id.contains("leaves")
            || id.endsWith("grass")          // 草 (tall_grass / fern)
            || id.contains("_sapling")       // 树苗
            || id.contains("flower")
            || id.contains("mushroom")
            || id.contains("carpet")         // 地毯
            || id.contains("glass")          // 玻璃（不含玻璃板单独处理）
            || id.contains("pane")           // 玻璃板
            || id.contains("fence")          // 栅栏（可穿）
            || id.contains("iron_bars")
            || id.contains("door")           // 门（开启时可穿）
            || id.contains("trapdoor")
            || id.contains("vine")           // 藤蔓
            || id.contains("ladder")         // 梯子
            || id.contains("snow")           // 雪层
            || id.contains("seagrass")
            || id.contains("kelp")
            || id.contains("lily_pad")
            || id.contains("torch")          // 火把
            || id.contains("lantern")
            || id.contains("cobweb")
            || id.contains("bamboo")         // 竹笋/竹子（细）
            || id.contains("reeds")          // 甘蔗
            || id.contains("coral")          // 珊瑚（非实心）
            || id.contains("banner")
            || id.contains("pressure_plate")
            || id.contains("button")
            || id.contains("rail")            // 铁轨
            || id.contains("_sign")
            || id.contains("candle")
            || id.contains("amethyst_cluster")
            || id.contains("bud")
            || id.contains("hang")) {
            return BlockClass.PASSABLE;
        }
        return BlockClass.SOLID;
    }

    // 身体格（脚/头）是否可通过：可穿过或水面
    static boolean bodyPassable(BlockClass c) {
        return c == BlockClass.PASSABLE || c == BlockClass.WATER;
    }

    static boolean isStandable(ServerLevel level, int bx, int by, int bz) {
        BlockClass feet = classify(level.getBlockState(new BlockPos(bx, by, bz)));
        BlockClass head = classify(level.getBlockState(new BlockPos(bx, by + 1, bz)));
        BlockClass floor = classify(level.getBlockState(new BlockPos(bx, by - 1, bz)));

        // 水下可游：脚/头可水，下方可水（漂浮），但岩浆致命
        if (feet == BlockClass.LAVA || head == BlockClass.LAVA || floor == BlockClass.LAVA)
            return false;
        if (feet == BlockClass.WATER || head == BlockClass.WATER || floor == BlockClass.WATER)
            return true;

        // 身体两格必须可穿过
        if (!bodyPassable(feet)) return false;
        if (!bodyPassable(head)) return false;

        // 脚底必须是实心可踩
        return floor == BlockClass.SOLID;
    }

    static int findLandingY(ServerLevel level, int bx, int bz, int startY) {
        for (int y = startY; y >= startY - MAX_FALL; y--) {
            if (isStandable(level, bx, y, bz)) {
                return y;
            }
        }
        return Integer.MIN_VALUE;
    }

    /** 在 (pos.x, pos.z) 附近 radius 格内查找有效站立位置。 */
    static BlockPos findStandingPos(ServerLevel level, BlockPos pos, int radius) {
        int x = pos.getX(), z = pos.getZ();
        for (int dy = 0; dy >= -8; dy--) {
            int y = pos.getY() + dy;
            if (y <= level.getMinY()) break;
            if (isStandable(level, x, y, z)) return new BlockPos(x, y, z);
        }
        // 向外扩展扫描
        for (int r = 1; r <= radius; r++) {
            for (int dx = -r; dx <= r; dx++) {
                for (int dz = -r; dz <= r; dz++) {
                    if (Math.abs(dx) != r && Math.abs(dz) != r) continue;
                    int nx = x + dx, nz = z + dz;
                    for (int dy = 0; dy >= -6; dy--) {
                        int y = pos.getY() + dy;
                        if (y <= level.getMinY()) break;
                        if (isStandable(level, nx, y, nz)) return new BlockPos(nx, y, nz);
                    }
                }
            }
        }
        return null;
    }

    static double octileDist(int x, int y, int z, BlockPos target, BlockPos origin) {
        double dx = Math.abs((origin.getX() + x) - target.getX());
        double dy = Math.abs((origin.getY() + y) - target.getY());
        double dz = Math.abs((origin.getZ() + z) - target.getZ());
        double h = Math.min(dx, dz);
        return h * SQRT2 + (dx + dz - 2 * h) + dy;
    }

    static List<Vec3> buildPath(Node end, BlockPos origin) {
        LinkedList<Vec3> path = new LinkedList<>();
        Node cur = end;
        while (cur != null) {
            int wx = origin.getX() + cur.x;
            int wy = origin.getY() + cur.y;
            int wz = origin.getZ() + cur.z;
            path.addFirst(Vec3.atCenterOf(new BlockPos(wx, wy, wz)));
            cur = cur.parent;
        }
        return path;
    }

    static List<Vec3> smoothPath(List<Vec3> path) {
        if (path.size() <= 2) return path;
        List<Vec3> result = new ArrayList<>();
        result.add(path.get(0));
        for (int i = 1; i < path.size() - 1; i++) {
            Vec3 prev = result.get(result.size() - 1);
            Vec3 cur = path.get(i);
            Vec3 next = path.get(i + 1);

            double dx1 = cur.x - prev.x;
            double dz1 = cur.z - prev.z;
            double dx2 = next.x - cur.x;
            double dz2 = next.z - cur.z;

            boolean sameDirX = Math.abs(dx1) < 0.01 || Math.abs(dx2) < 0.01
                || Math.signum(dx1) == Math.signum(dx2);
            boolean sameDirZ = Math.abs(dz1) < 0.01 || Math.abs(dz2) < 0.01
                || Math.signum(dz1) == Math.signum(dz2);
            boolean sameY = Math.abs(cur.y - prev.y) < 0.01 && Math.abs(next.y - cur.y) < 0.01;

            if (!sameDirX || !sameDirZ || !sameY) {
                result.add(cur);
            }
        }
        result.add(path.get(path.size() - 1));
        return result;
    }
}
