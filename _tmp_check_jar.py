import os, glob, time
src = 'd:/Craft-Agent/mods/craft-agent-bridge/build/libs/craft-agent-bridge-0.1.0.jar'
print(f'src:    {time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(os.path.getmtime(src)))}  {os.path.getsize(src)} bytes')
for p in sorted(glob.glob('D:/Game/PCL2/.minecraft/versions/26.2-Fabric 0.19.3/mods/craft-agent-bridge-*.jar')):
    print(f'deploy: {time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(os.path.getmtime(p)))}  {os.path.getsize(p)} bytes  {os.path.basename(p)}')
