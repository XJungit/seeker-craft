package com.craftagent.bridge;

import com.craftagent.bridge.pathing.*;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.Nested;
import static org.junit.jupiter.api.Assertions.*;
import net.minecraft.core.BlockPos;
import java.util.List;

public class PathingTest {

    @Nested
    class NavGoalTests {
        @Test
        void exactGoal() {
            BlockPos pos = new BlockPos(10, 64, 20);
            NavGoal g = NavGoal.exact(pos);
            assertTrue(g.isAt(pos));
            assertFalse(g.isAt(new BlockPos(10, 64, 21)));
            assertEquals(0, g.heuristic(pos), 0.001);
            assertTrue(g.heuristic(new BlockPos(10, 65, 20)) > 0);
            assertEquals(pos, g.center());
        }

        @Test
        void nearGoal() {
            BlockPos pos = new BlockPos(10, 64, 20);
            NavGoal g = NavGoal.near(pos, 2.5);
            assertTrue(g.isAt(new BlockPos(10, 64, 20)));
            assertTrue(g.isAt(new BlockPos(11, 64, 22)));
            assertFalse(g.isAt(new BlockPos(20, 64, 20)));
            assertEquals(pos, g.center());
        }

        @Test
        void adjacentGoal() {
            BlockPos pos = new BlockPos(10, 64, 20);
            NavGoal g = NavGoal.adjacent(pos);
            assertTrue(g.isAt(pos), "adjacent should accept same pos (dx+dz=0 <=1)");
            assertTrue(g.isAt(new BlockPos(11, 64, 20)));
            assertTrue(g.isAt(new BlockPos(10, 64, 21)));
            assertFalse(g.isAt(new BlockPos(12, 64, 20)));
            assertEquals(0, g.heuristic(new BlockPos(11, 64, 20)), 0.001);
        }

        @Test
        void columnGoal() {
            NavGoal g = NavGoal.column(10, 20);
            assertTrue(g.isAt(new BlockPos(10, 64, 20)));
            assertTrue(g.isAt(new BlockPos(10, 128, 20)));
            assertFalse(g.isAt(new BlockPos(11, 64, 20)));
        }

        @Test
        void yLevelGoal() {
            NavGoal g = NavGoal.yLevel(64);
            assertTrue(g.isAt(new BlockPos(0, 64, 0)));
            assertFalse(g.isAt(new BlockPos(0, 65, 0)));
        }

        @Test
        void compositeGoal() {
            BlockPos a = new BlockPos(10, 64, 20);
            BlockPos b = new BlockPos(-10, 64, -20);
            NavGoal g = NavGoal.composite(List.of(NavGoal.exact(a), NavGoal.exact(b)));
            assertTrue(g.isAt(a));
            assertTrue(g.isAt(b));
            assertFalse(g.isAt(new BlockPos(0, 64, 0)));
        }

        @Test
        void pointBoundHeuristic() {
            BlockPos a = new BlockPos(0, 64, 0);
            BlockPos b = new BlockPos(3, 64, 4);
            double h = NavGoal.pointBound(a, b);
            double expected = 3 * Math.sqrt(2) + (3 + 4 - 2 * 3); // octile: min(3,4)*√2 + (7-6)
            assertEquals(expected, h, 0.001);
        }

        @Test
        void runAwayGoal() {
            BlockPos from = new BlockPos(0, 64, 0);
            NavGoal g = NavGoal.runAway(from, 10);
            double h1 = g.heuristic(new BlockPos(8, 64, 0));
            double h2 = g.heuristic(new BlockPos(2, 64, 0));
            assertTrue(h1 < h2, "further from danger = more negative = better");
        }
    }

    @Nested
    class NavContextClassificationTests {
        @Test
        void leavesArePassable() {
            assertEquals(NavContext.BlockClass.PASSABLE, NavContext.classifyId("minecraft:oak_leaves", false));
            assertEquals(NavContext.BlockClass.PASSABLE, NavContext.classifyId("minecraft:azalea_leaves", false));
        }

        @Test
        void fluidsClassified() {
            assertEquals(NavContext.BlockClass.WATER, NavContext.classifyId("minecraft:water", false));
            assertEquals(NavContext.BlockClass.LAVA, NavContext.classifyId("minecraft:lava", false));
        }

        @Test
        void commonPassables() {
            assertEquals(NavContext.BlockClass.PASSABLE, NavContext.classifyId("minecraft:oak_fence", false));
            assertEquals(NavContext.BlockClass.PASSABLE, NavContext.classifyId("minecraft:glass_pane", false));
            assertEquals(NavContext.BlockClass.PASSABLE, NavContext.classifyId("minecraft:oak_door", false));
            assertEquals(NavContext.BlockClass.PASSABLE, NavContext.classifyId("minecraft:rail", false));
            assertEquals(NavContext.BlockClass.PASSABLE, NavContext.classifyId("minecraft:torch", false));
            assertEquals(NavContext.BlockClass.PASSABLE, NavContext.classifyId("minecraft:snow", false));
            assertEquals(NavContext.BlockClass.PASSABLE, NavContext.classifyId("minecraft:glass", false));
            assertEquals(NavContext.BlockClass.PASSABLE, NavContext.classifyId("minecraft:vine", false));
            assertEquals(NavContext.BlockClass.PASSABLE, NavContext.classifyId("minecraft:ladder", false));
            assertEquals(NavContext.BlockClass.PASSABLE, NavContext.classifyId("minecraft:cobweb", false));
            assertEquals(NavContext.BlockClass.PASSABLE, NavContext.classifyId("minecraft:bamboo", false));
        }

