//! 语义记忆层（P97，pi-memory 的 MC 适配）
//!
//! 与 [`super::memory::WorldMemory`]（空间坐标记忆，几何邻近自动渲染）互补：
//! 语义记忆是无坐标的**知识/策略/经验**记忆，跨会话持久化，由 LLM 主动
//! `remember` 写入，每轮按相关性注入用户消息（不占系统提示，字节稳定）。
//!
//! 注入链路（pi-memory 三层注入的适配）：
//! - 索引：条目标题列表就是"知道什么"——注入时一并展示标题
//! - 按需浮现：每轮用当前目标 + 最近 perceive + 最近工具名做查询词，
//!   按 tag 命中 + 词元重叠 + 使用频次/新鲜度评分，注入最相关 ≤N 条
//! - 写入：`remember` 工具（LLM 主动），同标题去重更新
//!
//! 存储：JSONL（与 session 同风格），`data/memory/agent.jsonl`。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::tool::{GameTool, ToolEffects, ToolResult};

#[inline]
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// 记忆类别（对齐 Claude Code auto memory 的分类，裁剪为 MC 场景）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryKind {
    /// 客观事实（如"家里箱子存了 32 铁锭"）
    Fact,
    /// 行动策略（如"下界要带抗火药水"）
    #[default]
    Strategy,
    /// 教训/洞察（如"徒手挖黑曜石 1 分钟挖不掉，必须先做钻石镐"）
    Insight,
    /// 用户/任务偏好（如"优先使用 auto_craft 而非逐项 craft"）
    Preference,
}

/// 一条语义记忆。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    /// 自包含标题（注入时只展示标题 + 截断内容，标题须能独立传达要点）
    pub title: String,
    /// 内容（注入时截断到 `content_max_chars`）
    pub content: String,
    /// 检索标签（方块/物品/位置/概念，如 ["diamond", "mining"]）
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub kind: MemoryKind,
    /// 作用域隔离：`None` = 全局通用知识（配方/策略/教训，任何世界有效）；
    /// `Some(server)` = 仅该服务器/世界有效（坐标、基地、传送门位置等）。
    /// 注入时只显示 scope 为 None 或与当前 scope 匹配的条目，防止跨图污染。
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub updated: i64,
    /// 最近一次被注入的轮次时间戳（recency 加权）
    #[serde(default)]
    pub last_used: i64,
    /// 被注入过的次数（frequency 加权）
    #[serde(default)]
    pub uses: u32,
}

/// 语义记忆库（内部可变，外部经 `Arc<Mutex<..>>` 共享给 remember 工具与注入）。
pub struct SemanticMemory {
    entries: Vec<MemoryEntry>,
    path: Option<PathBuf>,
    /// 每轮最多注入条数
    pub max_injected: usize,
    /// 单条内容注入截断字符数
    pub content_max_chars: usize,
}

impl Default for SemanticMemory {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            path: None,
            max_injected: 4,
            content_max_chars: 200,
        }
    }
}

/// 把文本切成检索词元：英文小写单词 + 中文 token（连续 CJK 串输出
/// bigram + 单字双路，孤立汉字输出单字——保证中英混合查询能命中）。
/// 查询与条目都用同一函数，取交集计数（无需外部分词库）。
pub fn tokens(s: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let lower = s.to_lowercase();
    let mut word = String::new();
    let mut cjk = String::new();
    for c in lower.chars() {
        if c.is_ascii_alphanumeric() {
            if !cjk.is_empty() {
                push_cjk_tokens(&cjk, &mut out);
                cjk.clear();
            }
            word.push(c);
        } else if c.is_ascii() {
            if !word.is_empty() {
                if word.len() >= 2 {
                    out.insert(word.clone());
                }
                word.clear();
            }
            if !cjk.is_empty() {
                push_cjk_tokens(&cjk, &mut out);
                cjk.clear();
            }
        } else if ('\u{4e00}'..='\u{9fff}').contains(&c) {
            if !word.is_empty() {
                if word.len() >= 2 {
                    out.insert(word.clone());
                }
                word.clear();
            }
            cjk.push(c);
        } else {
            if !word.is_empty() {
                if word.len() >= 2 {
                    out.insert(word.clone());
                }
                word.clear();
            }
            if !cjk.is_empty() {
                push_cjk_tokens(&cjk, &mut out);
                cjk.clear();
            }
        }
    }
    if !word.is_empty() && word.len() >= 2 {
        out.insert(word);
    }
    if !cjk.is_empty() {
        push_cjk_tokens(&cjk, &mut out);
    }
    out
}

