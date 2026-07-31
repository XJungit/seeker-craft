//! Session 持久化 —— 严格基于 pi_agent_rust src/session.rs 核心机制（适配 Minecraft Agent）
//!
//! 与 pi 同构的部分:
//!   - JSONL 树: 文件第 1 行 = `SessionHeader`, 后续每行 = 一个 `SessionEntry`
//!   - 每个 entry 带 `id` + `parent_id`, 形成分支树; `header.current_leaf` 指向当前分支末端
//!   - `entries_for_current_path()`: 从 leaf 沿 `parent_id` 回溯重建当前分支
//!     (pi 快路径 `is_linear` 为真时直接返回全部 entries, 否则回溯)
//!   - 增量持久化: `persisted_count`(high-water mark) 只 append 新 entries;
//!     header 变脏(branch/leaf 改变)时 `full_rewrite` 重写整个文件
//!   - `branch_from`: 从任意 entry fork 出新分支 (pi `/fork` / `ForkPlan`)
//!   - `append_checkpoint`: 保存 Agent 状态恢复点 (pi `CompactionEntry` + snapshot 思想)
//!   - `finalize()`: 加载后重建 `entry_index` / `leaf_id` / `is_linear` (pi `finalize_loaded_entries`)
//!
//! 相对 pi 的简化（场景无关部分直接去掉，不抄名字）:
//!   - 去掉 TUI / extensions / session_index 集成（MC agent 单进程不需要）
//!   - 去掉 write-behind autosave 后台线程（每轮 save 一次足够，崩溃可恢复）
//!   - id 用 `时间戳 + 原子计数`（pi 用 uuid 8-hex）；timestamp 用毫秒字符串（pi 用 RFC3339）

use crate::core::message::{Message, Usage};
use crate::core::prompt::WorldInfo;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Session 文件格式版本（与 pi `SESSION_VERSION` 同构，独立演进）
pub const SESSION_VERSION: u8 = 1;

/// 文件第 1 行（与 pi `SessionHeader` 同构：id/timestamp/cwd 对应我们的 id/timestamp/game，
/// 额外用 `leafId` 记录当前分支末端）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionHeader {
    #[serde(rename = "type")]
    pub type_: String,
    pub version: u8,
    pub id: String,
    pub timestamp: String,
    /// 游戏标识（pi 用 cwd，我们是 `minecraft`）
    pub game: String,
    /// 当前分支末端 entry id（pi `current_leaf`）
    #[serde(rename = "leafId", skip_serializing_if = "Option::is_none")]
    pub current_leaf: Option<String>,
    /// 知识自初始化是否已完成（避免每次重开都让 LLM 重新播种 WorldInfo）
    #[serde(default)]
    pub knowledge_bootstrapped: bool,
}

impl SessionHeader {
    pub fn new(game: &str) -> Self {
        Self {
            type_: "session".into(),
            version: SESSION_VERSION,
            id: gen_id(),
            timestamp: now_ms(),
            game: game.into(),
            current_leaf: None,
            knowledge_bootstrapped: false,
        }
    }
}

/// 一个 session entry（树节点，与 pi `SessionEntry` 同构：tag=type 区分变体）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEntry {
    Message(MessageEntry),
    Checkpoint(CheckpointEntry),
    Compaction(CompactionEntry),
    BranchSummary(BranchSummaryEntry),
    WorldInfo(WorldInfoEntry),
    Memory(MemoryEntry),
    Custom(CustomEntry),
}

impl SessionEntry {
    pub fn id(&self) -> &str {
        match self {
            Self::Message(e) => &e.id,
            Self::Checkpoint(e) => &e.id,
            Self::Compaction(e) => &e.id,
            Self::BranchSummary(e) => &e.id,
            Self::WorldInfo(e) => &e.id,
            Self::Memory(e) => &e.id,
            Self::Custom(e) => &e.id,
        }
    }
    pub fn parent_id(&self) -> Option<&str> {
        match self {
            Self::Message(e) => e.parent_id.as_deref(),
            Self::Checkpoint(e) => e.parent_id.as_deref(),
            Self::Compaction(e) => e.parent_id.as_deref(),
            Self::BranchSummary(e) => e.parent_id.as_deref(),
            Self::WorldInfo(e) => e.parent_id.as_deref(),
            Self::Memory(e) => e.parent_id.as_deref(),
            Self::Custom(e) => e.parent_id.as_deref(),
        }
    }
}

/// 一条对话消息 entry（与 pi `MessageEntry` 同构）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MessageEntry {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub timestamp: String,
    /// 直接复用我们已可序列化的 `Message`（User/Assistant/ToolResult）
    pub message: Message,
}

