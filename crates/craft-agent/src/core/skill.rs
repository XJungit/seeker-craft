//! Skill Library — pi_agent_rust 风格的轻量程序性记忆。
//!
//! 核心思想：Agent 成功完成任务后，提取动作序列为"技能"；
//! 后续遇到相似场景时，匹配器根据关键词找到相关技能，
//! 注入 PromptBuilder examples，让 LLM 直接复用已验证的模式。
//!
//! 设计参考 Mindcraft 的 memory_bank + Voyager 的 skill library：
//! - 存储：键值对（场景描述 → 动作步骤序列）
//! - 检索：关键词匹配（轻量，无外部依赖）
//! - 注入：`PromptBuilder.add_example()` 到 system prompt
//! - 演化：成功次数增加权重，长期未命中衰减

use serde::{Deserialize, Serialize};

/// 一条可复用技能。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    /// 触发关键词（全小写匹配，任意命中即触发检索）
    pub trigger_keywords: Vec<String>,
    /// 技能描述（给 LLM 看的自然语言解释）
    pub description: String,
    /// 工具调用序列（按执行顺序排列的工具名）
    pub steps: Vec<String>,
    /// 成功执行次数（成功 +1，失败/遗忘 -1）
    pub success_count: u32,
    /// 被检索次数（每次命中 +1，用于淘汰低频技能）
    pub hit_count: u32,
    /// 最后命中时间戳（ms，用于衰减）
    pub last_used: i64,
}

impl Skill {
    /// 从场景文本中判定是否触发此技能。
    pub fn matches(&self, scene_text: &str) -> bool {
        let lower = scene_text.to_lowercase();
        self.trigger_keywords
            .iter()
            .any(|k| lower.contains(k.as_str()))
    }

    /// 转为 prompt 示例（给 LLM 看）。
    /// 格式：`[Skill: 采集橡木→合成木板] 动作: collect → craft → place`
    pub fn to_example(&self) -> String {
        format!(
            "[Skill: {}] Execute: {}",
            self.description,
            self.steps.join(" → ")
        )
    }

    /// 标记一次命中（增加 hit_count，刷新时间戳）。
    pub fn mark_hit(&mut self, now_ms: i64) {
        self.hit_count += 1;
        self.last_used = now_ms;
    }

    /// 标记成功（增加 success_count）。
    pub fn mark_success(&mut self) {
        self.success_count += 1;
    }

    /// 衰减权重。
    /// `now_ms`: 当前时间戳  
    /// `decay_after_ms`: 超过此时间未命中则衰减
    pub fn decay(&mut self, now_ms: i64, decay_after_ms: i64) {
        if now_ms - self.last_used > decay_after_ms && self.success_count > 0 {
            self.success_count = self.success_count.saturating_sub(1);
        }
    }
}

/// 技能库：存储 + 检索 + 演化。
///
/// 设计原则（参考 pi agent 的内存管理）：
/// - 技能按触发关键词匹配，无外部依赖（无向量数据库）
/// - 命中次数 + 成功次数双重排序
/// - 低命中率技能自动淘汰（`cleanup()`）
/// - Session 持久化（序列化/反序列化）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillLibrary {
    skills: Vec<Skill>,
    /// 最大技能数（超出时淘汰最低命中率技能）
    max_skills: usize,
}

impl SkillLibrary {
    pub fn new(max_skills: usize) -> Self {
        Self {
            skills: Vec::new(),
            max_skills,
        }
    }

    /// 技能数量。
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    /// 添加或更新一条技能。
    /// 如果已有相同 description 的技能，更新其 steps 和 success_count。
    pub fn upsert(&mut self, description: &str, steps: Vec<String>, keywords: Vec<String>) {
        if let Some(existing) = self
            .skills
            .iter_mut()
            .find(|s| s.description == description)
        {
            existing.steps = steps;
            existing.trigger_keywords = keywords;
            existing.success_count += 1;
            return;
        }

        // 容量超限：淘汰 hit_count 最低的技能
        while self.skills.len() >= self.max_skills {
            if let Some(idx) = self
                .skills
                .iter()
                .enumerate()
                .min_by_key(|(_, s)| s.hit_count)
                .map(|(i, _)| i)
            {
                self.skills.remove(idx);
            } else {
                break;
            }
        }

        self.skills.push(Skill {
            trigger_keywords: keywords,
            description: description.to_string(),
            steps,
            success_count: 1,
            hit_count: 0,
            last_used: 0,
        });
    }