/// CJK token：bigram + 单字双路。单字保证孤立汉字（"挖钻石矿" 中的每个字）
/// 可被中英混合查询（"挖 diamond"）命中；bigram 提供连续串的精确度。
fn push_cjk_tokens(s: &str, out: &mut HashSet<String>) {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() == 1 {
        out.insert(chars[0].to_string());
        return;
    }
    for w in chars.windows(2) {
        out.insert(format!("{}{}", w[0], w[1]));
    }
    for c in chars {
        out.insert(c.to_string());
    }
}

impl SemanticMemory {
    pub fn new() -> Self {
        Self::default()
    }

    /// 绑定持久化路径（存在则加载）。
    pub fn with_path(mut self, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        if path.exists() {
            let _ = self.load(&path);
        }
        self.path = Some(path);
        self
    }

    /// 记录一条记忆：同标题去重更新（保留旧 uses/last_used）。
    /// `scope`: `None` = 全局通用知识；`Some(server)` = 仅该服务器有效。
    pub fn remember(
        &mut self,
        title: &str,
        content: &str,
        tags: &[String],
        kind: MemoryKind,
        scope: Option<&str>,
    ) -> String {
        let now = now_ms();
        if let Some(e) = self.entries.iter_mut().find(|e| e.title == title) {
            e.content = content.to_string();
            e.tags = tags.to_vec();
            e.kind = kind;
            e.scope = scope.map(str::to_string);
            e.updated = now;
            let _ = self.save();
            return format!("已更新记忆「{title}」");
        }
        self.entries.push(MemoryEntry {
            title: title.to_string(),
            content: content.to_string(),
            tags: tags.to_vec(),
            kind,
            scope: scope.map(str::to_string),
            created: now,
            updated: now,
            last_used: 0,
            uses: 0,
        });
        let _ = self.save();
        format!(
            "已记录记忆「{title}」（共 {} 条）。它会按相关性自动注入到未来的决策上下文。",
            self.entries.len()
        )
    }

