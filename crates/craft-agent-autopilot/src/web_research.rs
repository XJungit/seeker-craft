//! 联网搜索

use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct ResearchResult {
    pub source: String,
    pub url: String,
    pub description: String,
    pub fix_hint: String,
}

pub struct WebResearch;

impl WebResearch {
    pub fn search(query: &str) -> Result<Vec<ResearchResult>> {
        let mut results = vec![];

        // Search mindcraft
        results.extend(Self::search_mindcraft(query)?);

        // Search general MC bot implementations
        results.extend(Self::search_general(query)?);

        Ok(results)
    }

    fn search_mindcraft(query: &str) -> Result<Vec<ResearchResult>> {
        let url = format!(
            "https://api.github.com/search/code?q={}+repo:mindcraft-bots/mindcraft",
            urlencoding::encode(query)
        );

        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        let resp = client.get(&url)
            .header("User-Agent", "craft-agent-autopilot")
            .send();

        match resp {
            Ok(resp) => {
                let json: serde_json::Value = resp.json().unwrap_or_default();
                let items = json["items"].as_array().cloned().unwrap_or_default();
                Ok(items.iter().take(5).filter_map(|item| {
                    Some(ResearchResult {
                        source: "mindcraft".into(),
                        url: item["html_url"].as_str()?.into(),
                        description: item["path"].as_str()?.into(),
                        fix_hint: format!("Check mindcraft implementation of {query}"),
                    })
                }).collect())
            }
            Err(e) => {
                eprintln!("Web search failed: {e}");
                Ok(vec![])
            }
        }
    }

    fn search_general(query: &str) -> Result<Vec<ResearchResult>> {
        // Placeholder for general web search
        // In production, could use a search API
        Ok(vec![])
    }
}
