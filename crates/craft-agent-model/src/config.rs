//! 后端配置：以 TOML 声明多个 **OpenAI 兼容** 后端，用 `active` 一键切换。
//!
//! 设计目标：模型/端点/密钥/私有参数全部外置到配置文件，UI 或用户只需
//! 改这一份 TOML 即可换后端（Agnes ↔ MiniCPM ↔ 任意 OpenAI 兼容服务），
//! **无需改动或重编译 Rust 代码**。
//!
//! 示例见项目根 `data/config/agent.toml`。
//!
//! ```toml
//! [vlm]
//! active = "minicpm"          # 当前启用的后端名
//!
//! [vlm.backends.minicpm]
//! base_url = "https://api.modelbest.co/v1"
//! model = "MiniCPM-V-4.6-Instruct"
//! api_key_env = "MINICPM_API_KEY"   # 从环境变量读，避免明文写进文件
//!
//! [vlm.backends.agnes]
//! base_url = "https://api.agnes-ai.cn/v1"
//! model = "agnes-2.5-flash"
//! api_key_env = "AGNES_API_KEY"
//! timeout_secs = 180
//! [vlm.backends.agnes.extra_body]                 # 非标准参数透传
//! chat_template_kwargs = { enable_thinking = false }
//! ```

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// perceive 工具的视觉后端模式。
///
/// - `vlm`：独立 VLM 把截图转成文字描述，文字进历史（默认，向后兼容）。
/// - `multimodal`：截图以 base64 直接作为工具结果的图像段返回，由决策 LLM 自己看像素
///   （去掉 VLM 中间层，对"编造木头"这类幻觉更稳，因为 LLM 直接面对真实画面）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum VisionMode {
    /// 独立 VLM 转文字（默认，向后兼容）
    #[default]
    Vlm,
    /// 多模态 LLM 直读截图
    Multimodal,
}

/// perceive 配置（可选；缺省 mode=vlm，保持向后兼容）。
#[derive(Debug, Clone, Deserialize)]
pub struct PerceiveConfig {
    /// 视觉后端模式：vlm=独立视觉模型转文字，multimodal=截图直传决策 LLM 由它看像素。
    #[serde(default)]
    pub mode: VisionMode,
    /// 多模态模式下发给 LLM 的截图最长边（像素）。
    /// 缺省 768（省 token）；显式 `0` 表示不缩放（发原图）。VLM 模式忽略。
    #[serde(default = "default_perceive_image_max_side")]
    pub image_max_side: Option<u32>,
}

/// 截图最长边默认值：768px（多模态模式发往 LLM 前缩放，省带宽/编码/token）。
fn default_perceive_image_max_side() -> Option<u32> {
    Some(768)
}

impl Default for PerceiveConfig {
    fn default() -> Self {
        Self {
            mode: VisionMode::Vlm,
            image_max_side: Some(768),
        }
    }
}

/// 顶层配置：目前含 VLM，后续可平行加入 `[llm]` 决策后端（结构完全同构）。
#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    pub vlm: BackendGroup,
    /// 决策 LLM 后端组（可选；未配置时用 mock / 环境变量）
    #[serde(default)]
    pub llm: Option<BackendGroup>,
    /// perceive 的视觉后端模式（独立 VLM 转文字，或多模态 LLM 直读截图）。
    /// 可选；缺省视为 vlm，保持向后兼容。换模式只改本配置，无需重编译。
    #[serde(default)]
    pub perceive: Option<PerceiveConfig>,
    /// 专用压缩模型后端组（可选）。不配则压缩复用主决策模型。
    /// 例：用免费、512K 上下文的 agnes-2.5-flash 做压缩，隔离主模型、避免小模型卡死。
    #[serde(default)]
    pub compaction: Option<BackendGroup>,
}

/// 一组同类后端 + 当前启用项。VLM 与 LLM 复用同一结构。
#[derive(Debug, Clone, Deserialize)]
pub struct BackendGroup {
    /// 当前启用的后端名（必须是 `backends` 里的某个 key）
    pub active: String,
    /// 命名后端表：name -> 配置
    pub backends: HashMap<String, BackendConfig>,
}

