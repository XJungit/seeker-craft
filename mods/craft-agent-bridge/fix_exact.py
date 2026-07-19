#!/usr/bin/env python3

file_path = r'd:\Craft-Agent\mods\craft-agent-bridge\src\main\java\com\craftagent\bridge\CraftAgentBridge.java'

with open(file_path, 'r', encoding='utf-8') as f:
    lines = f.readlines()

# 删除第 120-140 行（0-indexed: 120-140）
# 这些行是残留的 onServerTick 方法体
new_lines = lines[:120] + lines[141:]

with open(file_path, 'w', encoding='utf-8') as f:
    f.writelines(new_lines)

print("Done!")
