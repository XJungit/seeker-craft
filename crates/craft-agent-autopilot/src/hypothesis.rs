//! 假设生成与验证

#[derive(Debug, Clone)]
pub struct Hypothesis {
    pub cause: String,
    pub prediction: String,
    pub test_description: String,
    pub impact_scope: u32,
    pub confidence: f64,
}

impl Hypothesis {
    pub fn summary(&self) -> String {
        format!("[conf={:.2}] {} → {}", self.confidence, self.cause, self.test_description)
    }
}
