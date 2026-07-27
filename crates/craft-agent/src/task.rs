//! 任务系统骨架（学习自 Mindcraft tasks/ 目录 + task_loader.js）。
//!
//! 设计要点：
//! - 任务是 JSON 文件，含 id/name/description/goal/objective/success_condition
//! - `goal` 是给 LLM 的 self_prompt（高层目标，如"收集木头做工作台"）
//! - `objective` 是结构化完成条件（如 inventory_has: crafting_table, count: 1）
//! - `success_condition` 是 Python 风格的简单表达式（暂不实现解释器，先用结构化条件）
//! - 加载：`tasks/` 目录扫描，每个 JSON 一个 Task
//! - 完成判定：检查背包/位置/状态是否满足 objective
//!
//! 与 Mindcraft 的差异：
//! - Mindcraft 用 JavaScript 函数判断完成；我们用结构化条件（更安全，避免代码注入）
//! - 任务不直接执行动作，只提供 goal + 完成判定；执行由 Agent 主循环驱动

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::path::PathBuf;

/// 单个完成条件（结构化，非代码）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum SuccessCondition {
    /// 背包含指定物品 ≥ count
    InventoryHas { item: String, count: u32 },
    /// 背包含指定物品 == count（精确）
    InventoryExact { item: String, count: u32 },
    /// bot 位于指定坐标附近（半径 radius 格）
    AtPosition { x: i32, y: i32, z: i32, radius: u32 },
    /// 已挖到指定 Y 坐标以下
    BelowY { y: i32 },
    /// 已击杀指定数量实体（统计用，需外部回填）
    Killed { entity_kind: String, count: u32 },
    /// 已合成指定物品 ≥ count（与 InventoryHas 类似但语义是"造出来过"）
    Crafted { item: String, count: u32 },
    /// 已放置指定方块 ≥ count
    Placed { block: String, count: u32 },
    /// 复合条件：全部满足
    All { conditions: Vec<SuccessCondition> },
    /// 复合条件：任一满足
    Any { conditions: Vec<SuccessCondition> },
}

/// 任务定义（对应 tasks/*.json）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// 任务唯一 id（如 "tier1_crafting_table"）
    pub id: String,
    /// 任务名（人类可读，如 "制作工作台"）
    pub name: String,
    /// 任务描述（背景故事/上下文，给 LLM 看）
    pub description: String,
    /// 给 LLM 的 self_prompt（高层目标，如"收集木头做工作台"）
    pub goal: String,
    /// 任务难度 tier（1-6，对齐 Mindcraft 的 6 tier）
    pub tier: u32,
    /// 完成条件（结构化）
    pub success: SuccessCondition,
    /// 可选：任务失败条件（如生命归零/掉出世界）
    #[serde(default)]
    pub failure: Option<SuccessCondition>,
    /// 可选：建议用时（秒），超时标记 failed
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    /// 可选：奖励物品（任务完成后给 LLM 的正反馈描述）
    #[serde(default)]
    pub reward: Option<String>,
}

/// 任务运行时状态。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskStatus {
    /// 未开始
    Pending,
    /// 进行中
    Running { started_at: u64 },
    /// 已完成
    Completed { finished_at: u64 },
    /// 失败
    Failed { reason: String, finished_at: u64 },
}

/// 任务实例（Task 定义 + 运行时状态）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInstance {
    #[serde(flatten)]
    pub task: Task,
    pub status: TaskStatus,
}

/// 从 `tasks/` 目录加载所有任务定义。
///
/// 目录结构：
/// ```text
/// tasks/
/// ├── tier1_crafting_table.json
/// ├── tier1_gather_wood.json
/// ├── tier2_stone_tools.json
/// └── ...
/// ```
///
/// 每个文件是一个 Task JSON。加载失败的单个文件跳过（不致命）。
pub fn load_tasks(dir: &Path) -> Result<Vec<Task>> {
    let mut tasks = Vec::new();
    if !dir.exists() {
        return Ok(tasks);
    }
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("读取 tasks 目录失败：{}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match load_task_file(&path) {
            Ok(t) => tasks.push(t),
            Err(e) => {
                eprintln!("[task] 加载失败 {}: {e}", path.display());
            }
        }
    }
    // 按 tier 升序、id 字母序排序
    tasks.sort_by(|a, b| a.tier.cmp(&b.tier).then_with(|| a.id.cmp(&b.id)));
    Ok(tasks)
}

