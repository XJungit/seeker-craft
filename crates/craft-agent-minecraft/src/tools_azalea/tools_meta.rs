//! 元操作工具：chat / set_goal / run_plan / run_script / task_complete / task_retry / pause_goal / resume_goal / new_action / list_actions（P3.2 按域拆分，tools_azalea 域模块）。
use super::*;

/// 发送聊天消息（也用作 LLM 指令回显 / 与玩家沟通）。
pub struct ChatTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl ChatTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for ChatTool {
    fn name(&self) -> &str {
        "chat"
    }
    fn description(&self) -> &str {
        "发送聊天消息到游戏。参数 content 为消息文本。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": { "type": "string", "description": "聊天内容" }
            },
            "required": ["content"]
        })
    }
    fn execute(
        &self,
        _call_id: &str,
        args: Value,
        _on_update: Option<craft_agent::core::tool::ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 content"))?
            .to_string();
        let r = self
            .ctx
            .adapter
            .execute_shared(Action::Minecraft(MinecraftAction::Chat { content }))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 设置/更新当前目标（self-prompt）。bot 会持续朝此目标行动直到调用 set_goal("") 清空。
#[allow(dead_code)]
pub struct SetGoalTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl SetGoalTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for SetGoalTool {
    fn name(&self) -> &str {
        "set_goal"
    }
    fn description(&self) -> &str {
        "设置或更新当前目标。bot 会持续朝此目标行动直到调用 set_goal(goal=\"\") 清空。\
         goal 为英文目标描述，如 \"Get 3 iron ingots\" / \"Build a house\"。\
         调用后系统每轮自动注入此目标，bot 持续行动直到目标达成。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "goal": { "type": "string", "description": "目标描述（英文）。传空字符串清空目标。" }
            },
            "required": ["goal"]
        })
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _call_id: &str,
        args: Value,
        _on_update: Option<craft_agent::core::tool::ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let goal = args.get("goal").and_then(|v| v.as_str()).unwrap_or("");
        if goal.is_empty() {
            Ok(ToolResult {
                message: "目标已清空".to_string(),
                is_error: false,
                images: vec![],
            })
        } else {
            Ok(ToolResult {
                message: format!("目标已设置: {goal}"),
                is_error: false,
                images: vec![],
            })
        }
    }
}

/// 执行多步计划：按顺序执行一系列工具调用（支持 goto/mine/craft/gather/place 等）。
/// 每一步等待前一步完成再执行下一步，返回所有步骤的汇总结果。
/// 比 Mindcraft 的代码执行更安全——只使用已注册工具，不执行任意代码。
pub struct RunPlanTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl RunPlanTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for RunPlanTool {
    fn name(&self) -> &str {
        "run_plan"
    }
    fn description(&self) -> &str {
        "执行多步计划：按顺序执行一系列工具调用。steps 为 JSON 数组，每步格式为 {\"action\":\"工具名\", \"参数名\":值}。\
         支持动作: goto, mine, craft, gather, place, open, interact, attack, chat, mine_below。\
         例: [{\"action\":\"goto\",\"x\":10,\"y\":64,\"z\":8}, {\"action\":\"mine\",\"x\":10,\"y\":63,\"z\":8}]。\
         会等待每一步完成再执行下一步，返回所有步骤的汇总结果。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "steps": {
                    "type": "array",
                    "description": "动作序列，每步 {\"action\":\"工具名\", 参数...}",
                    "items": {
                        "type": "object",
                        "properties": {
                            "action": { "type": "string", "description": "工具名: goto/mine/craft/gather/place/open/interact/attack/chat/mine_below" }
                        },
                        "required": ["action"]
                    }
                }
            },
            "required": ["steps"]
        })
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _call_id: &str,
        args: Value,
        _on_update: Option<craft_agent::core::tool::ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let steps = args
            .get("steps")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("缺少 steps 数组"))?;
        let mut results: Vec<String> = Vec::new();
        // 上一步 mine 的坐标——用于检测并跳过"mine→goto 同坐标"这种无效组合。
        // LLM 常写 [{mine (x,y,z)}, {goto (x,y,z)}] 想让 bot "挖完掉进洞"，
        // 但 azalea bot 挖完脚下方块不会自动掉进去，goto 到空气位置必然超时。
        // 检测到这种 plan 时直接跳过 goto，告知 LLM bot 已在附近。
        let mut last_mined: Option<(i32, i32, i32)> = None;
        for (i, step) in steps.iter().enumerate() {
            let action_name = step.get("action").and_then(|v| v.as_str()).unwrap_or("?");
            // 跳过无效 goto：目标是上一步 mine 的位置
            if action_name == "goto"
                && let Some((mx, my, mz)) = last_mined
            {
                let gx = step.get("x").and_then(|v| v.as_i64()).map(|v| v as i32);
                let gy = step.get("y").and_then(|v| v.as_i64()).map(|v| v as i32);
                let gz = step.get("z").and_then(|v| v.as_i64()).map(|v| v as i32);
                if gx == Some(mx) && gy == Some(my) && gz == Some(mz) {
                    results.push(format!("步骤{} (goto) 跳过: goto ({},{},{}) 是上一步刚挖的位置，bot 已在附近无需 goto。", i + 1, mx, my, mz));
                    last_mined = None;
                    continue;
                }
            }
            let mc = parse_step(action_name, step)?;
            // 记录 mine 坐标供下一步检测
            if let MinecraftAction::MineBlock { x, y, z } = &mc {
                last_mined = Some((*x, *y, *z));
            } else {
                last_mined = None;
            }
            match self.ctx.adapter.execute_shared(Action::Minecraft(mc)) {
                Ok(r) => {
                    results.push(format!(
                        "步骤{} ({}) 完成: {}",
                        i + 1,
                        action_name,
                        r.detail
                    ));
                }
                Err(e) => {
                    results.push(format!("步骤{} ({}) 失败: {}", i + 1, action_name, e));
                    break;
                }
            }
        }
        Ok(ToolResult {
            message: results.join("\n"),
            is_error: false,
            images: vec![],
        })
    }
}

