//! Derives verified progress signals from the persisted session JSONL.

use serde_json::Value;
use std::path::Path;

#[derive(Debug, Default)]
pub struct SessionAnalysis {
    pub assistant_steps: u32,
    pub tool_calls: u32,
    pub errors: u32,
    pub successful_productive_tools: u32,
    pub position_changes: u32,
    pub last_position: Option<(i32, i32, i32)>,
    pub last_inventory: Option<String>,
    pub summary: String,
}

impl SessionAnalysis {
    pub fn has_progress_since(&self, previous: &Self) -> bool {
        self.position_changes > previous.position_changes
            || (self.last_inventory.is_some() && self.last_inventory != previous.last_inventory)
    }

    pub fn delta_summary(&self, previous: &Self) -> String {
        format!(
            "steps=+{} productive=+{} moves=+{} inventory_changed={}",
            self.assistant_steps
                .saturating_sub(previous.assistant_steps),
            self.successful_productive_tools
                .saturating_sub(previous.successful_productive_tools),
            self.position_changes
                .saturating_sub(previous.position_changes),
            self.last_inventory != previous.last_inventory,
        )
    }
}

pub fn analyze_session(session_path: &Path) -> SessionAnalysis {
    let mut analysis = SessionAnalysis::default();
    let Ok(content) = std::fs::read_to_string(session_path) else {
        return analysis;
    };

    for line in content.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if entry["type"] != "message" {
            continue;
        }

        let message = &entry["message"];
        match message["role"].as_str().unwrap_or_default() {
            "assistant" => {
                analysis.assistant_steps += 1;
                analysis.tool_calls += message["tool_calls"]
                    .as_array()
                    .map_or(0, |calls| calls.len() as u32);
            }
            "toolresult" => analyze_tool_result(message, &mut analysis),
            _ => {}
        }
    }

    analysis.summary = format!(
        "steps={} tools={} errors={} productive={} moves={} position={:?}",
        analysis.assistant_steps,
        analysis.tool_calls,
        analysis.errors,
        analysis.successful_productive_tools,
        analysis.position_changes,
        analysis.last_position,
    );
    analysis
}

fn analyze_tool_result(message: &Value, analysis: &mut SessionAnalysis) {
    let is_error = message["is_error"].as_bool().unwrap_or(false);
    if is_error {
        analysis.errors += 1;
    } else if matches!(
        message["tool_name"].as_str().unwrap_or_default(),
        "gather"
            | "craft"
            | "craft_3x3"
            | "auto_craft"
            | "smelt"
            | "mine"
            | "mine_below"
            | "mine_above"
            | "pickup"
            | "place"
            | "build"
            | "attack"
            | "trade"
            | "enchant"
    ) {
        analysis.successful_productive_tools += 1;
    }

    let content = message["content"].as_str().unwrap_or_default();
    if let Some(position) = parse_position(content) {
        if analysis.last_position.is_some() && analysis.last_position != Some(position) {
            analysis.position_changes += 1;
        }
        analysis.last_position = Some(position);
    }
    if let Some(inventory) = extract_inventory(content) {
        analysis.last_inventory = Some(inventory);
    }
}

fn parse_position(content: &str) -> Option<(i32, i32, i32)> {
    let after_marker = ["位置:", "position:"]
        .iter()
        .find_map(|marker| content.split_once(marker).map(|(_, after)| after))?;
    let coordinates = after_marker.split_once('(')?.1.split_once(')')?.0;
    let mut parts = coordinates.split(',').map(str::trim);
    let x = parts.next()?.parse::<f64>().ok()?.round() as i32;
    let y = parts.next()?.parse::<f64>().ok()?.round() as i32;
    let z = parts.next()?.parse::<f64>().ok()?.round() as i32;
    Some((x, y, z))
}

fn extract_inventory(content: &str) -> Option<String> {
    let after_marker = ["背包:", "Inventory:"]
        .iter()
        .find_map(|marker| content.split_once(marker).map(|(_, after)| after))?;
    let line = after_marker.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_perception_format() {
        let content = "位置: (-478.4, 28, -177.6)\n背包: [cobblestone:259, coal:11]";
        assert_eq!(parse_position(content), Some((-478, 28, -178)));
        assert_eq!(
            extract_inventory(content).as_deref(),
            Some("[cobblestone:259, coal:11]")
        );
    }

    #[test]
    fn successful_tool_without_state_change_is_not_progress() {
        let previous = SessionAnalysis {
            successful_productive_tools: 4,
            ..SessionAnalysis::default()
        };
        let current = SessionAnalysis {
            successful_productive_tools: 5,
            ..SessionAnalysis::default()
        };

        assert!(!current.has_progress_since(&previous));
    }

    #[test]
    fn inventory_change_is_progress() {
        let previous = SessionAnalysis {
            last_inventory: Some("[iron_pickaxe:1]".into()),
            ..SessionAnalysis::default()
        };
        let current = SessionAnalysis {
            last_inventory: Some("[iron_pickaxe:1, diamond:3]".into()),
            ..SessionAnalysis::default()
        };

        assert!(current.has_progress_since(&previous));
    }
}