impl BackendGroup {
    /// 取当前启用的后端配置。
    pub fn active_backend(&self) -> Result<&BackendConfig> {
        self.backends.get(&self.active).with_context(|| {
            format!(
                "active=\"{}\" 不在 backends 列表中（可选：{:?}）",
                self.active,
                self.backends.keys().collect::<Vec<_>>()
            )
        })
    }
}

/// 单个 OpenAI 兼容后端的完整配置。
#[derive(Debug, Clone, Deserialize)]
pub struct BackendConfig {
    /// 基址，如 `https://api.modelbest.co/v1`（末尾 /chat/completions 由代码补）
    pub base_url: String,
    /// 模型名，如 `MiniCPM-V-4.6-Instruct`
    pub model: String,
    /// 明文密钥（不推荐，优先用 `api_key_env`）
    #[serde(default)]
    pub api_key: Option<String>,
    /// 从该环境变量读密钥（推荐，避免密钥进版本库）
    #[serde(default)]
    pub api_key_env: Option<String>,
    /// 请求超时秒数（默认 180）
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// 是否强制 HTTP/1.1（默认 true；Windows schannel 对部分 CDN 的 HTTP/2 ALPN 会卡握手）
    #[serde(default = "default_true")]
    pub force_http1: bool,
    /// 采样温度（默认 0.2）
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// 最大生成 token（默认 2048）
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// 模型上下文窗口 token 数。Agent 自动压缩预算必须从这里读取，不能按模型名硬编码。
    #[serde(default = "default_context_window")]
    pub context_window: u32,
    /// 图像预处理：最长边缩放到该像素（None=不缩放）。由适配器层负责实际缩放。
    #[serde(default)]
    pub max_side: Option<u32>,
    /// 非标准参数透传：整体合并进请求 body 顶层。
    /// 例如 Agnes 关思考：`{ "chat_template_kwargs": { "enable_thinking": false } }`
    #[serde(default)]
    pub extra_body: Option<serde_json::Value>,
}

fn default_timeout() -> u64 {
    180
}
fn default_true() -> bool {
    true
}
fn default_temperature() -> f32 {
    0.2
}
fn default_max_tokens() -> u32 {
    2048
}
fn default_context_window() -> u32 {
    128_000
}

impl BackendConfig {
    /// 解析出可用密钥：优先 `api_key`，否则从 `api_key_env` 指定的环境变量读。
    pub fn resolve_api_key(&self) -> Result<String> {
        if let Some(k) = &self.api_key
            && !k.is_empty()
        {
            return Ok(k.clone());
        }
        if let Some(env_name) = &self.api_key_env {
            return std::env::var(env_name).with_context(|| {
                format!("配置指定 api_key_env=\"{env_name}\"，但该环境变量未设置")
            });
        }
        anyhow::bail!("后端未提供 api_key 也未提供 api_key_env")
    }

    /// 拼出完整的 chat/completions URL。
    pub fn chat_endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }
}

impl AgentConfig {
    /// 从 TOML 文件加载（OnceLock 缓存，同一路径只解析一次）。
    ///
    /// 如果给定路径不存在，自动向上搜索父目录（最多 5 层），方便从子目录运行。
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        static CACHE: OnceLock<(PathBuf, AgentConfig)> = OnceLock::new();
        let path = path.as_ref();
        if let Some(cached) = CACHE.get()
            && cached.0 == path
        {
            return Ok(cached.1.clone());
        }
        let resolved = Self::resolve_path(path);
        let text = std::fs::read_to_string(&resolved)
            .with_context(|| format!("读取配置文件失败: {}", path.display()))?;
        let cfg: AgentConfig =
            toml::from_str(&text).with_context(|| format!("解析 TOML 失败: {}", path.display()))?;
        CACHE.set((resolved, cfg.clone())).ok();
        Ok(cfg)
    }

    /// 尝试解析路径：若原始路径存在则直接返回，否则向上搜索父目录。
    fn resolve_path(path: &Path) -> PathBuf {
        if path.exists() {
            return path.to_path_buf();
        }
        // 最多向上搜索 5 层父目录
        let mut current = std::env::current_dir().unwrap_or_default();
        for _ in 0..5 {
            let candidate = current.join(path);
            if candidate.exists() {
                return candidate;
            }
            if !current.pop() {
                break;
            }
        }
        path.to_path_buf()
    }
}