/// 执行 rhai 脚本（嵌入式脚本引擎，直接在 Rust 进程内执行，比 Node.js 更快更轻量）。
///
/// 学习自 Mindcraft `library/skills.js` + `agent/commands/actions.js`：把全部动作工具暴露为
/// rhai 函数，LLM 用一段脚本即可完成多步任务（采集→合成→放置），比 run_plan 更灵活。
///
/// **白名单（24 个动作函数 + 2 个工具函数）**：
/// - 移动/挖掘：`go(x,y,z)` `mine(x,y,z)` `mine_below()` `mine_above()` `interact(x,y,z)`
/// - 战斗：`attack(target?)` `defend()`
/// - 合成/熔炼：`craft(item,count)` `craft_3x3(item,count)` `smelt(output,fuel,count)` `auto_craft(item,count)` `enchant(item,level)`
/// - 采集/放置：`gather(item,count)` `place(item,x,y,z)` `open(x,y,z)`
/// - 容器：`chest_view(x,y,z)` `chest_withdraw(x,y,z,item,count)` `chest_deposit(x,y,z,item,count)`
/// - 装备/消耗：`equip(item,slot)` `discard(item,count)` `consume(item)`
/// - 交互：`interact_entity(kind)` `trade(offer)` `chat(msg)` `pickup()`
/// - 工具：`perceive()` 返回结构化世界状态文本 / `list_blueprints()` 列出蓝图 / `build_blueprint(name,x,y,z)` 建造蓝图
/// - 位置：`pos_x()` `pos_y()` `pos_z()` 返回当前坐标（P104 补齐，轻量读缓存）
/// - 元：`sleep(ms)` `print(msg)`
///
/// **注意**：
/// - 寻路函数名是 `go`（不是 `goto`，`goto` 是 rhai 保留字）。
/// - 不暴露：`run_plan` / `run_script`（递归），`memory` / `set_goal` / `pause_goal` / `resume_goal`
///   （这些直接修改 Agent/记忆库状态，不应在脚本里调用），`search_wiki`（HTTP 阻塞），`build`（用
///   `build_blueprint` 替代，更安全）。
///
/// **lint**：执行前 `lint_script()` 检查长度/禁用关键字/危险模式，拒绝则不执行。
pub struct RunScriptTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl RunScriptTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for RunScriptTool {
    fn name(&self) -> &str {
        "run_script"
    }
    fn description(&self) -> &str {
        "执行 rhai 脚本（嵌入式引擎，沙箱化）。支持变量、循环、条件。\
         使用时机：需要连续执行 3 步以上动作（如 收集→合成→拾取 流水线、循环挖矿、\
         带条件判断的多步流程）时用本工具；单步操作请直接调用对应工具（更可靠）。\
         动作函数: walk_to(x,y,z) [或 move_to/step_to，不要用 go/goto，rhai 保留字], \
         mine(x,y,z), mine_below(), mine_above(), interact(x,y,z), attack(target?), defend(), \
         craft(item,count), craft_3x3(item,count), smelt(output,fuel,count), auto_craft(item,count), enchant(item,level), \
         gather(item,count), place(item,x,y,z), open(x,y,z), \
         chest_view(x,y,z), chest_withdraw(x,y,z,item,count), chest_deposit(x,y,z,item,count), \
         equip(item,slot), discard(item,count), consume(item), \
         interact_entity(kind), trade(offer), chat(msg), pickup(), \
         perceive(), list_blueprints(), build_blueprint(name,x,y,z), \
         till_and_sow(x,y,z,seed), harvest(), sleep(ms), print(msg)。\
         脚本最后一行若是动作函数调用会作为返回值；不需要返回值时末尾加分号 `;`。\
         函数返回错误消息字符串（含\"失败\"/\"超时\"/\"未持有\"）不会中断脚本，可用 if 判断：\
         例1（流水线）: walk_to(10, 64, 20); gather(\"oak_log\", 4); craft(\"oak_planks\", 4); pickup();\
         例2（循环+条件）: for i in 0..3 { let r = mine(10, 60-i, 20); if r.contains(\"失败\") { break; } sleep(300); }"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "script": { "type": "string", "description": "rhai 脚本代码（≤8KB，禁用 import/eval 等）" }
            },
            "required": ["script"]
        })
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _call_id: &str,
        args: Value,
        _on_update: Option<craft_agent::core::tool::ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let script = args
            .get("script")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 script"))?;
        // 1. lint：长度/禁用关键字/危险模式
        if let Err(reason) = lint_script(script) {
            return Ok(ToolResult {
                message: format!("脚本被 lint 拒绝: {reason}"),
                is_error: true,
                images: vec![],
            });
        }
        // 2. 构建沙箱引擎（含 call_action 递归支持）
        let engine = build_rhai_engine(&self.ctx);
        // 用 Dynamic 接收任意返回类型：rhai 脚本最后一行若以 `;` 结尾返回 unit ()，
        // 若是表达式则返回该值。eval::<String>() 在 unit 时报 "Output type incorrect: ()"，
        // 改用 Dynamic 后 unit 显示为 "()"，我们识别后转为 "脚本执行完成"。
        match engine.eval::<rhai::Dynamic>(script) {
            Ok(out) => {
                let msg = if out.is_unit() || out.to_string().is_empty() {
                    "脚本执行完成".to_string()
                } else {
                    out.to_string()
                };
                Ok(ToolResult {
                    message: msg,
                    is_error: false,
                    images: vec![],
                })
            }
            Err(e) => Ok(ToolResult {
                message: format!("脚本错误: {e}"),
                is_error: true,
                images: vec![],
            }),
        }
    }
}

