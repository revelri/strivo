use rand::Rng;

/// API key used both for the `X-Api-Key` header and the session cookie.
/// Generated on first run and persisted in `[web] api_key` in `config.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKey(pub String);

impl ApiKey {
    pub fn generate() -> Self {
        const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
        let mut rng = rand::thread_rng();
        let s: String = (0..32)
            .map(|_| {
                let i = rng.gen_range(0..CHARSET.len());
                CHARSET[i] as char
            })
            .collect();
        ApiKey(s)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Constant-time compare to mitigate timing oracles (M4.6.9). The
    /// API key is short and low-rate, but using a `==` compare on the
    /// raw bytes leaks length-prefix information — better to spend
    /// the cycles.
    pub fn matches(&self, candidate: &str) -> bool {
        let a = self.0.as_bytes();
        let b = candidate.as_bytes();
        let mut diff: u8 = (a.len() as u8) ^ (b.len() as u8);
        let n = a.len().min(b.len());
        for i in 0..n {
            diff |= a[i] ^ b[i];
        }
        diff == 0
    }
}