    pub fn forget(&mut self, title: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.title != title);
        let removed = self.entries.len() != before;
        if removed {
            let _ = self.save();
        }
        removed
    }

    pub fn list(&self) -> &[MemoryEntry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 相关性评分：tag 命中 ×3 + 词元交集 + uses 对数 + recency（天级半衰）。
    fn score(&self, entry: &MemoryEntry, query_tokens: &HashSet<String>) -> f64 {
        let mut s = 0.0;
        let et = tokens(&entry.title);
        let ct = tokens(&entry.content);
        let overlap = et.union(&ct).filter(|t| query_tokens.contains(*t)).count();
        s += overlap as f64;
        let tag_hits = entry
            .tags
            .iter()
            .filter(|t| query_tokens.contains(&t.to_lowercase()))
            .count();
        s += tag_hits as f64 * 3.0;
        if s > 0.0 {
            s += (entry.uses as f64 + 1.0).ln();
            // 新鲜度：1 天内满权重，每天减半
            let days =
                (now_ms() - entry.last_used.max(entry.updated)) as f64 / (24.0 * 3600.0 * 1000.0);
            s *= 0.5f64.max(1.0 - days * 0.5);
        }
        s
    }

    /// scope 语义归一：None/空串/"global"/"any"/"*" 都视为全局通用知识。
    /// LLM 自由填值时常用 "global" 表示通用（工具描述只引导了"留空"），
    /// 若不归一化，全局记忆会被服务器 scope 过滤永不注入（实机验证发现）。
    pub fn scope_is_global(scope: &Option<String>) -> bool {
        matches!(
            scope.as_deref(),
            None | Some("") | Some("global") | Some("any") | Some("*")
        )
    }

    /// 按相关性取 top-N 条（query 空时按 recency 取最近）。
    /// `scope`（当前服务器/世界）：只返回全局条目（见 [`scope_is_global`]）
    /// 或与当前 scope 匹配的条目，杜绝跨图记忆污染（如别的世界的坐标）。
    pub fn relevant(&self, query: &str, scope: Option<&str>, limit: usize) -> Vec<&MemoryEntry> {
        let q = tokens(query);
        let mut ranked: Vec<(&MemoryEntry, f64, usize)> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| Self::scope_is_global(&e.scope) || e.scope.as_deref() == scope)
            .map(|(i, e)| (e, self.score(e, &q), i))
            .collect();
        if q.is_empty() {
            // 空查询按 recency：updated 降序，同毫秒时后插入的（索引大）优先
            ranked.sort_by(|a, b| b.0.updated.cmp(&a.0.updated).then_with(|| b.2.cmp(&a.2)));
        } else {
            ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        }
        ranked.into_iter().take(limit).map(|(e, _, _)| e).collect()
    }

    /// 注入文本：渲染「相关记忆 + 标题索引」块，返回 (文本, 被消费的标题)。
    /// 消费后调用 [`Self::touch`] 更新频率统计。`scope` 过滤见 [`Self::relevant`]。
    pub fn injection_text(&self, query: &str, scope: Option<&str>) -> (String, Vec<String>) {
        let relevant = self.relevant(query, scope, self.max_injected);
        if relevant.is_empty() {
            return (String::new(), vec![]);
        }
        let mut s = String::from("相关长期记忆：\n");
        let mut touched = Vec::new();
        for (i, e) in relevant.iter().enumerate() {
            let content = truncate(&e.content, self.content_max_chars);
            s.push_str(&format!(
                "{}. [{}] {}：{}\n",
                i + 1,
                kind_label(e.kind),
                e.title,
                content
            ));
            touched.push(e.title.clone());
        }
        s.push_str(&format!(
            "（另有 {} 条记忆未被注入，可通过 remember 查询或写入）",
            self.entries.len().saturating_sub(relevant.len())
        ));
        (s, touched)
    }

    /// 消费统计：更新 last_used / uses。
    pub fn touch(&mut self, titles: &[String]) {
        let now = now_ms();
        for e in self.entries.iter_mut() {
            if titles.contains(&e.title) {
                e.last_used = now;
                e.uses = e.uses.saturating_add(1);
            }
        }
    }

    /// JSONL 持久化：一行一条。追加 + 全量重写（简单可靠，条目数少）。
    pub fn save(&self) -> anyhow::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = std::fs::File::create(path)?;
        let mut w = BufWriter::new(file);
        for e in &self.entries {
            w.write_all(serde_json::to_string(e)?.as_bytes())?;
            w.write_all(b"\n")?;
        }
        w.flush()?;
        Ok(())
    }

    pub fn load(&mut self, path: &Path) -> anyhow::Result<()> {
        let file = std::fs::File::open(path)?;
        let reader = BufReader::new(file);
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(e) = serde_json::from_str::<MemoryEntry>(&line) {
                self.entries.push(e);
            }
        }
        Ok(())
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

fn kind_label(k: MemoryKind) -> &'static str {
    match k {
        MemoryKind::Fact => "事实",
        MemoryKind::Strategy => "策略",
        MemoryKind::Insight => "教训",
        MemoryKind::Preference => "偏好",
    }
}

/// `remember` 工具：LLM 主动写入/查询语义记忆（与 WorldMemory 空间记忆互补）。
/// 经 Arc<Mutex> 与 Agent 注入共享同一实例。
pub struct SemanticMemoryTool {
    pub mem: Arc<Mutex<SemanticMemory>>,
}

impl GameTool for SemanticMemoryTool {
    fn name(&self) -> &str {
        "remember"
    }