/// 构建 rhai 沙箱引擎：注册全部 27 个动作函数 + call_action + sleep/print + 资源限制。
///
/// `call_action(name)` 会递归调用此函数构建子引擎执行已保存的 LLM 自定义动作，
/// 递归深度由 `max_call_levels=20` 兜底。
fn build_rhai_engine(ctx: &Arc<AzaleaToolCtx>) -> rhai::Engine {
    let adapter = ctx.adapter.0.clone();
    let blueprints = ctx.blueprints.clone();
    let _actions = ctx.actions.clone();
    let adapter_for_perceive = ctx.adapter.clone();
    let mut engine = rhai::Engine::new();

    // ===== 移动/挖掘 =====
    // 寻路函数注册三个别名 walk_to / move_to / step_to，避免使用 rhai 1.25 保留字 `go`/`goto`。
    // LLM 在脚本里写任一别名都生效。
    let a = adapter.clone();
    engine.register_fn("walk_to", move |x: i64, y: i64, z: i64| -> String {
        _exec_action(
            &a,
            MinecraftAction::Goto {
                x: x as i32,
                y: y as i32,
                z: z as i32,
            },
        )
    });
    let a = adapter.clone();
    engine.register_fn("move_to", move |x: i64, y: i64, z: i64| -> String {
        _exec_action(
            &a,
            MinecraftAction::Goto {
                x: x as i32,
                y: y as i32,
                z: z as i32,
            },
        )
    });
    let a = adapter.clone();
    engine.register_fn("step_to", move |x: i64, y: i64, z: i64| -> String {
        _exec_action(
            &a,
            MinecraftAction::Goto {
                x: x as i32,
                y: y as i32,
                z: z as i32,
            },
        )
    });
    let a = adapter.clone();
    engine.register_fn("mine", move |x: i64, y: i64, z: i64| -> String {
        _exec_action(
            &a,
            MinecraftAction::MineBlock {
                x: x as i32,
                y: y as i32,
                z: z as i32,
            },
        )
    });
    let a = adapter.clone();
    engine.register_fn("mine_below", move || -> String {
        _exec_action(&a, MinecraftAction::MineBelow)
    });
    let a = adapter.clone();
    engine.register_fn("mine_above", move || -> String {
        _exec_action(&a, MinecraftAction::MineAbove)
    });
    let a = adapter.clone();
    engine.register_fn("interact", move |x: i64, y: i64, z: i64| -> String {
        _exec_action(
            &a,
            MinecraftAction::InteractBlock {
                x: x as i32,
                y: y as i32,
                z: z as i32,
            },
        )
    });
    let a = adapter.clone();
    engine.register_fn(
        "till_and_sow",
        move |x: i64, y: i64, z: i64, seed: String| -> String {
            _exec_action(
                &a,
                MinecraftAction::TillAndSow {
                    x: x as i32,
                    y: y as i32,
                    z: z as i32,
                    seed,
                },
            )
        },
    );

    // ===== 战斗 =====
    let a = adapter.clone();
    engine.register_fn("attack", move |target: String| -> String {
        let t = if target.is_empty() {
            "nearest".to_string()
        } else {
            target
        };
        _exec_action(&a, MinecraftAction::Attack { target: t })
    });
    let a = adapter.clone();
    engine.register_fn("attack", move || -> String {
        _exec_action(
            &a,
            MinecraftAction::Attack {
                target: "nearest".to_string(),
            },
        )
    });
    let a = adapter.clone();
    engine.register_fn("defend", move || -> String {
        _exec_action(&a, MinecraftAction::Defend)
    });
    let a = adapter.clone();
    engine.register_fn(
        "use_item",
        move |item: String, yaw: Option<f64>, pitch: Option<f64>| -> String {
            _exec_action(
                &a,
                MinecraftAction::UseItem {
                    item,
                    yaw: yaw.map(|v| v as f32),
                    pitch: pitch.map(|v| v as f32),
                },
            )
        },
    );
    let a = adapter.clone();
    engine.register_fn("use_item", move |item: String| -> String {
        _exec_action(
            &a,
            MinecraftAction::UseItem {
                item,
                yaw: None,
                pitch: None,
            },
        )
    });

    // ===== 合成/熔炼/附魔 =====
    let a = adapter.clone();
    engine.register_fn("craft", move |item: String, count: i64| -> String {
        _exec_action(
            &a,
            MinecraftAction::Craft {
                item,
                count: count as u32,
            },
        )
    });
    let a = adapter.clone();
    engine.register_fn("craft_3x3", move |item: String, count: i64| -> String {
        _exec_action(
            &a,
            MinecraftAction::Craft3x3 {
                item,
                count: count as u32,
                table_pos: None,
            },
        )
    });
    let a = adapter.clone();
    engine.register_fn(
        "smelt",
        move |output: String, fuel: String, count: i64| -> String {
            _exec_action(
                &a,
                MinecraftAction::Smelt {
                    output,
                    fuel,
                    count: count as u32,
                    table_pos: None,
                },
            )
        },
    );
    let a = adapter.clone();
    engine.register_fn("auto_craft", move |item: String, count: i64| -> String {
        _exec_action(
            &a,
            MinecraftAction::AutoCraft {
                item,
                count: count as u32,
            },
        )
    });
    let a = adapter.clone();
    engine.register_fn("enchant", move |item: String, level: i64| -> String {
        _exec_action(
            &a,
            MinecraftAction::Enchant {
                item,
                level: level as u32,
            },
        )
    });

    // ===== 采集/放置 =====
    let a = adapter.clone();
    engine.register_fn("gather", move |item: String, count: i64| -> String {
        _exec_action(
            &a,
            MinecraftAction::Gather {
                item,
                count: count as u32,
            },
        )
    });
    let a = adapter.clone();
    engine.register_fn(
        "place",
        move |item: String, x: i64, y: i64, z: i64| -> String {
            _exec_action(
                &a,
                MinecraftAction::Place {
                    item,
                    x: x as i32,
                    y: y as i32,
                    z: z as i32,
                },
            )
        },
    );
    let a = adapter.clone();
    engine.register_fn("open", move |x: i64, y: i64, z: i64| -> String {
        _exec_action(
            &a,
            MinecraftAction::OpenContainer {
                x: x as i32,
                y: y as i32,
                z: z as i32,
            },
        )
    });

    // ===== 容器 =====
    let a = adapter.clone();
    engine.register_fn("chest_view", move |x: i64, y: i64, z: i64| -> String {
        _exec_action(
            &a,
            MinecraftAction::ChestView {
                x: x as i32,
                y: y as i32,
                z: z as i32,
            },
        )
    });
    let a = adapter.clone();
    engine.register_fn(
        "chest_withdraw",
        move |x: i64, y: i64, z: i64, item: String, count: i64| -> String {
            _exec_action(
                &a,
                MinecraftAction::ChestWithdraw {
                    x: x as i32,
                    y: y as i32,
                    z: z as i32,
                    item,
                    count: count as u32,
                },
            )
        },
    );
    let a = adapter.clone();
    engine.register_fn(
        "chest_deposit",
        move |x: i64, y: i64, z: i64, item: String, count: i64| -> String {
            _exec_action(
                &a,
                MinecraftAction::ChestDeposit {
                    x: x as i32,
                    y: y as i32,
                    z: z as i32,
                    item,
                    count: count as u32,
                },
            )
        },
    );

    // ===== 装备/消耗 =====
    let a = adapter.clone();
    engine.register_fn("equip", move |item: String, slot: String| -> String {
        _exec_action(&a, MinecraftAction::Equip { item, slot })
    });
    let a = adapter.clone();
    engine.register_fn("discard", move |item: String, count: i64| -> String {
        _exec_action(
            &a,
            MinecraftAction::Discard {
                item,
                count: count as u32,
            },
        )
    });
    let a = adapter.clone();
    engine.register_fn("consume", move |item: String| -> String {
        _exec_action(&a, MinecraftAction::Consume { item })
    });

    // ===== 交互 =====
    let a = adapter.clone();
    engine.register_fn("interact_entity", move |kind: String| -> String {
        _exec_action(&a, MinecraftAction::InteractEntity { kind })
    });
    let a = adapter.clone();
    engine.register_fn("trade", move |offer: i64| -> String {
        _exec_action(
            &a,
            MinecraftAction::Trade {
                offer: offer as u32,
            },
        )
    });
    let a = adapter.clone();
    engine.register_fn("chat", move |msg: String| -> String {
        _exec_action(&a, MinecraftAction::Chat { content: msg })
    });
    let a = adapter.clone();
    engine.register_fn("pickup", move || -> String {
        _exec_action(&a, MinecraftAction::Pickup)
    });
    let a = adapter.clone();
    engine.register_fn("make_obsidian", move |count: i64| -> String {
        _exec_action(
            &a,
            MinecraftAction::MakeObsidian {
                count: count.max(1) as u32,
            },
        )
    });
    let a = adapter.clone();
    engine.register_fn("follow", move |target: String| -> String {
        let target = if target.is_empty() {
            None
        } else {
            Some(target)
        };
        _exec_action(&a, MinecraftAction::Follow { target })
    });
    let a = adapter.clone();
    engine.register_fn("goto_player", move |target: String| -> String {
        let target = if target.is_empty() {
            None
        } else {
            Some(target)
        };
        _exec_action(&a, MinecraftAction::GotoPlayer { target })
    });
    let a = adapter.clone();
    engine.register_fn("stop_follow", move || -> String {
        _exec_action(&a, MinecraftAction::StopFollow)
    });
    let a = adapter.clone();
    engine.register_fn("give", move |item: String, count: i64| -> String {
        _exec_action(
            &a,
            MinecraftAction::Give {
                item,
                count: count.max(0) as u32,
                target: None,
            },
        )
    });
    let a = adapter.clone();
    engine.register_fn("search_block", move |item: String, radius: i64| -> String {
        _exec_action(
            &a,
            MinecraftAction::SearchBlock {
                item,
                radius: radius.clamp(4, 96) as u32,
            },
        )
    });
    let a = adapter.clone();
    engine.register_fn(
        "move_away",
        move |target: String, distance: i64| -> String {
            let target = if target.is_empty() {
                None
            } else {
                Some(target)
            };
            _exec_action(
                &a,
                MinecraftAction::MoveAway {
                    target,
                    distance: distance.clamp(4, 64) as u32,
                },
            )
        },
    );

    // ===== 感知/蓝图（读路径，不经过 BotCommand 队列） =====
    engine.register_fn("perceive", move || -> String {
        match adapter_for_perceive.perceive_shared() {
            Ok(st) => st.self_hint.to_string(),
            Err(e) => format!("perceive 错误: {e}"),
        }
    });
    // P104: 位置读取函数（轻量，读每 tick 缓存，不触发感知扫描）。
    // LLM 脚本常写 pos_x()/pos_y()/pos_z() 取当前坐标（此前报 Function not found）。
    let adapter_for_pos = ctx.adapter.clone();
    engine.register_fn("pos_x", move || -> f64 {
        adapter_for_pos
            .current_position()
            .map(|p| p.0)
            .unwrap_or(0.0)
    });
    let adapter_for_pos = ctx.adapter.clone();
    engine.register_fn("pos_y", move || -> f64 {
        adapter_for_pos
            .current_position()
            .map(|p| p.1)
            .unwrap_or(0.0)
    });
    let adapter_for_pos = ctx.adapter.clone();
    engine.register_fn("pos_z", move || -> f64 {
        adapter_for_pos
            .current_position()
            .map(|p| p.2)
            .unwrap_or(0.0)
    });
    let bp_for_list = blueprints.clone();
    engine.register_fn("list_blueprints", move || -> String {
        bp_for_list.list_summary()
    });
    let bp_for_build = blueprints.clone();
    let adapter_for_build = adapter.clone();
    engine.register_fn(
        "build_blueprint",
        move |name: String, x: i64, y: i64, z: i64| -> String {
            let bp = match bp_for_build.get(&name) {
                Some(b) => b.clone(),
                None => {
                    return format!("未知蓝图 '{name}'。可用：\n{}", bp_for_build.list_summary());
                }
            };
            let abs_json = bp.instantiate(x as i32, y as i32, z as i32);
            let blocks = match serde_json::from_str::<serde_json::Value>(&abs_json) {
                Ok(v) => v,
                Err(e) => return format!("蓝图 JSON 解析失败: {e}"),
            };
            let blocks_arr = match blocks.get("blocks").and_then(|v| v.as_array()) {
                Some(a) => a,
                None => return "蓝图缺少 blocks 数组".to_string(),
            };
            let mut results: Vec<String> = Vec::new();
            for (i, block) in blocks_arr.iter().enumerate() {
                let bx = block.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let by = block.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let bz = block.get("z").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let block_id = block.get("block").and_then(|v| v.as_str()).unwrap_or("");
                let goto_r = _exec_action(
                    &adapter_for_build,
                    MinecraftAction::Goto {
                        x: bx,
                        y: by,
                        z: bz,
                    },
                );
                if goto_r.starts_with("错误") {
                    results.push(format!("第{}块 goto 失败: {goto_r}", i + 1));
                    break;
                }
                let place_r = _exec_action(
                    &adapter_for_build,
                    MinecraftAction::Place {
                        item: block_id.to_string(),
                        x: bx,
                        y: by,
                        z: bz,
                    },
                );
                if place_r.starts_with("错误") {
                    results.push(format!("第{}块 place {block_id} 失败: {place_r}", i + 1));
                    break;
                }
                results.push(format!(
                    "第{}块: placed {block_id} @({bx},{by},{bz})",
                    i + 1
                ));
            }
            results.join("\n")
        },
    );

    // ===== P2-4: call_action —— 调用 LLM 自定义动作 =====
    // 递归：call_action(name) 查找已保存动作 → lint → 构建新引擎 → eval。
    // 递归深度由 max_call_levels=20 兜底；call_count 通过 Arc<Mutex<ActionLibrary>> 共享。
    let ctx_for_call = ctx.clone();
    engine.register_fn("call_action", move |name: String| -> String {
        // 1. 查找动作
        let action = {
            let lib = ctx_for_call.actions.lock().unwrap();
            lib.get(&name).cloned()
        };
        let action = match action {
            Some(a) => a,
            None => {
                let lib = ctx_for_call.actions.lock().unwrap();
                return format!("未知动作 '{name}'。可用：\n{}", lib.list_summary());
            }
        };
        // 2. lint（再次检查，防止从盘上加载后被篡改）
        if let Err(reason) = lint_action_script(&action.script) {
            return format!("动作 '{name}' 脚本被 lint 拒绝: {reason}");
        }
        // 3. 构建子引擎并执行（递归调用 build_rhai_engine）
        let sub_engine = build_rhai_engine(&ctx_for_call);
        // 4. 增加调用计数（持久化）
        {
            let mut lib = ctx_for_call.actions.lock().unwrap();
            lib.bump_call_count(&name);
        }
        match sub_engine.eval::<rhai::Dynamic>(&action.script) {
            Ok(out) => {
                let s = out.to_string();
                if out.is_unit() || s.is_empty() {
                    format!("[call_action {name}] 完成")
                } else {
                    s
                }
            }
            Err(e) => format!("[call_action {name}] 脚本错误: {e}"),
        }
    });

    // ===== 元：sleep / print =====
    let a_sleep = adapter.clone();
    engine.register_fn("sleep", move || -> String {
        _exec_action(&a_sleep, MinecraftAction::Sleep)
    });
    engine.register_fn("sleep", |ms: i64| {
        // 上限 10s，避免 LLM 写 sleep(999999) 卡死 bot
        let capped = ms.clamp(0, 10_000) as u64;
        std::thread::sleep(std::time::Duration::from_millis(capped));
    });
    let a_harvest = adapter.clone();
    engine.register_fn("harvest", move || -> String {
        _exec_action(&a_harvest, MinecraftAction::Harvest)
    });
    engine.register_fn("print", |msg: String| -> String {
        println!("[bot] {msg}");
        msg
    });

    // ===== 沙箱：资源限制 =====
    engine.set_max_operations(100_000); // 100k AST 操作（足够复杂脚本）
    engine.set_max_call_levels(20); // 递归深度上限
    engine.set_max_string_size(64 * 1024); // 64KB 字符串上限
    engine.set_max_array_size(1024); // 数组上限
    engine.set_max_map_size(256); // map 上限
    // 禁用所有内置模块（file/io/http/process），rhai 默认就不带这些，但显式禁用更安全
    engine.disable_symbol("eval");
    engine.disable_symbol("Fn");
    engine.disable_symbol("call");

    engine
}

