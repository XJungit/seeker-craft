#!/usr/bin/env python3

file_path = r'd:\Craft-Agent\mods\craft-agent-bridge\src\main\java\com\craftagent\bridge\CraftAgentBridge.java'

with open(file_path, 'r', encoding='utf-8') as f:
    lines = f.readlines()

# 删除残留的 onServerTick 方法体
# 从 "    }"（onInitialize 后的多余 }）到 "    }"（方法结束）
new_lines = []
skip = False
brace_depth = 0
for i, line in enumerate(lines):
    if not skip and line.strip() == '}' and i + 1 < len(lines) and '// 设置朝向' in lines[i + 1]:
        skip = True
        brace_depth = 0
        continue
    if skip:
        brace_depth += line.count('{') - line.count('}')
        if brace_depth <= 0 and line.strip() == '}':
            skip = False
            new_lines.append(line)
        continue
    new_lines.append(line)

with open(file_path, 'w', encoding='utf-8') as f:
    f.writelines(new_lines)

print("Done!")
