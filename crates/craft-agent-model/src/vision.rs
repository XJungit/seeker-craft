//! 视觉理解客户端：把截图 + 标记元素文本 → 场景自然语言描述

use anyhow::Result;
use craft_agent::core::types::Screenshot;

/// 视觉理解客户端接口
pub trait VisionClient: Send + Sync {
    /// 通用视觉问答：**PNG 编码截图** + 任意文本 prompt → 模型文本回复。
    ///
    /// 这是核心方法；[`describe`] 是其便捷封装（固定"场景描述"prompt）。
    fn chat(&self, screenshot: &Screenshot, prompt: &str) -> Result<String>;

    /// 便捷封装：用固定的"场景描述"prompt 调用 [`chat`]。
    ///
    /// `marked_elements` 为已标注元素清单文本（如 "① crafting_table ② furnace"），
    /// 拼进 prompt 作为上下文。
    fn describe(&self, screenshot: &Screenshot, marked_elements: &str) -> Result<String> {
        let prompt = format!(
            "这是一张 Minecraft 游戏截图。已标注的可点击元素编号如下：\n{marked_elements}\n\
             请用简洁中文分点说明：1) 当前界面（游戏世界/暂停菜单/背包/合成台/主菜单）；\
             2) 画面中的关键物体与可交互 UI 及大致位置；\
             3) 玩家状态（血量/饥饿/快捷栏/准星指向）。"
        );
        self.chat(screenshot, &prompt)
    }
}

/// 离线 mock：返回固定描述，便于无网络/无密钥时单测主循环
pub struct MockVisionClient;

impl VisionClient for MockVisionClient {
    fn chat(&self, _screenshot: &Screenshot, _prompt: &str) -> Result<String> {
        // 同时含 "crafting_table"（供 describe 测试）与 "2"（供 SoM 选号测试解析）
        Ok("[mock-vision] 场景含 crafting_table；选中元素 2".to_string())
    }
}

#[cfg(feature = "real")]
pub mod real {
    use super::*;
    use crate::config::BackendConfig;
    use anyhow::Context;
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use reqwest::blocking::Client;
    use serde_json::{Value, json};