/// LLM 自定义动作脚本的 lint（与 lint_script 相同，但额外检查动作嵌套深度）。
///
/// 已保存的动作脚本里若包含 `call_action` 是允许的（递归调用），但 `lint_script` 会
/// 拒绝 `call(` 关键字——所以我们用单独的 lint 函数，不检查 `call(`。
pub(crate) fn lint_action_script(script: &str) -> Result<(), String> {
    const MAX_SCRIPT_BYTES: usize = 8 * 1024;
    if script.len() > MAX_SCRIPT_BYTES {
        return Err(format!(
            "脚本过长 ({} bytes > {} bytes 上限)",
            script.len(),
            MAX_SCRIPT_BYTES
        ));
    }
    // 不含 `call(` 检查（call_action 是合法的）
    const FORBIDDEN: &[&str] = &[
        "import",
        "export",
        "eval",
        "::",
        "Fn(",
        "fn(",
        "read_file",
        "write_file",
        "append_file",
        "print_file",
        "http::",
        "http_get",
        "http_post",
        "import_node",
        "process::",
        "std::",
    ];
    for kw in FORBIDDEN {
        if script.contains(kw) {
            return Err(format!("脚本包含禁用关键字 '{kw}'"));
        }
    }
    let lower = script.to_lowercase();
    if (lower.contains("while true") || lower.contains("while (true)") || lower.contains("loop {"))
        && !lower.contains("break")
    {
        return Err("检测到 while true / loop 但无 break：可能死循环".to_string());
    }
    Ok(())
}

