//! 模式反应系统（学习自 Mindcraft 的 10 个 modes）。
//!
//! 三个层次：
//! 1. **建议**：注入 `[MODE: ...]` user 消息给 LLM（原实现）
//! 2. **强制动作**：直接通过 ActionManager 提交高优先级命令（self_preservation 等）
//! 3. **重 prompt**：紧急情况（如血量危急）设置 `force_reprompt` 标志，
//!    让 agent 主循环立即跳过后续步骤再跑一轮 LLM，不等下一轮
//!
//! 10 个 modes（对齐 profile.rs 的 Modes struct）：
//! - self_preservation：生命/饱食危急 → 建议避险 + 强制脱困
//! - self_defense：敌对生物靠近 → 建议攻击
//! - unstuck：连续观察 5+ 步 → 建议换工具
//! - cowardice：见敌对生物就逃跑（与 self_defense 互斥，优先）
//! - hunting：主动狩猎附近动物获取食物
//! - item_collecting：挖矿/战斗后主动捡掉落物
//! - torch_placing：黑暗处自动放火把
//! - elbow_room：周围太挤时自动腾空间
//! - idle_staring：空闲时四处看（自然感）
//! - cheat：创造模式作弊（飞行/瞬移/给物品）

use crate::core::message::Message;

use super::Agent;

/// Mode 触发结果（re-prompt 通道）。
#[derive(Debug, Clone)]
pub struct ModeReaction {
    /// 注入给 LLM 的提示文本（None 表示不注入）
    pub prompt: Option<String>,
    /// 是否强制立即重新跑一轮 LLM（不等下一轮 turn）
    pub force_reprompt: bool,
    /// 标识是否产生了 mode 触发（用于 last_mode_trigger 去重）
    pub mode_id: u32,
}

