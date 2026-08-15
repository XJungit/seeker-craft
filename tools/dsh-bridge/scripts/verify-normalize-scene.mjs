/**
 * 验证 normalizeSceneDesc 排序稳定化：
 * 1. bot 挂机（仅枚举顺序抖动）→ 规范化后文本字节相同（消除无效追加）
 * 2. bot 实质变化（位置/数量变）→ 规范化后仍不同（保真）
 * 3. 非枚举行（位置/生命/记忆）不受影响
 */
import { normalizeSceneDesc } from '../index.js'

let pass = 0, fail = 0
function check(name, cond, detail = '') {
  if (cond) { pass++; console.log(`✅ ${name}`) }
  else { fail++; console.log(`❌ ${name} ${detail}`) }
}

// 模拟 bot 挂机：同一状态，仅枚举顺序不同（HashMap 抖动）
const base = `位置: (-480, 81, -173)
生命: 20/20  饱食: 20/20  主手: diamond_pickaxe (耐久 1557/1561)
当前动作: 空闲
维度: minecraft:overworld
群系: lush_caves  脚下: stone  前方: stone
附近: [stone:7]
特殊方块: [coal_ore:4]
实体: [player:1, item:1@5m@(-481, 84, -170)]
背包: [cobblestone:112, cobbled_deepslate:55, raw_copper:32]
hotbar: [tuff x11, diamond_sword x1]
资源: 木材:0 石头:1070 矿石:4`

// 抖动版：枚举顺序不同，但语义相同
const jittered = `位置: (-480, 81, -173)
生命: 20/20  饱食: 20/20  主手: diamond_pickaxe (耐久 1557/1561)
当前动作: 空闲
维度: minecraft:overworld
群系: lush_caves  脚下: stone  前方: stone
附近: [stone:7]
特殊方块: [coal_ore:4]
实体: [item:1@5m@(-481, 84, -170), player:1]
背包: [cobbled_deepslate:55, raw_copper:32, cobblestone:112]
hotbar: [diamond_sword x1, tuff x11]
资源: 木材:0 矿石:4 石头:1070`

const n1 = normalizeSceneDesc(base)
const n2 = normalizeSceneDesc(jittered)
check('挂机抖动 → 规范化后字节相同', n1 === n2, `\n---n1---\n${n1}\n---n2---\n${n2}`)

// 实质变化：cobblestone 112→120
const changed = `位置: (-480, 81, -173)
生命: 20/20  饱食: 20/20  主手: diamond_pickaxe (耐久 1557/1561)
当前动作: 空闲
维度: minecraft:overworld
群系: lush_caves  脚下: stone  前方: stone
附近: [stone:7]
特殊方块: [coal_ore:4]
实体: [player:1, item:1@5m@(-481, 84, -170)]
背包: [cobblestone:120, cobbled_deepslate:55, raw_copper:32]
hotbar: [tuff x11, diamond_sword x1]
资源: 木材:0 石头:1070 矿石:4`

const n3 = normalizeSceneDesc(changed)
check('实质变化 → 规范化后仍不同', n1 !== n3, `\n---n1---\n${n1}\n---n3---\n${n3}`)

// 非枚举行不受影响
const withMem = base + `\n记忆: [已知世界记忆·邻近]\n- 资源点: 矿石 [coal_ore] @(-481,83,-193) 距离2格\n[锚点]\n- __self__: 当前位置 (-480,81,-173)`
const withMem2 = base + `\n记忆: [已知世界记忆·邻近]\n- 资源点: 矿石 [coal_ore] @(-481,83,-193) 距离2格\n[锚点]\n- __self__: 当前位置 (-480,81,-173)`
check('记忆/锚点行不受影响', normalizeSceneDesc(withMem) === normalizeSceneDesc(withMem2))

// 空串/非字符串安全
check('空串安全', normalizeSceneDesc('') === '')
check('非字符串安全', normalizeSceneDesc(undefined) === undefined)

console.log(`\n结果: ${pass} 通过, ${fail} 失败`)
process.exit(fail === 0 ? 0 : 1)