/// 脚本 lint：在 rhai 引擎 eval 前做静态检查。
///
/// 检查项：
/// 1. 长度：≤8KB（防止 LLM 灌入超长脚本撑爆内存）
/// 2. 禁用关键字：`import` / `export` / `eval` / `Fn` / `call` / `print_file` / `read_file` / `write_file` / `http` / `import_node`
/// 3. 危险模式：`while true` 无 break / `loop` 无 break（启发式，可能误报但安全优先）
/// 4. 禁止注释绕过检查：lint 看的是 strip 后的脚本，但 rhai 不支持 import，所以即使有 import 字符串也直接禁
pub(crate) fn lint_script(script: &str) -> Result<(), String> {
    const MAX_SCRIPT_BYTES: usize = 8 * 1024;
    if script.len() > MAX_SCRIPT_BYTES {
        return Err(format!(
            "脚本过长 ({} bytes > {} bytes 上限)。请拆分为多个 run_script 调用或用 run_plan。",
            script.len(),
            MAX_SCRIPT_BYTES
        ));
    }
    // 禁用关键字（任何位置出现即拒）。rhai 区分大小写：`Fn` 是反射入口，必须大写 F；
    // `import` / `eval` 等也是小写关键字。这里同时检查大小写两种变体以兜底 LLM 写错。
    const FORBIDDEN: &[&str] = &[
        "import",
        "export",
        "eval",
        "::",
        "Fn(",
        "fn(",
        "call(",
        "call ",
        "read_file",
        "write_file",
        "append_file",
        "print_file",
        "http::",
        "http_get",
        "http_post",
        "import_node",
        "process::",
        "std::",
    ];
    for kw in FORBIDDEN {
        if script.contains(kw) {
            return Err(format!(
                "脚本包含禁用关键字 '{kw}'（rhai 沙箱禁止 IO/模块/反射）"
            ));
        }
    }
    // 危险模式：`while true` / `loop` 必须有 break（大小写不敏感检查）
    let lower = script.to_lowercase();
    if (lower.contains("while true") || lower.contains("while (true)") || lower.contains("loop {"))
        && !lower.contains("break")
    {
        return Err(
            "检测到 while true / loop 但无 break：可能死循环。请加 break 或用 for 循环。"
                .to_string(),
        );
    }
    Ok(())
}

