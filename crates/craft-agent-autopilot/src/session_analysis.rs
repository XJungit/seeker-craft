//! Session analyzer - analyze LLM behavior from session JSONL

use serde_json::Value;
use std::path::Path;

#[derive(Debug, Default)]
pub struct SessionAnalysis {
    pub total_steps: u32,
    pub tool_calls: u32,
    pub errors: u32,
    pub gathers: u32,
    pub crafts: u32,
    pub mines: u32,
    pub gotes: u32,
    pub attacks: u32,
    pub perceives: u32,
    pub position_changes: u32,
    pub last_position: Option<(i32, i32, i32)>,
    pub inventory_items: Vec<String>,
    pub is_making_progress: bool,
    pub is_stuck: bool,
    pub summary: String,
}

pub fn analyze_session(session_path: &Path) -> SessionAnalysis {
    let mut analysis = SessionAnalysis::default();

    let content = match std::fs::read_to_string(session_path) {
        Ok(c) => c,
        Err(_) => return analysis,
    };

    for line in content.lines() {
        let val: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if val["type"] != "message" {
            continue;
        }

        let msg = &val["message"];
        let role = msg["role"].as_str().unwrap_or("");

        match role {
            "assistant" => {
                analysis.total_steps += 1;
                if let Some(calls) = msg["tool_calls"].as_array() {
                    for call in calls {
                        let name = call["name"].as_str().unwrap_or("");
                        analysis.tool_calls += 1;
                        match name {
                            "gather" => analysis.gathers += 1,
                            "craft" | "craft_3x3" => analysis.crafts += 1,
                            "mine" | "mine_below" | "mine_above" => analysis.mines += 1,
                            "goto" | "go" => analysis.gotes += 1,
                            "attack" => analysis.attacks += 1,
                            "perceive" => analysis.perceives += 1,
                            _ => {}
                        }
                    }
                }
            }
            "toolresult" => {
                let is_err = msg["is_error"].as_bool().unwrap_or(false);
                if is_err {
                    analysis.errors += 1;
                }

                // Extract position from perceive results
                let content_str = msg["content"].as_str().unwrap_or("");
                if content_str.contains("位置:") || content_str.contains("position:") {
                    // Parse position
                    if let Some(pos) = parse_position(content_str) {
                        if analysis.last_position != Some(pos) {
                            analysis.position_changes += 1;
                        }
                        analysis.last_position = Some(pos);
                    }
                }

                // Extract inventory
                if content_str.contains("背包:") || content_str.contains("Inventory:") {
                    for item in extract_inventory_items(content_str) {
                        if !analysis.inventory_items.contains(&item) {
                            analysis.inventory_items.push(item);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Determine if making progress
    analysis.is_making_progress = analysis.gathers > 0
        || analysis.crafts > 0
        || analysis.mines > 0
        || analysis.position_changes > 2;

    // Determine if stuck
    analysis.is_stuck = analysis.errors > 5
        || (analysis.gotes > 3 && analysis.position_changes < 2)
        || (analysis.total_steps > 10 && analysis.gathers == 0 && analysis.crafts == 0);

    analysis.summary = format!(
        "steps={} tools={} errors={} gather={} craft={} mine={} goto={} pos_changes={} stuck={} progress={}",
        analysis.total_steps,
        analysis.tool_calls,
        analysis.errors,
        analysis.gathers,
        analysis.crafts,
        analysis.mines,
        analysis.gotes,
        analysis.position_changes,
        analysis.is_stuck,
        analysis.is_making_progress,
    );

    analysis
}

fn parse_position(content: &str) -> Option<(i32, i32, i32)> {
    // Match "位置: (x, y, z)" or "position: (x, y, z)"
    let markers = ["位置:", "position:"];
    for marker in &markers {
        if let Some(pos) = content.find(marker) {
            let after = &content[pos + marker.len()..];
            if let Some(start) = after.find('(') {
                let rest = &after[start + 1..];
                if let Some(end) = rest.find(')') {
                    let coords = &rest[..end];
                    let parts: Vec<&str> = coords.split(',').collect();
                    if parts.len() == 3 {
                        if let (Ok(x), Ok(y), Ok(z)) = (
                            parts[0].trim().parse::<i32>(),
                            parts[1].trim().parse::<i32>(),
                            parts[2].trim().parse::<i32>(),
                        ) {
                            return Some((x, y, z));
                        }
                    }
                }
            }
        }
    }
    None
}

fn extract_inventory_items(content: &str) -> Vec<String> {
    let mut items = vec![];
    // Match "背包: item:count, item:count"
    if let Some(pos) = content.find("背包:") {
        let inv = &content[pos + 3..];
        if let Some(end) = inv.find('\n') {
            let inv = &inv[..end];
            for part in inv.split(',') {
                let part = part.trim();
                if let Some(colon) = part.find(':') {
                    let item = part[..colon].trim().to_string();
                    if !item.is_empty() {
                        items.push(item);
                    }
                }
            }
        }
    }
    items
}