    /// 从场景文本中匹配技能。
    /// 返回按 score = (hit_count + success_count) 排序的技能引用列表，取前 `limit` 条。
    pub fn match_skills(&mut self, scene_text: &str, limit: usize, now_ms: i64) -> Vec<&Skill> {
        // Collect indices of matching skills
        let mut matched_indices: Vec<(usize, u32)> = self
            .skills
            .iter()
            .enumerate()
            .filter(|(_, s)| s.matches(scene_text))
            .map(|(i, s)| {
                let score = s.hit_count + s.success_count;
                (i, score)
            })
            .collect();

        // Sort by score descending
        matched_indices.sort_by(|a, b| b.1.cmp(&a.1));

        // Mark hits and collect references
        let mut result = Vec::new();
        for (i, _) in matched_indices.iter().take(limit) {
            self.skills[*i].mark_hit(now_ms);
        }
        // Now collect immutable references (safe after mutable borrows are done)
        for (i, _) in matched_indices.iter().take(limit) {
            result.push(&self.skills[*i]);
        }
        result
    }

    /// 获取所有技能作为 prompt 示例列表。
    pub fn to_examples(&mut self, scene_text: &str, limit: usize, now_ms: i64) -> Vec<String> {
        self.match_skills(scene_text, limit, now_ms)
            .into_iter()
            .map(|s| s.to_example())
            .collect()
    }

    /// 从工具调用序列中提取技能。
    /// `turn_log`: 本轮的工具名序列（如 ["collect", "craft", "craft", "place"]）
    /// `goal`: 当前目标（用于生成描述和关键词）
    pub fn extract_from_turn(
        &mut self,
        turn_log: &[String],
        goal: &str,
        scene_text: &str,
    ) -> Option<String> {
        if turn_log.len() < 2 {
            return None; // 单步操作不值得记忆
        }

        // 去重相邻的重复工具（如 mine, mine, mine → mine）
        let mut deduped: Vec<String> = Vec::new();
        for step in turn_log {
            if deduped.last().map_or(true, |last| last != step) {
                deduped.push(step.clone());
            }
        }

        if deduped.len() < 2 {
            return None;
        }

        // 生成描述和关键词
        let description = format!("Achieve '{}' via steps: {}", goal, deduped.join(", "));

        // 从场景文本中提取关键词（取出现频率最高的非停用词）
        let keywords: Vec<String> = scene_text
            .split(|c: char| !c.is_alphanumeric())
            .map(|w| w.to_lowercase())
            .filter(|w| w.len() >= 3 && !SKIP_WORDS.contains(&w.as_str()))
            .collect();

        // 去重保留前 5 个
        let mut seen = std::collections::HashSet::new();
        let unique_keywords: Vec<String> = keywords
            .into_iter()
            .filter(|k| seen.insert(k.clone()))
            .take(5)
            .collect();

        if unique_keywords.is_empty() {
            return None;
        }

        self.upsert(&description, deduped, unique_keywords);
        Some(description)
    }

    /// 淘汰低价值技能。
    /// - hit_count = 0 且超过 `stale_after_ms` 未命中 → 删除
    /// - success_count 低于阈值且 hit_count 低 → 删除
    pub fn cleanup(&mut self, now_ms: i64, stale_after_ms: i64) {
        self.skills.retain(|s| {
            let is_stale = now_ms - s.last_used > stale_after_ms;
            let is_unused = s.hit_count == 0;
            let is_low_value = s.success_count == 0 && s.hit_count < 2;
            !(is_stale && (is_unused || is_low_value))
        });
    }