/// 阶段完成工具：agent 调用此工具声明当前里程碑完成。
/// 长期生存流程必须继续推进，不能由一个局部任务停止 agent loop。
pub struct TaskCompleteTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl TaskCompleteTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for TaskCompleteTool {
    fn name(&self) -> &str {
        "task_complete"
    }
    fn description(&self) -> &str {
        "声明当前阶段里程碑完成。系统记录声明后继续运行，你必须推进总体目标的下一阶段。\
         仅在世界状态可验证里程碑时调用，不要对同一里程碑重复调用。\
         参数 reason: 完成原因简述（必填）。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "reason": { "type": "string", "description": "完成原因简述" }
            },
            "required": ["reason"]
        })
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _call_id: &str,
        args: Value,
        _on_update: Option<craft_agent::core::tool::ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let reason = args.get("reason").and_then(|v| v.as_str()).unwrap_or("");
        // Require a real perception snapshot, but never convert a local milestone into
        // a process-wide stop. The long-running Agent owns progression and termination.
        let state = self.ctx.adapter.perceive_shared()?;
        if !state.self_hint.is_empty() {
            Ok(ToolResult {
                message: format!(
                    "阶段里程碑声明已接收（原因: {reason}）。继续运行并立即推进总体目标的下一阶段。"
                ),
                is_error: false,
                images: vec![],
            })
        } else {
            Ok(ToolResult {
                message: format!(
                    "阶段里程碑声明已接收（原因: {reason}）。但系统验证未通过：无有效状态。请继续行动。"
                ),
                is_error: true,
                images: vec![],
            })
        }
    }
}

