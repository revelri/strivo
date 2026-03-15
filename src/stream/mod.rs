pub mod resolver;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub url: String,
    pub quality: String,
    pub is_live: bool,
}
