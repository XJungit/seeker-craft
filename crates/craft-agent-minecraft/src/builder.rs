//! Minecraft Agent unified builder.
//!
//! Supports two runtime adapter modes:
//! - `McAdapter::ModBridge`: structured mod TCP bridge (primary path).
//! - `McAdapter::Real`: xcap + enigo real-machine path (preserved).
//!
//! The builder hides adapter-specific wiring and returns a ready-to-run `Agent`
//! with MC tools registered.

#[cfg(all(feature = "real", not(feature = "mod-bridge")))]
use anyhow::Context;

#[cfg(all(feature = "real", not(feature = "mod-bridge")))]
use crate::adapter::MinecraftAdapter;
#[cfg(feature = "mod-bridge")]
use crate::adapter_mod::MinecraftModAdapter;
#[cfg(all(feature = "real", not(feature = "mod-bridge")))]
use crate::tools::create_mc_tools;
#[cfg(feature = "mod-bridge")]
use crate::tools_mod::create_mc_mod_tools;
use anyhow::Result;
use craft_agent::agent::{Agent, AgentConfig};
use craft_agent::core::adapter::GameAdapter;
use craft_agent::core::session::Session;
use craft_agent::core::tool::ToolRegistry;
use craft_agent_model::config::VisionMode;
use craft_agent_model::vision::VisionClient;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[cfg(feature = "real")]
use enigo::{Enigo, Settings};

/// MC runtime adapter selection.
pub enum McAdapter {
    /// Mod bridge path: connect to local MC mod TCP server.
    #[cfg(feature = "mod-bridge")]
    ModBridge {
        host: String,
        port: u16,
        vision: Option<Arc<dyn VisionClient>>,
    },
    /// Real machine path: xcap + enigo real-machine path (preserved).
    Real {
        fullscreen: bool,
        vlm: std::sync::Arc<dyn VisionClient>,
        capture: Arc<dyn Fn() -> anyhow::Result<Vec<u8>> + Send + Sync>,
        #[cfg(feature = "real")]
        enigo: Arc<Mutex<Enigo>>,
        perceive_mode: VisionMode,
    },
}

/// Unified Minecraft Agent builder.
pub struct McAgentBuilder {
    adapter: McAdapter,
    config: AgentConfig,
    session: Option<Session>,
    prompt: Option<String>,
    enable_visual_perceive: bool,
    image_max_side: Option<u32>,
    shots_dir: Option<PathBuf>,
}

impl McAgentBuilder {
    /// Create a builder for mod-bridge mode (primary path).
    #[cfg(feature = "mod-bridge")]
    pub fn mod_bridge(host: impl Into<String>, port: u16) -> Self {
        Self {
            adapter: McAdapter::ModBridge {
                host: host.into(),
                port,
                vision: None,
            },
            config: AgentConfig::new(String::new(), 50),
            session: None,
            prompt: None,
            enable_visual_perceive: false,
            image_max_side: None,
            shots_dir: None,
        }
    }

    /// Create a builder for real machine mode (xcap + enigo).
    #[cfg(feature = "real")]
    pub fn real(
        vlm: std::sync::Arc<dyn VisionClient>,
        capture: Arc<dyn Fn() -> anyhow::Result<Vec<u8>> + Send + Sync>,
        fullscreen: bool,
    ) -> Self {
        let enigo = Arc::new(Mutex::new(
            Enigo::new(&Settings::default()).expect("create enigo failed"),
        ));
        Self {
            adapter: McAdapter::Real {
                fullscreen,
                vlm,
                capture,
                enigo,
                perceive_mode: VisionMode::Multimodal,
            },
            config: AgentConfig::new(String::new(), 50),
            session: None,
            prompt: None,
            enable_visual_perceive: false,
            image_max_side: None,
            shots_dir: None,
        }
    }

    /// Use an existing session for persistence / recovery.
    pub fn with_session(mut self, session: Session) -> Self {
        self.session = Some(session);
        self
    }

    /// Load an existing session file to continue from a previous run.
    pub fn with_session_file(mut self, path: impl AsRef<std::path::Path>) -> Result<Self> {
        let session = Session::open(path.as_ref())
            .map_err(|e| anyhow::anyhow!("打开 session {} 失败: {e}", path.as_ref().display()))?;
        self.session = Some(session);
        Ok(self)
    }