/// 检查点 entry（手动保存的可恢复点）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CheckpointEntry {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub timestamp: String,
    pub label: String,
    /// 可恢复的 Agent 状态快照
    pub snapshot: AgentSnapshot,
}

/// Agent 状态快照（中断恢复 / 压缩点）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSnapshot {
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_summary: Option<String>,
    pub usage: Usage,
    pub turn: u32,
    /// 技能库 JSON（序列化的 SkillLibrary，加载时回放）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills_json: Option<String>,
}

/// 压缩 entry（与 pi `CompactionEntry` 同构，记录摘要和保留区间）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CompactionEntry {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub timestamp: String,
    /// LLM 生成的压缩摘要
    pub summary: String,
    /// 压缩后第一条保留的 entry id（之前的历史被摘要替代）
    pub first_kept_entry_id: String,
    /// 压缩前上下文 token 数
    pub tokens_before: u64,
    /// 可选元数据（如 readFiles / modifiedFiles）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

/// 分支摘要 entry（与 pi `BranchSummaryEntry` 同构，记录从哪个节点 fork）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BranchSummaryEntry {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub timestamp: String,
    pub from_id: String,
    pub summary: String,
}

/// 自定义 entry（与 pi `CustomEntry` 同构，给外围挂载任意数据）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CustomEntry {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub timestamp: String,
    pub custom_type: String,
    pub data: Value,
}

pub const SESSION_ROLLOVER_CUSTOM_TYPE: &str = "session_rollover";
/// Custom entry type used by the deterministic Minecraft task chain.
/// Session rollover preserves it without depending on the task module.
pub const TASK_STATE_CUSTOM_TYPE: &str = "task_manager_state";

/// Runtime state carried into a compact replacement session.
#[derive(Debug, Clone)]
pub struct SessionRolloverContext {
    pub recovery_summary: String,
    pub current_goal: Option<String>,
    pub position: Option<[f64; 3]>,
    pub health: Option<f32>,
    pub hunger: Option<u32>,
}

/// Audit link from a compact active session to its exact archived predecessor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRolloverMetadata {
    pub archive_path: String,
    pub archived_session_id: String,
    pub archived_leaf_id: Option<String>,
    pub archived_at: String,
    pub current_goal: Option<String>,
    pub position: Option<[f64; 3]>,
    pub health: Option<f32>,
    pub hunger: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct SessionRolloverResult {
    pub archive_path: PathBuf,
    pub archived_session_id: String,
    pub archived_leaf_id: Option<String>,
    pub active_session_id: String,
}

/// WorldInfo 知识库变更 entry（Agent 长期知识持久化，跨重启保留）
///
/// 与 `CustomEntry` 的区别：它是 `WorldInfoLib` 增删操作的一等公民，
/// `Agent::with_session` 打开时会回放所有 `WorldInfo` 沿当前分支的 entry，
/// 重建 `world_info`，因此 LLM 在上一局学到的知识在下一局开局即生效。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorldInfoEntry {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub timestamp: String,
    /// "add" 或 "remove"
    pub action: String,
    /// add 时携带完整条目；remove 时为 None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<WorldInfo>,
    /// remove 时按 id 精确删除
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remove_id: Option<String>,
    /// remove 时按关键词删除
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remove_keys: Option<Vec<String>>,
}

/// WorldMemory 持久化 entry（空间-状态长期记忆跨重启保留）。
///
/// 与 `WorldInfoEntry` 类似，是 `WorldMemory` 变更的一等公民：
/// `Agent` 每轮把当前 `WorldMemory` 快照作为 `Memory` entry append，
/// 重新打开 session 时回放所有 `Memory` entry 重建记忆库。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MemoryEntry {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub timestamp: String,
    /// WorldMemory 的完整 JSON 快照（cells + anchors）。
    pub snapshot: String,
}

/// 一个可持久化的 session（与 pi `Session` 同构，去掉 TUI/extensions/autosave 线程）
pub struct Session {
    pub header: SessionHeader,
    pub entries: Vec<SessionEntry>,
    pub path: Option<PathBuf>,
    /// 当前分支末端 entry id（pi `leaf_id`，直接修改会破坏 `is_linear` 缓存，故 pub(crate)）
    pub(crate) leaf_id: Option<String>,
    /// 所有 entry 形成线性链（无分支）时为 true —— 快路径直接返回全部（pi `is_linear`）
    is_linear: bool,
    /// id → entries 下标，O(1) 查找（pi `entry_index`）
    entry_index: HashMap<String, usize>,
    /// 已持久化到磁盘的 entry 数（high-water mark，pi `persisted_entry_count`）
    persisted_count: usize,
    /// header 是否改动需全量重写（pi `header_dirty`）
    header_dirty: bool,
}

