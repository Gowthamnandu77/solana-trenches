//! Explicit read-only JSON-RPC transport: no hidden SDK 429 retry loops.
use serde_json::{json, Value};
use std::{future::Future, time::Duration};

pub struct Transaction {
    pub json: Value,
    pub block_time: Option<i64>,
    pub slot: u64,
}
#[derive(Debug, Clone, Copy)]
pub enum FetchError {
    Transient,
    RateLimited(Duration),
    Permanent,
}
pub trait Transport: Send + Sync {
    fn fetch(
        &self,
        signature: &str,
    ) -> impl Future<Output = Result<Transaction, FetchError>> + Send;
}
pub struct HttpTransport {
    client: reqwest::Client,
    url: String,
}
impl HttpTransport {
    pub fn new(url: String) -> Result<Self, reqwest::Error> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            url,
        })
    }
}
impl Transport for HttpTransport {
    async fn fetch(&self, signature: &str) -> Result<Transaction, FetchError> {
        let response = self.client.post(&self.url).json(&json!({
            "jsonrpc":"2.0", "id":1, "method":"getTransaction",
            "params":[signature, {"encoding":"jsonParsed", "commitment":"confirmed", "maxSupportedTransactionVersion":0}]
        })).send().await.map_err(|_| FetchError::Transient)?;
        let status = response.status();
        if status.as_u16() == 429 {
            return Err(FetchError::RateLimited(retry_after(
                response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok()),
            )));
        }
        if status.is_server_error() || status.as_u16() == 408 {
            return Err(FetchError::Transient);
        }
        if !status.is_success() {
            return Err(FetchError::Permanent);
        }
        parse_response(response.json().await.map_err(|_| FetchError::Transient)?)
    }
}
fn retry_after(value: Option<&str>) -> Duration {
    let seconds = value
        .and_then(|v| {
            v.parse::<u64>().ok().or_else(|| {
                chrono::DateTime::parse_from_rfc2822(v)
                    .ok()
                    .map(|d| (d.timestamp() - chrono::Utc::now().timestamp()).max(0) as u64)
            })
        })
        .unwrap_or(2)
        .clamp(1, 120);
    Duration::from_secs(seconds)
}
fn parse_response(value: Value) -> Result<Transaction, FetchError> {
    if let Some(error) = value.get("error") {
        // Providers also sometimes expose throttling as a JSON-RPC code/message.
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        if code == 429 || message.contains("rate limit") || message.contains("too many requests") {
            return Err(FetchError::RateLimited(Duration::from_secs(2)));
        }
        return Err(if matches!(code, -32700 | -32602..=-32600 | -32015) {
            FetchError::Permanent
        } else {
            FetchError::Transient
        });
    }
    let result = value
        .get("result")
        .filter(|v| !v.is_null())
        .ok_or(FetchError::Transient)?;
    let slot = result
        .get("slot")
        .and_then(Value::as_u64)
        .ok_or(FetchError::Permanent)?;
    if result.pointer("/meta/err") != Some(&Value::Null) {
        return Err(FetchError::Permanent);
    }
    if !result
        .pointer("/transaction/message")
        .is_some_and(Value::is_object)
    {
        return Err(FetchError::Permanent);
    }
    Ok(Transaction {
        json: result.clone(),
        slot,
        block_time: result.get("blockTime").and_then(Value::as_i64),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn null_failed_and_throttled_responses() {
        assert!(matches!(
            parse_response(json!({"result":null})),
            Err(FetchError::Transient)
        ));
        assert!(matches!(
            parse_response(json!({"error":{"code":429}})),
            Err(FetchError::RateLimited(_))
        ));
        assert!(matches!(
            parse_response(json!({"result":{"slot":1,"meta":{"err":"failed"}}})),
            Err(FetchError::Permanent)
        ));
        assert_eq!(retry_after(Some("9999")), Duration::from_secs(120));
        assert_eq!(retry_after(Some("invalid")), Duration::from_secs(2));
    }
}
