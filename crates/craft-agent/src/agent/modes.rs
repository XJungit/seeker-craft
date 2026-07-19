use crate::core::message::Message;

use super::Agent;

impl Agent {
    pub fn check_modes(&mut self) -> Option<String> {
        let perception = self.messages.iter().rev().find_map(|m| match m {
            Message::User(u) if u.content.starts_with("【当前游戏状态") => {
                Some(u.content.as_str())
            }
            _ => None,
        })?;

        let health_low = (0..=5).any(|n| perception.contains(&format!("Health: {n}/")));
        let hunger_low = (0..=5).any(|n| perception.contains(&format!("Hunger: {n}/")));
        if health_low || hunger_low {
            if self.last_mode_trigger != 1 {
                self.last_mode_trigger = 1;
                let action = if health_low {
                    "血量危急！立即 combat(\"retreat\",100) 撤退，然后 consume 食物恢复。"
                } else {
                    "饥饿危急！立即 consume 食物（检查 HOTBAR 找食物）。"
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
                    "苦力怕靠近！立即 combat(\"kite\",200) 风筝攻击，保持距离。"
                } else {
                    "敌对生物靠近！立即 combat(\"melee\",200) 近战攻击。"
                };
                return Some(format!("[MODE: self_defense] {action}"));
            }
            return None;
        }

        if self.obs_streak >= 5 && self.last_mode_trigger != 3 {
            self.last_mode_trigger = 3;
            return Some(format!(
                "[MODE: unstuck] 已连续 {} 步纯观察！选一个完全不同的工具立即行动：collect, craft, build, combat, move_to — 不要再用 perceive/look。",
                self.obs_streak
            ));
        }

        self.last_mode_trigger = 0;
        None
    }
}
