#!/usr/bin/env python3

file_path = r'd:\Craft-Agent\mods\craft-agent-bridge\src\main\java\com\craftagent\bridge\CraftAgentBridge.java'

with open(file_path, 'r', encoding='utf-8') as f:
    lines = f.readlines()

# 修复 1: 删除重复的 @Override（第 102 行，索引 101）
# 查找 "    @Override" 后面紧跟另一个 "    @Override" 的情况
new_lines = []
prev_line = None
for i, line in enumerate(lines):
    if prev_line is not None and prev_line.strip() == '@Override' and line.strip() == '@Override':
        # 跳过第二个 @Override
        continue
    new_lines.append(line)
    prev_line = line.rstrip('\n')

lines = new_lines

# 修复 2: 删除多余的 }（在 scheduleMoveTick 注释前）
# 找 "    /** 递归调度移动..." 前的一个 `}`
new_lines = []
for i, line in enumerate(lines):
    if line.strip() == '}' and i + 1 < len(lines) and '/** 递归调度移动' in lines[i + 1]:
        # 检查前一行是否是 scheduleMoveTick 的注释
        if i > 0 and '/** 递归调度移动' not in lines[i - 1]:
            # 这是多余的 }，跳过
            continue
    new_lines.append(line)

lines = new_lines

# 修复 3: 删除残留的 onServerTick 方法体
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