    fn description(&self) -> &str {
        "语义长期记忆（跨会话）：记录/查询/遗忘知识、策略与教训（非坐标类事实）。\
         与 memory 工具（空间记忆）互补。action=save 写入或更新（同标题更新）；\
         action=forget 按标题删除；action=list 列出全部标题。写入的记忆会自动\
         按相关性注入到后续决策上下文。"
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["save", "forget", "list"] },
                "title": { "type": "string", "description": "自包含标题（去重键），如「下界安全策略」" },
                "content": { "type": "string", "description": "记忆内容（策略/事实/教训）" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "检索标签，如 [diamond, mining]" },
                "kind": { "type": "string", "enum": ["fact", "strategy", "insight", "preference"], "description": "类别，默认 strategy" },
                "scope": { "type": "string", "description": "作用域：坐标/基地/传送门位置等只对当前服务器有效的记忆填当前服务器地址；配方/策略/通用教训留空（全局通用）" }
            },
            "required": ["action"]
        })
    }

    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }

    fn execute(
        &self,
        _call_id: &str,
        args: Value,
        _on_update: Option<crate::core::tool::ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let action = args["action"].as_str().unwrap_or("");
        let mut mem = self
            .mem
            .lock()
            .map_err(|_| anyhow::anyhow!("semantic memory poisoned"))?;
        let message = match action {
            "save" => {
                let title = args["title"]
                    .as_str()
                    .filter(|t| !t.trim().is_empty())
                    .ok_or_else(|| anyhow::anyhow!("save 需要非空 title"))?;
                let content = args["content"]
                    .as_str()
                    .filter(|c| !c.trim().is_empty())
                    .ok_or_else(|| anyhow::anyhow!("save 需要非空 content"))?;
                let tags = args["tags"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str())
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let kind = match args["kind"].as_str() {
                    Some("fact") => MemoryKind::Fact,
                    Some("insight") => MemoryKind::Insight,
                    Some("preference") => MemoryKind::Preference,
                    _ => MemoryKind::Strategy,
                };
                let scope = args["scope"].as_str().filter(|s| !s.trim().is_empty());
                mem.remember(title, content, &tags, kind, scope)
            }
            "forget" => {
                let title = args["title"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("forget 需要 title"))?;
                if mem.forget(title) {
                    format!("已删除记忆「{title}」")
                } else {
                    format!("未找到记忆「{title}」")
                }
            }
            "list" => {
                if mem.is_empty() {
                    "暂无语义记忆".to_string()
                } else {
                    let mut lines = String::from("现有语义记忆：\n");
                    for e in mem.list() {
                        lines.push_str(&format!(
                            "- [{}] {}（{}）\n",
                            kind_label(e.kind),
                            e.title,
                            e.updated
                        ));
                    }
                    lines
                }
            }
            _ => "action 必须是 save/forget/list".to_string(),
        };
        Ok(ToolResult {
            message,
            is_error: false,
            images: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> SemanticMemory {
        SemanticMemory::new()
    }

    #[test]
    fn remember_dedupes_by_title() {
        let mut m = mem();
        m.remember(
            "家里",
            "箱子在 (-10,64,-20)",
            &["base".into()],
            MemoryKind::Fact,
            None,
        );
        m.remember(
            "家里",
            "箱子搬到 (100,64,-5)",
            &["base".into()],
            MemoryKind::Fact,
            None,
        );
        assert_eq!(m.list().len(), 1, "同标题应去重更新");
        assert!(m.list()[0].content.contains("(100,64,-5)"));
    }

    #[test]
    fn forget_removes_by_title() {
        let mut m = mem();
        m.remember("a", "x", &[], MemoryKind::Fact, None);
        m.remember("b", "y", &[], MemoryKind::Fact, None);
        assert!(m.forget("a"));
        assert!(!m.forget("a"));
        assert_eq!(m.list().len(), 1);
    }

    #[test]
    fn tokens_mixes_english_and_cjk() {
        let t = tokens("挖 diamond 矿 用 iron_pickaxe");
        assert!(t.contains("diamond"), "英文词应被提取");
        assert!(t.contains("iron"), "下划线拆分的单词段应被提取");
        assert!(t.contains("pickaxe"));
        assert!(!t.contains("iron_pickaxe"), "下划线是分隔符，不保留整体");
        assert!(t.contains("挖"), "孤立汉字输出单字 token");
        assert!(t.contains("矿"));
    }

    #[test]
    fn tokens_continuous_cjk_emits_bigrams_and_singles() {
        let t = tokens("挖钻石矿");
        assert!(t.contains("挖钻"), "连续 CJK 输出 bigram");
        assert!(t.contains("钻石"));
        assert!(t.contains("石矿"));
        assert!(t.contains("钻"), "同时输出单字，保证中英混合查询命中");
        assert!(t.contains("矿"));
    }

    #[test]
    fn relevant_ranks_tag_hits_first() {
        let mut m = mem();
        m.remember("无用记忆", "今天天气不错", &[], MemoryKind::Fact, None);
        m.remember(
            "钻石策略",
            "用钻石镐挖钻石最快",
            &["diamond".into(), "mining".into()],
            MemoryKind::Strategy,
            None,
        );
        let top = m.relevant("挖 diamond 用什么镐", None, 4);
        assert_eq!(top[0].title, "钻石策略", "tag 命中应排最前");
        assert!(top[0].kind == MemoryKind::Strategy);
    }

    #[test]
    fn injection_renders_block_and_touch_updates_stats() {
        let mut m = mem();
        m.remember(
            "挖矿策略",
            "y<16 才有钻石",
            &["diamond".into()],
            MemoryKind::Strategy,
            None,
        );
        let (text, touched) = m.injection_text("挖 diamond", None);
        assert!(text.contains("相关长期记忆"), "应渲染注入块");
        assert!(text.contains("挖矿策略"));
        assert_eq!(touched.len(), 1);
        let before = m.list()[0].uses;
        m.touch(&touched);
        assert_eq!(m.list()[0].uses, before + 1, "消费应更新频率统计");
    }

    #[test]
    fn injection_empty_when_no_entries() {
        let m = mem();
        let (text, touched) = m.injection_text("diamond", None);
        assert!(text.is_empty());
        assert!(touched.is_empty());
    }

    #[test]
    fn save_load_roundtrips() {
        let dir = std::env::temp_dir().join(format!("sem_mem_test_{}", now_ms()));
        let path = dir.join("mem.jsonl");
        {
            let mut m = SemanticMemory::new().with_path(&path);
            m.remember(
                "坐标",
                "基地在 (0,64,0)",
                &["base".into()],
                MemoryKind::Fact,
                Some("s1"),
            );
            assert!(path.exists(), "remember 应落盘");
        }
        let mut m2 = SemanticMemory::new();
        m2.load(&path).unwrap();
        assert_eq!(m2.list().len(), 1);
        assert_eq!(m2.list()[0].title, "坐标");
        assert_eq!(
            m2.list()[0].scope.as_deref(),
            Some("s1"),
            "scope 应随 JSONL 持久化"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn relevant_empty_query_returns_recent() {
        let mut m = mem();
        m.remember("旧", "旧内容", &[], MemoryKind::Fact, None);
        m.remember("新", "新内容", &[], MemoryKind::Fact, None);
        let top = m.relevant("", None, 1);
        assert_eq!(top[0].title, "新");
    }

    #[test]
    fn scope_global_string_is_treated_as_universal() {
        let mut m = mem();
        // LLM 自由填值：用 "global" 字符串表示通用知识（非空 scope）
        m.remember(
            "通用策略",
            "先做工具再砍树",
            &["wood".into()],
            MemoryKind::Strategy,
            Some("global"),
        );
        m.remember(
            "服务器坐标",
            "基地在 (10,64,-20)",
            &["base".into()],
            MemoryKind::Fact,
            Some("s1"),
        );
        // 当前服务器 s2：global 应注入，s1 应隔离
        let (text, touched) = m.injection_text("工具 砍树", Some("s2"));
        assert!(
            text.contains("通用策略"),
            "scope=global 应视为全局知识注入: {text}"
        );
        assert!(!text.contains("服务器坐标"), "其他服务器坐标仍隔离");
        assert_eq!(touched.len(), 1);
        // 空 scope（无服务器会话）：global 同样注入
        let (text2, _) = m.injection_text("工具 砍树", None);
        assert!(text2.contains("通用策略"));
    }

    #[test]
    fn scope_isolation_keeps_world_specific_memories_out() {
        let mut m = mem();
        // 全局通用知识（配方/策略）：任何世界都注入
        m.remember(
            "钻石镐配方",
            "3 钻石 + 2 木棍",
            &["diamond".into()],
            MemoryKind::Fact,
            None,
        );
        // 服务器特定事实（坐标/基地）：只在对应服务器注入
        m.remember(
            "基地坐标",
            "基地在 (10,64,-20)",
            &["base".into()],
            MemoryKind::Fact,
            Some("s1"),
        );
        m.remember(
            "旧服基地",
            "旧服基地在 (999,64,999)",
            &["base".into()],
            MemoryKind::Fact,
            Some("s2"),
        );

        // 在 s1：全局 + s1 条目可见，s2 条目被隔离
        let (text_s1, touched_s1) = m.injection_text("基地", Some("s1"));
        assert!(text_s1.contains("基地坐标"), "当前服务器条目应注入");
        assert!(!text_s1.contains("旧服基地"), "其他服务器条目必须隔离");
        assert!(text_s1.contains("钻石镐配方"), "全局知识应始终注入");
        assert_eq!(touched_s1.len(), 2);

        // 无 scope（通用会话）：只注入全局知识，坐标类全部隔离
        let (text_none, _) = m.injection_text("基地", None);
        assert!(
            !text_none.contains("基地坐标"),
            "无 scope 时服务器特定记忆不得注入"
        );
        assert!(text_none.contains("钻石镐配方"));

        // 其他服务器：只看到自己的 + 全局
        let (text_s2, _) = m.injection_text("基地", Some("s2"));
        assert!(text_s2.contains("旧服基地"));
        assert!(!text_s2.contains("基地坐标"));
    }
}
