use anyhow::Result;

use crate::config::CrunchrAnalysisConfig;

/// Result of LLM analysis on a video's transcript.
#[derive(Debug, Clone)]
pub struct AnalysisResult {
    pub summary: String,
    pub topics: Vec<String>,
    pub sentiment: String,
}

/// Analyze a transcript using OpenRouter's LLM API.
pub async fn analyze_transcript(
    config: &CrunchrAnalysisConfig,
    channel_name: &str,
    title: &str,
    transcript: &str,
) -> Result<AnalysisResult> {
    let api_key = config
        .openrouter_api_key_env
        .as_deref()
        .and_then(|env_name| std::env::var(env_name).ok())
        .ok_or_else(|| anyhow::anyhow!("OpenRouter API key not configured"))?;

    // Truncate transcript if very long (limit to ~6000 words / ~8K tokens)
    let truncated: String = transcript
        .split_whitespace()
        .take(6000)
        .collect::<Vec<_>>()
        .join(" ");

    let prompt = format!(
        r#"Analyze this transcript from a live stream recording.

Channel: {channel_name}
Title: {title}

Transcript:
{truncated}

Respond in this exact JSON format (no markdown, just raw JSON):
{{
  "summary": "2-3 sentence summary of the content",
  "topics": ["topic1", "topic2", "topic3"],
  "sentiment": "positive|negative|neutral|mixed"
}}"#
    );

    let request_body = serde_json::json!({
        "model": config.model,
        "messages": [
            {
                "role": "user",
                "content": prompt,
            }
        ],
        "temperature": 0.3,
        "max_tokens": 500,
    });

    let client = reqwest::Client::new();
    let response = client
        .post("https://openrouter.ai/api/v1/chat/completions")
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .header("HTTP-Referer", "https://github.com/strivo")
        .header("X-Title", "StriVo CrunchR")
        .json(&request_body)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "unknown error".to_string());
        anyhow::bail!("OpenRouter API returned {status}: {}", body.chars().take(300).collect::<String>());
    }

    let parsed: serde_json::Value = response.json().await?;

    let content = parsed["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("{}");

    // Parse the LLM's JSON response
    let analysis: serde_json::Value = serde_json::from_str(content)
        .unwrap_or_else(|_| {
            // Fallback: try to extract from markdown code blocks
            let cleaned = content
                .trim()
                .strip_prefix("```json")
                .unwrap_or(content)
                .strip_prefix("```")
                .unwrap_or(content)
                .strip_suffix("```")
                .unwrap_or(content)
                .trim();
            serde_json::from_str(cleaned).unwrap_or_default()
        });

    Ok(AnalysisResult {
        summary: analysis["summary"]
            .as_str()
            .unwrap_or("Analysis unavailable")
            .to_string(),
        topics: analysis["topics"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        sentiment: analysis["sentiment"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
    })
}
