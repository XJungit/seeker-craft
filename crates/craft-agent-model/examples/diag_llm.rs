/// Min LLM diagnostic — uses from_config, needs LONGCAT_API_KEY in env
use craft_agent_model::config::AgentConfig;
use craft_agent_model::decision::real::OpenAiLlmClient;
use serde_json::json;
use std::time::Instant;

fn main() {
    let cfg = AgentConfig::load("config/agent.toml").expect("load config");
    let llm_group = cfg.llm.as_ref().expect("llm group");
    let backend = llm_group.active_backend().expect("active backend");
    println!("LLM: {} @ {} | timeout={}s max_tokens={} force_http1={}",
        backend.model, backend.chat_endpoint(),
        backend.timeout_secs, backend.max_tokens, backend.force_http1);

    // from_config resolves api_key from env var (LONGCAT_API_KEY)
    let client = OpenAiLlmClient::from_config(backend).expect("create client");
    let tools = json!([{"type":"function","function":{"name":"collect","description":"collect blocks","parameters":{"type":"object","properties":{"target":{"type":"string"},"count":{"type":"integer"}},"required":["target"]}}}]);
    let messages = json!([{"role":"user","content":"收集8个橡木然后合成木板"}]);

    println!("Sending...");
    let t0 = Instant::now();
    match client.chat_tools(&messages, &tools) {
        Ok(resp) => {
            println!("Done in {:.2}s", t0.elapsed().as_secs_f64());
            println!("Content: {:?}", resp.content);
            for tc in &resp.tool_calls {
                println!("  Tool: {} -> {:?}", tc.name, tc.arguments);
            }
        }
        Err(e) => println!("Error after {:.2}s: {e}", t0.elapsed().as_secs_f64()),
    }
}
