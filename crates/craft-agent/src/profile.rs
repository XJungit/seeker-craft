//! Prompt profile 系统（学习自 Mindcraft 三层叠加机制）。
//!
//! 设计要点：
//! - profile 是 JSON 文件，含 system_prompt / mc_knowledge / modes / conversation_examples
//! - 三层叠加：`profiles/_default.json`（基线）→ `profiles/defaults/{mode}.json`（模式覆盖）
//!   → `profiles/{individual}.json`（个体覆盖）。字段级覆盖，非整体替换。
//! - 改 prompt 无需重编译 Rust（hot reload by file edit）。
//! - 占位符替换：`$NAME` / `$SELF_PROMPT` / `$MEMORY` 等在 render 阶段替换。
//!
//! 与 Mindcraft 的差异：
//! - Craft-Agent 用 OpenAI tool_call JSON，不是 Mindcraft 的 `!command` 文本协议，
//!   所以 conversation_examples 里的 `!cmd(...)` 要替换为真实 tool_calls 示例。
//! - Craft-Agent 没有 saving_memory / bot_responder / image_analysis prompt 模板
//!   （azalea 路线用结构化 perceive 替代 VLM），只保留 conversing 主模板。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// 模式开关（对齐 Mindcraft 的 10 个 modes）。
/// false 表示关闭对应自动行为，true 表示开启。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Modes {
    /// 生命/饱食危急时自动避险（火/岩浆/掉血）。
    #[serde(default = "default_true")]
    pub self_preservation: bool,
    /// 自动攻击附近敌对生物。
    #[serde(default = "default_true")]
    pub self_defense: bool,
    /// 卡住时自动脱困（跳/挖开/换路）。
    #[serde(default = "default_true")]
    pub unstuck: bool,
    /// 见敌对生物就逃跑（与 self_defense 互斥，cowardice 优先）。
    #[serde(default)]
    pub cowardice: bool,
    /// 主动狩猎附近动物获取食物。
    #[serde(default = "default_true")]
    pub hunting: bool,
    /// 主动捡起附近掉落物。
    #[serde(default = "default_true")]
    pub item_collecting: bool,
    /// 黑暗处自动放火把。
    #[serde(default = "default_true")]
    pub torch_placing: bool,
    /// 周围太挤时自动腾出空间。
    #[serde(default = "default_true")]
    pub elbow_room: bool,
    /// 空闲时四处看（增加自然感）。
    #[serde(default = "default_true")]
    pub idle_staring: bool,
    /// 创造模式作弊（飞行/瞬移/给物品）。
    #[serde(default)]
    pub cheat: bool,
}