/// 加载单个任务 JSON 文件。
pub fn load_task_file(path: &Path) -> Result<Task> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("读取 task 文件失败：{}", path.display()))?;
    let task: Task = serde_json::from_str(&content)
        .with_context(|| format!("解析 task JSON 失败：{}", path.display()))?;
    Ok(task)
}

/// 任务完成判定器：根据 perceive 文本/WorldState 判断 success 条件是否满足。
///
/// 设计要点：
/// - 不直接访问 bot，由调用方传入 perceive 文本 + 可选背包快照
/// - inventory_has 通过 perceive 文本中的"背包: [...]"行解析
/// - at_position 通过 perceive 文本中的"位置: (x, y, z)"行解析
/// - 复合条件递归判断
pub struct TaskChecker;

impl TaskChecker {
    /// 判断 perceive 文本是否满足 success 条件。
    ///
    /// `perceive_text` 是 perceive 工具返回的完整文本（含"位置: ..."、"背包: [...]"等行）。
    pub fn check(success: &SuccessCondition, perceive_text: &str) -> bool {
        match success {
            SuccessCondition::InventoryHas { item, count } => {
                let have = parse_inventory_count(perceive_text, item);
                have >= *count
            }
            SuccessCondition::InventoryExact { item, count } => {
                let have = parse_inventory_count(perceive_text, item);
                have == *count
            }
            SuccessCondition::AtPosition { x, y, z, radius } => {
                if let Some((px, py, pz)) = parse_position(perceive_text) {
                    let dx = (px - *x as f64).abs();
                    let dy = (py - *y as f64).abs();
                    let dz = (pz - *z as f64).abs();
                    let r = *radius as f64;
                    dx <= r && dy <= r && dz <= r
                } else {
                    false
                }
            }
            SuccessCondition::BelowY { y } => {
                if let Some((_, py, _)) = parse_position(perceive_text) {
                    (py as i32) < *y
                } else {
                    false
                }
            }
            SuccessCondition::Killed { .. }
            | SuccessCondition::Crafted { .. }
            | SuccessCondition::Placed { .. } => {
                // 这些需要外部统计回填，perceive 文本无法直接判断
                // 调用方应在外部记录并转换为 InventoryHas 检查
                false
            }
            SuccessCondition::All { conditions } => {
                conditions.iter().all(|c| Self::check(c, perceive_text))
            }
            SuccessCondition::Any { conditions } => {
                conditions.iter().any(|c| Self::check(c, perceive_text))
            }
        }
    }
}

/// 从 perceive 文本解析背包中指定物品的数量。
///
/// perceive 文本中背包行格式：`背包: [oak_log:8, stick:4, ...]`
/// 物品 id 可能带或不带 minecraft: 前缀，统一 strip 后比较。
fn parse_inventory_count(perceive: &str, item: &str) -> u32 {
    let item_norm = item.strip_prefix("minecraft:").unwrap_or(item);
    for line in perceive.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("背包:") {
            continue;
        }
        // 提取方括号内的内容
        if let Some(start) = trimmed.find('[') {
            if let Some(end) = trimmed.find(']').or(Some(trimmed.len())) {
                let inner = &trimmed[start + 1..end];
                for entry in inner.split(',') {
                    let entry = entry.trim();
                    // 格式 "oak_log:8" 或 "minecraft:oak_log:8"
                    let mut parts = entry.split(':');
                    // 处理 minecraft: 前缀：split(':') 会产生 ["minecraft", "oak_log", "8"]
                    let parts_vec: Vec<&str> = parts.by_ref().collect();
                    if parts_vec.len() < 2 {
                        continue;
                    }
                    // 最后一个是 count，前面拼起来是 item id
                    let count_str = parts_vec.last().unwrap();
                    let item_id = parts_vec[..parts_vec.len() - 1].join(":");
                    let item_id_norm = item_id.strip_prefix("minecraft:").unwrap_or(&item_id);
                    if item_id_norm == item_norm {
                        if let Ok(n) = count_str.trim().parse::<u32>() {
                            return n;
                        }
                    }
                }
            }
        }
        break;
    }
    0
}

