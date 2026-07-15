//! Skill Library — 场景指纹匹配 + 强披露的程序性记忆。
//!
//! 三层设计:
//!   1. 场景指纹: 从感知文本提取 {goal, inventory, nearby} 特征向量
//!   2. 加权匹配: goal_sim×3 + inv_sim×2 + nearby_sim×1 → 置信度
//!   3. 强披露: confidence > threshold → DIRECTIVE 格式强制复用
//!      confidence <= threshold → SUGGESTION 格式作为参考

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// 场景指纹: 从感知文本中提取的轻量特征向量。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SceneFingerprint {
    /// 目标关键词 (从 config.prompt 提取)
    pub goal_tokens: Vec<String>,
    /// 背包物品特征
    pub inv_tokens: Vec<String>,
    /// 附近方块特征
    pub nearby_tokens: Vec<String>,
}

impl SceneFingerprint {
    /// 从感知文本中提取指纹。
    pub fn from_scene(scene: &str, goal: &str) -> Self {
        let goal_tokens = tokenize(goal);
        let mut inv_tokens = Vec::new();
        let mut nearby_tokens = Vec::new();
        let mut in_inv = false;
        let mut in_nearby = false;
        for line in scene.lines() {
            if line.contains("HOTBAR") || line.contains("INVENTORY") { in_inv = true; in_nearby = false; continue; }
            if line.contains("NEARBY BLOCKS") { in_nearby = true; in_inv = false; continue; }
            if line.trim().is_empty() || line.starts_with("  Light") || line.starts_with("  Held") { continue; }
            if in_inv { inv_tokens.extend(tokenize(line)); }
            if in_nearby { nearby_tokens.extend(tokenize(line)); }
        }
        Self { goal_tokens, inv_tokens, nearby_tokens }
    }

    /// 与另一个指纹计算相似度（Jaccard 相似度）。返回 goal/inv/nearby 三维相似度。
    fn similarity(&self, other: &SceneFingerprint) -> (f64, f64, f64) {
        let jaccard = |a: &[String], b: &[String]| -> f64 {
            if a.is_empty() || b.is_empty() { return 0.0; }
            let sa: HashSet<_> = a.iter().collect();
            let sb: HashSet<_> = b.iter().collect();
            let inter = sa.intersection(&sb).count();
            let union = sa.union(&sb).count();
            if union == 0 { 0.0 } else { inter as f64 / union as f64 }
        };
        (jaccard(&self.goal_tokens, &other.goal_tokens), jaccard(&self.inv_tokens, &other.inv_tokens), jaccard(&self.nearby_tokens, &other.nearby_tokens))
    }
}

/// 分词: 按非字母数字分割，过滤停用词和短词，转小写。
fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(|w| w.to_lowercase())
        .filter(|w| w.len() >= 3 && !SKIP_WORDS.contains(&w.as_str()))
        .collect()
}

/// 一条可复用技能。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    /// 触发关键词（回退用，主匹配用 scene_fingerprint）
    pub trigger_keywords: Vec<String>,
    /// 技能描述
    pub description: String,
    /// 工具调用序列
    pub steps: Vec<String>,
    /// 学习时的场景指纹（用于相似度计算）
    pub fingerprint: SceneFingerprint,
    /// 成功次数
    pub success_count: u32,
    /// 被检索次数
    pub hit_count: u32,
    /// 最后命中时间戳
    pub last_used: i64,
}

impl Skill {
    /// 回退关键词匹配（仅当指纹无法计算时用）
    fn keyword_match(&self, scene: &str) -> bool {
        let lower = scene.to_lowercase();
        self.trigger_keywords.iter().any(|k| lower.contains(k.as_str()))
    }

    /// 转为建议格式（低置信度）
    fn to_suggestion(&self) -> String {
        format!(
            "[Skill: {}] Suggested steps: {}",
            self.description,
            self.steps.join(" → ")
        )
    }

    /// 转为指令格式（高置信度，强披露）
    fn to_directive(&self, confidence: f64) -> String {
        format!(
            "[DIRECTIVE — {:.0}% match — You have done this before. Follow these exact steps.] {}\n  Execute: {}",
            confidence * 100.0,
            self.description,
            self.steps.join(" → ")
        )
    }

    fn mark_hit(&mut self, now_ms: i64) { self.hit_count += 1; self.last_used = now_ms; }
    fn mark_success(&mut self) { self.success_count += 1; }
    fn decay(&mut self, now_ms: i64, decay_after_ms: i64) {
        if now_ms - self.last_used > decay_after_ms && self.success_count > 0 { self.success_count = self.success_count.saturating_sub(1); }
    }
}

/// 匹配结果: 包含技能引用和置信度。
pub struct SkillMatch<'a> {
    pub skill: &'a Skill,
    pub confidence: f64, // 0.0 ~ 1.0
}

/// 技能库。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillLibrary {
    skills: Vec<Skill>,
    max_skills: usize,
}

impl SkillLibrary {
    pub fn new(max_skills: usize) -> Self { Self { skills: Vec::new(), max_skills } }
    pub fn len(&self) -> usize { self.skills.len() }
    pub fn is_empty(&self) -> bool { self.skills.is_empty() }

    /// 插入或更新技能（带去重）。
    pub fn upsert(&mut self, desc: &str, steps: Vec<String>, keywords: Vec<String>, fingerprint: SceneFingerprint) {
        if let Some(existing) = self.skills.iter_mut().find(|s| s.description == desc) {
            existing.steps = steps; existing.trigger_keywords = keywords; existing.fingerprint = fingerprint; existing.success_count += 1; return;
        }
        while self.skills.len() >= self.max_skills {
            if let Some(idx) = self.skills.iter().enumerate().min_by_key(|(_, s)| s.hit_count).map(|(i, _)| i) { self.skills.remove(idx); } else { break; }
        }
        self.skills.push(Skill { trigger_keywords: keywords, description: desc.to_string(), steps, fingerprint, success_count: 1, hit_count: 0, last_used: 0 });
    }