    /// 把 PNG 图缩放到「最长边 ≤ `max_side`」，返回新的 PNG 字节。
    ///
    /// VLM 输入优化：截图原始分辨率（MC 窗口约 1091×724）对场景理解而言过大，
    /// 缩到 ~768px 可显著减小 base64 体积与上传/编码开销，多数 VLM 内部本就
    /// 按 768/1024 分块，缩到单块尺寸还能省一次切片。
    ///
    /// 语义：
    /// - `max_side == 0` 或原图最长边已 `≤ max_side`：原样返回（不做无谓重编码）。
    /// - 否则按等比缩放，用 Lanczos3（对 UI 文字/边缘更友好）重采样后重编码 PNG。
    ///
    /// 返回 `(png, (w, h))`：缩放后的 PNG 字节与其像素尺寸，便于调用方打点统计。
    pub fn downscale_png(png: &[u8], max_side: u32) -> Result<(Vec<u8>, (u32, u32))> {
        use image::ImageFormat;
        let img = image::load_from_memory_with_format(png, ImageFormat::Png)
            .context("解码 PNG 失败（downscale_png 期望 PNG 编码字节）")?;
        let (w, h) = (img.width(), img.height());
        let longest = w.max(h);
        if max_side == 0 || longest <= max_side {
            return Ok((png.to_vec(), (w, h)));
        }
        let scale = max_side as f32 / longest as f32;
        let nw = ((w as f32 * scale).round() as u32).max(1);
        let nh = ((h as f32 * scale).round() as u32).max(1);
        let resized = img.resize(nw, nh, image::imageops::FilterType::Lanczos3);
        let mut out = Vec::new();
        resized
            .write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png)
            .context("重新编码 PNG 失败")?;
        Ok((out, (nw, nh)))
    }

    /// 通用 **OpenAI 兼容** 视觉客户端（chat/completions）。
    ///
    /// 一份实现打所有 OpenAI 兼容后端：Agnes、MiniCPM-V、以及任意兼容服务，
    /// 只靠 `endpoint / model / api_key / extra_body` 区分——这些全部来自配置
    /// （[`crate::config::BackendConfig`]），换后端**无需改代码、无需重编译**。
    ///
    /// 已实测（2026-07-13）：图像用 **base64 data URI** 内联（`image_url.url`），
    /// 无需第三方图床；`extra_body` 可透传非标准参数（如 Agnes 关思考
    /// `chat_template_kwargs.enable_thinking=false`）。
    pub struct OpenAiVisionClient {
        /// 完整 chat/completions URL
        endpoint: String,
        model: String,
        api_key: String,
        temperature: f32,
        max_tokens: u32,
        /// 图像输入最长边上限（像素）；None=不缩放。发送前用 [`downscale_png`] 应用。
        max_side: Option<u32>,
        /// 透传进请求体顶层的非标准参数（可选）
        extra_body: Option<Value>,
        client: Client,
    }

    impl OpenAiVisionClient {
        /// 直接用四要素构造（temperature/max_tokens 取默认，无 extra_body）。
        pub fn new(
            endpoint: impl Into<String>,
            model: impl Into<String>,
            api_key: impl Into<String>,
        ) -> Self {
            Self::build(
                endpoint.into(),
                model.into(),
                api_key.into(),
                0.2,
                2048,
                None,
                None,
                180,
                true,
            )
        }

        /// **推荐入口**：从配置后端构造。密钥按 `api_key`/`api_key_env` 解析。
        pub fn from_config(cfg: &BackendConfig) -> Result<Self> {
            let api_key = cfg.resolve_api_key()?;
            Ok(Self::build(
                cfg.chat_endpoint(),
                cfg.model.clone(),
                api_key,
                cfg.temperature,
                cfg.max_tokens,
                cfg.max_side,
                cfg.extra_body.clone(),
                cfg.timeout_secs,
                cfg.force_http1,
            ))
        }

        /// 从环境变量构造（快速本地测试用，等价于 Agnes 默认后端）：
        /// - `AGNES_API_KEY`（必需）
        /// - `AGNES_API_BASE`（可选，默认 `https://api.agnes-ai.cn/v1`）
        /// - `AGNES_MODEL`（可选，默认 `agnes-2.5-flash`）
        pub fn from_env() -> Result<Self> {
            let api_key = std::env::var("AGNES_API_KEY")
                .map_err(|_| anyhow::anyhow!("未设置环境变量 AGNES_API_KEY"))?;
            let base = std::env::var("AGNES_API_BASE")
                .unwrap_or_else(|_| "https://api.agnes-ai.cn/v1".to_string());
            let endpoint = format!("{}/chat/completions", base.trim_end_matches('/'));
            let model =
                std::env::var("AGNES_MODEL")            .unwrap_or_else(|_| "agnes-2.5-flash".to_string());
            Ok(Self::new(endpoint, model, api_key))
        }

        #[allow(clippy::too_many_arguments)]
        fn build(
            endpoint: String,
            model: String,
            api_key: String,
            temperature: f32,
            max_tokens: u32,
            max_side: Option<u32>,
            extra_body: Option<Value>,
            timeout_secs: u64,
            force_http1: bool,
        ) -> Self {
            // 共用配置化 HTTP 客户端构造器（含 Windows http1_only 修复），见 config::build_http_client
            let client = crate::config::build_http_client(timeout_secs, force_http1);
            Self {
                endpoint,
                model,
                api_key,
                temperature,
                max_tokens,
                max_side,
                extra_body,
                client,
            }
        }

        /// 通用视觉问答：PNG 编码图 + 文本 prompt → 模型文本回复。
        ///
        /// `png` 必须是 **PNG 编码字节**（不是原始 RGBA）；由适配器层负责编码。
        pub fn chat_image_png(&self, png: &[u8], prompt: &str) -> Result<String> {
            // VLM 输入优化：按配置把最长边缩到 max_side，减小 base64 体积/编码开销。
            // 未配置 max_side 时零成本跳过（downscale_png 直接返回原图）。
            let scaled;
            let png: &[u8] = match self.max_side {
                Some(ms) => {
                    scaled = downscale_png(png, ms)?.0;
                    &scaled
                }
                None => png,
            };
            let data_uri = format!("data:image/png;base64,{}", STANDARD.encode(png));
            let mut body = json!({
                "model": self.model,
                "messages": [{
                    "role": "user",
                    "content": [
                        {"type": "text", "text": prompt},
                        {"type": "image_url", "image_url": {"url": data_uri}},
                    ],
                }],
                "temperature": self.temperature,
                "max_tokens": self.max_tokens,
            });
            // 合并 extra_body 的顶层键（如 chat_template_kwargs），实现私有参数透传
            if let (Some(Value::Object(extra)), Value::Object(base)) = (&self.extra_body, &mut body)
            {
                for (k, v) in extra {
                    base.insert(k.clone(), v.clone());
                }
            }
            let resp = self
                .client
                .post(&self.endpoint)
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()?
                .error_for_status()?
                .json::<serde_json::Value>()?;
            // OpenAI 兼容响应；部分推理型后端答案可能落在 reasoning_content，做兜底
            let msg = &resp["choices"][0]["message"];
            let content = msg["content"].as_str().filter(|s| !s.is_empty());
            let content = content
                .or_else(|| msg["reasoning_content"].as_str())
                .ok_or_else(|| {
                    anyhow::anyhow!("VLM 响应缺少 choices[0].message.content: {resp}")
                })?;
            Ok(content.to_string())
        }
    }

    impl VisionClient for OpenAiVisionClient {
        /// `screenshot` 在此视为 **PNG 编码字节**（适配器已编码），非原始 RGBA。
        fn chat(&self, screenshot: &Screenshot, prompt: &str) -> Result<String> {
            self.chat_image_png(screenshot, prompt)
        }
    }

    /// 向后兼容别名：旧代码里的 `AgnesVisionClient` 现指向通用客户端。
    pub type AgnesVisionClient = OpenAiVisionClient;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_vision_describes_marked_elements() {
        let v = MockVisionClient;
        let out = v
            .describe(
                &std::sync::Arc::new(vec![0u8; 16]),
                "① crafting_table ② furnace",
            )
            .unwrap();
        assert!(out.contains("crafting_table"));
    }
}

