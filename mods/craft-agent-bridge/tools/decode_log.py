import sys
raw = open(r'D:\Craft-Agent\mods\craft-agent-bridge\build_log.txt', 'rb').read()
print("file bytes:", len(raw))
txt = None
for enc in ['gb18030', 'gbk', 'utf-8', 'utf-16']:
    try:
        txt = raw.decode(enc); print("decoded with", enc, "len", len(txt)); break
    except Exception as e:
        print(enc, "fail", e)
if txt is None:
    txt = raw.decode('gb18030', 'replace')
kws = [b'BUILD FAILED', b'BUILD SUCCESSFUL', b'What went wrong', b'Could not resolve',
       b'FAILED', b'error:', b'502', b'UNEXPECTED_EOF', b'Cannot remap',
       b'minecraft-merged', b'Repository', b'disabled', b'Exception']
for k in kws:
    idx = raw.find(k)
    if idx >= 0:
        seg = raw[max(0, idx - 80):idx + 320]
        s = seg.decode('gb18030', 'replace')
        print('\n###', k.decode())
        print(s.replace('\n', ' '))
print('\n===== TAIL =====')
print(txt[-700:].replace('\n', ' '))
