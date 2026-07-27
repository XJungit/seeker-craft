//! LLM 自定义动作库（P2-4：newAction 等价物）。
//!
//! 学习自 Mindcraft `src/agent/commands/code.js` 的 `newAction`：LLM 可写一段
//! JavaScript 函数作为新动作，后续按名调用。我们用 rhai 脚本替代 JS（嵌入式更安全）。
//!
//! 与 `SkillLibrary` 的区别：
//! - SkillLibrary 是自动从 tool_call 历史抽取的「步骤序列」（被动学习）
//! - ActionLibrary 是 LLM 主动写的「带名字 rhai 脚本」（主动编码）
//!
//! 与 `run_script` 的区别：
//! - `run_script` 是一次性执行：脚本执行完就丢弃
//! - `new_action` 是持久化：脚本保存到 `actions/<name>.rhai.json`，跨会话可复用
//! - `call_action(name)` 在 `run_script` 内调用已保存的动作
//!
//! 文件格式（`actions/<name>.rhai.json`）：
//! ```json
//! {
//!   "name": "gather_and_craft",
//!   "description": "采集 N 个 item 并合成 target",
//!   "script": "let r = gather(item, n); craft(target, n); r",
//!   "created_at": 1784953000
//! }
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 一个 LLM 自定义动作（命名 rhai 脚本 + 元信息）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmAction {
    /// 动作名（与文件名同步，不带 .rhai.json 后缀）。必须是合法标识符 `[a-z_][a-z0-9_]*`。
    pub name: String,
    /// 人类/LLM 可读描述：什么时候该用这个动作。
    pub description: String,
    /// rhai 脚本代码（lint 通过 + parse 通过）。
    pub script: String,
    /// 创建时间戳（毫秒）。
    pub created_at: u64,
    /// 调用次数（每次 call_action 时 +1）。
    #[serde(default)]
    pub call_count: u32,
}

impl LlmAction {
    /// 校验动作名：必须是 `[a-z_][a-z0-9_]*`，长度 1..=32。
    pub fn is_valid_name(name: &str) -> bool {
        if name.is_empty() || name.len() > 32 {
            return false;
        }
        let mut chars = name.chars();
        let first = chars.next().unwrap();
        if !(first.is_ascii_lowercase() || first == '_') {
            return false;
        }
        chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    }
}

/// 动作库：按名称索引的 LLM 自定义动作集合。
///
/// 加载自 `actions/` 目录（与 `blueprints/` `tasks/` 同级）。每个 `*.rhai.json` 一个动作。
/// 运行时通过 `call_action(name)` 调用。
#[derive(Debug, Clone, Default)]
pub struct ActionLibrary {
    actions: HashMap<String, LlmAction>,
    /// 动作目录路径（用于运行时新增动作时写入文件）。
    dir: Option<PathBuf>,
}

