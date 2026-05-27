//! Twitch EventSub over WebSocket — real-time `stream.online` push so the PVR
//! catches a broadcast start in ~seconds instead of waiting for the next poll.
//!
//! Design: on any `stream.online` notification we fire the monitor's
//! `poll_notify`. The existing batched `/streams` poll then confirms the live
//! state and triggers auto-record, reusing the proven path — so EventSub adds
//! latency-killing detection without duplicating recording logic or risking
//! double-starts. Falls back transparently to plain polling if the socket
//! drops (the monitor keeps polling regardless).

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{Notify, RwLock};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

const WS_URL: &str = "wss://eventsub.wss.twitch.tv/ws";
const SUB_URL: &str = "https://api.twitch.tv/helix/eventsub/subscriptions";

pub struct EventSubClient {
    pub client_id: String,
    /// Shared with the TwitchPlatform so subscription creation always uses the
    /// current (possibly refreshed) user token.
    pub token: Arc<RwLock<Option<String>>>,
    pub channel_ids: Vec<String>,
    pub poll_notify: Arc<Notify>,
    pub cancel: CancellationToken,
}

impl EventSubClient {
    pub async fn run(self) {
        let mut backoff = Duration::from_secs(2);
        loop {
            if self.cancel.is_cancelled() {
                return;
            }
            match self.connect_once().await {
                Ok(()) => return, // cancelled
                Err(e) => {
                    tracing::warn!(
                        "twitch eventsub: {e:#}; reconnecting in {}s",
                        backoff.as_secs()
                    );
                    tokio::select! {
                        _ = tokio::time::sleep(backoff) => {}
                        _ = self.cancel.cancelled() => return,
                    }
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                }
            }
        }
    }

    async fn connect_once(&self) -> anyhow::Result<()> {
        let (ws, _) = tokio_tungstenite::connect_async(WS_URL).await?;
        let (mut write, mut read) = ws.split();
        let mut subscribed = false;
        // Reconnect if no message arrives within keepalive + grace. Twitch
        // sends keepalives every ~10s; the welcome reports the real value.
        let mut keepalive = Duration::from_secs(20);

        loop {
            let next = tokio::select! {
                m = read.next() => m,
                _ = self.cancel.cancelled() => return Ok(()),
                _ = tokio::time::sleep(keepalive + Duration::from_secs(10)) => {
                    anyhow::bail!("keepalive timeout");
                }
            };
            let Some(msg) = next else {
                anyhow::bail!("socket closed");
            };
            match msg? {
                Message::Text(txt) => {
                    let v: Value = match serde_json::from_str(&txt) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    match v["metadata"]["message_type"].as_str().unwrap_or("") {
                        "session_welcome" => {
                            if let Some(k) =
                                v["payload"]["session"]["keepalive_timeout_seconds"].as_u64()
                            {
                                keepalive = Duration::from_secs(k);
                            }
                            if !subscribed {
                                if let Some(sid) = v["payload"]["session"]["id"].as_str() {
                                    self.subscribe_all(sid).await;
                                    subscribed = true;
                                }
                            }
                        }
                        "session_keepalive" => {}
                        "notification"
                            if v["payload"]["subscription"]["type"].as_str()
                                == Some("stream.online") =>
                        {
                            let who = v["payload"]["event"]["broadcaster_user_login"]
                                .as_str()
                                .unwrap_or("?");
                            tracing::info!(
                                "twitch eventsub: {who} went live — triggering immediate poll"
                            );
                            self.poll_notify.notify_one();
                        }
                        "notification" => {}
                        "session_reconnect" => {
                            // Simplest robust path: drop and let run() reconnect
                            // fresh (new session → re-subscribe). Tiny gap is
                            // covered by the monitor's ongoing poll.
                            anyhow::bail!("session_reconnect requested");
                        }
                        "revocation" => {
                            tracing::warn!("twitch eventsub: subscription revoked: {txt}");
                        }
                        _ => {}
                    }
                }
                Message::Ping(p) => {
                    let _ = write.send(Message::Pong(p)).await;
                }
                Message::Close(_) => anyhow::bail!("server closed connection"),
                _ => {}
            }
        }
    }

    async fn subscribe_all(&self, session_id: &str) {
        let Some(token) = self.token.read().await.clone() else {
            tracing::warn!("twitch eventsub: no token; cannot subscribe");
            return;
        };
        let client = reqwest::Client::new();
        let (mut ok, mut fail) = (0u32, 0u32);
        for id in &self.channel_ids {
            let body = json!({
                "type": "stream.online",
                "version": "1",
                "condition": { "broadcaster_user_id": id },
                "transport": { "method": "websocket", "session_id": session_id },
            });
            // Twitch rate-limits subscription creation; pace the calls and
            // retry once on 429 so a large follow list doesn't get dropped.
            let mut attempt = 0;
            loop {
                attempt += 1;
                let resp = client
                    .post(SUB_URL)
                    .header("Authorization", format!("Bearer {token}"))
                    .header("Client-Id", &self.client_id)
                    .json(&body)
                    .send()
                    .await;
                match resp {
                    Ok(r) if r.status().is_success() => {
                        ok += 1;
                        break;
                    }
                    Ok(r) if r.status().as_u16() == 429 && attempt < 4 => {
                        tokio::time::sleep(Duration::from_millis(800 * attempt)).await;
                        continue;
                    }
                    Ok(r) => {
                        fail += 1;
                        if fail <= 3 {
                            tracing::warn!("twitch eventsub: subscribe {id} -> HTTP {}", r.status());
                        }
                        break;
                    }
                    Err(e) => {
                        fail += 1;
                        if fail <= 3 {
                            tracing::warn!("twitch eventsub: subscribe {id} failed: {e}");
                        }
                        break;
                    }
                }
            }
            // Gentle pacing between channels to stay under the create limit.
            tokio::time::sleep(Duration::from_millis(120)).await;
        }
        tracing::info!(
            "twitch eventsub: subscribed stream.online for {ok} channels ({fail} failed)"
        );
    }
}
