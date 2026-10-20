//! LLM refinement via an OpenAI-compatible chat completions API.

use crate::config::LlmConfig;
use log::{error, info, warn};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(20);

/// Ultra-conservative system prompt: fix only obvious speech-recognition errors.
fn system_prompt() -> &'static str {
    r#"你是一个语音识别结果校对器。规则（必须严格遵守）：
1. 只修正明显的语音识别错误，例如中文同音字错误，以及被误听成中文的技术名词（如：配森→Python、杰森→JSON、派森→Python、瑞斯特→Rust）。
2. 绝不改写、润色、扩写、缩写、翻译或删除任何内容。
3. 如果输入看起来基本正确，原样返回输入，一个字都不要动。
4. 只输出修正后的文本，不要任何解释、引号或多余内容。"#
}

/// Join base_url with an endpoint path, tolerating a trailing slash.
fn join_url(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    // If the user already included /chat/completions, don't append again.
    if base.ends_with(path) {
        return base.to_string();
    }
    format!("{base}{path}")
}

fn build_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| format!("HTTP client: {e}"))
}

#[derive(serde::Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(serde::Deserialize)]
struct ChatChoice {
    message: ResponseMessage,
}

#[derive(serde::Deserialize)]
struct ResponseMessage {
    #[serde(default)]
    content: String,
}

/// One-off chat call used by the Settings "Test" button.
pub fn test(cfg: &LlmConfig) -> Result<String, String> {
    let url = join_url(&cfg.base_url, "/chat/completions");
    let payload = serde_json::json!({
        "model": cfg.model,
        "messages": [
            {"role": "system", "content": "You are a connectivity test."},
            {"role": "user", "content": "Reply with exactly: pong"}
        ],
        "temperature": 0.0,
        // glm-5.3-flash emits reasoning_content before the final content; a
        // small budget gets consumed by the chain-of-thought and truncates the
        // answer, so the allowance must cover both.
        "max_tokens": 2048,
        "stream": false
    });

    let client = build_client()?;
    let resp = client
        .post(&url)
        .bearer_auth(&cfg.api_key)
        .json(&payload)
        .send()
        .map_err(|e| format!("请求失败: {e}"))?;

    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if !status.is_success() {
        return Err(format!("HTTP {status}: {}", truncate(&text, 200)));
    }
    let parsed: ChatResponse = serde_json::from_str(&text)
        .map_err(|e| format!("解析响应失败: {e}: {}", truncate(&text, 200)))?;
    parsed
        .choices
        .first()
        .map(|c| c.message.content.trim().to_string())
        .ok_or_else(|| "响应中没有 choices".to_string())
}

/// Refine raw ASR text. On any failure, returns the raw text unchanged.
pub fn refine(cfg: &LlmConfig, raw: &str) -> String {
    if raw.trim().is_empty() {
        return raw.to_string();
    }
    match refine_inner(cfg, raw) {
        Ok(t) => {
            info!("LLM refinement ok");
            t
        }
        Err(e) => {
            warn!("LLM refinement failed, falling back to raw text: {e}");
            raw.to_string()
        }
    }
}

fn refine_inner(cfg: &LlmConfig, raw: &str) -> Result<String, String> {
    if cfg.api_key.trim().is_empty() {
        return Err("api_key 未配置".into());
    }
    let url = join_url(&cfg.base_url, "/chat/completions");
    let payload = serde_json::json!({
        "model": cfg.model,
        "messages": [
            {"role": "system", "content": system_prompt()},
            {"role": "user", "content": raw}
        ],
        "temperature": 0.0,
        // glm-5.3-flash emits reasoning_content before the final content; a
        // small budget gets consumed by the chain-of-thought and truncates the
        // answer, so the allowance must cover both.
        "max_tokens": 2048,
        "stream": false
    });

    let client = build_client()?;
    let resp = client
        .post(&url)
        .bearer_auth(&cfg.api_key)
        .json(&payload)
        .send()
        .map_err(|e| format!("请求失败: {e}"))?;

    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if !status.is_success() {
        return Err(format!("HTTP {status}: {}", truncate(&text, 200)));
    }
    let parsed: ChatResponse = serde_json::from_str(&text)
        .map_err(|e| format!("解析响应失败: {e}"))?;
    let content = parsed
        .choices
        .first()
        .map(|c| c.message.content.clone())
        .ok_or_else(|| "响应中没有 choices".to_string())?;
    if content.trim().is_empty() {
        error!("LLM returned empty content");
        return Err("LLM 返回为空".into());
    }
    Ok(content.trim().to_string())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n).collect();
        format!("{t}…")
    }
}
