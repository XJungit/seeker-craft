import os, re, glob

base = 'crates/craft-agent-minecraft/src/tools_azalea/'
files = ['tools_container.rs','tools_social.rs','tools_inventory.rs','tools_farming.rs','tools_mining.rs','tools_interact.rs','tools_movement.rs','tools_perceive.rs','tools_placement.rs','tools_crafting.rs','tools_meta.rs']

for f in files:
    path = base + f
    if not os.path.exists(path):
        continue
    c = open(path, encoding='utf-8').read()
    # 找所有 fn name -> 'xxx' 的工具名（每个工具 struct 的 name 方法）
    for m in re.finditer(r'fn name\(&self\) -> &str \{\s*\n\s*"([a-z_0-9]+)"', c):
        name = m.group(1)
        start = m.start()
        # 找到该工具的 execute 方法体（从 fn name 开始到下一个 fn name 或 5000 字符）
        nxt = c.find('fn name(&self) -> &str {', start + 10)
        end = nxt if nxt > 0 else start + 6000
        seg = c[start:end]
        has_exec = 'fn execute' in seg
        has_adapter = ('execute_shared' in seg or 'perceive_shared' in seg or '_exec_action' in seg)
        has_http = 'reqwest' in seg
        has_local = ('ctx.actions' in seg or 'ctx.blueprints' in seg or 'ctx.memory' in seg)
        has_lock = '.lock()' in seg
        status = []
        if not has_exec:
            status.append('NO-EXEC')
        if not has_adapter and not has_http and not has_local and not has_lock:
            status.append('EMPTY-SHELL')
        elif not has_adapter and not has_http:
            status.append('LOCAL-ONLY')
        print(f'{f}: {name} [{" ".join(status) if status else "OK"}]')
