use crate::core::message::Message;
use crate::core::prompt::WorldInfo;
use crate::core::session::SessionEntry as SessionFileEntry;
use crate::core::session::{AgentSnapshot, Session};

use super::Agent;

impl Agent {
    pub fn with_session(mut self, sess: Session) -> Self {
        let messages = sess.messages_for_current_path();
        if !messages.is_empty() {
            self.session_msg_offset = messages.len();
            self.messages = messages;
        }

        let path = sess.entries_for_current_path();
        for e in path.iter().rev() {
            if let SessionFileEntry::Checkpoint(cp) = e {
                self.previous_summary = cp.snapshot.previous_summary.clone();
                self.usage = cp.snapshot.usage.clone();
                self.turn = cp.snapshot.turn;
                if let Some(skills_json) = &cp.snapshot.skills_json
                    && let Ok(skill_lib) =
                        serde_json::from_str::<crate::core::skill::SkillLibrary>(skills_json)
                {
                    self.skill_lib = skill_lib;
                }
                break;
            }
        }

        for e in &path {
            if let SessionFileEntry::WorldInfo(wi) = e {
                match wi.action.as_str() {
                    "add" => {
                        if let Some(info) = &wi.info {
                            self.world_info.add(info.clone());
                        }
                    }
                    "remove" => {
                        if let Some(id) = &wi.remove_id {
                            self.world_info.remove_by_id(id);
                        }
                        if let Some(keys) = &wi.remove_keys {
                            self.world_info.remove_by_keys(keys);
                        }
                    }
                    _ => {}
                }
            }
        }

        self.knowledge_bootstrapped = sess.header.knowledge_bootstrapped;
        self.session = Some(sess);
        self
    }

    pub fn persist_turn(&mut self) -> anyhow::Result<()> {
        let Some(sess) = &mut self.session else {
            return Ok(());
        };
        if let Some(compaction) = self.pending_compaction.take() {
            sess.append_compaction(
                compaction.summary,
                compaction.first_kept_entry_id,
                compaction.tokens_before,
                None,
            );
        }
        if self.pending_checkpoint {
            let skills_json = serde_json::to_string(&self.skill_lib).ok();
            let snapshot = AgentSnapshot {
                messages: self.messages.clone(),
                previous_summary: self.previous_summary.clone(),
                usage: self.usage.clone(),
                turn: self.turn,
                skills_json,
            };
            sess.append_checkpoint("compaction", snapshot);
            self.pending_checkpoint = false;
            self.session_msg_offset = self.messages.len();
        } else {
            let new_msgs: Vec<Message> = self.messages[self.session_msg_offset..].to_vec();
            for m in new_msgs {
                sess.append_message(m);
            }
            self.session_msg_offset = self.messages.len();
        }
        sess.save()?;
        Ok(())
    }

    pub fn manage_knowledge(&mut self, args: &serde_json::Value) -> (String, bool) {
        let action = match args["action"].as_str() {
            Some(a) => a,
            None => {
                return (
                    "manage_knowledge missing 'action' parameter (add/remove)".into(),
                    true,
                );
            }
        };
        match action {
            "add" => {
                let keys: Vec<String> = args["keys"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_lowercase()))
                            .collect()
                    })
                    .unwrap_or_default();
                let template = args["template"].as_str().unwrap_or("Detected {label}");
                let id = args["id"].as_str().map(|s| s.to_string());
                let wi = WorldInfo {
                    keys: keys.clone(),
                    template: template.to_string(),
                    priority: 0,
                    id: id.clone(),
                };
                self.world_info.add(wi.clone());
                if let Some(ref mut sess) = self.session {
                    sess.append_world_info("add", Some(wi), None, None);
                }
                (
                    format!(
                        "Added knowledge entry (id={:?}). Total entries: {}",
                        id,
                        self.world_info.len()
                    ),
                    false,
                )
            }
            "remove" => {
                let before = self.world_info.len();
                let remove_id = args["id"].as_str().map(|s| s.to_string());
                let remove_keys: Option<Vec<String>> = args["keys"].as_array().map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_lowercase()))
                        .collect()
                });
                if let Some(id) = &remove_id {
                    self.world_info.remove_by_id(id);
                } else if let Some(keys) = &remove_keys {
                    self.world_info.remove_by_keys(keys);
                } else {
                    return ("remove needs 'id' or 'keys' parameter".into(), true);
                }
                let removed = before - self.world_info.len();
                if let Some(ref mut sess) = self.session {
                    sess.append_world_info("remove", None, remove_id.clone(), remove_keys.clone());
                }
                (
                    format!(
                        "Removed {removed} entries. Total: {}",
                        self.world_info.len()
                    ),
                    false,
                )
            }
            other => (format!("Unknown action: {other}"), true),
        }
    }
}