impl Session {
    /// 新建内存 session（不绑定路径，save 前需 save_to）
    pub fn new(game: &str) -> Self {
        Self {
            header: SessionHeader::new(game),
            entries: vec![],
            path: None,
            leaf_id: None,
            is_linear: true,
            entry_index: HashMap::new(),
            persisted_count: 0,
            header_dirty: false,
        }
    }

    /// 从磁盘打开已有 session（pi `open_jsonl_blocking`）
    pub fn open(path: &Path) -> Result<Self> {
        let file =
            File::open(path).with_context(|| format!("打开 session 失败: {}", path.display()))?;
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .context("session 文件为空或无法读取 header")?;
        let header: SessionHeader =
            serde_json::from_str(line.trim()).context("解析 session header 失败")?;

        let mut entries: Vec<SessionEntry> = vec![];
        let mut buf = String::new();
        loop {
            buf.clear();
            let n = reader.read_line(&mut buf)?;
            if n == 0 {
                break;
            }
            let t = buf.trim();
            if t.is_empty() {
                continue;
            }
            // pi 会记录 skipped_entries + orphaned；我们简化：解析失败告警并跳过
            match serde_json::from_str::<SessionEntry>(t) {
                Ok(e) => entries.push(e),
                Err(err) => eprintln!("[session] 跳过无法解析的 entry: {err}"),
            }
        }

        let mut s = Self {
            header,
            entries,
            path: Some(path.to_path_buf()),
            leaf_id: None,
            is_linear: true,
            entry_index: HashMap::new(),
            persisted_count: 0,
            header_dirty: false,
        };
        s.finalize();
        s.persisted_count = s.entries.len();
        Ok(s)
    }

    /// 重建 entry_index / leaf_id / is_linear（pi `finalize_loaded_entries`）
    fn finalize(&mut self) {
        self.entry_index.clear();
        let mut has_branching = false;
        let mut root_count = u32::MIN;
        let mut leaf: Option<String> = None;
        let mut parent_child_count: HashMap<Option<String>, u32> = HashMap::new();

        for (idx, e) in self.entries.iter().enumerate() {
            let id = e.id().to_string();
            self.entry_index.insert(id, idx);
            leaf = Some(e.id().to_string());

            let p = e.parent_id().map(|s| s.to_string());
            match &p {
                Some(_) => {}
                None => root_count += 1,
            }
            // branch 检测：同一 parent 出现 >1 次子节点
            if !has_branching {
                let count = parent_child_count.entry(p).or_insert(0);
                *count += 1;
                if *count > 1 {
                    has_branching = true;
                }
            }
        }

        // leaf: 优先用 header.current_leaf（有效时），否则取自然末端。
        // 选中的 leaf 不是自然末端时禁止 linear 快路径，否则会越过该 leaf。
        let natural_leaf = leaf;
        if let Some(l) = &self.header.current_leaf
            && self.entry_index.contains_key(l)
        {
            self.leaf_id = Some(l.clone());
        } else {
            self.leaf_id = natural_leaf.clone();
        }
        self.is_linear = !has_branching && root_count <= 1 && self.leaf_id == natural_leaf;
    }

    /// 保存到已绑定路径（pi `save`：header 脏 → full_rewrite，否则 append 新 entries）
    pub fn save(&mut self) -> Result<()> {
        let path = self
            .path
            .clone()
            .context("session 未绑定路径，请先用 save_to 或 open")?;
        // 先增量追加新 entries（如果有）
        if self.persisted_count < self.entries.len() {
            self.append_entries(&path)?;
        }
        // 再更新 header（如果 dirty）——只重写 header 行，不重写全部 entries
        if self.header_dirty || self.persisted_count == 0 {
            self.rewrite_header_only(&path)?;
            self.header_dirty = false;
        }
        Ok(())
    }

    /// 保存并绑定到指定路径（首次落盘用）
    pub fn save_to(&mut self, path: &Path) -> Result<()> {
        self.path = Some(path.to_path_buf());
        self.full_rewrite(path)
    }

