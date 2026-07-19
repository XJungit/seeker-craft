with open(r'd:\Craft-Agent\mods\craft-agent-bridge\src\main\java\com\craftagent\bridge\CraftAgentBridge.java', 'r', encoding='utf-8') as f:
    content = f.read()

# 在 performCollectItems 方法后面插入所有 B 类 action 的 TCP 线程版本
insert_after = '        return o;\n    }\n\n    // ══════════════════════════════════════════════════════════════\n    // 状态查询'

new_methods = '''        return o;
    }

    /** attack_player 在 TCP 线程执行：onServer 单次执行，循环+等待在 TCP 线程 */
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
        long timeout = ticks * 50L;
        int attackCooldown = 0;

        while (System.currentTimeMillis() - start < timeout) {
            if (shouldStop) { shouldStop = false; break; }

            double[] info = onServer(() -> {
                ServerPlayer p = getFirstPlayer(serverInstance);
                if (p == null) return null;
                ServerPlayer target = null;
                for (ServerPlayer pp : serverInstance.getPlayerList().getPlayers()) {
                    if (pp.getUUID().toString().equals(targetId)) { target = pp; break; }
                }
                if (target == null || !target.isAlive()) return null;
                double dist = p.distanceTo(target);
                return new double[]{target.getX(), target.getY(), target.getZ(), dist, target.getHealth()};
            });

            if (info == null) break;

            double tx = info[0], ty = info[1], tz = info[2], dist = info[3];

            if (dist > 4.5) {
                // 距离太远，移动靠近
                moveReached = false;
                moveFinalDist = 999;
                moveStuck = false;
                moveTicksLeft = 40;
                moveTarget = new double[]{tx, ty, tz};
                long moveStart = System.currentTimeMillis();
                while (moveTarget != null && System.currentTimeMillis() - moveStart < 2000) {
                    if (shouldStop) break;
                    try { Thread.sleep(50); } catch (InterruptedException e) { break; }
                }
            } else {
                // 在范围内，攻击（带冷却）
                if (attackCooldown <= 0) {
                    final boolean[] hit = {false};
                    onServer(() -> {
                        ServerPlayer p = getFirstPlayer(serverInstance);
                        if (p == null) return null;
                        ServerPlayer target = null;
                        for (ServerPlayer pp : serverInstance.getPlayerList().getPlayers()) {
                            if (pp.getUUID().toString().equals(targetId)) { target = pp; break; }
                        }
                        if (target != null && target.isAlive() && p.distanceTo(target) <= 5.0) {
                            equipBestWeapon(p);
                            p.setYRot((float) Math.toDegrees(Math.atan2(-(target.getX() - p.getX()), target.getZ() - p.getZ())));
                            p.attack(target);
                            p.containerMenu.broadcastChanges();
                            hit[0] = true;
                        }
                        return null;
                    });
                    if (hit[0]) hitCount++;
                    attackCooldown = 10;
                } else {
                    attackCooldown--;
                }
                try { Thread.sleep(50); } catch (InterruptedException e) { break; }
            }
        }

        moveTarget = null;

        o.addProperty("status", "ok");
        o.addProperty("hits", hitCount);
        o.addProperty("detail", "attack_player " + targetName + " hits=" + hitCount);
        return o;
    }

    /** follow_player 在 TCP 线程执行：循环更新目标位置并移动 */
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

        long start = System.currentTimeMillis();
        long timeout = totalTicks * 50L;
        int followTicks = 0;

        while (System.currentTimeMillis() - start < timeout) {
            if (shouldStop) { shouldStop = false; break; }

            double[] targetPos = onServer(() -> {
                for (ServerPlayer p : serverInstance.getPlayerList().getPlayers()) {
                    if (p.getUUID().toString().equals(targetId)) {
                        if (!p.isAlive()) return null;
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

            if (dist > followDist + 0.5) {
                moveReached = false;
                moveFinalDist = 999;
                moveStuck = false;
                moveTicksLeft = 30;
                moveTarget = targetPos.clone();
                long moveStart = System.currentTimeMillis();
                while (moveTarget != null && System.currentTimeMillis() - moveStart < 1500) {
                    if (shouldStop) break;
                    try { Thread.sleep(50); } catch (InterruptedException e) { break; }
                }
            } else if (dist < followDist - 0.5) {
                double backX = myPos[0] - dx / dist * 2.0;
                double backZ = myPos[2] - dz / dist * 2.0;
                moveReached = false;
                moveFinalDist = 999;
                moveStuck = false;
                moveTicksLeft = 20;
                moveTarget = new double[]{backX, myPos[1], backZ};
                long moveStart = System.currentTimeMillis();
                while (moveTarget != null && System.currentTimeMillis() - moveStart < 1000) {
                    if (shouldStop) break;
                    try { Thread.sleep(50); } catch (InterruptedException e) { break; }
                }
            } else {
                try { Thread.sleep(100); } catch (InterruptedException e) { break; }
            }
            followTicks++;
        }

        moveTarget = null;
        o.addProperty("status", "ok");
        o.addProperty("followed_ticks", followTicks);
        o.addProperty("detail", "follow_player " + targetName + " for " + followTicks + " ticks");
        return o;
    }

    /** combat 在 TCP 线程执行：循环找目标+决策，单次动作在 onServer 执行 */
    private JsonObject performCombat(JsonObject req) {
        JsonObject o = new JsonObject();
        if (serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "服务器未就绪");
            return o;
        }
        String mode = req.has("mode") ? req.get("mode").getAsString() : "melee";
        int maxTicks = req.has("ticks") ? req.get("ticks").getAsInt() : 200;

        String result = "none";
        String targetType = "";
        long start = System.currentTimeMillis();
        long timeout = maxTicks * 50L;
        int attackCooldown = 0;

        while (System.currentTimeMillis() - start < timeout) {
            if (shouldStop) { shouldStop = false; break; }

            final String[] tType = {""};
            double[] info = onServer(() -> {
                ServerPlayer p = getFirstPlayer(serverInstance);
                if (p == null) return null;
                ServerLevel lvl = p.level();
                LivingEntity target = null;
                double minDist = Double.MAX_VALUE;
                AABB scanArea = AABB.ofSize(p.position(), 32, 32, 32);
                for (Entity e : lvl.getEntities(p, scanArea)) {
                    if (!(e instanceof LivingEntity le)) continue;
                    String tn = BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).getPath();
                    if (!isHostile(tn)) continue;
                    double d = e.distanceTo(p);
                    if (d < minDist) { minDist = d; target = le; }
                }
                if (target == null) return null;
                tType[0] = BuiltInRegistries.ENTITY_TYPE.getKey(target.getType()).getPath();
                return new double[]{target.getX(), target.getY(), target.getZ(), minDist, target.getHealth(), p.getHealth()};
            });

            if (info == null) { result = "no_target"; break; }
            targetType = tType[0];
            double tx = info[0], ty = info[1], tz = info[2], dist = info[3], pHp = info[5];

            // 濒死撤退
            if (pHp < 5.0f) {
                result = "retreated";
                double[] myPos = onServer(() -> {
                    ServerPlayer p = getFirstPlayer(serverInstance);
                    if (p == null) return null;
                    return new double[]{p.getX(), p.getY(), p.getZ()};
                });
                if (myPos != null) {
                    double dx = myPos[0] - tx;
                    double dz = myPos[2] - tz;
                    double len = Math.sqrt(dx*dx + dz*dz);
                    if (len > 0) {
                        moveReached = false;
                        moveFinalDist = 999;
                        moveTicksLeft = 100;
                        moveTarget = new double[]{myPos[0] + dx/len * 15.0, myPos[1], myPos[2] + dz/len * 15.0};
                        long moveStart = System.currentTimeMillis();
                        while (moveTarget != null && System.currentTimeMillis() - moveStart < 5000) {
                            if (shouldStop) break;
                            try { Thread.sleep(50); } catch (InterruptedException e) { break; }
                        }
                    }
                }
                break;
            }

            boolean isCreeper = targetType.contains("creeper");
            if (isCreeper && dist < 6.0 && !mode.equals("retreat")) {
                double[] myPos = onServer(() -> {
                    ServerPlayer p = getFirstPlayer(serverInstance);
                    if (p == null) return null;
                    return new double[]{p.getX(), p.getY(), p.getZ()};
                });
                if (myPos != null) {
                    double dx = myPos[0] - tx;
                    double dz = myPos[2] - tz;
                    double len = Math.sqrt(dx*dx + dz*dz);
                    if (len > 0) {
                        moveReached = false;
                        moveFinalDist = 999;
                        moveTicksLeft = 30;
                        moveTarget = new double[]{myPos[0] + dx/len * 8.0, myPos[1], myPos[2] + dz/len * 8.0};
                        long moveStart = System.currentTimeMillis();
                        while (moveTarget != null && System.currentTimeMillis() - moveStart < 1500) {
                            try { Thread.sleep(50); } catch (InterruptedException e) { break; }
                        }
                    }
                }
                continue;
            }

            if (mode.equals("retreat")) {
                double[] myPos = onServer(() -> {
                    ServerPlayer p = getFirstPlayer(serverInstance);
                    if (p == null) return null;
                    return new double[]{p.getX(), p.getY(), p.getZ()};
                });
                if (myPos != null) {
                    double dx = myPos[0] - tx;
                    double dz = myPos[2] - tz;
                    double len = Math.sqrt(dx*dx + dz*dz);
                    if (len > 0 && dist < 15.0) {
                        moveReached = false;
                        moveFinalDist = 999;
                        moveTicksLeft = 50;
                        moveTarget = new double[]{myPos[0] + dx/len * 18.0, myPos[1], myPos[2] + dz/len * 18.0};
                        long moveStart = System.currentTimeMillis();
                        while (moveTarget != null && System.currentTimeMillis() - moveStart < 2500) {
                            if (shouldStop) break;
                            try { Thread.sleep(50); } catch (InterruptedException e) { break; }
                        }
                    }
                }
                if (dist > 15.0) { result = "retreated"; break; }
                continue;
            }

            if (dist > 4.0) {
                moveReached = false;
                moveFinalDist = 999;
                moveStuck = false;
                moveTicksLeft = 30;
                moveTarget = new double[]{tx, ty, tz};
                long moveStart = System.currentTimeMillis();
                while (moveTarget != null && System.currentTimeMillis() - moveStart < 1500) {
                    if (shouldStop) break;
                    try { Thread.sleep(50); } catch (InterruptedException e) { break; }
                }
            } else {
                if (attackCooldown <= 0) {
                    final boolean[] killed = {false};
                    onServer(() -> {
                        ServerPlayer p = getFirstPlayer(serverInstance);
                        if (p == null) return null;
                        ServerLevel lvl = p.level();
                        LivingEntity target = null;
                        double minDist = Double.MAX_VALUE;
                        AABB scanArea = AABB.ofSize(p.position(), 10, 10, 10);
                        for (Entity e : lvl.getEntities(p, scanArea)) {
                            if (!(e instanceof LivingEntity le)) continue;
                            String tn = BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).getPath();
                            if (!isHostile(tn)) continue;
                            double d = e.distanceTo(p);
                            if (d < minDist) { minDist = d; target = le; }
                        }
                        if (target != null && minDist <= 5.0) {
                            equipBestWeapon(p);
                            p.lookAt(EntityAnchorArgument.Anchor.EYES, target.position().add(0, 1.0, 0));
                            p.attack(target);
                            p.containerMenu.broadcastChanges();
                            if (!target.isAlive()) killed[0] = true;
                        }
                        return null;
                    });
                    if (killed[0]) { result = "killed"; break; }
                    attackCooldown = 10;
                } else {
                    attackCooldown--;
                }

                if (mode.equals("kite") && attackCooldown > 5) {
                    double[] myPos = onServer(() -> {
                        ServerPlayer p = getFirstPlayer(serverInstance);
                        if (p == null) return null;
                        return new double[]{p.getX(), p.getY(), p.getZ()};
                    });
                    if (myPos != null) {
                        double dx = myPos[0] - tx;
                        double dz = myPos[2] - tz;
                        double len = Math.sqrt(dx*dx + dz*dz);
                        if (len > 0 && dist < 6.0) {
                            moveReached = false;
                            moveFinalDist = 999;
                            moveTicksLeft = 15;
                            moveTarget = new double[]{myPos[0] + dx/len * 8.0, myPos[1], myPos[2] + dz/len * 8.0};
                            long moveStart = System.currentTimeMillis();
                            while (moveTarget != null && System.currentTimeMillis() - moveStart < 800) {
                                try { Thread.sleep(50); } catch (InterruptedException e) { break; }
                            }
                        }
                    }
                }

                try { Thread.sleep(50); } catch (InterruptedException e) { break; }
            }
        }

        moveTarget = null;
        if (result.equals("none")) result = "timeout";
        o.addProperty("status", "ok");
        o.addProperty("result", result);
        o.addProperty("target", targetType);
        o.addProperty("detail", "combat mode=" + mode + " -> " + result + " (target=" + targetType + ")");
        return o;
    }

    /** use_item 在 TCP 线程执行：useItem 在 onServer，等待在 TCP 线程 */
    private JsonObject performUseItem(JsonObject req) {
        JsonObject o = new JsonObject();
        if (serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "服务器未就绪");
            return o;
        }
        int ticks = req.has("ticks") ? req.get("ticks").getAsInt() : 5;

        final boolean[] consumed = {false};
        final String[] itemId = {""};
        onServer(() -> {
            ServerPlayer p = getFirstPlayer(serverInstance);
            if (p == null) return null;
            ItemStack held = p.getMainHandItem();
            if (held.isEmpty()) return null;
            itemId[0] = BuiltInRegistries.ITEM.getKey(held.getItem()).getPath();
            var result = p.gameMode.useItem(p, p.level(), held, InteractionHand.MAIN_HAND);
            consumed[0] = result.consumesAction();
            p.containerMenu.broadcastChanges();
            return null;
        });

        if (itemId[0].isEmpty()) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "use_item: main hand empty");
            return o;
        }

        if (consumed[0] && ticks > 1) {
            try { Thread.sleep(ticks * 50L); } catch (InterruptedException e) { /* ignore */ }
        }

        o.addProperty("status", "ok");
        o.addProperty("consumed", consumed[0]);
        o.addProperty("detail", "use_item " + itemId[0] + " (consumed=" + consumed[0] + ")");
        return o;
    }

    /** eat_item 在 TCP 线程执行：找物品+切换在 onServer，等待在 TCP 线程 */
    private JsonObject performEatItem(JsonObject req) {
        JsonObject o = new JsonObject();
        if (serverInstance == null) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "服务器未就绪");
            return o;
        }
        String itemName = req.has("item") ? req.get("item").getAsString() : "";
        int eatTicks = req.has("ticks") ? req.get("ticks").getAsInt() : 32;
        String search = itemName.replace("minecraft:", "").toLowerCase();

        final boolean[] found = {false};
        final boolean[] consumed = {false};
        onServer(() -> {
            ServerPlayer p = getFirstPlayer(serverInstance);
            if (p == null) return null;
            Inventory inv = p.getInventory();
            int eatSlot = -1;
            for (int i = 0; i < inv.getContainerSize(); i++) {
                ItemStack s = inv.getItem(i);
                if (s.isEmpty()) continue;
                String key = BuiltInRegistries.ITEM.getKey(s.getItem()).toString().toLowerCase();
                if (key.contains(search)) { eatSlot = i; break; }
            }
            if (eatSlot < 0) return null;
            found[0] = true;
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
            var result = p.gameMode.useItem(p, p.level(), p.getMainHandItem(), InteractionHand.MAIN_HAND);
            consumed[0] = result.consumesAction();
            p.containerMenu.broadcastChanges();
            return null;
        });

        if (!found[0]) {
            o.addProperty("status", "fail");
            o.addProperty("detail", "eat_item: " + itemName + " not found");
            return o;
        }

        if (consumed[0]) {
            try { Thread.sleep(eatTicks * 50L); } catch (InterruptedException e) { /* ignore */ }
            onServer(() -> {
                ServerPlayer p = getFirstPlayer(serverInstance);
                if (p != null) p.containerMenu.broadcastChanges();
                return null;
            });
        }

        o.addProperty("status", "ok");
        o.addProperty("consumed", consumed[0]);
        o.addProperty("detail", "eat_item " + itemName + " (consumed=" + consumed[0] + ")");
        return o;
    }

    /** wait 在 TCP 线程执行：纯 sleep，不占服务端线程 */
    private JsonObject performWait(JsonObject req) {
        JsonObject o = new JsonObject();
        int seconds = req.has("seconds") ? req.get("seconds").getAsInt() : 1;
        try { Thread.sleep(seconds * 1000L); } catch (InterruptedException e) { /* ignore */ }
        o.addProperty("status", "ok");
        o.addProperty("detail", "wait " + seconds + "s");
        return o;
    }

    // ══════════════════════════════════════════════════════════════
    // 状态查询'''

content = content.replace(insert_after, new_methods)

with open(r'd:\Craft-Agent\mods\craft-agent-bridge\src\main\java\com\craftagent\bridge\CraftAgentBridge.java', 'w', encoding='utf-8') as f:
    f.write(content)

print('B-class action methods added')
