package com.craftagent.bridge;

import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

public class AStarTest {
    @Test
    void leavesArePassable() {
        assertEquals(AStar.BlockClass.PASSABLE, AStar.classifyId("minecraft:oak_leaves", false));
        assertEquals(AStar.BlockClass.PASSABLE, AStar.classifyId("minecraft:azalea_leaves", false));
        assertEquals(AStar.BlockClass.PASSABLE, AStar.classifyId("minecraft:mangrove_leaves", false));
    }

    @Test
    void fluidsClassified() {
        assertEquals(AStar.BlockClass.WATER, AStar.classifyId("minecraft:water", false));
        assertEquals(AStar.BlockClass.WATER, AStar.classifyId("minecraft:flowing_water", false));
        assertEquals(AStar.BlockClass.LAVA, AStar.classifyId("minecraft:lava", false));
    }

    @Test
    void commonPassables() {
        assertEquals(AStar.BlockClass.PASSABLE, AStar.classifyId("minecraft:grass", false));
        assertEquals(AStar.BlockClass.PASSABLE, AStar.classifyId("minecraft:tall_grass", false));
        assertEquals(AStar.BlockClass.PASSABLE, AStar.classifyId("minecraft:oak_sapling", false));
        assertEquals(AStar.BlockClass.PASSABLE, AStar.classifyId("minecraft:red_flower", false));
        assertEquals(AStar.BlockClass.PASSABLE, AStar.classifyId("minecraft:oak_fence", false));
        assertEquals(AStar.BlockClass.PASSABLE, AStar.classifyId("minecraft:glass_pane", false));
        assertEquals(AStar.BlockClass.PASSABLE, AStar.classifyId("minecraft:oak_door", false));
        assertEquals(AStar.BlockClass.PASSABLE, AStar.classifyId("minecraft:rail", false));
        assertEquals(AStar.BlockClass.PASSABLE, AStar.classifyId("minecraft:torch", false));
        assertEquals(AStar.BlockClass.PASSABLE, AStar.classifyId("minecraft:snow", false));
    }

    @Test
    void airAndReplaceableArePassable() {
        assertEquals(AStar.BlockClass.PASSABLE, AStar.classifyId("minecraft:stone", true));
        assertEquals(AStar.BlockClass.PASSABLE, AStar.classifyId("minecraft:oak_leaves", true));
    }

    @Test
    void solidsRemainSolid() {
        assertEquals(AStar.BlockClass.SOLID, AStar.classifyId("minecraft:stone", false));
        assertEquals(AStar.BlockClass.SOLID, AStar.classifyId("minecraft:dirt", false));
        assertEquals(AStar.BlockClass.SOLID, AStar.classifyId("minecraft:oak_log", false));
        assertEquals(AStar.BlockClass.SOLID, AStar.classifyId("minecraft:cobblestone", false));
        assertEquals(AStar.BlockClass.SOLID, AStar.classifyId("minecraft:deepslate", false));
    }
}
