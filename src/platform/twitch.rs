use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::RwLock;
use std::sync::Arc;

use crate::config::credentials;
use crate::platform::{ChannelEntry, Platform, PlatformKind};

const TWITCH_AUTH_URL: &str = "https://id.twitch.tv/oauth2";
const TWITCH_API_URL: &str = "https://api.twitch.tv/helix";

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    token_type: String,
}

#[derive(Debug, Deserialize)]
struct TokenErrorResponse {
    status: Option<u16>,
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TwitchUser {
    id: String,
    login: String,
    display_name: String,
}

#[derive(Debug, Deserialize)]
struct UsersResponse {
    data: Vec<TwitchUser>,
}

#[derive(Debug, Deserialize)]
struct FollowedChannel {
    broadcaster_id: String,
    broadcaster_login: String,
    broadcaster_name: String,
}

#[derive(Debug, Deserialize)]
struct FollowedResponse {
    data: Vec<FollowedChannel>,
    pagination: Pagination,
}

#[derive(Debug, Deserialize)]
struct Pagination {
    cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamData {
    id: String,
    user_id: String,
    user_login: String,
    user_name: String,
    game_name: Option<String>,
    title: Option<String>,
    viewer_count: Option<u64>,
    started_at: Option<String>,
    thumbnail_url: Option<String>,
    #[serde(rename = "type")]
    stream_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamsResponse {
    data: Vec<StreamData>,
}

pub struct TwitchPlatform {
    client: Client,
    client_id: String,
    access_token: Arc<RwLock<Option<String>>>,
    refresh_token_value: Arc<RwLock<Option<String>>>,
    user_id: Arc<RwLock<Option<String>>>,
    /// Set during device code flow for the TUI to display
    pub pending_device_code: Arc<RwLock<Option<DeviceCodeInfo>>>,
}

#[derive(Debug, Clone)]
pub struct DeviceCodeInfo {
    pub user_code: String,
    pub verification_uri: String,
}

impl TwitchPlatform {
    pub fn new(client_id: String) -> Self {
        Self {
            client: Client::new(),
            client_id,
            access_token: Arc::new(RwLock::new(None)),
            refresh_token_value: Arc::new(RwLock::new(None)),
            user_id: Arc::new(RwLock::new(None)),
            pending_device_code: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn load_stored_tokens(&self) -> Result<bool> {
        if let Some(token) = credentials::get_secret("twitch_access_token")? {
            *self.access_token.write().await = Some(token);
            if let Some(refresh) = credentials::get_secret("twitch_refresh_token")? {
                *self.refresh_token_value.write().await = Some(refresh);
            }
            // Validate the token
            if self.validate_token().await? {
                self.fetch_user_id().await?;
                return Ok(true);
            }
            // Try refresh
            if self.refresh_token_value.read().await.is_some() {
                if self.do_refresh_token().await.is_ok() {
                    self.fetch_user_id().await?;
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    async fn validate_token(&self) -> Result<bool> {
        let token = self.access_token.read().await;
        let Some(token) = token.as_ref() else {
            return Ok(false);
        };
        let resp = self
            .client
            .get(format!("{TWITCH_AUTH_URL}/validate"))
            .header("Authorization", format!("OAuth {token}"))
            .send()
            .await?;
        Ok(resp.status().is_success())
    }

    async fn fetch_user_id(&self) -> Result<()> {
        let token = self.access_token.read().await.clone();
        let Some(token) = token else {
            bail!("No access token");
        };
        let resp: UsersResponse = self
            .client
            .get(format!("{TWITCH_API_URL}/users"))
            .header("Authorization", format!("Bearer {token}"))
            .header("Client-Id", &self.client_id)
            .send()
            .await?
            .json()
            .await?;
        if let Some(user) = resp.data.into_iter().next() {
            *self.user_id.write().await = Some(user.id);
        }
        Ok(())
    }

    async fn device_code_flow(&self) -> Result<()> {
        // Step 1: Request device code
        let resp: DeviceCodeResponse = self
            .client
            .post(format!("{TWITCH_AUTH_URL}/device"))
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("scopes", "user:read:follows"),
            ])
            .send()
            .await?
            .json()
            .await
            .context("Failed to get device code from Twitch")?;

        // Store device code info for TUI display
        *self.pending_device_code.write().await = Some(DeviceCodeInfo {
            user_code: resp.user_code.clone(),
            verification_uri: resp.verification_uri.clone(),
        });

        tracing::info!(
            "Twitch auth: go to {} and enter code: {}",
            resp.verification_uri,
            resp.user_code
        );

        // Step 2: Poll for token
        let interval = std::time::Duration::from_secs(resp.interval.max(5));
        let deadline = tokio::time::Instant::now()
            + std::time::Duration::from_secs(resp.expires_in);

        loop {
            tokio::time::sleep(interval).await;

            if tokio::time::Instant::now() > deadline {
                *self.pending_device_code.write().await = None;
                bail!("Device code expired");
            }

            let token_resp = self
                .client
                .post(format!("{TWITCH_AUTH_URL}/token"))
                .form(&[
                    ("client_id", self.client_id.as_str()),
                    ("device_code", resp.device_code.as_str()),
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ])
                .send()
                .await?;

            let status = token_resp.status();
            let body = token_resp.text().await?;

            if status.is_success() {
                let token: TokenResponse = serde_json::from_str(&body)?;
                credentials::store_secret("twitch_access_token", &token.access_token)?;
                if let Some(ref refresh) = token.refresh_token {
                    credentials::store_secret("twitch_refresh_token", refresh)?;
                    *self.refresh_token_value.write().await = Some(refresh.clone());
                }
                *self.access_token.write().await = Some(token.access_token);
                *self.pending_device_code.write().await = None;
                self.fetch_user_id().await?;
                return Ok(());
            }

            // Check if still pending (authorization_pending) or an actual error
            if let Ok(err) = serde_json::from_str::<TokenErrorResponse>(&body) {
                if err.status == Some(400) {
                    // Still pending, continue polling
                    continue;
                }
            }
        }
    }

    async fn do_refresh_token(&self) -> Result<()> {
        let refresh = self.refresh_token_value.read().await.clone();
        let Some(refresh) = refresh else {
            bail!("No refresh token available");
        };

        let resp = self
            .client
            .post(format!("{TWITCH_AUTH_URL}/token"))
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("refresh_token", refresh.as_str()),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            bail!("Token refresh failed: {}", resp.status());
        }

        let token: TokenResponse = resp.json().await?;
        credentials::store_secret("twitch_access_token", &token.access_token)?;
        if let Some(ref new_refresh) = token.refresh_token {
            credentials::store_secret("twitch_refresh_token", new_refresh)?;
            *self.refresh_token_value.write().await = Some(new_refresh.clone());
        }
        *self.access_token.write().await = Some(token.access_token);
        Ok(())
    }

    async fn api_get<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let token = self.access_token.read().await.clone();
        let Some(token) = token else {
            bail!("Not authenticated");
        };

        let resp = self
            .client
            .get(url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Client-Id", &self.client_id)
            .send()
            .await?;

        if resp.status().as_u16() == 401 {
            drop(resp);
            // Try refresh
            self.do_refresh_token().await?;
            let token = self.access_token.read().await.clone().unwrap();
            let resp = self
                .client
                .get(url)
                .header("Authorization", format!("Bearer {token}"))
                .header("Client-Id", &self.client_id)
                .send()
                .await?;
            Ok(resp.json().await?)
        } else {
            Ok(resp.json().await?)
        }
    }

    pub async fn is_authenticated(&self) -> bool {
        self.access_token.read().await.is_some()
    }
}

#[async_trait::async_trait]
impl Platform for TwitchPlatform {
    fn kind(&self) -> PlatformKind {
        PlatformKind::Twitch
    }

    async fn authenticate(&mut self) -> Result<()> {
        if self.load_stored_tokens().await? {
            tracing::info!("Twitch: authenticated from stored tokens");
            return Ok(());
        }
        self.device_code_flow().await
    }

    async fn fetch_followed_channels(&self) -> Result<Vec<ChannelEntry>> {
        let user_id = self.user_id.read().await.clone();
        let Some(user_id) = user_id else {
            bail!("User ID not available - not authenticated");
        };

        let mut channels = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let mut url = format!(
                "{TWITCH_API_URL}/channels/followed?user_id={user_id}&first=100"
            );
            if let Some(ref c) = cursor {
                url.push_str(&format!("&after={c}"));
            }

            let resp: FollowedResponse = self.api_get(&url).await?;

            for ch in resp.data {
                channels.push(ChannelEntry {
                    id: ch.broadcaster_id,
                    platform: PlatformKind::Twitch,
                    name: ch.broadcaster_login,
                    display_name: ch.broadcaster_name,
                    is_live: false,
                    stream_title: None,
                    game_or_category: None,
                    viewer_count: None,
                    started_at: None,
                    thumbnail_url: None,
                    auto_record: false,
                });
            }

            match resp.pagination.cursor {
                Some(c) if !c.is_empty() => cursor = Some(c),
                _ => break,
            }
        }

        Ok(channels)
    }

    async fn check_live_status(&self, channel_ids: &[String]) -> Result<Vec<ChannelEntry>> {
        let mut live_channels = Vec::new();

        // Twitch allows up to 100 user_id params per request
        for chunk in channel_ids.chunks(100) {
            let params: String = chunk
                .iter()
                .map(|id| format!("user_id={id}"))
                .collect::<Vec<_>>()
                .join("&");

            let url = format!("{TWITCH_API_URL}/streams?{params}");
            let resp: StreamsResponse = self.api_get(&url).await?;

            for stream in resp.data {
                let started_at = stream.started_at.as_deref().and_then(|s| {
                    chrono::DateTime::parse_from_rfc3339(s)
                        .ok()
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                });

                let thumbnail = stream.thumbnail_url.map(|url| {
                    url.replace("{width}", "440").replace("{height}", "248")
                });

                live_channels.push(ChannelEntry {
                    id: stream.user_id,
                    platform: PlatformKind::Twitch,
                    name: stream.user_login,
                    display_name: stream.user_name,
                    is_live: stream
                        .stream_type
                        .as_deref()
                        .map_or(true, |t| t == "live"),
                    stream_title: stream.title,
                    game_or_category: stream.game_name,
                    viewer_count: stream.viewer_count,
                    started_at,
                    thumbnail_url: thumbnail,
                    auto_record: false,
                });
            }
        }

        Ok(live_channels)
    }

    async fn refresh_token(&mut self) -> Result<()> {
        self.do_refresh_token().await
    }
}
