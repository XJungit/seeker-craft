#!/usr/bin/env python3

file_path = r'd:\Craft-Agent\mods\craft-agent-bridge\src\main\java\com\craftagent\bridge\CraftAgentBridge.java'

with open(file_path, 'r', encoding='utf-8') as f:
    content = f.read()

# 修复 1: 删除多余的 }（在 scheduleMoveTick 注释前）
# 从 "    /** 递归调度移动..." 到 "    private void scheduleMoveTick() {" 之间有一个多余的 }
content = content.replace(
    '\n\n    /** 递归调度移动：每 tick 在服务端线程执行一次 move()，直到到达或超时。 */\n    }\n    private void scheduleMoveTick() {',
    '\n\n    /** 递归调度移动：每 tick 在服务端线程执行一次 move()，直到到达或超时。 */\n    private void scheduleMoveTick() {'
)

# 修复 2: 删除残留的 onServerTick 方法体
# 从 "/** 服务端每 tick 调用..." 到 "    }"（方法结束）
lines = content.split('\n')
new_lines = []
skip = False
for i, line in enumerate(lines):
    if not skip and '/** 服务端每 tick 调用：处理移动目标。 */' in line:
        skip = True
        continue
    if skip:
        if line.strip() == '}':
            skip = False
            new_lines.append(line)
        continue
    new_lines.append(line)
content = '\n'.join(new_lines)

with open(file_path, 'w', encoding='utf-8') as f:
    f.write(content)

print("Done!")
