//! 提示词组装 — 酒馆 PromptManager 风格的五层 prompt + World Info 动态注入
//!
//! 参考 SillyTavern PromptManager.js (12 层, 我们取核心 5 层):
//!   main → worldInfoBefore → charDescription → examples → jailbreak
//!
//! 参考 SillyTavern world-info.js (关键词触发 + 7 种注入位置):
//!   perceive 结果 → WorldInfo 条目 → 匹配触发 → 注入到 scenario 位置

use serde::{Deserialize, Serialize};

/// 五层 prompt 组装器 (酒馆 PromptManager 风格)
///
/// 分层原则:
/// 1. identity — 1 句话身份 (替代酒馆的 main prompt)
/// 2. role_desc — 角色描述/偏好 (替代酒馆的 charDescription + charPersonality)
/// 3. scenario — 动态场景 (替代酒馆的 scenario, 每轮可更新)
/// 4. examples — 示例对话 (替代酒馆的 dialogueExamples, 比规则更有效)
/// 5. jailbreak — 后置强制指令 (替代酒馆的 jailbreak)
#[derive(Debug, Clone)]
pub struct PromptBuilder {
    /// 1. 身份: "你是 Minecraft AI 玩家"
    pub identity: String,
    /// 2. 角色描述: "你擅长采集资源, 优先挖树"
    pub role_desc: String,
    /// 3. 动态场景 (每轮 perceive 后更新)
    pub scenario: String,
    /// 4. 示例对话 (最重要的行为塑造手段)
    pub examples: Vec<String>,
    /// 5. 后置指令: "不要问问题。直接行动。"
    pub jailbreak: String,
}

impl PromptBuilder {
    pub fn new() -> Self {
        Self {
            identity: String::new(),
            role_desc: String::new(),
            scenario: String::new(),
            examples: Vec::new(),
            jailbreak: String::new(),
        }
    }

    /// 设置身份
    pub fn identity(mut self, text: impl Into<String>) -> Self {
        self.identity = text.into();
        self
    }

    /// 设置角色描述
    pub fn role_desc(mut self, text: impl Into<String>) -> Self {
        self.role_desc = text.into();
        self
    }

    /// 更新动态场景 (每轮调用)
    pub fn set_scenario(&mut self, text: impl Into<String>) {
        self.scenario = text.into();
    }

    /// 添加一条示例
    pub fn add_example(mut self, text: impl Into<String>) -> Self {
        self.examples.push(text.into());
        self
    }

    /// 设置后置指令
    pub fn jailbreak(mut self, text: impl Into<String>) -> Self {
        self.jailbreak = text.into();
        self
    }

    /// 组装为最终 system prompt 字符串
    ///
    /// 酒馆的组装顺序: identity → role → scenario → examples → jailbreak
    pub fn build(&self) -> String {
        let mut parts = Vec::new();

        if !self.identity.is_empty() {
            parts.push(self.identity.clone());
        }
        if !self.role_desc.is_empty() {
            parts.push(self.role_desc.clone());
        }
        if !self.scenario.is_empty() {
            parts.push(format!("[当前场景]\n{}", self.scenario));
        }
        if !self.examples.is_empty() {
            let examples = self
                .examples
                .iter()
                .map(|e| format!("- {}", e))
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!("[行为示例]\n{}", examples));
        }
        if !self.jailbreak.is_empty() {
            parts.push(self.jailbreak.clone());
        }

        parts.join("\n\n")
    }
}

impl Default for PromptBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ── World Info 动态注入 (酒馆 world-info.js 模式) ──

/// World Info 条目: 按关键词触发, 动态注入到上下文
///
/// 酒馆的 WI 支持 sticky/cooldown/delay + 7 种注入位置。
/// 我们取最核心的模式: 关键词触发 → 注入到 scenario 位置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldInfo {
    /// 稳定标识，供 `remove_by_id` 精确删除。add 时由调用方提供。
    pub id: Option<String>,
    /// 触发关键词 (都转小写匹配)；空列表表示常驻。
    pub keys: Vec<String>,
    /// 注入内容模板 (支持 {label} {offset_x} {offset_y} 变量)
    pub template: String,
    /// 数值越大越优先，预算不足时优先保留。
    pub priority: i32,
}

impl WorldInfo {
    pub fn new(keys: Vec<String>, template: impl Into<String>) -> Self {
        Self {
            id: None,
            keys: keys.into_iter().map(|k| k.to_lowercase()).collect(),
            template: template.into(),
            priority: 0,
        }
    }

    /// 设置稳定 id，便于日后按 id 删除（推荐每次 add 都给）。
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// 检查给定文本是否触发此条目
    pub fn matches(&self, text: &str) -> bool {
        if self.keys.is_empty() {
            return true;
        }
        let lower = text.to_lowercase();
        self.keys.iter().any(|k| lower.contains(k))
    }

    /// 用目标信息填充模板
    pub fn render(&self, label: &str, offset_x: i32, offset_y: i32) -> String {
        self.template
            .replace("{label}", label)
            .replace("{offset_x}", &offset_x.to_string())
            .replace("{offset_y}", &offset_y.to_string())
    }
}