impl Default for Modes {
    fn default() -> Self {
        Self {
            self_preservation: true,
            self_defense: true,
            unstuck: true,
            cowardice: false,
            hunting: true,
            item_collecting: true,
            torch_placing: true,
            elbow_room: true,
            idle_staring: true,
            cheat: false,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_cooldown_ms() -> u64 {
    3000
}

/// 分阶段知识块（A2，2026-08-02）：按任务 tier 生效的知识段。
/// 早期只注入低 tier 知识（省 token + 注意力聚焦），tier 推进后累积注入。
/// 从 system prompt 拆出——system 只留核心规则（DeepSeek 前缀缓存更省）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StageKnowledge {
    /// 生效所需的最低任务 tier（1=木石期 → 6=末地龙）。0 表示始终注入。
    #[serde(default)]
    pub tier: u8,
    /// 该阶段知识文本（markdown 风格，与 system_prompt 拼接方式一致）
    #[serde(default)]
    pub text: String,
}

/// Prompt profile（对应一个 JSON 文件）。
///
/// 字段全部 `Option` 化以支持叠加：下层 profile 的 `None` 字段不被上层覆盖，
/// 上层 profile 的 `Some` 字段才覆盖下层。这样 `_default.json` 可以只设
/// `system_prompt`，`survival.json` 只设 `modes`，互不干扰。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Profile {
    /// Profile 名字（如 "_default" / "survival" / "deepseek"）。
    #[serde(default)]
    pub name: String,

    /// 主 system prompt 模板。支持 `$NAME` / `$SELF_PROMPT` / `$MEMORY` /
    /// `$STATS` / `$INVENTORY` / `$COMMAND_DOCS` / `$EXAMPLES` 等占位符。
    #[serde(default)]
    pub system_prompt: Option<String>,

    /// MC 常识段（拼在 system_prompt 末尾，含矿物分布/工具规则/合成链等）。
    #[serde(default)]
    pub mc_knowledge: Option<String>,

    /// 后置强制指令（替代硬编码 jailbreak，改 prompt 无需重编译）。
    /// `None` = 使用 Rust 内置默认。
    #[serde(default)]
    pub jailbreak: Option<String>,

    /// 分阶段知识（A2）：按任务 tier 注入 user 消息，不进 system prompt。
    #[serde(default)]
    pub stage_knowledge: Vec<StageKnowledge>,

    /// 模式开关。
    #[serde(default)]
    pub modes: Modes,

    /// few-shot 对话示例。每个示例是一组消息（role/content 序列）。
    /// 叠加时 append（个体 profile 可加自己的示例），不覆盖。
    #[serde(default)]
    pub conversation_examples: Vec<Vec<serde_json::Value>>,

    /// 轮次冷却（毫秒）。两层间默认 3000ms。
    #[serde(default = "default_cooldown_ms")]
    pub cooldown_ms: u64,
}

impl Profile {
    /// 加载并合并三层 profile：
    /// 1. `{profiles_dir}/_default.json` —— 基线，必须存在
    /// 2. `{profiles_dir}/defaults/{mode}.json` —— 模式覆盖（可选）
    /// 3. `{profiles_dir}/{individual}.json` —— 个体覆盖（可选）
    ///
    /// 任意层缺失就跳过；只有 _default.json 必须存在。
    pub fn load(
        profiles_dir: &Path,
        mode: Option<&str>,
        individual: Option<&str>,
    ) -> Result<Profile> {
        let mut merged;

        // 1. 加载 _default.json
        let default_path = profiles_dir.join("_default.json");
        if !default_path.exists() {
            anyhow::bail!("_default.json 不存在：{}", default_path.display());
        }
        let base: Profile = parse_json_file(&default_path)?;
        merged = base;
        merged.name = "_default".to_string();

        // 2. 叠加 mode profile
        if let Some(mode) = mode {
            let mode_path = profiles_dir.join("defaults").join(format!("{mode}.json"));
            if mode_path.exists() {
                let mode_profile: Profile = parse_json_file(&mode_path)?;
                merged.merge_from(&mode_profile);
                merged.name = mode.to_string();
            }
        }

        // 3. 叠加 individual profile
        if let Some(ind) = individual {
            let ind_path = profiles_dir.join(format!("{ind}.json"));
            if ind_path.exists() {
                let ind_profile: Profile = parse_json_file(&ind_path)?;
                merged.merge_from(&ind_profile);
                merged.name = ind.to_string();
            }
        }

        Ok(merged)
    }

    /// 用 `other` 的 `Some` 字段覆盖 `self` 的对应字段。
    /// `conversation_examples` 是 append（不覆盖），其他字段是替换。
    pub fn merge_from(&mut self, other: &Profile) {
        if other.system_prompt.is_some() {
            self.system_prompt = other.system_prompt.clone();
        }
        if other.mc_knowledge.is_some() {
            self.mc_knowledge = other.mc_knowledge.clone();
        }
        if other.jailbreak.is_some() {
            self.jailbreak = other.jailbreak.clone();
        }
        if !other.stage_knowledge.is_empty() {
            self.stage_knowledge = other.stage_knowledge.clone();
        }
        // modes 字段级合并（直接替换，因为 bool 没法区分"未设"和"false"，
        // Mindcraft 的设计是 modes 一次性整组替换，不是逐字段叠加）
        self.modes = other.modes.clone();
        // examples append
        if !other.conversation_examples.is_empty() {
            self.conversation_examples
                .extend(other.conversation_examples.iter().cloned());
        }
        if other.cooldown_ms != 3000 {
            self.cooldown_ms = other.cooldown_ms;
        }
    }

