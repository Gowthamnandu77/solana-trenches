//! Bounded, read-only archive-RPC transaction export for offline derivation.

use serde::Serialize;
use serde_json::{json, Value};
use std::{collections::BTreeSet, env, fs, io, path::Path, time::Duration};

#[derive(Debug, Clone, Copy)]
pub struct FetchOptions {
    pub max_pages: usize,
    pub page_size: usize,
    pub fetch_transactions: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct FetchSummary {
    pub targets_attempted: usize,
    pub signatures_fetched: usize,
    pub signatures_in_window: usize,
    pub transactions_fetched: usize,
    pub existing_transactions_reused: usize,
    pub signatures_without_block_time: usize,
    pub transactions_unavailable: usize,
    pub rpc_failures: usize,
    pub reached_creation_boundary: bool,
    pub window_complete: bool,
}

pub fn configured_rpc_url() -> Result<String, String> {
    dotenvy::from_path("ingestion/.env")
        .map_err(|_| "could not load configured ingestion/.env RPC settings".to_owned())?;
    let url = env::var("SOLANA_HTTP_URL")
        .map_err(|_| "SOLANA_HTTP_URL is not configured in ingestion/.env".to_owned())?;
    if url.trim().is_empty() {
        return Err("SOLANA_HTTP_URL is empty".into());
    }
    Ok(url)
}

pub async fn fetch_target(
    rpc_url: String,
    launch_account: &str,
    creation_time: i64,
    output: &Path,
    options: FetchOptions,
) -> Result<FetchSummary, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "could not create read-only RPC client".to_owned())?;
    let end_time = creation_time
        .checked_add(3600)
        .ok_or("invalid creation timestamp")?;
    let existing = existing_signatures(output).map_err(|_| "could not read existing raw export")?;
    let mut summary = FetchSummary {
        targets_attempted: 1,
        ..Default::default()
    };
    let mut before: Option<String> = None;
    let mut selected = Vec::new();
    let mut seen = BTreeSet::new();

    for _ in 0..options.max_pages {
        let mut config = json!({"limit": options.page_size});
        if let Some(cursor) = before.as_deref() {
            config["before"] = Value::String(cursor.to_owned());
        }
        let page = match rpc_call(
            &client,
            &rpc_url,
            "getSignaturesForAddress",
            json!([launch_account, config]),
        )
        .await
        {
            Ok(value) => value,
            Err(()) => {
                summary.rpc_failures += 1;
                return Ok(summary);
            }
        };
        let Some(rows) = page.as_array() else {
            summary.rpc_failures += 1;
            return Ok(summary);
        };
        if rows.is_empty() {
            summary.reached_creation_boundary = true;
            break;
        }
        let mut oldest_time = None;
        for row in rows {
            let Some(signature) = row["signature"].as_str() else {
                continue;
            };
            before = Some(signature.to_owned());
            if !seen.insert(signature.to_owned()) {
                continue;
            }
            summary.signatures_fetched += 1;
            let Some(block_time) = row["blockTime"].as_i64() else {
                summary.signatures_without_block_time += 1;
                continue;
            };
            oldest_time = Some(oldest_time.map_or(block_time, |old: i64| old.min(block_time)));
            if (creation_time..=end_time).contains(&block_time) {
                selected.push(signature.to_owned());
            }
        }
        if oldest_time.is_some_and(|time| time <= creation_time) {
            summary.reached_creation_boundary = true;
            break;
        }
        if rows.len() < options.page_size {
            break;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    summary.window_complete = summary.reached_creation_boundary && summary.rpc_failures == 0;
    summary.signatures_in_window = selected.len();
    if !options.fetch_transactions {
        return Ok(summary);
    }
    let mut lines = Vec::new();
    for signature in selected {
        if existing.contains(&signature) {
            summary.existing_transactions_reused += 1;
            continue;
        }
        match rpc_call(
            &client,
            &rpc_url,
            "getTransaction",
            json!([signature, {"encoding":"jsonParsed", "commitment":"confirmed", "maxSupportedTransactionVersion":0}]),
        )
        .await
        {
            Ok(value) if !value.is_null() => {
                lines.push(serde_json::to_string(&value).map_err(|_| "could not serialize transaction")?);
                summary.transactions_fetched += 1;
            }
            Ok(_) => summary.transactions_unavailable += 1,
            Err(()) => summary.rpc_failures += 1,
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    if !lines.is_empty() {
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|_| "could not create raw export directory")?;
        }
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(output)
            .map_err(|_| "could not write raw export")?;
        for line in lines {
            writeln!(file, "{line}").map_err(|_| "could not write raw export")?;
        }
    }
    Ok(summary)
}

fn existing_signatures(path: &Path) -> io::Result<BTreeSet<String>> {
    let Ok(file) = fs::File::open(path) else {
        return Ok(BTreeSet::new());
    };
    use std::io::BufRead;
    let signatures = io::BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| {
            serde_json::from_str::<Value>(&line)
                .ok()?
                .pointer("/transaction/signatures/0")?
                .as_str()
                .map(str::to_owned)
        })
        .collect::<BTreeSet<_>>();
    Ok(signatures)
}

async fn rpc_call(
    client: &reqwest::Client,
    url: &str,
    method: &str,
    params: Value,
) -> Result<Value, ()> {
    for attempt in 0..3u32 {
        let response = client
            .post(url)
            .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
            .send()
            .await;
        match response {
            Ok(response) if response.status().is_success() => {
                match response.json::<Value>().await {
                    Ok(value) if value.get("error").is_none() => return Ok(value["result"].clone()),
                    _ => {}
                }
            }
            Ok(response) if response.status().as_u16() == 429 => {}
            _ => {}
        }
        tokio::time::sleep(Duration::from_millis(250 * 2u64.pow(attempt))).await;
    }
    Err(())
}
