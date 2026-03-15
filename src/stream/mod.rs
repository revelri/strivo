pub mod resolver;

#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub url: String,
    pub quality: String,
    pub is_live: bool,
}