/// World Info 库: 按触发规则匹配, 生成动态场景描述
pub struct WorldInfoLib {
    entries: Vec<WorldInfo>,
}

impl WorldInfoLib {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn add(&mut self, entry: WorldInfo) {
        self.entries.push(entry);
    }

    /// 当前条目数（含默认库与运行时动态新增）。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 按稳定 id 精确删除一条（add 时给的 id）。
    pub fn remove_by_id(&mut self, id: &str) {
        self.entries.retain(|e| e.id.as_deref() != Some(id));
    }

    /// 按关键词删除所有 keys 与给定任一关键词相等的条目。
    pub fn remove_by_keys(&mut self, keys: &[String]) {
        let lower: Vec<String> = keys.iter().map(|k| k.to_lowercase()).collect();
        self.entries
            .retain(|e| !e.keys.iter().any(|k| lower.contains(k)));
    }

    /// 对任意感知文本做关键词扫描，按优先级去重并限制字符预算。
    pub fn scan_text(&self, text: &str, char_budget: usize) -> Vec<String> {
        let mut matched: Vec<&WorldInfo> =
            self.entries.iter().filter(|e| e.matches(text)).collect();
        matched.sort_by_key(|e| std::cmp::Reverse(e.priority));
        let mut used = 0usize;
        let mut hints = Vec::new();
        for entry in matched {
            let rendered = entry.render("当前场景", 0, 0);
            if hints.contains(&rendered) {
                continue;
            }
            if used.saturating_add(rendered.len()) > char_budget {
                continue;
            }
            used += rendered.len();
            hints.push(rendered);
        }
        hints
    }

    /// 扫描目标列表, 对每个目标匹配 WI 条目, 生成场景提示
    pub fn scan(&self, targets: &[crate::core::types::Target]) -> Vec<String> {
        let mut hints = Vec::new();
        for target in targets {
            for entry in &self.entries {
                if entry.matches(&target.label) {
                    let rendered = entry.render(
                        &target.label,
                        target.offset_from_crosshair.0,
                        target.offset_from_crosshair.1,
                    );
                    if !hints.contains(&rendered) {
                        hints.push(rendered);
                    }
                }
            }
        }
        hints
    }
}

impl Default for WorldInfoLib {
    fn default() -> Self {
        Self::new()
    }
}

/// Minecraft 场景的默认 World Info 库
///
/// 注入的提示必须引用真实工具名（perceive/look/press/mine），
/// 不能用已被 `craft-agent-minecraft::tools` 取代的旧名（aim_and_mine/move_forward）。
pub fn default_mc_world_info() -> WorldInfoLib {
    let mut lib = WorldInfoLib::new();
    lib.add(WorldInfo::new(
        vec!["tree".into(), "oak".into(), "橡树".into(), "树".into()],
        "Wood source nearby: {label} (offset {offset_x},{offset_y}). Use collect() to gather.",
    ));
    lib.add(WorldInfo::new(
        vec!["stone".into(), "石头".into()],
        "Stone source nearby: {label} (offset {offset_x},{offset_y}). Use collect() with a pickaxe equipped.",
    ));
    lib.add(WorldInfo::new(
        vec!["ore".into(), "coal".into(), "iron".into(), "矿石".into()],
        "Ore detected: {label} (offset {offset_x},{offset_y}). Mine with appropriate pickaxe.",
    ));
    lib.add(WorldInfo::new(
        vec!["water".into(), "水".into()],
        "Water nearby: {label} (offset {offset_x},{offset_y}). Avoid drowning.",
    ));
    lib.add(WorldInfo::new(
        vec!["creeper".into(), "zombie".into(), "skeleton".into(), "spider".into()],
        "Hostile mob nearby: {label} (offset {offset_x},{offset_y}). Attack or flee based on health and equipment.",
    ));
    lib
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_builder_layers() {
        let prompt = PromptBuilder::new()
            .identity("你是测试Agent")
            .role_desc("擅长测试")
            .add_example("输入A -> 输出B")
            .jailbreak("不要出错")
            .build();
        assert!(prompt.contains("你是测试Agent"));
        assert!(prompt.contains("擅长测试"));
        assert!(prompt.contains("输入A -> 输出B"));
        assert!(prompt.contains("不要出错"));
    }

    #[test]
    fn world_info_matches() {
        let wi = WorldInfo::new(vec!["tree".into(), "树".into()], "看到{label}");
        assert!(wi.matches("前方有tree"));
        assert!(wi.matches("一棵树"));
        assert!(!wi.matches("石头"));
    }

    #[test]
    fn world_info_add_and_remove() {
        let mut lib = WorldInfoLib::new();
        lib.add(WorldInfo::new(vec!["creeper".into()], "苦力怕会爆炸").with_id("mob_creeper"));
        lib.add(WorldInfo::new(vec!["zombie".into()], "僵尸近战").with_id("mob_zombie"));
        assert_eq!(lib.len(), 2);
        lib.remove_by_id("mob_creeper");
        assert_eq!(lib.len(), 1);
        assert_ne!(lib.entries[0].id.as_deref(), Some("mob_creeper"));
        lib.remove_by_keys(&["zombie".to_string()]);
        assert!(lib.is_empty());
    }
}
