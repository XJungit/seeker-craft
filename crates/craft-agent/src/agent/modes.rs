use crate::core::message::Message;

use super::Agent;

impl Agent {
    /// 模式反应：每轮检查 perceive 文本，注入提示性 `[MODE: ...]` user 消息给 LLM。
    /// **不直接执行动作**（动作执行由 handler 层 Tick 处理，见 azalea/mod.rs）。
    ///
    /// 关键修复：
    /// 1. 字符串匹配改为中文（perceive 输出是中文 "生命: 5/20" 不是英文 "Health: 5/20"）
    /// 2. 工具名改为真实工具（attack/goto/gather/craft，不是虚构的 combat/consume/nav_to）
    /// 3. unstuck 模式不再列虚构工具名
    pub fn check_modes(&mut self) -> Option<String> {
        let perception = self.messages.iter().rev().find_map(|m| match m {
            Message::User(u) if u.content.starts_with("【当前游戏状态") => {
                Some(u.content.as_str())
            }
            _ => None,
        })?;

        // 生命/饱食检测：perceive 输出格式 "生命: 5/20  饱食: 20/20"
        // 旧代码匹配英文 "Health: 5/" 永远不触发——已修。
        let health_low = (0..=6).any(|n| perception.contains(&format!("生命: {n}/")));
        let hunger_low = (0..=6).any(|n| perception.contains(&format!("饱食: {n}/")));
        if health_low || hunger_low {
            if self.last_mode_trigger != 1 {
                self.last_mode_trigger = 1;
                let action = if health_low {
                    "血量危急！立即 goto 远离危险区域，饱食度够的话原地等待回血。\
                     若附近有敌对生物用 attack(target=\"zombie\") 清理威胁后撤退。"
                } else {
                    "饥饿危急！检查背包是否有食物（cooked_beef/bread/apple 等），\
                     若有就找安全地方停下等饱食度回血。没食物就 attack 附近动物获取肉。"
                };
                return Some(format!("[MODE: self_preservation] {action}"));
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
        if has_hostile {
            if self.last_mode_trigger != 2 {
                self.last_mode_trigger = 2;
                let action = if has_creeper {
                    "苦力怕靠近！立即 goto 拉开距离（至少 5 格），\
                     等它冷却后再 attack(target=\"creeper\") 攻击。不要原地硬扛。"
                } else {
                    "敌对生物靠近！立即 attack(target=\"zombie\") 攻击最近敌对生物。\
                     血量低时先 goto 撤退到安全区域。"
                };
                return Some(format!("[MODE: self_defense] {action}"));
            }
            return None;
        }

        // unstuck：连续观察 5+ 步，提示换工具。**只列真实工具名**。
        // 旧代码列了 nav_to/collect/combat 等虚构工具，LLM 跟着学写伪调用——已修。
        if self.obs_streak >= 5 && self.last_mode_trigger != 3 {
            self.last_mode_trigger = 3;
            return Some(format!(
                "[MODE: unstuck] 已连续 {} 步纯观察！立即选一个真实工具行动：\
                 goto / gather / mine / craft / attack / build。\
                 **禁止**再用 perceive，**禁止**在文字里写 tool() 伪调用。",
                self.obs_streak
            ));
        }

        self.last_mode_trigger = 0;
        None
    }
}
