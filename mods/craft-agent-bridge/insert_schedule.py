#!/usr/bin/env python3

file_path = r'd:\Craft-Agent\mods\craft-agent-bridge\src\main\java\com\craftagent\bridge\CraftAgentBridge.java'

with open(file_path, 'r', encoding='utf-8') as f:
    lines = f.readlines()

# 在 onInitialize() 的 } 后插入 scheduleMoveTick() 方法
schedule_method = '''
    /** 递归调度移动：每 tick 在服务端线程执行一次 move()，直到到达或超时。 */
    private void scheduleMoveTick() {
        MinecraftServer server = serverInstance;
        if (server == null) return;
        server.executeIfPossible(() -> {
            if (moveTarget == null) return;
            ServerPlayer player = getFirstPlayer(server);
            if (player == null) { moveTarget = null; return; }

            double tx = moveTarget[0], tz = moveTarget[2];
            double ddx = tx - player.getX(), ddz = tz - player.getZ();
            double horiz = Math.sqrt(ddx * ddx + ddz * ddz);
            moveFinalDist = horiz;
            moveTicksLeft--;

            if (horiz < 0.8 || moveTicksLeft <= 0) {
                moveReached = horiz < 0.8;
                moveTarget = null;
                player.zza = 0;
                player.xxa = 0;
                return;
            }

            float yaw = (float) Math.toDegrees(Math.atan2(-ddx, ddz));
            player.setYRot(yaw);

            double step = 0.3;
            double ratio = step / horiz;
            double dx = ddx * ratio;
            double dz = ddz * ratio;
            
            player.move(MoverType.SELF, new Vec3(dx, 0, dz));
            player.setPosRaw(player.getX(), player.getY(), player.getZ());
            System.out.println("[move] tick x=" + player.getX() + " z=" + player.getZ() + " horiz=" + horiz);

            player.zza = 1.0f;
            player.xxa = 0.0f;

            // 继续下一个 tick
            scheduleMoveTick();
        });
    }
'''

# 找 onInitialize() 的闭合 }（第 119 行）
insert_idx = -1
for i in range(len(lines) - 1, -1, -1):
    if lines[i].strip() == '}':
        # 确认这是 onInitialize() 的 }
        for j in range(i - 1, max(0, i - 30), -1):
            if 'onInitialize' in lines[j]:
                insert_idx = i + 1
                break
        if insert_idx >= 0:
            break

if insert_idx >= 0:
    lines.insert(insert_idx, schedule_method)

with open(file_path, 'w', encoding='utf-8') as f:
    f.writelines(lines)

print("Done!")
