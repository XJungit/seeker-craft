#!/usr/bin/env python3
"""一次性完成 mod 侧所有优化：
1. B 类 action 移到 TCP 线程（attack_player、follow_player、use_item、eat_item、wait）
2. 改进 attack 为原生攻击 + attack 持续循环移到 TCP 线程
3. 添加 open_gui 命令（打开附近的容器方块）
4. 添加 smelt 持续循环移到 TCP 线程
"""

import re

filepath = r"d:\Craft-Agent\mods\craft-agent-bridge\src\main\java\com\craftagent\bridge\CraftAgentBridge.java"

with open(filepath, 'r', encoding='utf-8') as f:
    code = f.read()

# ══════════════════════════════════════════════════════════════
# 1. 在 dispatch 中添加更多 TCP 线程路由
# ══════════════════════════════════════════════════════════════

old_dispatch = '''        if ("collect_items".equals(type)) {
            return performCollectItems(req);
        }
        return runOnServerThread(() -> {'''

new_dispatch = '''        if ("collect_items".equals(type)) {
            return performCollectItems(req);
        }
        if ("attack".equals(type)) {
            return performAttack(req);
        }
        if ("attack_player".equals(type)) {
            return performAttackPlayer(req);
        }
        if ("follow_player".equals(type)) {
            return performFollowPlayer(req);
        }
        if ("use_item".equals(type)) {
            return performUseItem(req);
        }
        if ("eat_item".equals(type)) {
            return performEatItem(req);
        }
        if ("wait".equals(type)) {
            return performWait(req);
        }
        if ("combat".equals(type)) {
            return performCombat(req);
        }
        if ("smelt".equals(type)) {
            return performSmelt(req);
        }
        return runOnServerThread(() -> {'''

code = code.replace(old_dispatch, new_dispatch)

# ══════════════════════════════════════════════════════════════
# 2. 添加 onServer 泛型辅助方法（如果还没有的话）
# ══════════════════════════════════════════════════════════════

# 检查是否已有 onServer 方法
if 'private <T> T onServer(' not in code:
    # 在 getFirstPlayer 方法后插入 onServer 方法
    old_getfirst = '''    /** 获取第一个在线玩家（单人游戏中只有一个）。 */
    private static ServerPlayer getFirstPlayer(MinecraftServer server) {
        var players = server.getPlayerList().getPlayers();
        return players.isEmpty() ? null : players.get(0);
    }'''

    new_getfirst = '''    /** 获取第一个在线玩家（单人游戏中只有一个）。 */
    private static ServerPlayer getFirstPlayer(MinecraftServer server) {
        var players = server.getPlayerList().getPlayers();
        return players.isEmpty() ? null : players.get(0);
    }

    /** 在服务端线程执行任务并返回结果（TCP 线程调用的便捷方法）。 */
    private static <T> T onServer(java.util.function.Supplier<T> task) {
        MinecraftServer server = serverInstance;
        if (server == null) return null;
        java.util.concurrent.CompletableFuture<T> future = new java.util.concurrent.CompletableFuture<>();
        server.executeIfPossible(() -> {
            try {
                future.complete(task.get());
            } catch (Exception e) {
                future.completeExceptionally(e);
            }
        });
        try {
            return future.get(10, java.util.concurrent.TimeUnit.SECONDS);
        } catch (Exception e) {
            return null;
        }
    }

    /** 在服务端线程执行 Runnable（无返回值）。 */
    private static void onServerVoid(Runnable task) {
        MinecraftServer server = serverInstance;
        if (server == null) return;
        server.executeIfPossible(task);
    }'''

    code = code.replace(old_getfirst, new_getfirst)

# ══════════════════════════════════════════════════════════════
# 3. 在 performCollectItems 后面添加新的 TCP 线程方法
# ══════════════════════════════════════════════════════════════

# 找到 performCollectItems 方法结束位置（在 "状态查询" 注释之前）
old_collect_end = '''        o.addProperty("detail", "collect_items: collected " + collected + " items");
        return o;
    }

    // ══════════════════════════════════════════════════════════════
    // 状态查询
    // ══════════════════════════════════════════════════════════════'''

