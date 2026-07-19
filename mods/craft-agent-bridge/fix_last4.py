#!/usr/bin/env python3

file_path = r'd:\Craft-Agent\mods\craft-agent-bridge\src\main\java\com\craftagent\bridge\CraftAgentBridge.java'

with open(file_path, 'r', encoding='utf-8') as f:
    lines = f.readlines()

# 删除残留的 onServerTick 方法体
# 从第 121 行的 "}" 开始，到第 140 行的 "}" 结束
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
        if brace_depth <= 0 and 'moveStuck = false;' in line:
            # 下一行应该是 "}"，跳过它
            skip = False
            continue
        if brace_depth <= 0 and line.strip() == '}':
            skip = False
            new_lines.append(line)
        continue
    new_lines.append(line)

with open(file_path, 'w', encoding='utf-8') as f:
    f.writelines(new_lines)

print("Done!")