    /// Archive an existing session byte-for-byte and atomically replace it with
    /// a compact recovery session. This must run while no Session writer exists.
    pub fn rollover_to(
        path: &Path,
        archive_dir: &Path,
        context: SessionRolloverContext,
    ) -> Result<Option<SessionRolloverResult>> {
        if !path.exists() || path.metadata()?.len() == 0 {
            return Ok(None);
        }

        // Parse before publishing files. Header corruption must leave the active
        // session untouched rather than silently discarding history.
        let old = Self::open(path)?;
        let original = std::fs::read(path)?;
        let archived_session_id = old.header.id.clone();
        let archived_leaf_id = old.current_leaf().map(str::to_string);
        let archived_at = now_ms();

        std::fs::create_dir_all(archive_dir)?;
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("session");
        let archive_path = unique_archive_path(
            archive_dir,
            &format!("{stem}.{archived_session_id}.{archived_at}"),
        );
        let archive_tmp = tempfile_path(&archive_path);
        write_new_synced(&archive_tmp, &original)?;
        if let Err(err) = std::fs::rename(&archive_tmp, &archive_path) {
            let _ = std::fs::remove_file(&archive_tmp);
            return Err(err.into());
        }

        let metadata = SessionRolloverMetadata {
            archive_path: archive_path.display().to_string(),
            archived_session_id: archived_session_id.clone(),
            archived_leaf_id: archived_leaf_id.clone(),
            archived_at,
            current_goal: context.current_goal,
            position: context.position,
            health: context.health,
            hunger: context.hunger,
        };
        let current_path = old.entries_for_current_path();
        let latest_memory = current_path.iter().rev().find_map(|entry| match entry {
            SessionEntry::Memory(memory) => Some(memory.snapshot.clone()),
            _ => None,
        });
        let latest_task_state = current_path.iter().rev().find_map(|entry| match entry {
            SessionEntry::Custom(custom) if custom.custom_type == TASK_STATE_CUSTOM_TYPE => {
                Some(custom.data.clone())
            }
            _ => None,
        });
        let world_info: Vec<WorldInfoEntry> = current_path
            .iter()
            .filter_map(|entry| match entry {
                SessionEntry::WorldInfo(info) => Some(info.clone()),
                _ => None,
            })
            .collect();

        let mut compact = Self::new(&old.header.game);
        compact.header.knowledge_bootstrapped = old.header.knowledge_bootstrapped;
        compact.append_custom(
            SESSION_ROLLOVER_CUSTOM_TYPE,
            serde_json::to_value(metadata)?,
        );
        compact.append_message(Message::user(context.recovery_summary));
        if let Some(task_state) = latest_task_state {
            compact.append_custom(TASK_STATE_CUSTOM_TYPE, task_state);
        }
        // Preserve WorldInfo operations in order so with_session rebuilds the
        // same effective library without retaining unrelated conversation.
        for info in world_info {
            compact.append_world_info(&info.action, info.info, info.remove_id, info.remove_keys);
        }
        if let Some(snapshot) = latest_memory {
            compact.append_memory(&snapshot);
        }

        let active_session_id = compact.header.id.clone();
        let active_tmp = tempfile_path(path);
        compact.full_rewrite(&active_tmp)?;
        if let Err(err) = replace_file(&active_tmp, path) {
            let _ = std::fs::remove_file(&active_tmp);
            return Err(err);
        }

        Ok(Some(SessionRolloverResult {
            archive_path,
            archived_session_id,
            archived_leaf_id,
            active_session_id,
        }))
    }

    /// 全量重写：临时文件写 header + 全部 entries，再 rename 原子替换（pi save_jsonl_full_rewrite_blocking）
    fn full_rewrite(&mut self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let tmp = tempfile_path(path);
        {
            let f = File::create(&tmp)?;
            let mut w = BufWriter::new(f);
            serde_json::to_writer(&mut w, &self.header)?;
            w.write_all(b"\n")?;
            for e in &self.entries {
                serde_json::to_writer(&mut w, e)?;
                w.write_all(b"\n")?;
            }
            w.flush()?;
            w.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        }
        replace_file(&tmp, path)?;
        self.persisted_count = self.entries.len();
        self.header_dirty = false;
        Ok(())
    }

    /// 增量追加：只写 `entries[persisted_count..]`（pi append_jsonl_entries_blocking）
    fn append_entries(&mut self, path: &Path) -> Result<()> {
        let mut f = std::fs::OpenOptions::new().append(true).open(path)?;
        for e in &self.entries[self.persisted_count..] {
            serde_json::to_writer(&mut f, e)?;
            f.write_all(b"\n")?;
        }
        f.flush()?;
        self.persisted_count = self.entries.len();
        Ok(())
    }

    /// 只重写 header 行（第一行），保留已有 entries。
    /// 比 full_rewrite 高效：不需要序列化全部 entries。
    fn rewrite_header_only(&mut self, path: &Path) -> Result<()> {
        if !path.exists() {
            // 文件不存在时走 full_rewrite
            return self.full_rewrite(path);
        }
        // 读取全部行，替换第一行
        let content = std::fs::read_to_string(path)?;
        let mut lines: Vec<&str> = content.lines().collect();
        if lines.is_empty() {
            return self.full_rewrite(path);
        }
        let new_header = serde_json::to_string(&self.header)?;
        lines[0] = &new_header;
        let tmp = tempfile_path(path);
        std::fs::write(&tmp, lines.join("\n") + "\n")?;
        replace_file(&tmp, path)?;
        Ok(())
    }

