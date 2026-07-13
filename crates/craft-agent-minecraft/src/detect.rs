//! P2 3D 目标检测（YOLO-World + CLIP ViT-B/32 文本嵌入）
//!
//! 流程：
//! 1. 加载 YOLO-World ONNX 模型（DirectML，含 images + txt_feats 双输入）
//! 2. 加载预计算的 CLIP ViT-B/32 文本嵌入（mc_classes_emb.npy）
//! 3. 截图 → resize 到 640×640 → normalize → NCHW 张量
//! 4. 推理（images + txt_feats 双输入）
//! 5. 后处理：sigmoid → NMS → 筛选置信度 → Vec<Target>

use anyhow::{Context, Result, anyhow};
use craft_agent::core::types::Target;
use ort::session::Session;
use ort::value::{DynTensor, Shape, Tensor};
use std::collections::HashMap;
use std::path::Path;

/// MC 检测目标的类别列表（对应 mc_classes_emb.npy 的嵌入顺序）。
pub static MC_CLASSES: &[&str] = &[
    "tree",
    "stone",
    "cow",
    "creeper",
    "grass",
    "dirt",
    "water",
    "crafting table",
    "flower",
    "leaves",
    "iron ore",
    "skeleton",
    "zombie",
    "chicken",
    "pig",
    "sheep",
    "sand",
    "sugar cane",
    "wood",
];

/// 模型输入尺寸（YOLO-World 默认 640）
const INPUT_SIZE: u32 = 640;
/// 置信度阈值
const CONF_THRESHOLD: f32 = 0.25;
/// NMS IoU 阈值
const NMS_THRESHOLD: f32 = 0.45;

/// 3D 目标检测器。
pub struct ObjectDetector {
    session: Session,
    /// 预计算的 CLIP 文本嵌入 [num_classes * 512]，L2 归一化
    txt_feats: Vec<f32>,
    num_classes: usize,
}

fn find_npy_header_end(data: &[u8]) -> Option<usize> {
    if data.len() < 10 || data[0] != 0x93 || data[1] != b'N' || data[2] != b'U' {
        return None;
    }
    let header_len = u16::from_le_bytes([data[8], data[9]]) as usize;
    let header_end = 10 + header_len;
    Some((header_end + 63) & !63)
}

impl ObjectDetector {
    /// 加载模型 + 文本嵌入。
    pub fn load(models_dir: &Path) -> Result<Self> {
        let model_path = models_dir.join("yolov8s-worldv2.onnx");
        let emb_path = models_dir.join("mc_classes_emb.npy");

        let session = Session::builder()
            .context("创建 ort 会话构建器失败")?
            .commit_from_file(&model_path)
            .context(format!(
                "加载 YOLO-World 模型失败: {}",
                model_path.display()
            ))?;

        // 加载预计算的 CLIP 文本嵌入 (num_classes, 512)
        let emb_bytes = std::fs::read(&emb_path)
            .context(format!("读取嵌入文件失败: {}", emb_path.display()))?;
        let header_end =
            find_npy_header_end(&emb_bytes).ok_or_else(|| anyhow!("无法解析 .npy header"))?;
        let data = &emb_bytes[header_end..];
        let num_floats = data.len() / 4;
        let num_classes = MC_CLASSES.len();
        let expected = num_classes * 512;
        if num_floats != expected {
            return Err(anyhow!(
                "嵌入数据大小不匹配：读取 {num_floats} 个 float，期望 {expected}"
            ));
        }
        let txt_feats = {
            let mut flat = vec![0f32; num_floats];
            unsafe {
                std::ptr::copy_nonoverlapping(
                    data.as_ptr() as *const f32,
                    flat.as_mut_ptr(),
                    num_floats,
                );
            }
            flat
        };

        Ok(Self {
            session,
            txt_feats,
            num_classes,
        })
    }

    /// 在单张 PNG 截图上执行检测。
    pub fn detect(&mut self, png: &[u8], _screen_w: u32, _screen_h: u32) -> Result<Vec<Target>> {
        let (orig_w, orig_h, output_data, num_anchors) = self.run_raw(png)?;
        Ok(self.postprocess(orig_w, orig_h, &output_data, num_anchors))
    }