        @Test
        void airIsPassable() {
            assertEquals(NavContext.BlockClass.PASSABLE, NavContext.classifyId("minecraft:air", true));
        }

        @Test
        void solidsRemainSolid() {
            assertEquals(NavContext.BlockClass.SOLID, NavContext.classifyId("minecraft:stone", false));
            assertEquals(NavContext.BlockClass.SOLID, NavContext.classifyId("minecraft:dirt", false));
            assertEquals(NavContext.BlockClass.SOLID, NavContext.classifyId("minecraft:oak_log", false));
            assertEquals(NavContext.BlockClass.SOLID, NavContext.classifyId("minecraft:cobblestone", false));
            assertEquals(NavContext.BlockClass.SOLID, NavContext.classifyId("minecraft:deepslate", false));
            assertEquals(NavContext.BlockClass.SOLID, NavContext.classifyId("minecraft:bedrock", false));
            assertEquals(NavContext.BlockClass.SOLID, NavContext.classifyId("minecraft:grass_block", false));
        }

        @Test
        void lavaOverridesAll() {
            assertEquals(NavContext.BlockClass.LAVA, NavContext.classifyId("minecraft:lava", true));
            assertEquals(NavContext.BlockClass.LAVA, NavContext.classifyId("minecraft:flowing_lava", false));
        }

        @Test
        void waterOverrides() {
            assertEquals(NavContext.BlockClass.WATER, NavContext.classifyId("minecraft:water", true));
            assertEquals(NavContext.BlockClass.WATER, NavContext.classifyId("minecraft:flowing_water", false));
        }

        @Test
        void additionalClassifyCases() {
            assertEquals(NavContext.BlockClass.PASSABLE, NavContext.classifyId("minecraft:structure_void", true));
            assertEquals(NavContext.BlockClass.PASSABLE, NavContext.classifyId("minecraft:light", true));
            assertEquals(NavContext.BlockClass.PASSABLE, NavContext.classifyId("minecraft:sculk_vein", true));
            assertEquals(NavContext.BlockClass.PASSABLE, NavContext.classifyId("minecraft:candle", false));
            assertEquals(NavContext.BlockClass.PASSABLE, NavContext.classifyId("minecraft:amethyst_cluster", false));
            assertEquals(NavContext.BlockClass.PASSABLE, NavContext.classifyId("minecraft:lantern", false));
            assertEquals(NavContext.BlockClass.SOLID, NavContext.classifyId("minecraft:netherrack", false));
            assertEquals(NavContext.BlockClass.SOLID, NavContext.classifyId("minecraft:sand", false));
            assertEquals(NavContext.BlockClass.SOLID, NavContext.classifyId("minecraft:gravel", false));
            assertEquals(NavContext.BlockClass.SOLID, NavContext.classifyId("minecraft:smooth_basalt", false));
        }
    }

    @Nested
    class MovementTests {
        @Test
        void movementConstruction() {
            BlockPos src = new BlockPos(0, 64, 0);
            BlockPos dest = new BlockPos(1, 64, 0);
            Movement m = new Movement(Movement.Kind.TRAVERSE, src, dest, 1.0);
            assertEquals(Movement.Kind.TRAVERSE, m.kind);
            assertEquals(src, m.src);
            assertEquals(dest, m.dest);
            assertEquals(1.0, m.cost, 0.001);
            assertTrue(m.toBreak.isEmpty());
            assertNull(m.toPlace);
        }

        @Test
        void movementWithBreakAndPlace() {
            BlockPos src = new BlockPos(0, 64, 0);
            BlockPos dest = new BlockPos(0, 65, 0);
            BlockPos toBreak = new BlockPos(0, 64, 1);
            BlockPos toPlace = new BlockPos(0, 63, 0);
            Movement m = new Movement(Movement.Kind.PILLAR, src, dest, 2.5,
                List.of(toBreak), toPlace);
            assertEquals(Movement.Kind.PILLAR, m.kind);
            assertEquals(1, m.toBreak.size());
            assertEquals(toBreak, m.toBreak.get(0));
            assertEquals(toPlace, m.toPlace);
        }

        @Test
        void allKindsCovered() {
            assertNotNull(Movement.Kind.valueOf("TRAVERSE"));
            assertNotNull(Movement.Kind.valueOf("ASCEND"));
            assertNotNull(Movement.Kind.valueOf("DESCEND"));
            assertNotNull(Movement.Kind.valueOf("FALL"));
            assertNotNull(Movement.Kind.valueOf("DIAGONAL"));
            assertNotNull(Movement.Kind.valueOf("PILLAR"));
            assertNotNull(Movement.Kind.valueOf("DIG_DOWN"));
            assertNotNull(Movement.Kind.valueOf("PARKOUR"));
            assertEquals(8, Movement.Kind.values().length);
        }
    }
}
