#!/usr/bin/env python3

file_path = r'd:\Craft-Agent\mods\craft-agent-bridge\src\main\java\com\craftagent\bridge\CraftAgentBridge.java'

with open(file_path, 'r', encoding='utf-8') as f:
    content = f.read()

# 删除重复的 @Override
content = content.replace('    @Override\n    @Override\n    public void onInitialize() {', '    @Override\n    public void onInitialize() {')

# 删除残留的 onServerTick 方法体
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

# 修复 onInitialize() 和 scheduleMoveTick() 的括号问题
lines = content.split('\n')
new_lines = []
i = 0
while i < len(lines):
    line = lines[i]
    # 在 scheduleMoveTick() 方法之前，确保 onInitialize() 已经闭合
    if 'private void scheduleMoveTick()' in line:
        # 向前找最近的 }，如果它前面不是 onInitialize() 的闭合，插入 }
        # 实际上，我们只需要确保在 scheduleMoveTick 之前有一个 } 闭合 onInitialize
        # 向前检查最近的非空行
        j = len(new_lines) - 1
        while j >= 0 and new_lines[j].strip() == '':
            j -= 1
        if j >= 0 and 'public void onInitialize()' not in new_lines[j]:
            # 需要插入 } 来闭合 onInitialize
            new_lines.append('    }')
    new_lines.append(line)
    i += 1

content = '\n'.join(new_lines)

with open(file_path, 'w', encoding='utf-8') as f:
    f.write(content)

print("Done!")
