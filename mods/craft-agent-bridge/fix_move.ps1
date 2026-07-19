$file = 'd:\Craft-Agent\mods\craft-agent-bridge\src\main\java\com\craftagent\bridge\CraftAgentBridge.java'
$lines = Get-Content $file

# 1) 找到 onEndServerTick 方法起止，删除它
$start = -1
$end = -1
$brace = 0
for ($i = 0; $i -lt $lines.Count; $i++) {
    if ($lines[$i] -match 'private void onEndServerTick') { $start = $i }
    if ($start -ge 0 -and $i -ge $start) {
        $brace += ($lines[$i] -split '{').Length - 1
        $brace -= ($lines[$i] -split '}').Length - 1
        if ($brace -le 0) { $end = $i; break }
    }
}
if ($start -ge 0 -and $end -ge $start) {
    $lines = $lines[0..($start-1)] + $lines[($end+1)..($lines.Count-1)]
}

# 2) 在 onInitialize() 中替换 tick 注册
for ($i = 0; $i -lt $lines.Count; $i++) {
    if ($lines[$i] -match 'ServerTickEvents.END_SERVER_TICK.register') {
        $lines[$i] = '        // 移动由 scheduleMoveTick() 通过 executeIfPossible 驱动'
        break
    }
}

# 3) 在 onInitialize() 结束 } 前插入 scheduleMoveTick() 方法
$insertPos = -1
for ($i = $lines.Count - 1; $i -ge 0; $i--) {
    if ($lines[$i] -match '^\s*}$' -and $lines[$i] -notmatch 'class|interface|enum|record') {
        # 找 onInitialize 的闭合 } — 向上找最近的空白或非方法行
        $j = $i - 1
        while ($j -ge 0 -and $lines[$j] -match '^\s*(private|public|protected).*\{') { $j-- }
        if ($j -ge 0 -and $lines[$j] -match 'onInitialize') {
            $insertPos = $i
            break
        }
    }
}

$method = @(
    ''
    '    /** 递归调度移动：每 tick 在服务端线程执行一次 move()，直到到达或超时。 */'
    '    private void scheduleMoveTick() {'
    '        MinecraftServer server = serverInstance;'
    '        if (server == null) return;'
    '        server.executeIfPossible(() -> {'
    '            if (moveTarget == null) return;'
    '            ServerPlayer player = getFirstPlayer(server);'
    '            if (player == null) { moveTarget = null; return; }'
    ''
    '            double tx = moveTarget[0], tz = moveTarget[2];'
    '            double ddx = tx - player.getX(), ddz = tz - player.getZ();'
    '            double horiz = Math.sqrt(ddx * ddx + ddz * ddz);'
    '            moveFinalDist = horiz;'
    '            moveTicksLeft--;'
    ''
    '            if (horiz < 0.8 || moveTicksLeft <= 0) {'
    '                moveReached = horiz < 0.8;'
    '                moveTarget = null;'
    '                player.zza = 0;'
    '                player.xxa = 0;'
    '                return;'
    '            }'
    ''
    '            float yaw = (float) Math.toDegrees(Math.atan2(-ddx, ddz));'
    '            player.setYRot(yaw);'
    ''
    '            double step = 0.3;'
    '            double ratio = step / horiz;'
    '            double dx = ddx * ratio;'
    '            double dz = ddz * ratio;'
    '            '
    '            player.move(MoverType.SELF, new Vec3(dx, 0, dz));'
    '            player.setPosRaw(player.getX(), player.getY(), player.getZ());'
    '            System.out.println("[move] tick x=" + player.getX() + " z=" + player.getZ() + " horiz=" + horiz);'
    ''
    '            player.zza = 1.0f;'
    '            player.xxa = 0.0f;'
    ''
    '            scheduleMoveTick();'
    '        });'
    '    }'
)

if ($insertPos -ge 0) {
    $lines = $lines[0..$insertPos] + $method + $lines[($insertPos+1)..($lines.Count-1)]
}

# 4) 在 6 处 moveTarget = new double[]{...} 后添加 scheduleMoveTick()
$targets = @(
    'moveTarget = new double[]{tx, ty, tz};',
    'moveTarget = new double[]{target.getX(), target.getY(), target.getZ()};',
    'moveTarget = new double[]{nearest.getX(), nearest.getY(), nearest.getZ()};',
    'moveTarget = new double[]{startX + awayDx, startY, startZ + awayDz};',
    'moveTarget = new double[]{startX, startY, startZ};'
)

for ($i = 0; $i -lt $lines.Count; $i++) {
    foreach ($t in $targets) {
        if ($lines[$i].Trim() -eq $t) {
            $indent = $lines[$i] -replace '(\S.*)', '$1'
            $lines[$i] = "$indent`n$indent`$scheduleMoveTick();"
            break
        }
    }
}

$lines | Set-Content $file -Encoding UTF8
Write-Output "Done. Lines: $($lines.Count)"
