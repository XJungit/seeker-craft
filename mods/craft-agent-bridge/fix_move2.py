#!/usr/bin/env python3

file_path = r'd:\Craft-Agent\mods\craft-agent-bridge\src\main\java\com\craftagent\bridge\CraftAgentBridge.java'

with open(file_path, 'r', encoding='utf-8') as f:
    content = f.read()

# 删除 ServerTickEvents.START_SERVER_TICK 注册
content = content.replace(
    '        // 服务端 tick：处理移动（setDeltaMovement + 朝向）\n        ServerTickEvents.START_SERVER_TICK.register(this::onServerTick);\n        System.out.println("[craft-agent-bridge] ServerTickEvents 已注册");\n',
    ''
)

# 删除 onServerTick 方法
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

with open(file_path, 'w', encoding='utf-8') as f:
    f.write(content)

print("Done!")