    /// debug 模式：打印分数分布 + top-20 候选。
    pub fn detect_debug(&mut self, png: &[u8], _screen_w: u32, _screen_h: u32) -> Result<()> {
        let (orig_w, orig_h, output_data, num_anchors) = self.run_raw(png)?;
        let num_classes = self.num_classes;

        let class_base = 4usize * num_anchors;
        let mut all_scores: Vec<(f32, usize, usize)> = Vec::new();

        for a in 0..num_anchors {
            for c in 0..num_classes {
                let logit = output_data[class_base + c * num_anchors + a];
                let score = sigmoid(logit);
                all_scores.push((score, c, a));
            }
        }

        // 直方图
        let mut hist = [0usize; 10];
        for (s, _, _) in &all_scores {
            let bin = (s * 10.0) as usize;
            if bin < 10 {
                hist[bin] += 1;
            }
        }
        println!(
            "[detect_debug] 分数直方图 (总锚框: {}):",
            num_anchors * num_classes
        );
        for b in 0..10 {
            let lo = b as f32 / 10.0;
            let hi = lo + 0.1;
            if hist[b] > 0 {
                println!("  {:.1}~{:.1}: {} 个", lo, hi, hist[b]);
            }
        }

        all_scores.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        let max_score = all_scores.first().map(|s| s.0).unwrap_or(0.0);
        println!("[detect_debug] 最高分: {:.4}", max_score);
        println!("[detect_debug] Top-20 候选:");
        for (i, (score, cls, anchor)) in all_scores.iter().take(20).enumerate() {
            let cx = output_data[*anchor] / INPUT_SIZE as f32 * orig_w as f32;
            let cy = output_data[num_anchors + anchor] / INPUT_SIZE as f32 * orig_h as f32;
            let bw = output_data[2 * num_anchors + anchor] / INPUT_SIZE as f32 * orig_w as f32;
            let bh = output_data[3 * num_anchors + anchor] / INPUT_SIZE as f32 * orig_h as f32;
            let area_ratio = (bw * bh) / (orig_w as f32 * orig_h as f32);
            println!(
                "  #{:2}  {:.4}  {:16} cx={:.1} cy={:.1} w={:.1} h={:.1} 面积比={:.4}",
                i + 1,
                score,
                MC_CLASSES[*cls],
                cx,
                cy,
                bw,
                bh,
                area_ratio
            );
        }
        println!("[detect_debug] 截图尺寸: {}x{}", orig_w, orig_h);
        Ok(())
    }

    /// 预处理和推理
    fn run_raw(&mut self, png: &[u8]) -> Result<(u32, u32, Vec<f32>, usize)> {
        let img = image::load_from_memory(png)
            .context("detect: 解码截图 PNG 失败")?
            .to_rgba8();
        let (orig_w, orig_h) = (img.width(), img.height());

        let resized = image::imageops::resize(
            &img,
            INPUT_SIZE,
            INPUT_SIZE,
            image::imageops::FilterType::CatmullRom,
        );

        let mut tensor_data = vec![0f32; (INPUT_SIZE * INPUT_SIZE * 3) as usize];
        for y in 0..INPUT_SIZE {
            for x in 0..INPUT_SIZE {
                let px = resized.get_pixel(x, y);
                let idx = (y * INPUT_SIZE + x) as usize;
                tensor_data[idx] = px[0] as f32 / 255.0;
                tensor_data[idx + (INPUT_SIZE * INPUT_SIZE) as usize] = px[1] as f32 / 255.0;
                tensor_data[idx + (2 * INPUT_SIZE * INPUT_SIZE) as usize] = px[2] as f32 / 255.0;
            }
        }

        let mut input_map: HashMap<&str, DynTensor> = HashMap::new();
        input_map.insert(
            "images",
            Tensor::from_array((
                vec![1i64, 3, INPUT_SIZE as i64, INPUT_SIZE as i64],
                tensor_data,
            ))
            .context("构建 images 张量失败")?
            .upcast(),
        );
        input_map.insert(
            "txt_feats",
            Tensor::from_array((
                vec![1i64, self.num_classes as i64, 512i64],
                self.txt_feats.clone(),
            ))
            .context("构建 txt_feats 张量失败")?
            .upcast(),
        );

        let outputs = self.session.run(input_map).context("YOLO-World 推理失败")?;

        let output = outputs
            .get("output0")
            .ok_or_else(|| anyhow!("输出无 output0"))?;
        let (output_shape, output_data) = output
            .try_extract_tensor::<f32>()
            .context("提取输出张量失败")?;

        let shape: &[i64] = &**output_shape;
        let num_anchors = shape[2] as usize;

        Ok((orig_w, orig_h, output_data.to_vec(), num_anchors))
    }