/// 从 perceive 文本解析 bot 当前坐标。
///
/// 格式：`位置: (10.5, 64.0, -20.0)`
fn parse_position(perceive: &str) -> Option<(f64, f64, f64)> {
    for line in perceive.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("位置:") {
            continue;
        }
        // 提取括号内数字
        if let Some(start) = trimmed.find('(') {
            if let Some(end) = trimmed.find(')') {
                let inner = &trimmed[start + 1..end];
                let parts: Vec<&str> = inner.split(',').map(|s| s.trim()).collect();
                if parts.len() == 3 {
                    let x = parts[0].parse::<f64>().ok()?;
                    let y = parts[1].parse::<f64>().ok()?;
                    let z = parts[2].parse::<f64>().ok()?;
                    return Some((x, y, z));
                }
            }
        }
        break;
    }
    None
}

/// 任务管理器：加载任务 + 跟踪运行状态 + 判定完成。
pub struct TaskManager {
    pub tasks: Vec<Task>,
    pub current: Option<TaskInstance>,
    pub tasks_dir: Option<PathBuf>,
}

impl TaskManager {
    /// 创建空 TaskManager。
    pub fn new() -> Self {
        Self {
            tasks: vec![],
            current: None,
            tasks_dir: None,
        }
    }

    /// 从目录加载任务清单。
    pub fn load_from_dir(&mut self, dir: &Path) -> Result<()> {
        self.tasks = load_tasks(dir)?;
        self.tasks_dir = Some(dir.to_path_buf());
        Ok(())
    }

    /// 按 id 选择任务开始。
    pub fn start_task(&mut self, task_id: &str, now_ms: u64) -> Result<()> {
        let task = self
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("任务不存在: {task_id}"))?;
        self.current = Some(TaskInstance {
            task,
            status: TaskStatus::Running { started_at: now_ms },
        });
        Ok(())
    }

    /// 直接用任务定义开始（不依赖加载的清单，用于运行时动态注入）。
    pub fn start_task_direct(&mut self, task: Task, now_ms: u64) {
        self.current = Some(TaskInstance {
            task,
            status: TaskStatus::Running { started_at: now_ms },
        });
    }

    /// 每轮调用：用 perceive 文本判定当前任务是否完成。
    ///
    /// 返回值：
    /// - `Some(true)`：任务完成（status 已更新为 Completed）
    /// - `Some(false)`：任务失败（status 已更新为 Failed）
    /// - `None`：任务进行中
    pub fn check_current(&mut self, perceive_text: &str, now_ms: u64) -> Option<bool> {
        let inst = self.current.as_mut()?;
        if !matches!(inst.status, TaskStatus::Running { .. }) {
            return None;
        }
        // 检查超时
        if let Some(timeout) = inst.task.timeout_secs {
            if let TaskStatus::Running { started_at } = inst.status {
                if now_ms.saturating_sub(started_at) > timeout * 1000 {
                    inst.status = TaskStatus::Failed {
                        reason: format!("超时 {}s", timeout),
                        finished_at: now_ms,
                    };
                    return Some(false);
                }
            }
        }
        // 检查失败条件
        if let Some(failure) = &inst.task.failure {
            if TaskChecker::check(failure, perceive_text) {
                inst.status = TaskStatus::Failed {
                    reason: "失败条件触发".to_string(),
                    finished_at: now_ms,
                };
                return Some(false);
            }
        }
        // 检查成功条件
        if TaskChecker::check(&inst.task.success, perceive_text) {
            inst.status = TaskStatus::Completed {
                finished_at: now_ms,
            };
            return Some(true);
        }
        None
    }

    /// 取当前任务的 goal（给 LLM 的 self_prompt）。
    pub fn current_goal(&self) -> Option<&str> {
        self.current.as_ref().map(|i| i.task.goal.as_str())
    }

    /// 当前任务状态。
    pub fn current_status(&self) -> Option<&TaskStatus> {
        self.current.as_ref().map(|i| &i.status)
    }

    /// 结束当前任务（不管是否完成）。
    pub fn end_current(&mut self) {
        self.current = None;
    }
}

