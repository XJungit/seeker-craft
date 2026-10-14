#!/usr/bin/env node
/**
 * 最小 asar 解包器（只读，用于比对 DSH 打包态里的 API 面）。
 *
 * DSH 桌面端 0.2.0-rc.2 把 @deepseek-ai/* 打进了 resources/app.asar，
 * 磁盘上没有散开的目录（profile 下的同名 junction 指向的是**已被卸载的**
 * npm 全局安装，已悬挂）。所以要核对兼容性，必须从 asar 里取文件。
 *
 * asar 格式：8 字节 pickle 头 + JSON 头（含每个文件的 offset/size）+ 文件数据。
 * 这里只做"列出 + 按路径提取文本"，不做完整重建。
 *
 * 用法:
 *   node scripts/extract-asar.mjs <app.asar> list <子串过滤>
 *   node scripts/extract-asar.mjs <app.asar> cat  <内部路径>
 *   node scripts/extract-asar.mjs <app.asar> extract <内部前缀> <目标目录>
 */
import { readFileSync, mkdirSync, writeFileSync, statSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'

const [, , asarPath, cmd, arg] = process.argv
if (!asarPath || !cmd) {
  console.error('用法: node scripts/extract-asar.mjs <app.asar> list|cat <参数>')
  process.exit(2)
}

const buf = readFileSync(asarPath)

// asar 头是两层 pickle，布局（实测 app.asar 头部字节）：
//   [0]  uint32 = 4                     pickle#1 载荷长度
//   [4]  uint32 = headerSize            pickle#2 总长度（下面读这么多个字节）
//   [8]  uint32 = headerSize - 4        pickle#2 载荷长度
//   [12] uint32 = jsonSize               JSON 字符串字节数
//   [16] ...                            JSON 本体
// 数据区起点 = 8 + headerSize（= 16 + jsonSize，后者按 4 字节对齐补齐）。
// 先前把 offset 4 当 JSON 长度、offset 8 起当 JSON，导致 JSON.parse 报
// "Unexpected non-whitespace character after JSON at position 1"。
const headerSize = buf.readUInt32LE(4)
const jsonSize = buf.readUInt32LE(12)
const headerStart = 16
const header = JSON.parse(buf.subarray(headerStart, headerStart + jsonSize).toString('utf8'))
const dataStart = 8 + headerSize
const files = []
;(function walk(node, prefix) {
  for (const [name, entry] of Object.entries(node.files ?? {})) {
    const p = prefix ? `${prefix}/${name}` : name
    if (entry.files) walk(entry, p)
    else files.push({ path: p, offset: Number(entry.offset), size: entry.size })
  }
})(header, '')

if (cmd === 'list') {
  const filter = arg ?? ''
  const hits = files.filter((f) => f.path.includes(filter))
  console.log(`总文件数 ${files.length}；匹配 "${filter}"：${hits.length}`)
  hits.slice(0, 400).forEach((f) => console.log(`  ${f.path}  (${f.size}B)`))
} else if (cmd === 'cat') {
  const f = files.find((x) => x.path === arg)
  if (!f) {
    console.error(`未找到: ${arg}`)
    process.exit(1)
  }
  process.stdout.write(buf.subarray(dataStart + f.offset, dataStart + f.offset + f.size))
} else if (cmd === 'extract') {
  // extract <内部前缀> <目标目录>：把匹配前缀的文件原样落地，保留相对路径。
  // profile 里那些 @deepseek-ai/* junction 指向的是**已被卸载的** npm 全局安装
  // （250 个全悬挂），所以想拿真实包做兼容性校验，只能从 asar 里抠出来。
  const prefix = arg
  const destArg = process.argv[5]
  if (!prefix || !destArg) {
    console.error('用法: node scripts/extract-asar.mjs <app.asar> extract <内部前缀> <目标目录>')
    process.exit(2)
  }
  const dest = resolve(destArg)
  const hits = files.filter((f) => f.path.startsWith(prefix))
  if (hits.length === 0) {
    console.error(`前缀无匹配: ${prefix}`)
    process.exit(1)
  }
  let bytes = 0
  for (const f of hits) {
    // 相对前缀取路径，去掉开头的 '/'（prefix 本身可能是目录名而非完整段）
    const rel = f.path.slice(prefix.length).replace(/^\/+/, '') || f.path.split('/').pop()
    const out = join(dest, rel)
    mkdirSync(dirname(out), { recursive: true })
    writeFileSync(out, buf.subarray(dataStart + f.offset, dataStart + f.offset + f.size))
    bytes += f.size
  }
  console.log(`已解出 ${hits.length} 个文件 / ${(bytes / 1048576).toFixed(1)}MB → ${dest}`)
} else {
  console.error(`未知命令: ${cmd}`)
  process.exit(2)
}