    /// 后处理：sigmoid → 过滤 → NMS → Target 列表
    fn postprocess(
        &self,
        orig_w: u32,
        orig_h: u32,
        output_data: &[f32],
        num_anchors: usize,
    ) -> Vec<Target> {
        let num_classes = self.num_classes;
        let class_base = 4 * num_anchors;

        // 收集通过置信度阈值的候选
        let mut candidates: Vec<(f32, usize, usize, f32, f32, f32, f32)> = Vec::new();
        for a in 0..num_anchors {
            for c in 0..num_classes {
                let logit = output_data[class_base + c * num_anchors + a];
                let score = sigmoid(logit);
                if score < CONF_THRESHOLD {
                    continue;
                }
                let cx = output_data[a] / INPUT_SIZE as f32 * orig_w as f32;
                let cy = output_data[num_anchors + a] / INPUT_SIZE as f32 * orig_h as f32;
                let w = output_data[2 * num_anchors + a] / INPUT_SIZE as f32 * orig_w as f32;
                let h = output_data[3 * num_anchors + a] / INPUT_SIZE as f32 * orig_h as f32;
                candidates.push((score, a, c, cx, cy, w, h));
            }
        }

        if candidates.is_empty() {
            return Vec::new();
        }

        // NMS
        candidates.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        let mut keep: Vec<bool> = vec![true; candidates.len()];
        for i in 0..candidates.len() {
            if !keep[i] {
                continue;
            }
            let (_si, _ai, _ci, cxi, cyi, wi, hi) = candidates[i];
            let (x1i, y1i, x2i, y2i) = (
                cxi - wi / 2.0,
                cyi - hi / 2.0,
                cxi + wi / 2.0,
                cyi + hi / 2.0,
            );
            for j in (i + 1)..candidates.len() {
                if !keep[j] {
                    continue;
                }
                let (_sj, _aj, _cj, cxj, cyj, wj, hj) = candidates[j];
                let (x1j, y1j, x2j, y2j) = (
                    cxj - wj / 2.0,
                    cyj - hj / 2.0,
                    cxj + wj / 2.0,
                    cyj + hj / 2.0,
                );
                let iou_val = iou(x1i, y1i, x2i, y2i, x1j, y1j, x2j, y2j);
                if iou_val > NMS_THRESHOLD {
                    keep[j] = false;
                }
            }
        }

        let (screen_cx, screen_cy) = (orig_w as f32 / 2.0, orig_h as f32 / 2.0);
        let mut targets = Vec::new();
        for (i, kept) in keep.iter().enumerate() {
            if !kept {
                continue;
            }
            let (score, _a, c, cx, cy, w, h) = candidates[i];
            let bbox = [
                (cx - w / 2.0).max(0.0) as i32,
                (cy - h / 2.0).max(0.0) as i32,
                w as i32,
                h as i32,
            ];
            targets.push(Target {
                label: MC_CLASSES[c].to_string(),
                bbox,
                offset_from_crosshair: ((cx - screen_cx) as i32, (cy - screen_cy) as i32),
            });
        }
        targets
    }
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

fn iou(x1a: f32, y1a: f32, x2a: f32, y2a: f32, x1b: f32, y1b: f32, x2b: f32, y2b: f32) -> f32 {
    let inter_x1 = x1a.max(x1b);
    let inter_y1 = y1a.max(y1b);
    let inter_x2 = x2a.min(x2b);
    let inter_y2 = y2a.min(y2b);
    let inter_w = (inter_x2 - inter_x1).max(0.0);
    let inter_h = (inter_y2 - inter_y1).max(0.0);
    let inter_area = inter_w * inter_h;
    if inter_area <= 0.0 {
        return 0.0;
    }
    let area_a = (x2a - x1a) * (y2a - y1a);
    let area_b = (x2b - x1b) * (y2b - y1b);
    let union_area = area_a + area_b - inter_area;
    if union_area <= 0.0 {
        return 0.0;
    }
    inter_area / union_area
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sigmoid_sanity() {
        assert!((sigmoid(0.0) - 0.5).abs() < 1e-6);
        assert!(sigmoid(100.0) > 0.999);
    }

    #[test]
    fn iou_sanity() {
        let v = iou(0.0, 0.0, 10.0, 10.0, 5.0, 5.0, 15.0, 15.0);
        assert!((v - 0.14285715).abs() < 1e-6);
    }
}
