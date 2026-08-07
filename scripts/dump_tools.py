# 临时脚本 v3：模拟 to_knowledge_string 精确输出（含 Rust 续行符还原 + 分组）
import io, sys, glob, os
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding='utf-8', errors='replace')

def join_literals(rest):
    """逐步消费相邻字符串字面量并拼接（处理 \n 转义 + Rust 续行符 '\\' + 换行）。"""
    parts = []
    seg = rest
    while True:
        seg = seg.lstrip()
        if not seg.startswith(('"', 'r"', 'r#')):
            break
        if seg.startswith('r#'):
            end = seg.find('"#', 2)
            if end == -1: break
            parts.append(seg[3:end]); seg = seg[end + 2:]
            continue
        if seg.startswith('r"'):
            end = seg.find('"', 2)
            if end == -1: break
            parts.append(seg[2:end]); seg = seg[end + 1:]
            continue
        out = []
        i = 1
        while i < len(seg):
            c = seg[i]
            if c == '"':
                i += 1
                break
            if c == '\\' and i + 1 < len(seg):
                n = seg[i + 1]
                if n == 'n': out.append('\n')
                elif n == 't': out.append('\t')
                elif n == '"': out.append('"')
                elif n == '\\': out.append('\\')
                elif n == 'r': out.append('\r')
                elif n == '\n':
                    # Rust 续行符：忽略换行及后续前导空白
                    i += 2
                    while i < len(seg) and seg[i] in ' \t':
                        i += 1
                    continue
                else: out.append('\\' + n)
                i += 2
            else:
                out.append(c); i += 1
        parts.append(''.join(out))
        seg = seg[i:]
    return ''.join(parts)

def extract(path):
    with open(path, encoding='utf-8', errors='replace') as f:
        src = f.read()
    tools = []
    for m in src.split('impl GameTool for ')[1:]:
        brace = m.find('{')
        if brace == -1:
            continue
        block = m[brace:]
        ni = block.find('fn name(')
        if ni == -1:
            continue
        rest = block[ni + len('fn name('):]
        si = rest.find('{'); ei = rest.find('}')
        seg = rest[si + 1:ei]
        nq = seg.find('"')
        nm = ""
        if nq != -1:
            j = nq + 1
            while j < len(seg) and seg[j] != '"':
                j += 1
            nm = seg[nq + 1:j]
        di = block.find('fn description(')
        desc = ""
        if di != -1:
            rest2 = block[di:]
            si2 = rest2.find('{')
            desc = join_literals(rest2[si2 + 1:]).strip()
        if nm:
            tools.append((nm, desc))
    return tools

all_tools = []
for p in glob.glob(os.path.join('crates', 'craft-agent-minecraft', 'src', 'tools_azalea', '*.rs')):
    all_tools += extract(p)
all_tools += extract(os.path.join('crates', 'craft-agent-minecraft', 'src', 'tools_azalea.rs'))

seen = set()
uniq = []
for t in all_tools:
    if t[0] not in seen:
        seen.add(t[0]); uniq.append(t)

tmap = dict(uniq)

# —— 复刻 tool.rs to_knowledge_string 的分组逻辑 ——
groups = [
    ("High-Level", ["collect", "craft", "place", "build", "blueprints", "combat", "attack"]),
    ("Utility", ["equip", "consume", "discard", "smeltItem"]),
    ("Navigation", ["searchForBlock", "move_to", "moveAway", "digDown"]),
    ("Aim", ["look_at", "look_at_player", "look_at_position"]),
    ("Query", ["perceive", "visual_perceive", "savedPlaces"]),
    ("Memory", ["rememberHere", "goToRememberedPlace"]),
]
all_grouped = set()
lines = []
for gname, names in groups:
    gl = []
    for n in names:
        if n in tmap:
            gl.append(f"{n}(...) — {tmap[n]}")
            all_grouped.add(n)
    if gl:
        lines.append("")
        lines.append(f"## {gname} Tools")
        lines.extend(gl)
ungrouped = [f"{n}(...) — {d}" for n, d in uniq if n not in all_grouped]
if ungrouped:
    lines.append("")
    lines.append("## Other Tools")
    lines.extend(ungrouped)

print(f"共 {len(uniq)} 个工具 | 分组: {[g[0] for g in groups if any(n in tmap for n in g[1])]} + Other({len(ungrouped)})")
print("=" * 60)
print('\n'.join(lines))