impl ActionLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    /// 从目录加载所有 `*.rhai.json` 动作。文件名（去 `.rhai.json`）即为动作 name。
    /// 文件内的 name 字段若与文件名不一致，以文件名为准（覆盖）。
    pub fn load_dir(dir: &Path) -> Self {
        let mut lib = Self {
            actions: HashMap::new(),
            dir: Some(dir.to_path_buf()),
        };
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return lib,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // 仅识别 .rhai.json 后缀（避免与普通 .json 混淆）
            let fname = match path.file_name().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            if !fname.ends_with(".rhai.json") {
                continue;
            }
            let stem = fname.trim_end_matches(".rhai.json").to_string();
            if !LlmAction::is_valid_name(&stem) {
                eprintln!("[action_lib] 跳过非法动作名: {}", path.display());
                continue;
            }
            let text = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            match serde_json::from_str::<LlmAction>(&text) {
                Ok(mut a) => {
                    a.name = stem.clone();
                    lib.actions.insert(stem, a);
                }
                Err(e) => {
                    eprintln!("[action_lib] 解析 {} 失败: {e}", path.display());
                }
            }
        }
        lib
    }

    /// 设置运行时目录（用于新增动作时写入文件）。
    pub fn with_dir(mut self, dir: PathBuf) -> Self {
        self.dir = Some(dir);
        self
    }

    /// 新增/覆盖一个动作。
    /// - 校验 name 合法
    /// - 写入 `actions/<name>.rhai.json`
    /// - 插入到内存库
    pub fn save(&mut self, mut action: LlmAction) -> Result<(), String> {
        if !LlmAction::is_valid_name(&action.name) {
            return Err(format!(
                "动作名 '{}' 非法（须 [a-z_][a-z0-9_]*，长度 1..=32）",
                action.name
            ));
        }
        // 写文件
        if let Some(dir) = &self.dir {
            std::fs::create_dir_all(dir).map_err(|e| format!("创建 actions/ 目录失败: {e}"))?;
            let path = dir.join(format!("{}.rhai.json", action.name));
            action.call_count = 0; // 重置计数
            let json = serde_json::to_string_pretty(&action)
                .map_err(|e| format!("序列化动作失败: {e}"))?;
            std::fs::write(&path, json)
                .map_err(|e| format!("写入 {} 失败: {e}", path.display()))?;
        }
        // 插入内存（覆盖同名）
        self.actions.insert(action.name.clone(), action);
        Ok(())
    }

    /// 按名查询动作。
    pub fn get(&self, name: &str) -> Option<&LlmAction> {
        self.actions.get(name)
    }

    /// 增加调用计数（运行时 call_action 调用后回写）。
    /// 返回 true 表示计数成功，false 表示动作不存在。
    pub fn bump_call_count(&mut self, name: &str) -> bool {
        if let Some(a) = self.actions.get_mut(name) {
            a.call_count += 1;
            // 同时持久化（异步写盘，失败不致命）
            if let Some(dir) = &self.dir {
                let path = dir.join(format!("{}.rhai.json", name));
                if let Ok(json) = serde_json::to_string_pretty(&a) {
                    let _ = std::fs::write(&path, json);
                }
            }
            true
        } else {
            false
        }
    }

    /// 列出所有动作 (name, description, call_count)。
    pub fn list(&self) -> Vec<(String, String, u32)> {
        let mut v: Vec<(String, String, u32)> = self
            .actions
            .iter()
            .map(|(k, a)| (k.clone(), a.description.clone(), a.call_count))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    /// 列出所有动作的人类可读摘要（供 LLM 决策"该调哪个动作"）。
    pub fn list_summary(&self) -> String {
        let items = self.list();
        if items.is_empty() {
            return "（无自定义动作）".to_string();
        }
        items
            .iter()
            .map(|(n, d, c)| format!("- {n} (调用 {c} 次): {d}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 动作数量。
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_valid_name_accepts_legal_identifiers() {
        assert!(LlmAction::is_valid_name("gather_and_craft"));
        assert!(LlmAction::is_valid_name("a"));
        assert!(LlmAction::is_valid_name("_private"));
        assert!(LlmAction::is_valid_name("abc123"));
    }

    #[test]
    fn is_valid_name_rejects_illegal() {
        assert!(!LlmAction::is_valid_name(""));
        assert!(!LlmAction::is_valid_name("123abc")); // 数字开头
        assert!(!LlmAction::is_valid_name("CamelCase")); // 大写
        assert!(!LlmAction::is_valid_name("has space"));
        assert!(!LlmAction::is_valid_name("has-dash"));
        assert!(!LlmAction::is_valid_name(&"x".repeat(33))); // 过长
    }

    #[test]
    fn library_load_dir_loads_rhai_json_files() {
        let tmp = std::env::temp_dir().join("craft_agent_action_test");
        let _ = std::fs::create_dir_all(&tmp);
        let path = tmp.join("hello.rhai.json");
        std::fs::write(
            &path,
            r#"{"name":"hello","description":"hi","script":"print(\"hi\")","created_at":0,"call_count":0}"#,
        )
        .unwrap();
        let lib = ActionLibrary::load_dir(&tmp);
        assert_eq!(lib.len(), 1);
        assert!(lib.get("hello").is_some());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn library_load_dir_ignores_plain_json() {
        let tmp = std::env::temp_dir().join("craft_agent_action_test2");
        let _ = std::fs::create_dir_all(&tmp);
        // 普通 .json 应被忽略
        std::fs::write(tmp.join("plain.json"), r#"{"name":"plain"}"#).unwrap();
        // .rhai.json 才是动作
        std::fs::write(
            tmp.join("real.rhai.json"),
            r#"{"name":"real","description":"r","script":"print(\"x\")","created_at":0}"#,
        )
        .unwrap();
        let lib = ActionLibrary::load_dir(&tmp);
        assert_eq!(lib.len(), 1);
        assert!(lib.get("plain").is_none());
        assert!(lib.get("real").is_some());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn save_writes_file_and_inserts_into_memory() {
        let tmp = std::env::temp_dir().join("craft_agent_action_save_test");
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&tmp);
        let mut lib = ActionLibrary::new().with_dir(tmp.clone());
        let action = LlmAction {
            name: "test_action".into(),
            description: "测试动作".into(),
            script: "print(\"hi\")".into(),
            created_at: 12345,
            call_count: 0,
        };
        lib.save(action).unwrap();
        assert!(lib.get("test_action").is_some());
        // 文件应当存在
        let path = tmp.join("test_action.rhai.json");
        assert!(path.exists(), "动作文件应当被写入");
        // 重新加载应当读回
        let reloaded = ActionLibrary::load_dir(&tmp);
        assert_eq!(reloaded.len(), 1);
        assert!(reloaded.get("test_action").is_some());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn save_rejects_invalid_name() {
        let tmp = std::env::temp_dir().join("craft_agent_action_invalid_test");
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&tmp);
        let mut lib = ActionLibrary::new().with_dir(tmp.clone());
        let action = LlmAction {
            name: "123Bad".into(),
            description: "非法".into(),
            script: "print(\"x\")".into(),
            created_at: 0,
            call_count: 0,
        };
        let r = lib.save(action);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("非法"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn bump_call_count_increments_and_persists() {
        let tmp = std::env::temp_dir().join("craft_agent_action_bump_test");
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&tmp);
        let mut lib = ActionLibrary::new().with_dir(tmp.clone());
        lib.save(LlmAction {
            name: "counted".into(),
            description: "".into(),
            script: "print(\"x\")".into(),
            created_at: 0,
            call_count: 0,
        })
        .unwrap();
        assert!(lib.bump_call_count("counted"));
        assert!(lib.bump_call_count("counted"));
        assert_eq!(lib.get("counted").unwrap().call_count, 2);
        // 重新加载应当读到 2
        let reloaded = ActionLibrary::load_dir(&tmp);
        assert_eq!(reloaded.get("counted").unwrap().call_count, 2);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn list_summary_handles_empty() {
        let lib = ActionLibrary::new();
        assert_eq!(lib.list_summary(), "（无自定义动作）");
    }
}