impl Default for TaskManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_inventory_count_simple() {
        let perceive = "位置: (10, 64, 20)\n背包: [oak_log:8, stick:4, cobblestone:32]";
        assert_eq!(parse_inventory_count(perceive, "oak_log"), 8);
        assert_eq!(parse_inventory_count(perceive, "stick"), 4);
        assert_eq!(parse_inventory_count(perceive, "dirt"), 0);
    }

    #[test]
    fn test_parse_inventory_count_with_namespace() {
        let perceive = "背包: [minecraft:iron_ingot:16, coal:8]";
        assert_eq!(parse_inventory_count(perceive, "iron_ingot"), 16);
        assert_eq!(parse_inventory_count(perceive, "minecraft:iron_ingot"), 16);
    }

    #[test]
    fn test_parse_position() {
        let perceive = "位置: (10.5, 64.0, -20.0)\n背包: []";
        assert_eq!(parse_position(perceive), Some((10.5, 64.0, -20.0)));
    }

    #[test]
    fn test_check_inventory_has() {
        let cond = SuccessCondition::InventoryHas {
            item: "oak_log".into(),
            count: 4,
        };
        let perceive_ok = "背包: [oak_log:8]";
        let perceive_fail = "背包: [oak_log:2]";
        assert!(TaskChecker::check(&cond, perceive_ok));
        assert!(!TaskChecker::check(&cond, perceive_fail));
    }

    #[test]
    fn test_check_at_position() {
        let cond = SuccessCondition::AtPosition {
            x: 10,
            y: 64,
            z: 20,
            radius: 3,
        };
        let perceive = "位置: (11.0, 64.0, 22.0)\n背包: []";
        assert!(TaskChecker::check(&cond, perceive));
        let perceive_far = "位置: (20.0, 64.0, 22.0)\n背包: []";
        assert!(!TaskChecker::check(&cond, perceive_far));
    }

    #[test]
    fn test_check_all_composite() {
        let cond = SuccessCondition::All {
            conditions: vec![
                SuccessCondition::InventoryHas {
                    item: "oak_log".into(),
                    count: 4,
                },
                SuccessCondition::InventoryHas {
                    item: "stick".into(),
                    count: 2,
                },
            ],
        };
        let perceive_ok = "背包: [oak_log:8, stick:4]";
        let perceive_partial = "背包: [oak_log:8]";
        assert!(TaskChecker::check(&cond, perceive_ok));
        assert!(!TaskChecker::check(&cond, perceive_partial));
    }

    #[test]
    fn test_check_any_composite() {
        let cond = SuccessCondition::Any {
            conditions: vec![
                SuccessCondition::InventoryHas {
                    item: "diamond".into(),
                    count: 1,
                },
                SuccessCondition::InventoryHas {
                    item: "iron_ingot".into(),
                    count: 4,
                },
            ],
        };
        let perceive = "背包: [iron_ingot:8]";
        assert!(TaskChecker::check(&cond, perceive));
    }

    #[test]
    fn test_load_tasks_from_dir() {
        let tmp = std::env::temp_dir().join("craft_agent_task_test");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("tier1_test.json"),
            r#"{
                "id": "tier1_test",
                "name": "测试任务",
                "description": "测试用",
                "goal": "做工作台",
                "tier": 1,
                "success": {"type": "InventoryHas", "item": "crafting_table", "count": 1}
            }"#,
        )
        .unwrap();

        let tasks = load_tasks(&tmp).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "tier1_test");
        assert_eq!(tasks[0].tier, 1);

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_task_manager_lifecycle() {
        let mut tm = TaskManager::new();
        let task = Task {
            id: "test".into(),
            name: "测试".into(),
            description: "测试任务".into(),
            goal: "收集 4 个原木".into(),
            tier: 1,
            success: SuccessCondition::InventoryHas {
                item: "oak_log".into(),
                count: 4,
            },
            failure: None,
            timeout_secs: None,
            reward: None,
        };
        tm.start_task_direct(task, 0);
        assert!(matches!(
            tm.current_status(),
            Some(TaskStatus::Running { .. })
        ));

        // 未完成
        let r = tm.check_current("背包: [oak_log:2]", 100);
        assert_eq!(r, None);

        // 完成
        let r = tm.check_current("背包: [oak_log:8]", 200);
        assert_eq!(r, Some(true));
        assert!(matches!(
            tm.current_status(),
            Some(TaskStatus::Completed { .. })
        ));
    }

    #[test]
    fn test_task_manager_timeout() {
        let mut tm = TaskManager::new();
        let task = Task {
            id: "test".into(),
            name: "测试".into(),
            description: "测试任务".into(),
            goal: "收集 4 个原木".into(),
            tier: 1,
            success: SuccessCondition::InventoryHas {
                item: "oak_log".into(),
                count: 4,
            },
            failure: None,
            timeout_secs: Some(10), // 10 秒超时
            reward: None,
        };
        tm.start_task_direct(task, 0);
        // 5 秒未超时
        let r = tm.check_current("背包: [oak_log:2]", 5_000);
        assert_eq!(r, None);
        // 11 秒超时
        let r = tm.check_current("背包: [oak_log:2]", 11_000);
        assert_eq!(r, Some(false));
        assert!(matches!(
            tm.current_status(),
            Some(TaskStatus::Failed { .. })
        ));
    }

    /// 回归测试：所有 tasks/ 目录下的 JSON 必须能正确加载并符合 Task schema。
    /// 若新增/修改任务后字段名写错（如把 Placed 的 block 写成 item），此测试会立即失败。
    #[test]
    fn regression_all_tasks_dir_json_loads() {
        let tasks_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("tasks");
        if !tasks_dir.exists() {
            // 在 cargo workspace 外运行测试时跳过（不强制）
            eprintln!("[test] tasks 目录不存在，跳过: {}", tasks_dir.display());
            return;
        }
        let tasks = load_tasks(&tasks_dir).expect("加载 tasks/ 目录失败");
        assert!(
            tasks.len() >= 18,
            "tasks/ 目录至少应有 18 个任务（tier1-6 各 3-4 个），实际 {}",
            tasks.len()
        );
        // 校验每个任务的基本字段
        let mut seen_ids = std::collections::HashSet::new();
        for t in &tasks {
            assert!(!t.id.is_empty(), "任务 id 不能为空");
            assert!(seen_ids.insert(&t.id), "任务 id 重复: {}", t.id);
            assert!(!t.name.is_empty(), "任务 {} 的 name 为空", t.id);
            assert!(!t.goal.is_empty(), "任务 {} 的 goal 为空", t.id);
            assert!(
                (1..=6).contains(&t.tier),
                "任务 {} 的 tier {} 不在 1-6 范围",
                t.id,
                t.tier
            );
            // timeout_secs 应当合理（10s ~ 1h）
            if let Some(to) = t.timeout_secs {
                assert!(
                    to >= 10 && to <= 3600,
                    "任务 {} 的 timeout {} 异常",
                    t.id,
                    to
                );
            }
        }
        // 校验 6 个 tier 各至少有 1 个任务
        for tier in 1..=6u32 {
            let count = tasks.iter().filter(|t| t.tier == tier).count();
            assert!(
                count >= 1,
                "tier {} 至少应有 1 个任务，实际 {}",
                tier,
                count
            );
        }
        println!("[test] 成功加载 {} 个任务（tier1-6）", tasks.len());
    }
}