    /// 追加一条对话消息，返回新 entry id（parent = 当前 leaf）
    pub fn append_message(&mut self, msg: Message) -> String {
        let id = gen_id();
        let entry = SessionEntry::Message(MessageEntry {
            id: id.clone(),
            parent_id: self.leaf_id.clone(),
            timestamp: now_ms(),
            message: msg,
        });
        self.push_entry(entry, false);
        id
    }

    /// 追加一个检查点（保存 Agent 状态恢复点）
    pub fn append_checkpoint(&mut self, label: &str, snapshot: AgentSnapshot) -> String {
        let id = gen_id();
        let entry = SessionEntry::Checkpoint(CheckpointEntry {
            id: id.clone(),
            parent_id: self.leaf_id.clone(),
            timestamp: now_ms(),
            label: label.into(),
            snapshot,
        });
        self.push_entry(entry, false);
        id
    }

    /// 追加一个压缩 entry（与 pi `CompactionEntry` 同构）
    pub fn append_compaction(
        &mut self,
        summary: String,
        first_kept_entry_id: String,
        tokens_before: u64,
        details: Option<Value>,
    ) -> String {
        let id = gen_id();
        let entry = SessionEntry::Compaction(CompactionEntry {
            id: id.clone(),
            parent_id: self.leaf_id.clone(),
            timestamp: now_ms(),
            summary,
            first_kept_entry_id,
            tokens_before,
            details,
        });
        self.push_entry(entry, false);
        id
    }

    /// 追加自定义 entry（外围挂数据用）
    pub fn append_custom(&mut self, custom_type: &str, data: Value) -> String {
        let id = gen_id();
        let entry = SessionEntry::Custom(CustomEntry {
            id: id.clone(),
            parent_id: self.leaf_id.clone(),
            timestamp: now_ms(),
            custom_type: custom_type.into(),
            data,
        });
        self.push_entry(entry, false);
        id
    }

    /// 追加一条 WorldInfo 知识库变更（add/remove）。`Agent::with_session` 打开时回放。
    pub fn append_world_info(
        &mut self,
        action: &str,
        info: Option<WorldInfo>,
        remove_id: Option<String>,
        remove_keys: Option<Vec<String>>,
    ) -> String {
        let id = gen_id();
        let entry = SessionEntry::WorldInfo(WorldInfoEntry {
            id: id.clone(),
            parent_id: self.leaf_id.clone(),
            timestamp: now_ms(),
            action: action.into(),
            info,
            remove_id,
            remove_keys,
        });
        self.push_entry(entry, false);
        id
    }

    /// 追加一条 WorldMemory 快照 entry（每轮记忆变更后调用）。
    pub fn append_memory(&mut self, snapshot: &str) -> String {
        let id = gen_id();
        let entry = SessionEntry::Memory(MemoryEntry {
            id: id.clone(),
            parent_id: self.leaf_id.clone(),
            timestamp: now_ms(),
            snapshot: snapshot.to_string(),
        });
        self.push_entry(entry, false);
        id
    }

    /// 从某个 entry 分叉出新分支。BranchSummary 本身是新分支的首节点，后续消息
    /// 挂在 summary 后面，确保摘要在当前路径可达。
    pub fn branch_from(&mut self, entry_id: &str) -> Result<()> {
        if !self.entry_index.contains_key(entry_id) {
            anyhow::bail!("branch 目标不存在: {entry_id}");
        }
        let summary_id = gen_id();
        let entry = SessionEntry::BranchSummary(BranchSummaryEntry {
            id: summary_id,
            parent_id: Some(entry_id.to_string()),
            timestamp: now_ms(),
            from_id: entry_id.to_string(),
            summary: format!("branch from {entry_id}"),
        });
        self.push_entry(entry, true);
        self.is_linear = false;
        Ok(())
    }

    /// 内部：push 一个 entry 并更新索引/leaf（touch_header=true 时标记 header 脏）
    fn push_entry(&mut self, entry: SessionEntry, touch_header: bool) {
        let id = entry.id().to_string();
        let idx = self.entries.len();
        self.entry_index.insert(id.clone(), idx);
        // 若已有兄弟节点（同一 parent），说明发生分支
        if let Some(pid) = entry.parent_id() {
            let siblings = self
                .entries
                .iter()
                .filter(|e| e.parent_id() == Some(pid))
                .count();
            if siblings > 0 {
                self.is_linear = false;
            }
        }
        self.entries.push(entry);
        self.leaf_id = Some(id.clone());
        self.header.current_leaf = Some(id);
        // 首次落盘前无需额外标脏；文件已存在时 leaf 变化需要更新 header。
        // 但增量 append 时不需要全量重写——save() 会判断是否只追加了 entries。
        if touch_header || self.persisted_count > 0 {
            self.header_dirty = true;
        }
    }