/// 构造一个统一配置的阻塞式 reqwest 客户端（VLM/LLM 客户端共用，避免各存一份）。
///
/// Windows 下 schannel(native-tls) 走 HTTP/2 ALPN 对部分 CDN 端点会卡握手 →
/// `force_http1` 默认强制 HTTP/1.1；同时禁系统代理、设显式超时，避免无限挂起。
/// 此规则已在 2026-07-13 真机验证：Agnes CDN 端点不加 http1_only 会无限超时。
#[cfg(feature = "real")]
pub fn build_http_client(timeout_secs: u64, force_http1: bool) -> reqwest::blocking::Client {
    let mut builder = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(timeout_secs));
    if force_http1 {
        builder = builder.http1_only();
    }
    builder.build().expect("构建 reqwest 客户端失败")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[vlm]
active = "minicpm"

[vlm.backends.minicpm]
base_url = "https://api.modelbest.co/v1"
model = "MiniCPM-V-4.6-Instruct"
api_key_env = "MINICPM_API_KEY"

[vlm.backends.agnes]
base_url = "https://api.agnes-ai.cn/v1"
model = "agnes-2.5-flash"
api_key_env = "AGNES_API_KEY"
timeout_secs = 180
[vlm.backends.agnes.extra_body]
chat_template_kwargs = { enable_thinking = false }
"#;

    #[test]
    fn parses_multi_backend_and_selects_active() {
        let cfg: AgentConfig = toml::from_str(SAMPLE).unwrap();
        let b = cfg.vlm.active_backend().unwrap();
        assert_eq!(b.model, "MiniCPM-V-4.6-Instruct");
        assert_eq!(
            b.chat_endpoint(),
            "https://api.modelbest.co/v1/chat/completions"
        );
        // 默认值生效
        assert_eq!(b.temperature, 0.2);
        assert_eq!(b.max_tokens, 2048);
        assert!(b.force_http1);
    }

    #[test]
    fn extra_body_and_explicit_key_parsed() {
        let cfg: AgentConfig = toml::from_str(SAMPLE).unwrap();
        let agnes = cfg.vlm.backends.get("agnes").unwrap();
        let expected = std::env::var("AGNES_API_KEY").unwrap_or_else(|_| "sk-test".to_string());
        assert_eq!(agnes.resolve_api_key().unwrap(), expected);
        let eb = agnes.extra_body.as_ref().unwrap();
        assert_eq!(eb["chat_template_kwargs"]["enable_thinking"], false);
    }

    #[test]
    fn perceive_config_parses_and_defaults_to_vlm() {
        // 缺省 [perceive] 时应为 None（向后兼容旧配置）
        let cfg: AgentConfig = toml::from_str(SAMPLE).unwrap();
        assert!(cfg.perceive.is_none(), "缺省配置不应含 perceive");

        // 显式配置 multimodal 模式 + 自定义最长边
        let with_perceive =
            format!("{SAMPLE}\n[perceive]\nmode = \"multimodal\"\nimage_max_side = 1024\n");
        let cfg: AgentConfig = toml::from_str(&with_perceive).unwrap();
        let p = cfg.perceive.expect("应解析出 perceive 配置");
        assert_eq!(p.mode, VisionMode::Multimodal);
        assert_eq!(p.image_max_side, Some(1024));

        // 仅给 mode=vlm 缺失 image_max_side 时取默认 768
        let vlm_only = format!("{SAMPLE}\n[perceive]\nmode = \"vlm\"\n");
        let cfg: AgentConfig = toml::from_str(&vlm_only).unwrap();
        let p = cfg.perceive.expect("应解析出 perceive 配置");
        assert_eq!(p.mode, VisionMode::Vlm);
        assert_eq!(
            p.image_max_side,
            Some(768),
            "vlm 模式缺省 image_max_side 应为 768"
        );
    }
}