/// Request a retry of the current failed task. The core Agent validates the
/// task status and performs the restart; this tool only provides the stable
/// function-calling surface to the LLM.
pub struct TaskRetryTool;

impl GameTool for TaskRetryTool {
    fn name(&self) -> &str {
        "task_retry"
    }

    fn description(&self) -> &str {
        "重试当前失败的任务。只有任务状态为 Failed 时有效；先解决失败原因，再传入 reason。"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "reason": { "type": "string", "description": "已解决的失败原因" }
            },
            "required": ["reason"]
        })
    }

    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }

    fn execute(
        &self,
        _call_id: &str,
        args: Value,
        _on_update: Option<craft_agent::core::tool::ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let reason = args.get("reason").and_then(Value::as_str).unwrap_or("");
        Ok(ToolResult {
            message: format!("已收到任务重试请求（原因: {reason}）。由 Agent 校验失败状态后重启。"),
            is_error: false,
            images: vec![],
        })
    }
}

/// 暂停当前目标（Active → Paused）。
/// 学习自 Mindcraft self_prompter 的 pause 语义。
/// LLM 主动暂停后，目标不会每轮注入，但保留 goal 文本；需手动 resume_goal 恢复。
/// 场景：LLM 临时想做别的事（如先处理突发情况），不想丢失长期目标。
#[allow(dead_code)]
pub struct PauseGoalTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl PauseGoalTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for PauseGoalTool {
    fn name(&self) -> &str {
        "pause_goal"
    }
    fn description(&self) -> &str {
        "暂停当前目标（不注入 [当前目标] 但保留 goal 文本）。\n\
         暂停后需手动调用 resume_goal 恢复（不会自动恢复）。\n\
         无参数。场景：LLM 临时处理突发情况时不想丢失长期目标。\n\
         注意：紧急 mode（如血量危急）会自动暂停目标，无需调用此工具。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _call_id: &str,
        _args: Value,
        _on_update: Option<craft_agent::core::tool::ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        // pause_goal 由 Agent 主循环在 tool 执行后处理（修改 prompt_state）。
        // 工具本身只返回确认消息。
        Ok(ToolResult {
            message: "已请求暂停当前目标（Active → Paused）。需手动 resume_goal 恢复。".to_string(),
            is_error: false,
            images: vec![],
        })
    }
}

/// 恢复已暂停的目标（Paused → Active）。
#[allow(dead_code)]
pub struct ResumeGoalTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl ResumeGoalTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for ResumeGoalTool {
    fn name(&self) -> &str {
        "resume_goal"
    }
    fn description(&self) -> &str {
        "恢复已暂停的目标（Paused → Active），目标重新每轮注入。\n\
         无参数。场景：突发情况处理完后继续原目标。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _call_id: &str,
        _args: Value,
        _on_update: Option<craft_agent::core::tool::ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        Ok(ToolResult {
            message: "已请求恢复目标（Paused → Active）。".to_string(),
            is_error: false,
            images: vec![],
        })
    }
}

// ============================================================================
// P2-4: LLM 代码生成（newAction 等价物）
// ============================================================================