    /// 时间衰减：长期未命中的技能 success_count 递减。
    pub fn decay_all(&mut self, now_ms: i64, decay_after_ms: i64) {
        for skill in &mut self.skills {
            skill.decay(now_ms, decay_after_ms);
        }
    }

    /// 获取所有技能引用（供 session 持久化使用）。
    pub fn skills(&self) -> &[Skill] {
        &self.skills
    }

    /// 批量加载技能（从 session 恢复）。
    pub fn load(&mut self, skills: Vec<Skill>) {
        self.skills = skills;
        // 截断到 max_skills
        self.skills
            .sort_by(|a, b| (b.hit_count + b.success_count).cmp(&(a.hit_count + a.success_count)));
        self.skills.truncate(self.max_skills);
    }
}

/// 停用词（不参与关键词提取）
const SKIP_WORDS: &[&str] = &[
    "the",
    "and",
    "for",
    "with",
    "from",
    "this",
    "that",
    "you",
    "are",
    "was",
    "has",
    "had",
    "not",
    "but",
    "all",
    "any",
    "can",
    "did",
    "get",
    "got",
    "her",
    "him",
    "his",
    "how",
    "its",
    "let",
    "may",
    "nor",
    "now",
    "our",
    "out",
    "own",
    "per",
    "put",
    "set",
    "she",
    "too",
    "try",
    "use",
    "via",
    "was",
    "who",
    "why",
    "will",
    "near",
    "nearby",
    "block",
    "blocks",
    "total",
    "showing",
    "top",
    "minecraft",
    "position",
    "health",
    "hunger",
    "gamemode",
    "weather",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_match_and_hit() {
        let mut lib = SkillLibrary::new(10);
        lib.upsert(
            "Collect oak logs and craft planks",
            vec!["collect".into(), "craft".into()],
            vec!["oak_log".into(), "planks".into()],
        );

        let now = 1000;
        let matched = lib.match_skills("I see oak_log nearby", 5, now);
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].hit_count, 1);
    }

    #[test]
    fn extract_from_turn() {
        let mut lib = SkillLibrary::new(10);
        let scene = "oak_log at (-35,68,56) dist=2m oak_log at (-36,68,57)";
        let result = lib.extract_from_turn(
            &["collect".into(), "collect".into(), "craft".into()],
            "get wood",
            scene,
        );
        assert!(result.is_some());
        assert!(lib.len() >= 1);
    }

    #[test]
    fn dedup_adjacent() {
        let mut lib = SkillLibrary::new(10);
        let scene = "oak_log nearby";
        lib.extract_from_turn(
            &[
                "collect".into(),
                "collect".into(),
                "collect".into(),
                "craft".into(),
            ],
            "gather wood",
            scene,
        );
        // 应该简化为 ["collect", "craft"]
        let skill = &lib.skills()[0];
        assert_eq!(skill.steps, vec!["collect", "craft"]);
    }

    #[test]
    fn cleanup_removes_stale() {
        let mut lib = SkillLibrary::new(10);
        lib.upsert("test skill", vec!["collect".into()], vec!["test".into()]);
        lib.skills[0].hit_count = 0;
        lib.skills[0].last_used = 0; // 很久以前

        lib.cleanup(100_000_000, 60_000); // 现在 vs 60s 过期
        assert_eq!(lib.len(), 0, "stale unused skill should be removed");
    }

    #[test]
    fn capacity_limit() {
        let mut lib = SkillLibrary::new(2);
        lib.upsert("skill 1", vec!["a".into()], vec!["k1".into()]);
        lib.upsert("skill 2", vec!["b".into()], vec!["k2".into()]);
        // Mark skill 1 as hit so skill 2 gets evicted first
        lib.skills[0].hit_count = 10;
        lib.skills[1].hit_count = 0;

        lib.upsert("skill 3", vec!["c".into()], vec!["k3".into()]);
        assert!(lib.len() <= 2, "should stay within capacity");
    }
}