impl Agent {
    /// 模式反应：每轮检查 perceive 文本，返回反应结果。
    ///
    /// 返回 `ModeReaction`：
    /// - `prompt=Some, force_reprompt=false`：仅注入提示（原行为）
    /// - `prompt=Some, force_reprompt=true`：注入提示 + 主循环立即重跑 LLM
    /// - `prompt=None`：无 mode 触发
    ///
    /// **去重机制**：同一 mode_id 连续触发只注入一次（避免每轮重复唠叨）。
    /// 不同 mode_id 触发时重置去重。
    pub fn check_modes(&mut self) -> Option<ModeReaction> {
        let perception = self.messages.iter().rev().find_map(|m| match m {
            Message::User(u) if u.content.starts_with("【当前游戏状态") => {
                Some(u.content.as_str())
            }
            _ => None,
        })?;

        // 模式开关从 AgentConfig.modes 读取（Modes struct 已定义 10 个开关）。
        // 每个模式触发前检查对应开关，关闭的模式不触发。

        // 生命/饱食检测：perceive 输出格式 "生命: 5/20  饱食: 20/20"
        let health_low = (0..=6).any(|n| perception.contains(&format!("生命: {n}/")));
        let hunger_low = (0..=6).any(|n| perception.contains(&format!("饱食: {n}/")));
        if (health_low || hunger_low) && self.config.modes.self_preservation {
            if self.last_mode_trigger != 1 {
                self.last_mode_trigger = 1;
                let action = if health_low {
                    "血量危急！立即 goto 远离危险区域，饱食度够的话原地等待回血。\
                     若附近有敌对生物用 attack(target=\"zombie\") 清理威胁后撤退。"
                } else {
                    "饥饿危急！检查背包是否有食物（cooked_beef/bread/apple 等），\
                     若有就找安全地方停下等饱食度回血。没食物就 attack 附近动物获取肉。"
                };
                return Some(ModeReaction {
                    prompt: Some(format!("[MODE: self_preservation] {action}")),
                    // 血量危急时强制重 prompt，让 LLM 立即响应
                    force_reprompt: health_low,
                    mode_id: 1,
                });
            }
            return None;
        }

        let has_hostile = perception.contains("zombie")
            || perception.contains("skeleton")
            || perception.contains("creeper")
            || perception.contains("spider")
            || perception.contains("phantom")
            || perception.contains("witch");
        let has_creeper = perception.contains("creeper");
        if has_hostile && self.config.modes.self_defense {
            if self.last_mode_trigger != 2 {
                self.last_mode_trigger = 2;
                let action = if has_creeper {
                    "苦力怕靠近！立即 goto 拉开距离（至少 5 格），\
                     等它冷却后再 attack(target=\"creeper\") 攻击。不要原地硬扛。"
                } else {
                    "敌对生物靠近！立即 attack(target=\"zombie\") 攻击最近敌对生物。\
                     血量低时先 goto 撤退到安全区域。"
                };
                return Some(ModeReaction {
                    prompt: Some(format!("[MODE: self_defense] {action}")),
                    // 敌对生物靠近时强制重 prompt
                    force_reprompt: true,
                    mode_id: 2,
                });
            }
            return None;
        }

        // hunting：附近有动物（非敌对、非玩家）且背包食物不足
        let has_animal = perception.contains("cow")
            || perception.contains("pig")
            || perception.contains("sheep")
            || perception.contains("chicken")
            || perception.contains("rabbit");
        let low_food = perception.contains("背包: [")
            && !perception.contains("cooked_beef")
            && !perception.contains("bread")
            && !perception.contains("apple");
        if has_animal && low_food && self.config.modes.hunting && self.last_mode_trigger != 4 {
            self.last_mode_trigger = 4;
            return Some(ModeReaction {
                prompt: Some(
                    "[MODE: hunting] 附近有动物且背包无食物。\
                     attack 附近动物（cow/pig/sheep）获取肉，\
                     再用 smelt(output=\"cooked_beef\", fuel=\"coal\", count=4) 熟食。"
                        .to_string(),
                ),
                force_reprompt: false,
                mode_id: 4,
            });
        }

        // item_collecting：背包刚挖/刚战斗后提示捡物
        // 简化判断：最近一轮 mine/attack 后 perceive 提示背包未满
        let just_mined = perception.contains("脚下: air") || perception.contains("前方: air");
        if just_mined && self.config.modes.item_collecting && self.last_mode_trigger != 5 {
            self.last_mode_trigger = 5;
            return Some(ModeReaction {
                prompt: Some(
                    "[MODE: item_collecting] 刚挖完方块，掉落物可能散落在地。\
                     调用 pickup 捡起附近掉落物，避免\"挖了 8 个但只捡到 3 个\"。"
                        .to_string(),
                ),
                force_reprompt: false,
                mode_id: 5,
            });
        }

        // torch_placing：黑暗环境提示放火把
        // perceive 输出未直接含光照信息，但若 bot 在洞穴/夜晚会显示 biome=deepslate 或时间
        let in_dark = perception.contains("biome: deep")
            || perception.contains("biome: cave")
            || perception.contains("biome: dripstone")
            || perception.contains("biome: lush");
        if in_dark && self.config.modes.torch_placing && self.last_mode_trigger != 6 {
            self.last_mode_trigger = 6;
            return Some(ModeReaction {
                prompt: Some(
                    "[MODE: torch_placing] 当前在黑暗环境（洞穴/深岩）。\
                     若背包有 torch 或 coal+stick，craft(torch, count=8) 后 place 在脚下照明，\
                     防止刷怪。"
                        .to_string(),
                ),
                force_reprompt: false,
                mode_id: 6,
            });
        }

        // unstuck：连续观察 5+ 步，提示换工具
        if self.obs_streak >= 5 && self.config.modes.unstuck && self.last_mode_trigger != 3 {
            self.last_mode_trigger = 3;
            return Some(ModeReaction {
                prompt: Some(format!(
                    "[MODE: unstuck] 已连续 {} 步纯观察！立即选一个真实工具行动：\
                     goto / gather / mine / craft / attack / build。\
                     **禁止**再用 perceive，**禁止**在文字里写 tool() 伪调用。",
                    self.obs_streak
                )),
                // 死循环风险时强制重 prompt
                force_reprompt: self.obs_streak >= 8,
                mode_id: 3,
            });
        }

        self.last_mode_trigger = 0;
        None
    }

    /// 兼容旧 API：返回 prompt 字符串（无 force_reprompt 信息）。
    /// 新代码应使用 `check_modes()` 返回的 `ModeReaction`。
    pub fn check_modes_legacy(&mut self) -> Option<String> {
        self.check_modes().and_then(|r| r.prompt)
    }
}

#[cfg(test)]
mod tests {
    use super::Agent;
    use crate::agent::LlmProvider;
    use crate::core::message::Message;

    fn make_agent(perception: &str) -> Agent {
        let mut agent = Agent::new(
            Box::new(MockProvider),
            crate::core::tool::ToolRegistry::new(),
            crate::agent::AgentConfig::new("test".into(), 5),
        );
        agent.messages.push(Message::user(format!(
            "【当前游戏状态（自动注入）】\n{perception}"
        )));
        agent
    }

