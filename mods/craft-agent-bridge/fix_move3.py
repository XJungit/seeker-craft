#!/usr/bin/env python3

file_path = r'd:\Craft-Agent\mods\craft-agent-bridge\src\main\java\com\craftagent\bridge\CraftAgentBridge.java'

with open(file_path, 'r', encoding='utf-8') as f:
    content = f.read()

# 修复 1: 删除残留的 onServerTick 方法体（从 "ServerPlayer player = getFirstPlayer(server);" 到 "}"）
# 这个残留方法体在 scheduleMoveTick 之后
lines = content.split('\n')
new_lines = []
skip = False
for i, line in enumerate(lines):
    # 找到残留方法体的开始（scheduleMoveTick 结束后，getFirstPlayer 之前）
    if not skip and i > 0 and 'scheduleMoveTick()' in lines[i-1] and 'ServerPlayer player = getFirstPlayer(server);' in line:
        skip = True
        continue
    if skip:
        # 跳过直到找到独立的 }（方法结束）
        if line.strip() == '}' and i + 1 < len(lines) and lines[i+1].strip().startswith('//'):
            skip = False
            new_lines.append(line)
        continue
    new_lines.append(line)

content = '\n'.join(new_lines)

# 修复 2: 确保 onInitialize() 正确闭合
# 找 onInitialize() 的 { 和 scheduleMoveTick 之间的 }，确保 onInitialize 在 scheduleMoveTick 之前闭合
lines = content.split('\n')

# 找 onInitialize() 开始
init_start = -1
for i, line in enumerate(lines):
    if 'public void onInitialize()' in line:
        init_start = i
        break

if init_start >= 0:
    # 找 scheduleMoveTick 方法
    sched_start = -1
    for i, line in enumerate(lines):
        if 'private void scheduleMoveTick()' in line:
            sched_start = i
            break
    
    if sched_start >= 0 and sched_start > init_start:
        # 在 init_start 和 sched_start 之间找 } 
        # 这个 } 应该是 onInitialize() 的闭合
        found_init_close = False
        for i in range(init_start, sched_start):
            if lines[i].strip() == '}':
                found_init_close = True
                break
        
        if not found_init_close:
            # onInitialize() 没有闭合，在 scheduleMoveTick 前插入 }
            indent = '    '
            lines.insert(sched_start, indent + '}')
            # 同时删除 scheduleMoveTick 后的多余 }
            # 找 scheduleMoveTick 方法结束位置
            brace = 0
            sched_end = -1
            for i in range(sched_start, len(lines)):
                brace += lines[i].count('{') - lines[i].count('}')
                if brace == 0 and 'scheduleMoveTick()' in lines[i]:
                    sched_end = i
                    break
            
            # 删除 scheduleMoveTick 后的多余 }（如果有）
            if sched_end >= 0 and sched_end + 1 < len(lines) and lines[sched_end + 1].strip() == '}':
                lines.pop(sched_end + 1)

content = '\n'.join(lines)

with open(file_path, 'w', encoding='utf-8') as f:
    f.write(content)

print("Done!")