    /// Set the agent system prompt / goal.
    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = Some(prompt.into());
        self
    }

    /// Set agent config (overrides default `AgentConfig::new`).
    pub fn with_config(mut self, config: AgentConfig) -> Self {
        self.config = config;
        self
    }

    /// Enable `visual_perceive` for mod-bridge mode (optional VLM GUI inspection).
    pub fn enable_visual_perceive(mut self, enable: bool) -> Self {
        self.enable_visual_perceive = enable;
        self
    }

    /// Resize screenshots before VLM / LLM input.
    pub fn image_max_side(mut self, max_side: u32) -> Self {
        self.image_max_side = Some(max_side);
        self
    }

    /// Save screenshots to directory for debugging / viewer replay.
    pub fn shots_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.shots_dir = Some(dir.into());
        self
    }

    /// Build adapter + tool registry without constructing `Agent`.
    pub fn build_adapter_and_tools(&self) -> Result<(Box<dyn GameAdapter>, ToolRegistry)> {
        match &self.adapter {
            #[cfg(feature = "mod-bridge")]
            McAdapter::ModBridge { host, port, vision } => {
                let vision = vision.clone();
                let adapter: MinecraftModAdapter =
                    MinecraftModAdapter::connect_with_vision(host, *port, vision)?;
                let adapter_arc = std::sync::Arc::new(std::sync::Mutex::new(adapter));
                let tools = create_mc_mod_tools(
                    adapter_arc.clone(),
                    self.image_max_side,
                    self.shots_dir.clone(),
                    self.enable_visual_perceive,
                    None,
                );
                let mut registry = ToolRegistry::new();
                registry.extend(tools);
                let adapter_wrapper = crate::adapter_mod::ArcGameAdapter(adapter_arc);
                Ok((Box::new(adapter_wrapper), registry))
            }
            #[cfg(all(feature = "real", not(feature = "mod-bridge")))]
            McAdapter::Real {
                fullscreen,
                vlm,
                capture,
                ..
            } => {
                let vlm_box = vlm.clone();
                let adapter = if *fullscreen {
                    MinecraftAdapter::new_fullscreen(vlm_box)?
                } else {
                    MinecraftAdapter::new(vlm_box)?
                };
                let enigo_rc = {
                    let enigo = Enigo::new(&Settings::default()).context("create enigo failed")?;
                    Arc::new(Mutex::new(enigo))
                };

                let tools = create_mc_tools(
                    vlm.clone(),
                    capture.clone(),
                    enigo_rc,
                    VisionMode::Multimodal,
                    self.image_max_side,
                    self.shots_dir.clone(),
                );
                let mut registry = ToolRegistry::new();
                registry.extend(tools);
                Ok((Box::new(adapter), registry))
            }
            #[cfg(all(feature = "real", feature = "mod-bridge"))]
            McAdapter::Real {
                fullscreen,
                vlm,
                capture,
                ..
            } => {
                let _ = (fullscreen, vlm, capture);
                anyhow::bail!("Real adapter path is disabled under mod-bridge feature")
            }
        }
    }

    /// Build a ready-to-run `Agent`.
    pub fn build(
        self,
        provider: impl Into<Box<dyn craft_agent::agent::LlmProvider>>,
    ) -> Result<Agent> {
        let (_adapter, tools) = self.build_adapter_and_tools()?;
        let mut agent = Agent::new(provider.into(), tools, self.config);
        if let Some(session) = self.session {
            agent = agent.with_session(session);
        }
        if let Some(prompt) = self.prompt {
            agent.set_self_prompt(prompt);
        }
        Ok(agent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_bridge_builder_compile_smoke() {
        let _builder = McAgentBuilder::mod_bridge("127.0.0.1", 25565)
            .with_prompt("收集木头")
            .enable_visual_perceive(true)
            .image_max_side(640)
            .shots_dir("/tmp/mc_shots");
    }

    #[cfg(all(feature = "real", not(feature = "mod-bridge")))]
    #[test]
    fn real_builder_compile_smoke() {
        struct DummyVision;
        impl VisionClient for DummyVision {
            fn describe(&self, _png: &[u8]) -> anyhow::Result<String> {
                Ok("dummy".into())
            }
        }
        let _builder =
            McAgentBuilder::real(Arc::new(DummyVision), Box::new(|| Ok(vec![1, 2, 3])), false)
                .with_prompt("收集石头");
    }
}