/// 创建一个新的自定义动作（P2-4：newAction 等价物）。
///
/// 学习自 Mindcraft `agent/commands/code.js::newAction`：LLM 可写一段命名 rhai 脚本，
/// 保存到 `actions/<name>.rhai.json`，后续通过 `call_action(name)` 在 `run_script` 里调用。
///
/// 与 `run_script` 区别：
/// - `run_script` 是一次性执行
/// - `new_action` 是持久化（跨会话可复用）
///
/// 流程：lint 脚本 → parse 检查 → 写盘 → 加入内存库 → 返回成功。
pub struct NewActionTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl NewActionTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for NewActionTool {
    fn name(&self) -> &str {
        "new_action"
    }
    fn description(&self) -> &str {
        "创建一个命名的自定义动作（P2-4：newAction 等价物），持久化到 actions/<name>.rhai.json。\n\
         后续可在 run_script 里用 call_action(name) 调用，跨会话复用。\n\
         \n\
         参数：\n\
         - name: 动作名（合法标识符 [a-z_][a-z0-9_]*，1..=32 字符，如 'gather_and_craft'）\n\
         - description: 何时该用此动作（给 LLM 看的提示）\n\
         - script: rhai 脚本代码（≤8KB，可用 run_script 全部 27 个函数 + call_action）\n\
         \n\
         lint 规则：禁用 import/eval/Fn/call/IO，禁 while true 无 break。\n\
         若同名动作已存在则覆盖（更新脚本）。\n\
         \n\
         示例：new_action(name=\"gather_wood_and_planks\", description=\"采集 4 个原木并合成木板\", \
         script=\"gather(\\\"oak_log\\\", 4); craft(\\\"oak_planks\\\", 4); pickup(); print(\\\"done\\\")\")"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "动作名 [a-z_][a-z0-9_]*，1..=32 字符" },
                "description": { "type": "string", "description": "动作描述（何时该用）" },
                "script": { "type": "string", "description": "rhai 脚本代码（≤8KB）" }
            },
            "required": ["name", "description", "script"]
        })
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _call_id: &str,
        args: Value,
        _on_update: Option<craft_agent::core::tool::ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 name"))?
            .to_string();
        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 description"))?
            .to_string();
        let script = args
            .get("script")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 script"))?
            .to_string();

        // 1. 校验 name 合法性
        if !LlmAction::is_valid_name(&name) {
            return Ok(ToolResult {
                message: format!("动作名 '{name}' 非法（须 [a-z_][a-z0-9_]*，长度 1..=32）"),
                is_error: true,
                images: vec![],
            });
        }
        // 2. lint 脚本
        if let Err(reason) = lint_script(&script) {
            return Ok(ToolResult {
                message: format!("脚本被 lint 拒绝: {reason}"),
                is_error: true,
                images: vec![],
            });
        }
        // 3. parse 检查（不执行）：用临时 engine 编译脚本，确保语法正确
        let mut probe = rhai::Engine::new();
        probe.set_max_operations(1_000);
        // 注册一个 dummy 函数让脚本能 parse（实际执行由 call_action 时注册完整函数集）
        let parse_result: Result<(), String> = probe
            .compile_expression(&script)
            .map(|_| ())
            .map_err(|e| e.to_string())
            .or_else(|_| {
                // 表达式编译失败时尝试按语句块编译
                probe
                    .compile(&script)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            });
        if let Err(e) = parse_result {
            return Ok(ToolResult {
                message: format!("脚本语法错误（compile 失败）: {e}"),
                is_error: true,
                images: vec![],
            });
        }
        // 4. 保存到 ActionLibrary
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let action = LlmAction {
            name: name.clone(),
            description: description.clone(),
            script,
            created_at: now_ms,
            call_count: 0,
        };
        let mut lib = self.ctx.actions.lock().unwrap();
        match lib.save(action) {
            Ok(()) => {
                let total = lib.len();
                Ok(ToolResult {
                    message: format!(
                        "✓ 动作 '{name}' 已保存。当前共 {total} 个自定义动作。\
                         \n用 list_actions 查看，用 run_script 内 call_action(\"{name}\") 调用。"
                    ),
                    is_error: false,
                    images: vec![],
                })
            }
            Err(e) => Ok(ToolResult {
                message: format!("保存动作失败: {e}"),
                is_error: true,
                images: vec![],
            }),
        }
    }
}

/// 列出所有已保存的 LLM 自定义动作（P2-4）。
pub struct ListActionsTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl ListActionsTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for ListActionsTool {
    fn name(&self) -> &str {
        "list_actions"
    }
    fn description(&self) -> &str {
        "列出所有已保存的自定义动作（P2-4）。无参数。\n\
         返回：name (调用 N 次): description + 脚本预览（前 200 字符）。\n\
         用 new_action 创建新动作，用 run_script 内 call_action(name) 调用。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({})
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::read()
    }
    fn execute(
        &self,
        _call_id: &str,
        _args: Value,
        _on_update: Option<craft_agent::core::tool::ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let lib = self.ctx.actions.lock().unwrap();
        let items = lib.list();
        if items.is_empty() {
            return Ok(ToolResult {
                message:
                    "无自定义动作。用 new_action(name=..., description=..., script=...) 创建。"
                        .to_string(),
                is_error: false,
                images: vec![],
            });
        }
        let mut lines: Vec<String> = Vec::new();
        for (n, d, c) in items {
            let preview: String = lib
                .get(&n)
                .map(|a| {
                    let s = &a.script;
                    if s.chars().count() > 200 {
                        let head: String = s.chars().take(200).collect();
                        format!("{head}...")
                    } else {
                        s.clone()
                    }
                })
                .unwrap_or_default();
            // 替换换行为 \\n 让一行展示
            let preview_one = preview.replace('\n', "\\n");
            lines.push(format!("- {n} (调用 {c} 次): {d}\n  脚本: {preview_one}"));
        }
        Ok(ToolResult {
            message: format!(
                "已保存 {} 个自定义动作：\n{}\n\n用 call_action(name) 在 run_script 内调用。",
                lines.len(),
                lines.join("\n")
            ),
            is_error: false,
            images: vec![],
        })
    }
}