    /// 渲染最终 system prompt：替换占位符 + 拼接 mc_knowledge。
    ///
    /// 占位符：`$NAME` / `$SELF_PROMPT` / `$MEMORY` / `$STATS` / `$INVENTORY` /
    /// `$COMMAND_DOCS` / `$EXAMPLES`。未提供的占位符替换为空字符串。
    pub fn render(&self, replacements: &HashMap<String, String>) -> String {
        let mut prompt = self.system_prompt.clone().unwrap_or_default();
        for (k, v) in replacements {
            prompt = prompt.replace(&format!("${k}"), v);
        }
        if let Some(mc) = &self.mc_knowledge {
            prompt.push('\n');
            prompt.push_str(mc);
        }
        prompt
    }
}

fn parse_json_file(path: &Path) -> Result<Profile> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("读取 profile 文件失败：{}", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("解析 profile JSON 失败：{}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_merge_from_overrides_some_fields() {
        let mut base = Profile {
            name: "_default".into(),
            system_prompt: Some("base prompt".into()),
            mc_knowledge: Some("base knowledge".into()),
            jailbreak: None,
            stage_knowledge: vec![],
            modes: Modes::default(),
            conversation_examples: vec![],
            cooldown_ms: 3000,
        };
        let overlay = Profile {
            name: "survival".into(),
            system_prompt: None,                             // 不覆盖
            mc_knowledge: Some("survival knowledge".into()), // 覆盖
            jailbreak: Some("jail".into()),
            stage_knowledge: vec![],
            modes: Modes {
                cheat: true,
                ..Modes::default()
            },
            conversation_examples: vec![vec![]],
            cooldown_ms: 5000,
        };
        base.merge_from(&overlay);
        assert_eq!(base.system_prompt, Some("base prompt".into())); // 未覆盖
        assert_eq!(base.mc_knowledge, Some("survival knowledge".into())); // 已覆盖
        assert!(base.modes.cheat); // 已覆盖
        assert_eq!(base.conversation_examples.len(), 1); // append
        assert_eq!(base.cooldown_ms, 5000); // 已覆盖
    }

    #[test]
    fn test_render_replaces_placeholders() {
        let p = Profile {
            system_prompt: Some("Hello $NAME, you are at $LOCATION.".into()),
            jailbreak: None,
            stage_knowledge: vec![],
            ..Default::default()
        };
        let mut reps = HashMap::new();
        reps.insert("NAME".into(), "CraftBot".into());
        reps.insert("LOCATION".into(), "plains".into());
        let rendered = p.render(&reps);
        assert_eq!(rendered, "Hello CraftBot, you are at plains.");
    }

    #[test]
    fn test_render_appends_mc_knowledge() {
        let p = Profile {
            system_prompt: Some("base".into()),
            mc_knowledge: Some("MC_TIPS_HERE".into()),
            jailbreak: None,
            stage_knowledge: vec![],
            ..Default::default()
        };
        let rendered = p.render(&HashMap::new());
        assert!(rendered.contains("base"));
        assert!(rendered.contains("MC_TIPS_HERE"));
    }

    #[test]
    fn test_load_three_layers() {
        // 临时目录：_default.json + defaults/survival.json + deepseek.json
        let tmp = std::env::temp_dir().join("craft_agent_profile_test");
        std::fs::create_dir_all(tmp.join("defaults")).unwrap();

        let mut f = std::fs::File::create(tmp.join("_default.json")).unwrap();
        writeln!(
            f,
            r#"{{"name":"_default","system_prompt":"base prompt","mc_knowledge":"base mc","modes":{{"self_preservation":true,"self_defense":true,"unstuck":true,"cowardice":false,"hunting":true,"item_collecting":true,"torch_placing":true,"elbow_room":true,"idle_staring":true,"cheat":false}},"conversation_examples":[],"cooldown_ms":3000}}"#
        ).unwrap();

        let mut f = std::fs::File::create(tmp.join("defaults").join("survival.json")).unwrap();
        writeln!(
            f,
            r#"{{"name":"survival","modes":{{"self_preservation":true,"self_defense":true,"unstuck":true,"cowardice":false,"hunting":true,"item_collecting":true,"torch_placing":true,"elbow_room":true,"idle_staring":true,"cheat":false}}}}"#
        ).unwrap();

        let mut f = std::fs::File::create(tmp.join("deepseek.json")).unwrap();
        writeln!(
            f,
            r#"{{"name":"deepseek","system_prompt":"deepseek prompt override"}}"#
        )
        .unwrap();

        let p = Profile::load(&tmp, Some("survival"), Some("deepseek")).unwrap();
        assert_eq!(p.system_prompt, Some("deepseek prompt override".into()));
        assert_eq!(p.mc_knowledge, Some("base mc".into())); // 未被覆盖

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_load_default_only() {
        let tmp = std::env::temp_dir().join("craft_agent_profile_test2");
        std::fs::create_dir_all(&tmp).unwrap();

        let mut f = std::fs::File::create(tmp.join("_default.json")).unwrap();
        writeln!(
            f,
            r#"{{"name":"_default","system_prompt":"only prompt","modes":{{}},"conversation_examples":[],"cooldown_ms":3000}}"#
        ).unwrap();

        let p = Profile::load(&tmp, None, None).unwrap();
        assert_eq!(p.system_prompt, Some("only prompt".into()));

        std::fs::remove_dir_all(&tmp).ok();
    }
}
