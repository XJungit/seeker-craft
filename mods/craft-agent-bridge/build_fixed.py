#!/usr/bin/env python3

# 从原始文件读取内容
original_path = r'd:\Craft-Agent\mods\craft-agent-bridge\CraftAgentBridge_original.java'
target_path = r'd:\Craft-Agent\mods\craft-agent-bridge\src\main\java\com\craftagent\bridge\CraftAgentBridge.java'

with open(original_path, 'r', encoding='utf-8') as f:
    lines = f.readlines()

# 修改 1: import DedicatedServerModInitializer -> ModInitializer
for i, line in enumerate(lines):
    if 'import net.fabricmc.api.DedicatedServerModInitializer;' in line:
        lines[i] = 'import net.fabricmc.api.ModInitializer;\n'
    if 'public class CraftAgentBridge implements DedicatedServerModInitializer {' in line:
        lines[i] = 'public class CraftAgentBridge implements ModInitializer {\n'

# 添加新 import (在 ServerLifecycleEvents 之后)
for i, line in enumerate(lines):
    if 'import net.fabricmc.fabric.api.event.lifecycle.v1.ServerLifecycleEvents;' in line:
        lines.insert(i + 1, 'import net.minecraft.world.entity.MoverType;\n')
        lines.insert(i + 2, 'import net.minecraft.world.phys.Vec3;\n')
        break

# 修改 2: onInitializeServer -> onInitialize
for i, line in enumerate(lines):
    if '    public void onInitializeServer() {' in line:
        lines[i] = '    @Override\n    public void onInitialize() {\n'
    if '        // 服务器启动时保存 serverInstance（onInitializeServer 没有 server 参数）' in line:
        lines[i] = '        // 服务器启动时保存 serverInstance\n'

# 修改 3: 删除 onServerTick 方法
new_lines = []
skip = False
brace_depth = 0
for i, line in enumerate(lines):
    if 'private void onServerTick(MinecraftServer server)' in line:
        skip = True
        brace_depth = 0
        continue
    if skip:
        brace_depth += line.count('{') - line.count('}')
        if brace_depth <= 0:
            skip = False
        continue
    new_lines.append(line)
lines = new_lines

# 删除 ServerTickEvents.START_SERVER_TICK 注册
new_lines = []
for i, line in enumerate(lines):
    if 'ServerTickEvents.START_SERVER_TICK.register(this::onServerTick);' in line:
        continue
    if 'System.out.println("[craft-agent-bridge] ServerTickEvents 已注册");' in line:
        continue
    if '        // 服务端 tick：处理移动（setDeltaMovement + 朝向）\n' in line:
        continue
    new_lines.append(line)
lines = new_lines

# 修改 4: 在 onInitialize() 结束后插入 scheduleMoveTick() 方法
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

# 找 onInitialize() 的闭合 }
insert_idx = -1
for i in range(len(lines) - 1, -1, -1):
    if lines[i].strip() == '}':
        # 确认这是 onInitialize() 的 }
        for j in range(i - 1, max(0, i - 30), -1):
            if 'onInitialize' in lines[j]:
                insert_idx = i
                break
        if insert_idx >= 0:
            break

if insert_idx >= 0:
    lines.insert(insert_idx, schedule_method)

# 修改 5: 在 6 处 moveTarget 赋值后添加 scheduleMoveTick()
targets_to_update = [
    'moveTarget = new double[]{tx, ty, tz};',
    'moveTarget = new double[]{target.getX(), target.getY(), target.getZ()};',
    'moveTarget = new double[]{nearest.getX(), nearest.getY(), nearest.getZ()};',
    'moveTarget = new double[]{startX + awayDx, startY, startZ + awayDz};',
    'moveTarget = new double[]{startX, startY, startZ};'
]

new_lines = []
for i, line in enumerate(lines):
    new_lines.append(line)
    stripped = line.strip()
    for t in targets_to_update:
        if stripped == t:
            indent = line[:len(line) - len(line.lstrip())]
            new_lines.append(indent + 'scheduleMoveTick();')
            break

lines = new_lines

with open(target_path, 'w', encoding='utf-8') as f:
    f.writelines(lines)

print("Done!")