    /// 固定返回停止的 mock provider（不触发任何工具调用）
    struct MockProvider;
    impl LlmProvider for MockProvider {
        fn complete(
            &self,
            _messages: &[serde_json::Value],
            _tools: &[serde_json::Value],
        ) -> anyhow::Result<crate::core::message::AssistantResponse> {
            Ok(crate::core::message::AssistantResponse {
                content: Some("ok".into()),
                reasoning: None,
                tool_calls: vec![],
                usage: crate::core::message::Usage::default(),
                stop_reason: crate::core::message::StopReason::Stop,
            })
        }
    }

    // ── self_preservation ──

    #[test]
    fn mode_self_preservation_health_low() {
        let mut agent = make_agent("生命: 4/20  饱食: 18/20  位置: 0,64,0");
        let r = agent
            .check_modes()
            .expect("should trigger self_preservation");
        assert!(
            r.prompt.unwrap().contains("self_preservation"),
            "should mention mode"
        );
        assert!(r.force_reprompt, "health low should force reprompt");
        assert_eq!(r.mode_id, 1);
    }

    #[test]
    fn mode_self_preservation_hunger_low() {
        let mut agent = make_agent("生命: 18/20  饱食: 3/20  位置: 0,64,0");
        let r = agent
            .check_modes()
            .expect("should trigger self_preservation");
        assert!(r.prompt.unwrap().contains("self_preservation"));
        assert!(!r.force_reprompt, "hunger low should not force reprompt");
        assert_eq!(r.mode_id, 1);
    }

    #[test]
    fn mode_self_preservation_healthy() {
        let mut agent = make_agent("生命: 20/20  饱食: 20/20  位置: 0,64,0");
        let r = agent.check_modes();
        assert!(r.is_none(), "healthy should not trigger self_preservation");
    }

    // ── self_defense ──

    #[test]
    fn mode_self_defense_zombie() {
        let mut agent = make_agent("生命: 20/20  饱食: 20/20  敌对: zombie[距离: 3]");
        let r = agent.check_modes().expect("should trigger self_defense");
        assert!(r.prompt.unwrap().contains("self_defense"));
        assert!(r.force_reprompt, "hostile should force reprompt");
        assert_eq!(r.mode_id, 2);
    }

    #[test]
    fn mode_self_defense_creeper() {
        let mut agent = make_agent("生命: 20/20  饱食: 20/20  敌对: creeper[距离: 2]");
        let r = agent.check_modes().expect("should trigger self_defense");
        let prompt = r.prompt.unwrap();
        assert!(prompt.contains("self_defense"));
        assert!(
            prompt.contains("拉开距离"),
            "creeper should suggest distance"
        );
        assert_eq!(r.mode_id, 2);
    }

    #[test]
    fn mode_self_defense_no_hostile() {
        let mut agent = make_agent("生命: 20/20  饱食: 20/20  动物: cow[距离: 5]");
        let r = agent.check_modes();
        assert!(r.is_none(), "no hostile should not trigger self_defense");
    }

    // ── hunting ──

    #[test]
    fn mode_hunting_animal_and_no_food() {
        let mut agent =
            make_agent("生命: 20/20  饱食: 20/20  animal: cow[距离: 5] 背包: [oak_log:4, dirt:2]");
        let r = agent.check_modes().expect("should trigger hunting");
        assert!(r.prompt.unwrap().contains("hunting"));
        assert_eq!(r.mode_id, 4);
    }

    #[test]
    fn mode_hunting_has_food_skips() {
        let mut agent = make_agent("动物: cow[距离: 5]  背包: [cooked_beef:4]");
        let r = agent.check_modes();
        assert!(r.is_none(), "already has food should not trigger hunting");
    }

    // ── item_collecting ──

    #[test]
    fn mode_item_collecting_just_mined() {
        let mut agent = make_agent("生命: 20/20  脚下: air  前方: stone");
        let r = agent.check_modes().expect("should trigger item_collecting");
        assert!(r.prompt.unwrap().contains("item_collecting"));
        assert_eq!(r.mode_id, 5);
    }

    // ── torch_placing ──

    #[test]
    fn mode_torch_placing_in_dark_biome() {
        let mut agent = make_agent("生命: 20/20  位置: 0,20,0  群系: biome: deepslate");
        let r = agent.check_modes().expect("should trigger torch_placing");
        assert!(r.prompt.unwrap().contains("torch_placing"));
        assert_eq!(r.mode_id, 6);
    }