    /// 当前分支的 entry 列表（沿 parent 链回溯，pi `entries_for_current_path`）
    pub fn entries_for_current_path(&self) -> Vec<&SessionEntry> {
        let Some(leaf) = &self.leaf_id else {
            return vec![];
        };
        // 快路径：线性 session 直接返回全部
        if self.is_linear {
            return self.entries.iter().collect();
        }
        let mut path: Vec<usize> = Vec::with_capacity(16);
        let mut visited: HashSet<String> = HashSet::new();
        let mut current = Some(leaf.clone());
        while let Some(id) = current {
            if !visited.insert(id.clone()) {
                eprintln!("[session] 检测到树中存在环，停止回溯");
                break;
            }
            let Some(&idx) = self.entry_index.get(&id) else {
                break;
            };
            path.push(idx);
            current = self.entries[idx].parent_id().map(|s| s.to_string());
        }
        path.reverse();
        path.into_iter()
            .filter_map(|i| self.entries.get(i))
            .collect()
    }

    /// 把当前分支的 entry 还原为 `Message` 列表（给 Agent 加载恢复，pi `to_messages_for_current_path`）
    /// 遇到最近的 Checkpoint 时，从它的快照开始（之后追加的 MessageEntry 继续）
    pub fn messages_for_current_path(&self) -> Vec<Message> {
        let path = self.entries_for_current_path();
        // 找路径上最近的 checkpoint
        let mut checkpoint_idx: Option<usize> = None;
        for (i, e) in path.iter().enumerate().rev() {
            if matches!(e, SessionEntry::Checkpoint(_)) {
                checkpoint_idx = Some(i);
                break;
            }
        }
        let mut out: Vec<Message> = Vec::new();
        match checkpoint_idx {
            Some(ci) => {
                if let SessionEntry::Checkpoint(cp) = &path[ci] {
                    out.extend(cp.snapshot.messages.clone());
                }
                for e in &path[ci + 1..] {
                    if let SessionEntry::Message(m) = e {
                        out.push(m.message.clone());
                    }
                }
            }
            None => {
                for e in &path {
                    if let SessionEntry::Message(m) = e {
                        out.push(m.message.clone());
                    }
                }
            }
        }
        out
    }

    /// 当前分支末端 id
    pub fn current_leaf(&self) -> Option<&str> {
        self.leaf_id.as_deref()
    }
    /// 标记 header 需全量重写（如知识自初始化标志变更时）
    pub fn mark_header_dirty(&mut self) {
        self.header_dirty = true;
    }
    /// 是否为线性 session（无分支）
    pub fn is_linear(&self) -> bool {
        self.is_linear
    }
    /// 全部 entries（只读）
    pub fn entries(&self) -> &[SessionEntry] {
        &self.entries
    }
}

// ── 辅助函数 ──

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 生成唯一 id（pi 用 uuid 8-hex；我们用 `纳秒时间戳 + 原子计数` 保证单机唯一）
fn gen_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let c = ID_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("{nanos:x}{c:x}")
}

fn now_ms() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
        .to_string()
}

fn tempfile_path(target: &Path) -> PathBuf {
    let mut p = target.to_path_buf();
    let name = format!(".session_{}.tmp", gen_id());
    p.set_file_name(name);
    p
}

fn unique_archive_path(dir: &Path, base: &str) -> PathBuf {
    let mut candidate = dir.join(format!("{base}.jsonl"));
    let mut suffix = 1u32;
    while candidate.exists() {
        candidate = dir.join(format!("{base}.{suffix}.jsonl"));
        suffix += 1;
    }
    candidate
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.flush()?;
    file.sync_all()?;
    Ok(())
}

