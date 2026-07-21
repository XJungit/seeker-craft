package com.craftagent.bridge.pathing;

import net.minecraft.core.BlockPos;
import net.minecraft.server.level.ServerPlayer;
import com.craftagent.bridge.FakePlayerManager;
import com.craftagent.bridge.CraftAgentBridge;
import java.util.function.Supplier;

public class PlayerNavManager {
    private static PlayerNavManager instance;
    private PlayerNav nav;
    private BlockPos target;
    private boolean active;

    private PlayerNavManager() {}

    public static synchronized PlayerNavManager get() {
        if (instance == null) instance = new PlayerNavManager();
        return instance;
    }

    public void navigateTo(double x, double y, double z) {
        ServerPlayer player = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
        if (player == null) return;
        this.target = BlockPos.containing(x, y, z);
        NavGoal goal = NavGoal.near(target, 1.5);
        this.nav = new PlayerNav(player, () -> goal, 1.0);
        this.nav.start();
        this.active = true;
    }

    public void navigateTo(BlockPos pos) {
        navigateTo(pos.getX() + 0.5, pos.getY(), pos.getZ() + 0.5);
    }

    public void stop() {
        if (nav != null) nav.stop();
        nav = null;
        active = false;
        target = null;
    }

    public void tick() {
        if (!active || nav == null) return;
        PlayerNav.Status s = nav.tick();
        if (s == PlayerNav.Status.ARRIVED) {
            System.out.println("[nav] ARRIVED at " + target);
            stop();
        } else if (s == PlayerNav.Status.FAILED) {
            System.out.println("[nav] FAILED: " + nav.failReason());
            stop();
        }
    }

    public boolean isActive() { return active; }
    public BlockPos target() { return target; }
    public PlayerNav.Status status() { return nav != null ? nav.status() : PlayerNav.Status.IDLE; }
    public String statusString() {
        if (!active) return "idle";
        if (nav == null) return "idle";
        switch (nav.status()) {
            case RUNNING: return "running (replans=" + nav.replans() + ", remaining="
                + (nav.executor() != null ? nav.executor().remaining() : "?") + ")";
            case ARRIVED: return "arrived";
            case FAILED: return "failed: " + nav.failReason();
            case IDLE: return "idle";
            default: return "unknown";
        }
    }
}