    /// 场景指纹匹配: 计算加权置信度，返回前 N 条匹配及其置信度。
    pub fn match_scene(&mut self, scene: &str, goal: &str, limit: usize, now_ms: i64) -> Vec<SkillMatch> {
        let current = SceneFingerprint::from_scene(scene, goal);
        let mut scored: Vec<(usize, f64)> = self.skills.iter().enumerate()
            .map(|(i, s)| {
                let keyword_fallback = s.keyword_match(scene) as u8 as f64 * 0.3;
                let (gl, iv, nb) = s.fingerprint.similarity(&current);
                let conf = (gl * 0.5 + iv * 0.3 + nb * 0.2).max(keyword_fallback);
                (i, conf)
            })
            .filter(|(_, c)| *c > 0.15)
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Mark hits (mutable borrow done first, then collect immutable refs)
        for (i, _) in scored.iter().take(limit) {
            self.skills[*i].mark_hit(now_ms);
        }
        // Now collect immutable references
        scored.iter().take(limit).map(|(i, conf)| SkillMatch { skill: &self.skills[*i], confidence: *conf }).collect()
    }

    /// 生成 prompt 示例: 高置信度用 DIRECTIVE，低置信度用 SUGGESTION。
    pub fn to_examples(&mut self, scene: &str, goal: &str, limit: usize, now_ms: i64) -> Vec<String> {
        self.match_scene(scene, goal, limit, now_ms)
            .into_iter()
            .map(|m| if m.confidence > 0.5 { m.skill.to_directive(m.confidence) } else { m.skill.to_suggestion() })
            .collect()
    }

    /// 从工具调用序列学习技能。
    pub fn extract_from_turn(&mut self, turn_log: &[String], goal: &str, scene: &str) -> Option<String> {
        if turn_log.len() < 2 { return None; }
        let mut deduped: Vec<String> = Vec::new();
        for step in turn_log { if deduped.last().map_or(true, |last| last != step) { deduped.push(step.clone()); } }
        if deduped.len() < 2 { return None; }
        let fingerprint = SceneFingerprint::from_scene(scene, goal);
        let keywords = tokenize(scene).into_iter().take(5).collect();
        let desc = format!("Achieve '{}' via: {}", goal, deduped.join(", "));
        self.upsert(&desc, deduped, keywords, fingerprint);
        Some(desc)
    }

    /// 淘汰低价值技能。
    pub fn cleanup(&mut self, now_ms: i64, stale_ms: i64) {
        self.skills.retain(|s| !(now_ms - s.last_used > stale_ms && (s.hit_count == 0 || s.success_count == 0)));
    }

    pub fn decay_all(&mut self, now_ms: i64, decay_ms: i64) { for s in &mut self.skills { s.decay(now_ms, decay_ms); } }
    pub fn skills(&self) -> &[Skill] { &self.skills }
    pub fn load(&mut self, skills: Vec<Skill>) { self.skills = skills; self.skills.truncate(self.max_skills); }
}

const SKIP_WORDS: &[&str] = &[
    "the","and","for","with","from","this","that","you","are","was","has","had","not","but","all","any","can","did",
    "get","got","her","him","his","how","its","let","may","nor","now","our","out","own","per","put","set","she","too",
    "try","use","via","was","who","why","will","near","nearby","block","blocks","total","showing","top","minecraft",
    "position","health","hunger","gamemode","weather","none","empty","slot","hotbar","inventory","total","stats",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_similarity() {
        let f1 = SceneFingerprint::from_scene("HOTBAR\n  [1] oak_planksx6\nNEARBY BLOCKS\n  oak_log at (-40,68,48)\n", "collect wood");
        let f2 = SceneFingerprint::from_scene("HOTBAR\n  [1] oak_planksx3\nNEARBY BLOCKS\n  oak_log at (-38,68,50)\n  birch_log", "collect wood");
        let (g, i, _) = f1.similarity(&f2);
        assert!(g > 0.5, "goal should match: {g}"); // "collect", "wood" shared
        assert!(i > 0.3, "inventory should match (oak_planks): {i}");
    }

    #[test]
    fn scene_match_directive() {
        let mut lib = SkillLibrary::new(10);
        let fp = SceneFingerprint::from_scene("HOTBAR\n  [1] oak_planksx4\nNEARBY BLOCKS\n  oak_log (-35,68,56)", "craft table");
        lib.upsert("craft crafting_table", vec!["craft".into(), "place".into()], vec!["craft".into()], fp);
        let matches = lib.match_scene("HOTBAR\n  [1] oak_planksx4\nNEARBY BLOCKS\n  oak_log", "craft table", 5, 1000);
        assert!(!matches.is_empty());
        assert!(matches[0].confidence > 0.3);
    }

    #[test]
    fn extract_and_retrieve() {
        let mut lib = SkillLibrary::new(10);
        let scene = "HOTBAR\n  [1] stickx4\n  [2] coalx2\nNEARBY BLOCKS\n  oak_log";
        lib.extract_from_turn(&["craft".into(), "craft".into()], "make torches", scene);
        let examples = lib.to_examples(scene, "make torches", 3, 1000);
        assert!(!examples.is_empty());
        assert!(examples[0].contains("Suggested") || examples[0].contains("DIRECTIVE"));
    }
}