/// 跨平台替换目标文件。Windows 的 rename 不能覆盖已有目标，因此先把旧文件改名为
/// backup，再把 tmp 提升为目标；失败时恢复 backup。成功后删除 backup。
fn replace_file(tmp: &Path, target: &Path) -> Result<()> {
    if !target.exists() {
        std::fs::rename(tmp, target)?;
        return Ok(());
    }
    let backup = target.with_extension(format!("jsonl.{}.bak", gen_id()));
    std::fs::rename(target, &backup)?;
    match std::fs::rename(tmp, target) {
        Ok(()) => {
            std::fs::remove_file(backup)?;
            Ok(())
        }
        Err(err) => {
            let _ = std::fs::rename(&backup, target);
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::message::Message;

    fn tmp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir();
        dir.join(format!("craft_agent_session_test_{name}.jsonl"))
    }

    #[test]
    fn persists_messages_and_reloads() {
        let path = tmp_path("persist");
        let _ = std::fs::remove_file(&path);
        let mut s = Session::new("minecraft");
        s.append_message(Message::user("hi"));
        s.append_message(Message::assistant_text("hello"));
        s.append_message(Message::tool_result("c1", "perceive", "tree"));
        s.save_to(&path).unwrap();

        let reloaded = Session::open(&path).unwrap();
        let msgs = reloaded.messages_for_current_path();
        assert_eq!(msgs.len(), 3, "应恢复 3 条消息");
        assert!(matches!(msgs[0], Message::User(_)));
        assert!(matches!(msgs[1], Message::Assistant(_)));
        assert!(matches!(msgs[2], Message::ToolResult(_)));
        assert!(reloaded.is_linear(), "纯追加应为线性");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn branch_creates_divergent_path() {
        let mut s = Session::new("minecraft");
        s.append_message(Message::user("a"));
        let b = s.append_message(Message::user("b"));
        s.append_message(Message::user("c")); // c 是 b 之后的另一条分支
        // 从 b 分叉
        s.branch_from(&b).unwrap();
        s.append_message(Message::user("d")); // d 在 b 之后，与 c 分叉

        let path = s.entries_for_current_path();
        let ids: Vec<&str> = path.iter().map(|e| e.id()).collect();
        assert!(ids.contains(&b.as_str()), "分支路径应包含 b");
        // 当前路径不应包含 c（c 在另一条分支）
        let c_present = path.iter().any(|e| match e {
            SessionEntry::Message(m) => matches!(&m.message, Message::User(u) if u.content == "c"),
            _ => false,
        });
        assert!(!c_present, "分支路径不应包含 c");
        let d_present = path.iter().any(|e| match e {
            SessionEntry::Message(m) => matches!(&m.message, Message::User(u) if u.content == "d"),
            _ => false,
        });
        assert!(d_present, "分支路径应包含 d");
        assert!(!s.is_linear(), "发生分叉后应为非线性");
    }

    #[test]
    fn checkpoint_roundtrip() {
        let path = tmp_path("checkpoint");
        let _ = std::fs::remove_file(&path);
        let mut s = Session::new("minecraft");
        s.append_message(Message::user("before"));
        s.append_checkpoint(
            "test",
            AgentSnapshot {
                messages: vec![Message::user("before")],
                previous_summary: Some("summary-x".into()),
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    total_tokens: 15,
                    ..Default::default()
                },
                turn: 3,
                skills_json: None,
            },
        );
        s.append_message(Message::assistant_text("after"));
        s.save_to(&path).unwrap();

        let reloaded = Session::open(&path).unwrap();
        let msgs = reloaded.messages_for_current_path();
        // 应从 checkpoint 快照 [before] + 之后的 [after]
        assert_eq!(msgs.len(), 2, "checkpoint 重建应包含 snapshot + 之后消息");
        assert!(matches!(msgs[0], Message::User(_)));
        assert!(matches!(msgs[1], Message::Assistant(_)));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn incremental_append_does_not_rewrite_header() {
        // 验证多次 append_message 走 append 模式（persisted_count 推进，不重写 header）
        let path = tmp_path("append");
        let _ = std::fs::remove_file(&path);
        let mut s = Session::new("minecraft");
        s.append_message(Message::user("x1"));
        s.save_to(&path).unwrap();
        let first_size = std::fs::metadata(&path).unwrap().len();
        s.append_message(Message::user("x2"));
        s.append_message(Message::user("x3"));
        s.save().unwrap();
        let second_size = std::fs::metadata(&path).unwrap().len();
        // 第二次只增量追加，文件应比首次大（含 2 条新 entry），且首行 header 不变
        assert!(second_size > first_size, "增量 append 应使文件变大");
        let reloaded = Session::open(&path).unwrap();
        assert_eq!(reloaded.messages_for_current_path().len(), 3);
        let _ = std::fs::remove_file(&path);
    }

    fn rollover_context(summary: &str) -> SessionRolloverContext {
        SessionRolloverContext {
            recovery_summary: summary.into(),
            current_goal: Some("find food".into()),
            position: Some([1.0, 2.0, 3.0]),
            health: Some(20.0),
            hunger: Some(10),
        }
    }

    #[test]
    fn rollover_archives_exact_original_and_creates_compact_session() {
        let path = tmp_path("rollover");
        let archive_dir = path.with_extension("archive");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&archive_dir);
        let mut old = Session::new("minecraft");
        old.header.knowledge_bootstrapped = true;
        old.append_message(Message::user("large old history"));
        old.append_memory(r#"{"cells":["latest"]}"#);
        old.save_to(&path).unwrap();
        let original = std::fs::read(&path).unwrap();
        let old_id = old.header.id.clone();

        let result = Session::rollover_to(&path, &archive_dir, rollover_context("recover"))
            .unwrap()
            .unwrap();
        assert_eq!(std::fs::read(&result.archive_path).unwrap(), original);
        assert_eq!(result.archived_session_id, old_id);

        let compact = Session::open(&path).unwrap();
        assert_ne!(compact.header.id, old_id);
        assert!(compact.header.knowledge_bootstrapped);
        assert!(compact.is_linear());
        assert_eq!(compact.entries_for_current_path().len(), 3);
        assert!(matches!(
            &compact.messages_for_current_path()[0],
            Message::User(user) if user.content == "recover"
        ));
        assert_eq!(
            compact.current_leaf(),
            compact.header.current_leaf.as_deref()
        );
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&archive_dir);
    }

    #[test]
    fn rollover_uses_latest_memory_on_selected_branch() {
        let path = tmp_path("rollover_branch");
        let archive_dir = path.with_extension("archive");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&archive_dir);
        let mut old = Session::new("minecraft");
        let branch_point = old.append_message(Message::user("root"));
        old.append_memory("abandoned");
        old.branch_from(&branch_point).unwrap();
        old.append_memory("selected");
        old.save_to(&path).unwrap();

        Session::rollover_to(&path, &archive_dir, rollover_context("recover")).unwrap();
        let compact = Session::open(&path).unwrap();
        let memories: Vec<&str> = compact
            .entries_for_current_path()
            .into_iter()
            .filter_map(|entry| match entry {
                SessionEntry::Memory(memory) => Some(memory.snapshot.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(memories, vec!["selected"]);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&archive_dir);
    }

    #[test]
    fn rollover_preserves_latest_task_state() {
        let path = tmp_path("rollover_task_state");
        let archive_dir = path.with_extension("archive");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&archive_dir);
        let mut old = Session::new("minecraft");
        old.append_custom(
            TASK_STATE_CUSTOM_TYPE,
            serde_json::json!({
                "current_id": "tier1_crafting_table",
                "statuses": {
                    "tier1_gather_wood": {"Completed": {"finished_at": 42}}
                }
            }),
        );
        old.save_to(&path).unwrap();

        Session::rollover_to(&path, &archive_dir, rollover_context("recover")).unwrap();
        let compact = Session::open(&path).unwrap();
        let state = compact
            .entries_for_current_path()
            .into_iter()
            .find_map(|entry| match entry {
                SessionEntry::Custom(custom) if custom.custom_type == TASK_STATE_CUSTOM_TYPE => {
                    Some(custom.data.clone())
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(state["current_id"].as_str(), Some("tier1_crafting_table"));
        assert_eq!(
            state["statuses"]["tier1_gather_wood"]["Completed"]["finished_at"],
            serde_json::json!(42)
        );
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&archive_dir);
    }

    #[test]
    fn rollover_metadata_links_archive_and_goal() {
        let path = tmp_path("rollover_metadata");
        let archive_dir = path.with_extension("archive");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&archive_dir);
        let mut old = Session::new("minecraft");
        old.append_message(Message::user("old"));
        old.save_to(&path).unwrap();
        let old_leaf = old.current_leaf().map(str::to_string);

        let result = Session::rollover_to(&path, &archive_dir, rollover_context("recover"))
            .unwrap()
            .unwrap();
        let compact = Session::open(&path).unwrap();
        let metadata = compact
            .entries_for_current_path()
            .into_iter()
            .find_map(|entry| match entry {
                SessionEntry::Custom(custom)
                    if custom.custom_type == SESSION_ROLLOVER_CUSTOM_TYPE =>
                {
                    serde_json::from_value::<SessionRolloverMetadata>(custom.data.clone()).ok()
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(metadata.archived_leaf_id, old_leaf);
        assert_eq!(metadata.current_goal.as_deref(), Some("find food"));
        assert_eq!(Path::new(&metadata.archive_path), result.archive_path);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&archive_dir);
    }

    #[test]
    fn corrupt_session_rollover_does_not_touch_active_file() {
        let path = tmp_path("rollover_corrupt");
        let archive_dir = path.with_extension("archive");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&archive_dir);
        let corrupt = b"not a session\n";
        std::fs::write(&path, corrupt).unwrap();

        assert!(Session::rollover_to(&path, &archive_dir, rollover_context("recover")).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), corrupt);
        assert!(!archive_dir.exists());
        let _ = std::fs::remove_file(&path);
    }
}
