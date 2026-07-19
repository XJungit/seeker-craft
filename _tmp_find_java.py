import os, glob
roots = [r'C:\Program Files', r'C:\Program Files (x86)', r'D:\Game', r'D:\\']
hits = []
for root in roots:
    for p in glob.glob(root + '/**/bin/java.exe', recursive=True):
        low = p.lower()
        if 'jdk' in low or 'jre' in low:
            hits.append(p)
for h in hits[:30]:
    print(h)
