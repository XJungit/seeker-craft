#!/usr/bin/env python3

file_path = r'd:\Craft-Agent\mods\craft-agent-bridge\src\main\java\com\craftagent\bridge\CraftAgentBridge.java'

with open(file_path, 'r', encoding='utf-8') as f:
    lines = f.readlines()

# 修复 1: 删除重复的 @Override
new_lines = []
prev_line = None
for i, line in enumerate(lines):
    if prev_line is not None and prev_line.strip() == '@Override' and line.strip() == '@Override':
        # 跳过第二个 @Override
        continue
    new_lines.append(line)
    prev_line = line.rstrip('\n')

lines = new_lines

# 修复 2: 删除残留的 onServerTick 方法体（从 "/** 服务端每 tick 调用..." 到 "}"）
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

with open(file_path, 'w', encoding='utf-8') as f:
    f.writelines(new_lines)

print("Done!")
