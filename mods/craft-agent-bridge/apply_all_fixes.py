#!/usr/bin/env python3
"""
从原始文件开始，应用所有修改：
1. entrypoint: DedicatedServerModInitializer -> ModInitializer
2. onInitializeServer -> onInitialize
3. 删除 onServerTick 方法和 ServerTickEvents.START_SERVER_TICK 注册
4. 新增 scheduleMoveTick() 方法
5. 在 6 处 moveTarget 赋值后添加 scheduleMoveTick() 调用
"""

file_path = r'd:\Craft-Agent\mods\craft-agent-bridge\src\main\java\com\craftagent\bridge\CraftAgentBridge.java'

with open(file_path, 'r', encoding='utf-8') as f:
    content = f.read()

# 1) entrypoint: DedicatedServerModInitializer -> ModInitializer
content = content.replace(
    'import net.fabricmc.api.DedicatedServerModInitializer;',
    'import net.fabricmc.api.ModInitializer;'
)
content = content.replace(
    'public class CraftAgentBridge implements DedicatedServerModInitializer {',
    'public class CraftAgentBridge implements ModInitializer {'
)

# 添加新 import
content = content.replace(
    'import net.fabricmc.fabric.api.event.lifecycle.v1.ServerLifecycleEvents;\nimport net.minecraft.server.MinecraftServer;',
    'import net.fabricmc.fabric.api.event.lifecycle.v1.ServerLifecycleEvents;\nimport net.minecraft.world.entity.MoverType;\nimport net.minecraft.world.phys.Vec3;\nimport net.minecraft.server.MinecraftServer;'
)

# 2) onInitializeServer -> onInitialize
content = content.replace('    public void onInitializeServer() {\n', '    @Override\n    public void onInitialize() {\n')
content = content.replace(
    '        // 服务器启动时保存 serverInstance（onInitializeServer 没有 server 参数）\n',
    '        // 服务器启动时保存 serverInstance\n'
)

# 3) 删除 onServerTick 方法和 ServerTickEvents.START_SERVER_TICK 注册
lines = content.split('\n')
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
content = '\n'.join(new_lines)

# 删除 ServerTickEvents.START_SERVER_TICK 注册
content = content.replace(
    '        // 服务端 tick：处理移动（setDeltaMovement + 朝向）\n        ServerTickEvents.START_SERVER_TICK.register(this::onServerTick);\n        System.out.println("[craft-agent-bridge] ServerTickEvents 已注册");\n',
    ''
)

# 4) 在 onInitialize() 的 } 前插入 scheduleMoveTick() 方法
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

lines = content.split('\n')
# 找 onInitialize() 的闭合 }
for i in range(len(lines) - 1, -1, -1):
    if lines[i].strip() == '}':
        # 确认这是 onInitialize() 的 }
        for j in range(i - 1, max(0, i - 30), -1):
            if 'onInitialize' in lines[j]:
                # 在 } 前插入 scheduleMoveTick()
                lines.insert(i, schedule_method)
                break
        break

content = '\n'.join(lines)

# 5) 在 6 处 moveTarget = new double[]{...} 后添加 scheduleMoveTick()
targets_to_update = [
    'moveTarget = new double[]{tx, ty, tz};',
    'moveTarget = new double[]{target.getX(), target.getY(), target.getZ()};',
    'moveTarget = new double[]{nearest.getX(), nearest.getY(), nearest.getZ()};',
    'moveTarget = new double[]{startX + awayDx, startY, startZ + awayDz};',
    'moveTarget = new double[]{startX, startY, startZ};'
]

lines = content.split('\n')
new_lines = []
for i, line in enumerate(lines):
    new_lines.append(line)
    stripped = line.strip()
    for t in targets_to_update:
        if stripped == t:
            indent = line[:len(line) - len(line.lstrip())]
            new_lines.append(indent + 'scheduleMoveTick();')
            break

content = '\n'.join(new_lines)

with open(file_path, 'w', encoding='utf-8') as f:
    f.write(content)

print("Done!")