    #[test]
    fn mode_torch_placing_in_cave() {
        let mut agent = make_agent("位置: 0,20,0  群系: biome: dripstone");
        let r = agent.check_modes().expect("should trigger torch_placing");
        assert!(r.prompt.unwrap().contains("torch_placing"));
        assert_eq!(r.mode_id, 6);
    }

    #[test]
    fn mode_torch_placing_surface_skips() {
        let mut agent = make_agent("位置: 0,64,0  群系: biome: plains");
        let r = agent.check_modes();
        assert!(
            r.is_none(),
            "surface biome should not trigger torch_placing"
        );
    }

    // ── unstuck ──

    #[test]
    fn mode_unstuck_obs_streak_5() {
        let mut agent = make_agent("生命: 20/20  位置: 0,64,0  群系: biome: plains");
        agent.obs_streak = 5;
        let r = agent.check_modes().expect("should trigger unstuck");
        assert!(r.prompt.unwrap().contains("unstuck"));
        assert!(!r.force_reprompt, "obs_streak=5 should not force reprompt");
        assert_eq!(r.mode_id, 3);
    }

    #[test]
    fn mode_unstuck_obs_streak_8_force_reprompt() {
        let mut agent = make_agent("生命: 20/20  位置: 0,64,0  群系: biome: plains");
        agent.obs_streak = 8;
        let r = agent.check_modes().expect("should trigger unstuck");
        assert!(r.force_reprompt, "obs_streak>=8 should force reprompt");
        assert_eq!(r.mode_id, 3);
    }

    #[test]
    fn mode_unstuck_obs_streak_3_skips() {
        let mut agent = make_agent("生命: 20/20  位置: 0,64,0");
        agent.obs_streak = 3;
        let r = agent.check_modes();
        assert!(r.is_none(), "obs_streak=3 should not trigger unstuck");
    }

    // ── 去重机制 ──

    #[test]
    fn mode_dedup_same_mode_id_does_not_retrigger() {
        let mut agent = make_agent("生命: 4/20  饱食: 18/20");
        // 第一轮触发
        let r1 = agent.check_modes();
        assert!(r1.is_some(), "first trigger should fire");
        // 第二轮相同状态：不应再触发
        let r2 = agent.check_modes();
        assert!(
            r2.is_none(),
            "same mode should not trigger twice consecutively"
        );
    }

    #[test]
    fn mode_dedup_different_mode_id_resets() {
        // 先触发 hunting（mode_id=4），然后清空背包/动物，换为 hostile 触发 self_defense（mode_id=2）
        let mut agent =
            make_agent("生命: 20/20  饱食: 20/20  animal: cow[距离: 5] 背包: [oak_log:4]");
        // 第一轮：hunting 触发（mode_id=4）
        let r1 = agent.check_modes();
        assert!(r1.is_some());
        assert_eq!(r1.unwrap().mode_id, 4, "first trigger should be hunting");
        // 换成 hostile 场景（clear 之前的 perceive，加敌对生物）
        agent.messages.clear();
        agent.messages.push(Message::user(
            "【当前游戏状态（自动注入）】\n生命: 20/20  饱食: 20/20  敌对: zombie[距离: 3]",
        ));
        // 第二轮：self_defense 应触发，因为 mode_id 从 4 变 2
        let r2 = agent.check_modes();
        assert!(r2.is_some(), "mode_id change should retrigger");
        assert_eq!(r2.unwrap().mode_id, 2, "should now trigger defense");
    }

    // ── 无 perceive 消息时 ──

    #[test]
    fn mode_no_perception_returns_none() {
        let mut agent = Agent::new(
            Box::new(MockProvider),
            crate::core::tool::ToolRegistry::new(),
            crate::agent::AgentConfig::new("test".into(), 5),
        );
        let r = agent.check_modes();
        assert!(r.is_none(), "no perception msg should return None");
    }

    // ── 优先级测试：self_preservation > self_defense > unstuck ──

    #[test]
    fn mode_priority_health_over_hostile() {
        // 同时有生命低和敌对生物，应触发 self_preservation 而非 self_defense
        let mut agent = make_agent("生命: 4/20  饱食: 20/20  敌对: zombie[距离: 3]");
        let r = agent.check_modes().expect("should trigger some mode");
        assert_eq!(r.mode_id, 1, "health low should take priority over hostile");
        assert!(r.force_reprompt, "health low should force reprompt");
    }
}
