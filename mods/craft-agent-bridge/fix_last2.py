#!/usr/bin/env python3

file_path = r'd:\Craft-Agent\mods\craft-agent-bridge\src\main\java\com\craftagent\bridge\CraftAgentBridge.java'

with open(file_path, 'r', encoding='utf-8') as f:
    lines = f.readlines()

# 删除残留的 onServerTick 方法体
new_lines = []
skip = False
for i, line in enumerate(lines):
    if not skip and '服务端每 tick 调用' in line and '处理移动目标' in line:
        skip = True
        continue
    if skip:
        if line.strip() == '}':
            skip = False
            new_lines.append(line)
        continue
    new_lines.append(line)

with open(file_path, 'w', encoding='utf-8') as f:
    f.writelines(new_lines)

print("Done!")
