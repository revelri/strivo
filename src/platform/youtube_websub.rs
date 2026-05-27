//! YouTube WebSub (PubSubHubbub) subscriber — pushes near-real-time "new
//! video / live broadcast" notifications from Google's hub to StriVo so the
//! PVR reacts within seconds instead of waiting for the next poll.
//!
//! Design: the daemon subscribes each followed channel's upload feed to
//! Google's hub, naming `strivo serve`'s public `/yt-websub` callback as the
//! delivery target. When the hub POSTs a notification there, the web handler
//! sends `ClientMessage::PollNow` over IPC, which fires the monitor's existing
//! batched live check (RSS candidate gather + one `videos.list`). So WebSub
//! only kills latency — confirmation and recording reuse the proven poll path.
//!
//! Leases expire (hub caps them at ~10 days); the subscriber renews well
//! before expiry. If the hub or `strivo serve` is unreachable a renewal just
//! fails and is retried next cycle — ordinary polling backstops throughout.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::platform::youtube::YouTubePlatform;

const HUB_URL: &str = "https://pubsubhubbub.appspot.com/subscribe";
/// Requested lease. Google's hub honors up to ~10 days; we ask for ~9 and
/// renew at two-thirds of that so a subscription never silently lapses.
const LEASE_SECONDS: u64 = 777_600; // 9 days
const RENEW_AFTER: Duration = Duration::from_secs(LEASE_SECONDS * 2 / 3);
/// On a failed pass (e.g. transient quota 403 while listing channels) retry
/// soon rather than waiting out the full renew interval.
const RETRY_AFTER: Duration = Duration::from_secs(30 * 60);

pub struct WebSubClient {
    pub callback_url: String,
    pub youtube: Arc<RwLock<YouTubePlatform>>,
    pub cancel: CancellationToken,
}

impl WebSubClient {
    pub async fn run(self) {
        // Wait for YouTube auth so the followed-channel list is populated.
        loop {
            if self.cancel.is_cancelled() {
                return;
            }
            if self.youtube.read().await.is_authenticated().await {
                break;
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(3)) => {}
                _ = self.cancel.cancelled() => return,
            }
        }

        loop {
            // Renew on success; retry sooner if the pass failed or subscribed
            // nothing (e.g. the channel-list fetch hit a transient quota 403).
            let wait = match self.subscribe_all().await {
                Ok(n) if n > 0 => {
                    tracing::info!(
                        "youtube websub: subscribed {n} channel feeds (lease ~{}d) -> {}",
                        LEASE_SECONDS / 86_400,
                        self.callback_url
                    );
                    RENEW_AFTER
                }
                Ok(_) => {
                    tracing::warn!("youtube websub: no channels subscribed; retrying soon");
                    RETRY_AFTER
                }
                Err(e) => {
                    tracing::warn!("youtube websub: subscribe pass failed: {e:#}");
                    RETRY_AFTER
                }
            };
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = self.cancel.cancelled() => return,
            }
        }
    }

    async fn subscribe_all(&self) -> anyhow::Result<usize> {
        use crate::platform::Platform;
        let channels = self.youtube.read().await.fetch_followed_channels().await?;
        let client = reqwest::Client::new();
        let (mut ok, mut fail) = (0usize, 0u32);
        for ch in &channels {
            let topic =
                format!("https://www.youtube.com/xml/feeds/videos.xml?channel_id={}", ch.id);
            let form = [
                ("hub.callback", self.callback_url.as_str()),
                ("hub.topic", topic.as_str()),
                ("hub.mode", "subscribe"),
                ("hub.verify", "async"),
                ("hub.lease_seconds", &LEASE_SECONDS.to_string()),
            ];
            match client.post(HUB_URL).form(&form).send().await {
                // Hub accepts async subscriptions with 202; it then GETs our
                // callback to verify before delivery begins.
                Ok(r) if r.status().is_success() => ok += 1,
                Ok(r) => {
                    fail += 1;
                    if fail <= 3 {
                        tracing::warn!(
                            "youtube websub: subscribe {} -> HTTP {}",
                            ch.id,
                            r.status()
                        );
                    }
                }
                Err(e) => {
                    fail += 1;
                    if fail <= 3 {
                        tracing::warn!("youtube websub: subscribe {} failed: {e}", ch.id);
                    }
                }
            }
            // Be polite to the shared public hub.
            tokio::time::sleep(Duration::from_millis(80)).await;
        }
        if fail > 0 {
            tracing::warn!("youtube websub: {fail} channel subscriptions failed this pass");
        }
        Ok(ok)
    }
}
