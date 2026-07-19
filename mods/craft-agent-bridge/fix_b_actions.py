import re

with open(r'd:\Craft-Agent\mods\craft-agent-bridge\src\main\java\com\craftagent\bridge\CraftAgentBridge.java', 'r', encoding='utf-8') as f:
    content = f.read()

# Step 1: 在 dispatch 方法中添加 B 类 action 的 TCP 线程路由
old_dispatch = '        if ("collect_items".equals(type)) {\n            return performCollectItems(req);\n        }\n        return runOnServerThread(() -> {'

new_dispatch = '''        if ("collect_items".equals(type)) {
            return performCollectItems(req);
        }
        // B 类 action：有 Thread.sleep 循环，必须在 TCP 线程执行
        if ("attack_player".equals(type)) {
            return performAttackPlayer(req);
        }
        if ("follow_player".equals(type)) {
            return performFollowPlayer(req);
        }
        if ("combat".equals(type)) {
            return performCombat(req);
        }
        if ("use_item".equals(type)) {
            return performUseItem(req);
        }
        if ("eat_item".equals(type)) {
            return performEatItem(req);
        }
        if ("wait".equals(type)) {
            return performWait(req);
        }
        return runOnServerThread(() -> {'''

content = content.replace(old_dispatch, new_dispatch)
print('Step 1: dispatch routing added')

# Step 2: 从 performAction 中删除 attack_player case
# 用正则匹配整个 case 块
pattern = r'            case "attack_player": \{.*?\n            \}'
match = re.search(pattern, content, re.DOTALL)
if match:
    content = content[:match.start()] + content[match.end():]
    print(f'Step 2: removed attack_player case ({len(match.group())} chars)')
else:
    print('WARNING: attack_player case not found')
    idx = content.find('case "attack_player"')
    print(f'  found at index: {idx}')

# Step 3: 删除 follow_player case
pattern = r'            case "follow_player": \{.*?\n            \}'
match = re.search(pattern, content, re.DOTALL)
if match:
    content = content[:match.start()] + content[match.end():]
    print(f'Step 3: removed follow_player case ({len(match.group())} chars)')
else:
    print('WARNING: follow_player case not found')
    idx = content.find('case "follow_player"')
    print(f'  found at index: {idx}')

# Step 4: 删除 combat case
pattern = r'            case "combat": \{.*?\n            \}'
match = re.search(pattern, content, re.DOTALL)
if match:
    content = content[:match.start()] + content[match.end():]
    print(f'Step 4: removed combat case ({len(match.group())} chars)')
else:
    print('WARNING: combat case not found')
    idx = content.find('case "combat"')
    print(f'  found at index: {idx}')

# Step 5: 删除 use_item case
pattern = r'            case "use_item": \{.*?\n            \}'
match = re.search(pattern, content, re.DOTALL)
if match:
    content = content[:match.start()] + content[match.end():]
    print(f'Step 5: removed use_item case ({len(match.group())} chars)')
else:
    print('WARNING: use_item case not found')
    idx = content.find('case "use_item"')
    print(f'  found at index: {idx}')

# Step 6: 删除 eat_item case
pattern = r'            case "eat_item": \{.*?\n            \}'
match = re.search(pattern, content, re.DOTALL)
if match:
    content = content[:match.start()] + content[match.end():]
    print(f'Step 6: removed eat_item case ({len(match.group())} chars)')
else:
    print('WARNING: eat_item case not found')
    idx = content.find('case "eat_item"')
    print(f'  found at index: {idx}')

# Step 7: 删除 wait case
pattern = r'            case "wait": \{.*?\n            \}'
match = re.search(pattern, content, re.DOTALL)
if match:
    content = content[:match.start()] + content[match.end():]
    print(f'Step 7: removed wait case ({len(match.group())} chars)')
else:
    print('WARNING: wait case not found')
    idx = content.find('case "wait"')
    print(f'  found at index: {idx}')

with open(r'd:\Craft-Agent\mods\craft-agent-bridge\src\main\java\com\craftagent\bridge\CraftAgentBridge.java', 'w', encoding='utf-8') as f:
    f.write(content)

print('All deletions done')