new_methods = '''        o.addProperty("detail", "collect_items: collected " + collected + " items");
        return o;
    }

    /** attack 在 TCP 线程执行：找目标 → 循环接近 + 攻击。 */
    private JsonObject performAttack(JsonObject req) {
        JsonObject o = new JsonObject();
        if (serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "服务器未就绪");
            return o;
        }
        int ticks = req.has("ticks") ? req.get("ticks").getAsInt() : 60;

        int hitCount = 0;
        long start = System.currentTimeMillis();
        long deadline = start + ticks * 50L;
        String targetType = "none";

        while (System.currentTimeMillis() < deadline) {
            if (shouldStop) { shouldStop = false; break; }

            double[] targetInfo = onServer(() -> {
                ServerPlayer p = getFirstPlayer(serverInstance);
                if (p == null) return null;
                ServerLevel lvl = p.level();
                LivingEntity target = null;
                double minDist = Double.MAX_VALUE;
                AABB scanArea = AABB.ofSize(p.position(), 16, 16, 16);
                for (Entity e : lvl.getEntities(p, scanArea)) {
                    if (!(e instanceof LivingEntity le)) continue;
                    String tn = BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).getPath();
                    if (!isHostile(tn)) continue;
                    double d = e.distanceTo(p);
                    if (d < minDist) { minDist = d; target = le; }
                }
                if (target == null) return null;
                return new double[]{target.getX(), target.getY(), target.getZ(), minDist, target.getHealth()};
            });

            if (targetInfo == null) break;

            double tx = targetInfo[0], ty = targetInfo[1], tz = targetInfo[2], dist = targetInfo[3];
            targetType = onServer(() -> {
                ServerPlayer p = getFirstPlayer(serverInstance);
                if (p == null) return "unknown";
                ServerLevel lvl = p.level();
                LivingEntity target = null;
                double minDist = Double.MAX_VALUE;
                AABB scanArea = AABB.ofSize(p.position(), 16, 16, 16);
                for (Entity e : lvl.getEntities(p, scanArea)) {
                    if (!(e instanceof LivingEntity le)) continue;
                    String tn = BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).getPath();
                    if (!isHostile(tn)) continue;
                    double d = e.distanceTo(p);
                    if (d < minDist) { minDist = d; target = le; }
                }
                if (target == null) return "none";
                return BuiltInRegistries.ENTITY_TYPE.getKey(target.getType()).getPath();
            });

            if (dist > 4.0) {
                // 距离太远，走过去
                moveReached = false;
                moveFinalDist = 999;
                moveTicksLeft = 40;
                moveTarget = new double[]{tx, ty, tz};
                long walkStart = System.currentTimeMillis();
                while (moveTarget != null && System.currentTimeMillis() - walkStart < 2000) {
                    if (shouldStop) { shouldStop = false; break; }
                    try { Thread.sleep(50); } catch (InterruptedException e) { break; }
                }
            } else {
                // 在范围内，攻击一次
                Boolean hit = onServer(() -> {
                    ServerPlayer p = getFirstPlayer(serverInstance);
                    if (p == null) return false;
                    ServerLevel lvl = p.level();
                    LivingEntity target = null;
                    double minDist = Double.MAX_VALUE;
                    AABB scanArea = AABB.ofSize(p.position(), 16, 16, 16);
                    for (Entity e : lvl.getEntities(p, scanArea)) {
                        if (!(e instanceof LivingEntity le)) continue;
                        String tn = BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).getPath();
                        if (!isHostile(tn)) continue;
                        double d = e.distanceTo(p);
                        if (d < minDist) { minDist = d; target = le; }
                    }
                    if (target == null || minDist > 5.0) return false;
                    equipBestWeapon(p);
                    p.lookAt(EntityAnchorArgument.Anchor.EYES, target.position().add(0, 1.0, 0));
                    p.attack(target);
                    p.containerMenu.broadcastChanges();
                    return true;
                });
                if (hit != null && hit) hitCount++;
                // 攻击冷却（Minecraft 攻击速度约 0.6s/次 = 12 ticks）
                try { Thread.sleep(300); } catch (InterruptedException e) { break; }
            }
        }

        o.addProperty("status", "ok");
        o.addProperty("hits", hitCount);
        o.addProperty("target", targetType);
        o.addProperty("detail", "attack " + targetType + " hits=" + hitCount);
        return o;
    }

    /** attack_player 在 TCP 线程执行：接近目标玩家 + 攻击。 */
    private JsonObject performAttackPlayer(JsonObject req) {
        JsonObject o = new JsonObject();
        if (serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "服务器未就绪");
            return o;
        }
        String targetName = req.has("player_name") ? req.get("player_name").getAsString() : "";
        int ticks = req.has("ticks") ? req.get("ticks").getAsInt() : 60;

        String targetId = onServer(() -> {
            for (ServerPlayer p : serverInstance.getPlayerList().getPlayers()) {
                if (p.getName().getString().equalsIgnoreCase(targetName)) return p.getUUID().toString();
            }
            return null;
        });
        if (targetId == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "attack_player: player '" + targetName + "' not found");
            return o;
        }

        int hitCount = 0;
        long start = System.currentTimeMillis();
        long deadline = start + ticks * 50L;

        while (System.currentTimeMillis() < deadline) {
            if (shouldStop) { shouldStop = false; break; }

            double[] targetPos = onServer(() -> {
                for (ServerPlayer p : serverInstance.getPlayerList().getPlayers()) {
                    if (p.getUUID().toString().equals(targetId)) {
                        if (p.isRemoved() || p.getHealth() <= 0) return null;
                        return new double[]{p.getX(), p.getY(), p.getZ(), p.getHealth()};
                    }
                }
                return null;
            });
            if (targetPos == null) break;

            double[] myPos = onServer(() -> {
                ServerPlayer p = getFirstPlayer(serverInstance);
                if (p == null) return null;
                return new double[]{p.getX(), p.getY(), p.getZ()};
            });
            if (myPos == null) break;

            double dx = targetPos[0] - myPos[0];
            double dz = targetPos[2] - myPos[2];
            double dist = Math.sqrt(dx * dx + dz * dz);

            if (dist > 4.0) {
                moveReached = false;
                moveFinalDist = 999;
                moveTicksLeft = 40;
                moveTarget = new double[]{targetPos[0], targetPos[1], targetPos[2]};
                long walkStart = System.currentTimeMillis();
                while (moveTarget != null && System.currentTimeMillis() - walkStart < 2000) {
                    if (shouldStop) { shouldStop = false; break; }
                    try { Thread.sleep(50); } catch (InterruptedException e) { break; }
                }
            } else {
                Boolean hit = onServer(() -> {
                    ServerPlayer p = getFirstPlayer(serverInstance);
                    if (p == null) return false;
                    ServerPlayer target = null;
                    for (ServerPlayer pp : serverInstance.getPlayerList().getPlayers()) {
                        if (pp.getUUID().toString().equals(targetId)) { target = pp; break; }
                    }
                    if (target == null || p.distanceTo(target) > 5.0) return false;
                    equipBestWeapon(p);
                    p.setYRot((float) Math.toDegrees(Math.atan2(-(target.getX() - p.getX()), target.getZ() - p.getZ())));
                    p.attack(target);
                    p.containerMenu.broadcastChanges();
                    return true;
                });
                if (hit != null && hit) hitCount++;
                try { Thread.sleep(300); } catch (InterruptedException e) { break; }
            }
        }

        o.addProperty("status", "ok");
        o.addProperty("hits", hitCount);
        o.addProperty("detail", "attack_player " + targetName + " hits=" + hitCount);
        return o;
    }

    /** follow_player 在 TCP 线程执行：持续跟随目标玩家。 */
    private JsonObject performFollowPlayer(JsonObject req) {
        JsonObject o = new JsonObject();
        if (serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "服务器未就绪");
            return o;
        }
        String targetName = req.has("player_name") ? req.get("player_name").getAsString() : "";
        double followDist = req.has("follow_dist") ? req.get("follow_dist").getAsDouble() : 3.0;
        int totalTicks = req.has("ticks") ? req.get("ticks").getAsInt() : 600;

        String targetId = onServer(() -> {
            for (ServerPlayer p : serverInstance.getPlayerList().getPlayers()) {
                if (p.getName().getString().equalsIgnoreCase(targetName)) return p.getUUID().toString();
            }
            return null;
        });
        if (targetId == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "follow_player: player '" + targetName + "' not found");
            return o;
        }

        int followedTicks = 0;
        long start = System.currentTimeMillis();
        long deadline = start + totalTicks * 50L;

        while (System.currentTimeMillis() < deadline && followedTicks < totalTicks) {
            if (shouldStop) { shouldStop = false; break; }

            double[] targetPos = onServer(() -> {
                for (ServerPlayer p : serverInstance.getPlayerList().getPlayers()) {
                    if (p.getUUID().toString().equals(targetId)) {
                        if (p.isRemoved() || !p.isAlive()) return null;
                        return new double[]{p.getX(), p.getY(), p.getZ()};
                    }
                }
                return null;
            });
            if (targetPos == null) break;

            double[] myPos = onServer(() -> {
                ServerPlayer p = getFirstPlayer(serverInstance);
                if (p == null) return null;
                return new double[]{p.getX(), p.getY(), p.getZ()};
            });
            if (myPos == null) break;

            double dx = targetPos[0] - myPos[0];
            double dz = targetPos[2] - myPos[2];
            double dist = Math.sqrt(dx * dx + dz * dz);

            if (dist > followDist + 1.0) {
                // 距离太远，走过去
                moveReached = false;
                moveFinalDist = 999;
                moveTicksLeft = 30;
                moveTarget = new double[]{targetPos[0], targetPos[1], targetPos[2]};
                long walkStart = System.currentTimeMillis();
                while (moveTarget != null && System.currentTimeMillis() - walkStart < 1500) {
                    if (shouldStop) { shouldStop = false; break; }
                    try { Thread.sleep(50); } catch (InterruptedException e) { break; }
                }
            } else {
                // 距离够近，休息一帧
                try { Thread.sleep(100); } catch (InterruptedException e) { break; }
            }
            followedTicks += 2;
        }

        o.addProperty("status", "ok");
        o.addProperty("followed_ticks", followedTicks);
        o.addProperty("detail", "follow_player " + targetName + " for " + followedTicks + " ticks");
        return o;
    }

    /** use_item 在 TCP 线程执行：切物品 → 使用 → 等待。 */
    private JsonObject performUseItem(JsonObject req) {
        JsonObject o = new JsonObject();
        if (serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "服务器未就绪");
            return o;
        }
        int ticks = req.has("ticks") ? req.get("ticks").getAsInt() : 5;

        String itemId = onServer(() -> {
            ServerPlayer p = getFirstPlayer(serverInstance);
            if (p == null) return null;
            ItemStack held = p.getMainHandItem();
            if (held.isEmpty()) return "air";
            var result = p.gameMode.useItem(p, p.level(), held, InteractionHand.MAIN_HAND);
            p.containerMenu.broadcastChanges();
            return BuiltInRegistries.ITEM.getKey(held.getItem()).getPath() + "|" + result.consumesAction();
        });

        if (itemId == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "use_item: no player");
            return o;
        }

        String[] parts = itemId.split("\\|");
        String itemName = parts[0];
        boolean consumed = parts.length > 1 && "true".equals(parts[1]);

        if (consumed && ticks > 1) {
            try { Thread.sleep(ticks * 50L); } catch (InterruptedException e) { /* ignore */ }
        }

        o.addProperty("status", "ok");
        o.addProperty("consumed", consumed);
        o.addProperty("detail", "use_item " + itemName + " (consumed=" + consumed + ")");
        return o;
    }

    /** eat_item 在 TCP 线程执行：找食物 → 切到快捷栏 → 吃 → 等待。 */
    private JsonObject performEatItem(JsonObject req) {
        JsonObject o = new JsonObject();
        if (serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "服务器未就绪");
            return o;
        }
        String itemName = req.has("item") ? req.get("item").getAsString() : "";
        int eatTicks = req.has("ticks") ? req.get("ticks").getAsInt() : 32;

        String result = onServer(() -> {
            ServerPlayer p = getFirstPlayer(serverInstance);
            if (p == null) return "fail|no player";
            Inventory inv = p.getInventory();
            String search = itemName.replace("minecraft:", "").toLowerCase();
            int eatSlot = -1;
            for (int i = 0; i < inv.getContainerSize(); i++) {
                ItemStack s = inv.getItem(i);
                if (s.isEmpty()) continue;
                String key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase();
                if (key.contains(search)) { eatSlot = i; break; }
            }
            if (eatSlot < 0) return "fail|" + itemName + " not found";
            if (eatSlot < 9) {
                inv.setSelectedSlot(eatSlot);
            } else {
                int dst = 0;
                for (int i = 0; i < 9; i++) { if (inv.getItem(i).isEmpty()) { dst = i; break; } }
                ItemStack tmp = inv.getItem(dst);
                inv.setItem(dst, inv.getItem(eatSlot));
                inv.setItem(eatSlot, tmp);
                inv.setSelectedSlot(dst);
            }
            p.containerMenu.broadcastChanges();
            String foodId = BuiltInRegistries.ITEM.getKey(p.getMainHandItem().getItem()).getPath();
            var useResult = p.gameMode.useItem(p, p.level(), p.getMainHandItem(), InteractionHand.MAIN_HAND);
            p.containerMenu.broadcastChanges();
            return "ok|" + foodId + "|" + useResult.consumesAction();
        });

        if (result == null || result.startsWith("fail|")) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "eat_item: " + (result != null ? result.substring(5) : "unknown error"));
            return o;
        }

        String[] parts = result.split("\\|");
        String foodId = parts.length > 1 ? parts[1] : itemName;
        boolean consumed = parts.length > 2 && "true".equals(parts[2]);

        if (consumed) {
            try { Thread.sleep(eatTicks * 50L); } catch (InterruptedException e) { /* ignore */ }
        }

        o.addProperty("status", "ok");
        o.addProperty("consumed", consumed);
        o.addProperty("detail", "eat_item " + foodId + " (consumed=" + consumed + ")");
        return o;
    }

    /** wait 在 TCP 线程执行：纯等待，不阻塞服务端。 */
    private JsonObject performWait(JsonObject req) {
        JsonObject o = new JsonObject();
        int seconds = req.has("seconds") ? req.get("seconds").getAsInt() : 1;
        long start = System.currentTimeMillis();
        long deadline = start + seconds * 1000L;
        while (System.currentTimeMillis() < deadline) {
            if (shouldStop) { shouldStop = false; break; }
            try { Thread.sleep(100); } catch (InterruptedException e) { break; }
        }
        o.addProperty("status", "ok");
        o.addProperty("detail", "wait " + seconds + "s");
        return o;
    }

    /** combat 在 TCP 线程执行：完整战斗循环。 */
    private JsonObject performCombat(JsonObject req) {
        JsonObject o = new JsonObject();
        if (serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "服务器未就绪");
            return o;
        }
        String mode = req.has("mode") ? req.get("mode").getAsString() : "melee";
        int ticks = req.has("ticks") ? req.get("ticks").getAsInt() : 200;

        // 用 onServer 执行一次完整的 combat（短时间内的），但 combat 内部也是循环的
        // 为了不阻塞服务端，我们拆分为多个短周期
        String resultTarget = "none";
        String resultResult = "timeout";
        int totalHits = 0;
        long start = System.currentTimeMillis();
        long deadline = start + ticks * 50L;

        while (System.currentTimeMillis() < deadline) {
            if (shouldStop) { shouldStop = false; resultResult