import re, io

files = ['README.md', 'README.zh-CN.md', 'ARCHITECTURE.md', 'docs/benchmarks.md']
# 只替换工具上下文里的 54/53 → 49，避免误伤
patterns = [
    (r'54 LLM [Tt]ools', '49 LLM Tools'),
    (r'54 个 LLM 工具', '49 个 LLM 工具'),
    (r'54 个类型化 LLM 工具', '49 个类型化 LLM 工具'),
    (r'54 typed LLM tools', '49 typed LLM tools'),
    (r'54 LLM tools', '49 LLM tools'),
    (r'54 tools', '49 tools'),
    (r'54 工具', '49 工具'),
    (r'54 个', '49 个'),
    (r'54 total', '49 total'),
    (r'54（`ALL_TOOL_NAMES`', '49（`ALL_TOOL_NAMES`'),
]

for f in files:
    try:
        c = io.open(f, encoding='utf-8').read()
    except FileNotFoundError:
        print(f'!! {f} not found')
        continue
    orig = c
    for pat, rep in patterns:
        c = re.sub(pat, rep, c)
    if c != orig:
        io.open(f, 'w', encoding='utf-8', newline='\n').write(c)
        print(f'updated {f}')
    else:
        print(f'unchanged {f}')
