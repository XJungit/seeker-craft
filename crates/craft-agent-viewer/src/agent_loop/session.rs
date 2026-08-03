//! Session 生命周期 helper：加载/保存（增量/全量回退）/自动滚动（P3"会话保存/滚动抽 helper"）。

use craft_agent::agent::Agent;
use craft_agent::core::session::Session;
use std::path::Path;

use super::events::EventSender;

/// 打开现有 session，损坏或不存在时创建新的。
/// 原逻辑内联在 run_agent 中（agent_loop.rs），拆分后收敛此处，行为不变。
pub fn open_or_create(path: &Path) -> Session {
    if path.exists() && path.metadata().map(|m| m.len() > 0).unwrap_or(false) {
        match Session::open(path) {
            Ok(s) => return s,
            Err(e) => {
                eprintln!("[agent_loop] session 文件损坏，创建新 session: {e}");
            }
        }
    }
    let mut s = Session::new("minecraft-control-panel");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = s.save_to(path);
    s
}

/// 首步完成后一次性全量落盘（确保父目录存在，事件通道失败提示）。
pub fn save_full(sess: &mut Session, path: &Path, ev: &EventSender) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = sess.save_to(path) {
        ev.error(format!("session 保存失败: {e}"));
        eprintln!("[agent_loop] session save_to 失败: {e}");
    }
}

/// 每步增量保存（`sess.save()`），失败时回退全量 `save_to` 并推错误事件。
pub fn save_incremental(sess: &mut Session, path: &Path, ev: &EventSender) {
    if let Err(e) = sess.save() {
        eprintln!("[agent_loop] session save 失败: {e}");
        if let Err(e2) = sess.save_to(path) {
            ev.error(format!("session 保存失败: {e2}"));
        }
    }
}

/// 防 OOM 自动滚动：每 40 步或会话文件 > 12MB 时原地归档并重置内存历史。
/// 返回是否执行了滚动（调用方据此推送提示事件）。
pub fn auto_rollover(agent: &mut Agent, session_path: &str, step: u32) -> bool {
    let session_too_big = std::fs::metadata(session_path)
        .map(|m| m.len() > 12 * 1024 * 1024)
        .unwrap_or(false);
    if step.is_multiple_of(40) || session_too_big {
        let goal_snapshot = agent.current_goal_snapshot().to_string();
        return agent.rollover_in_place(session_path, &goal_snapshot);
    }
    false
}