#[cfg(all(test, feature = "real"))]
mod real_tests {
    use super::real::downscale_png;

    /// 造一张 w×h 的 PNG（纯色即可，只验证尺寸逻辑）。
    fn make_png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([120, 60, 30]));
        let dynimg = image::DynamicImage::ImageRgb8(img);
        let mut out = Vec::new();
        dynimg
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn downscale_shrinks_long_side_and_keeps_aspect() {
        // 1091×724（近似 MC 窗口），缩到最长边 768
        let src = make_png(1091, 724);
        let (out, (w, h)) = downscale_png(&src, 768).unwrap();
        assert_eq!(w, 768, "最长边应精确等于 max_side");
        // 等比：724 * 768/1091 ≈ 510
        assert_eq!(h, 510);
        // 缩放后体积应明显更小
        assert!(out.len() < src.len(), "缩放后 PNG 体积应减小");
    }

    #[test]
    fn downscale_noop_when_already_small() {
        let src = make_png(640, 480);
        let (out, (w, h)) = downscale_png(&src, 768).unwrap();
        assert_eq!((w, h), (640, 480), "原图已小于上限应原样返回尺寸");
        assert_eq!(out, src, "原图已达标应字节原样返回，不重编码");
    }

    #[test]
    fn downscale_zero_max_side_is_noop() {
        let src = make_png(1091, 724);
        let (out, (w, h)) = downscale_png(&src, 0).unwrap();
        assert_eq!((w, h), (1091, 724));
        assert_eq!(out, src);
    }
}
