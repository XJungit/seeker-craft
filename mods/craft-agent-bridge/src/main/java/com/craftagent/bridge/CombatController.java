package com.craftagent.bridge;

import com.google.gson.JsonObject;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;
import com.craftagent.bridge.pathing.PlayerNavManager;

public class CombatController {

    private static CombatController instance;
    private String mode = "idle";
    private String result = "";
    private int ticksLeft;
    private int attackCooldown;
    private String targetType = "";

    private CombatController() {}

    public static synchronized CombatController get() {
        if (instance == null) instance = new CombatController();
        return instance;
    }

    public JsonObject start(String combatMode, int ticks) {
        JsonObject o = new JsonObject();
        this.mode = combatMode;
        this.ticksLeft = ticks;
        this.attackCooldown = 0;
        this.result = "";
        this.targetType = "";
        System.out.println("[combat] START mode=" + combatMode + " ticks=" + ticks);
        o.addProperty("status", "ok");
        o.addProperty("detail", "combat " + combatMode + " for " + ticks + " ticks started");
        return o;
    }

    public void tick() {
        if ("idle".equals(mode)) return;
        // #3/#5 仲裁：GoalEngine 正在执行 LLM 委托目标时，战斗交由目标内部逻辑，
        // 避免 CombatController 自动战斗与 GoalEngine 抢夺控制权。
        if (GoalEngine.get().isRunning()) return;
        if (ticksLeft <= 0) {
            finish("timeout");
            return;
        }
        ticksLeft--;
        if (attackCooldown > 0) { attackCooldown--; return; }

        ServerPlayer player = FakePlayerManager.getFirstPlayer(CraftAgentBridge.serverInstance);
        if (player == null) return;
        ServerLevel level = (ServerLevel) player.level();

        float hp = player.getHealth();

        // Find nearest threat
        LivingEntity target = null;
        double minDist = Double.MAX_VALUE;
        AABB area = AABB.ofSize(player.position(), 32, 32, 32);
        for (Entity e : level.getEntities(player, area)) {
            if (!(e instanceof LivingEntity le)) continue;
            String tn = BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).getPath();
            if (!InventoryHelper.isHostile(tn)) continue;
            double d = e.distanceTo(player);
            if (d < minDist) { minDist = d; target = le; targetType = tn; }
        }

        if (target == null) {
            finish("no_target");
            return;
        }

        // Retreat if low health
        if (hp < 6.0 && !"retreat".equals(mode)) {
            flee(player, target);
            finish("retreated_low_health");
            return;
        }

        // Retreat mode
        if ("retreat".equals(mode)) {
            flee(player, target);
            return;
        }

        // Kite: run away from creeper/skeleton
        if ("kite".equals(mode) && minDist < 6.0) {
            flee(player, target);
        } else if (minDist > 4.0) {
            if (!PlayerNavManager.get().isActive()) {
                PlayerNavManager.get().navigateTo(target.getX(), target.getY(), target.getZ());
            }
        } else {
            // In range: attack
            PlayerNavManager.get().stop();
            player.lookAt(net.minecraft.commands.arguments.EntityAnchorArgument.Anchor.EYES,
                target.position().add(0, 1, 0));
            InventoryHelper.equipBestWeapon(player);
            player.attack(target);
            player.containerMenu.broadcastChanges();
            attackCooldown = 10; // 0.5s cooldown
            System.out.println("[combat] ATTACK " + targetType + " hp=" + String.format("%.1f", target.getHealth()));
        }
    }

    private void flee(ServerPlayer player, LivingEntity threat) {
        PlayerNavManager.get().stop();
        double dx = player.getX() - threat.getX();
        double dz = player.getZ() - threat.getZ();
        double len = Math.sqrt(dx * dx + dz * dz);
        if (len > 0.01) {
            double fx = player.getX() + dx / len * 15.0;
            double fz = player.getZ() + dz / len * 15.0;
            PlayerNavManager.get().navigateTo(fx, player.getY(), fz);
        }
    }

    private void finish(String reason) {
        if (!"idle".equals(mode)) {
            System.out.println("[combat] DONE: " + reason + " target=" + targetType);
        }
        result = reason;
        mode = "idle";
        PlayerNavManager.get().stop();
    }

    public void stop() {
        finish("stopped");
    }

    public String statusString() {
        if ("idle".equals(mode)) return "idle";
        return "running: " + mode + " ticksLeft=" + ticksLeft + " target=" + targetType;
    }

    public static JsonObject actCombat(ServerPlayer player, ServerLevel level, JsonObject req) {
        String mode = req.has("mode") ? req.get("mode").getAsString() : "melee";
        int ticks = req.has("ticks") ? req.get("ticks").getAsInt() : 200;
        return CombatController.get().start(mode, ticks);
    }

    public static JsonObject actCombatStatus(ServerPlayer player, ServerLevel level, JsonObject req) {
        JsonObject o = new JsonObject();
        o.addProperty("status", "ok");
        o.addProperty("detail", CombatController.get().statusString());
        return o;
    }
}