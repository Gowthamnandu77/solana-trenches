//! Offline, fail-closed LaunchLab transaction-export price derivation.

use crate::{normalize_observations, timestamp, PriceObservation};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

const LAUNCHLAB_PROGRAM: &str = "LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj";
const SWAP_DISCRIMINATORS: [[u8; 8]; 4] = [
    [250, 234, 13, 123, 213, 156, 19, 236],
    [24, 211, 116, 40, 105, 3, 153, 56],
    [149, 39, 222, 155, 211, 124, 152, 26],
    [95, 200, 71, 34, 8, 9, 11, 166],
];

#[derive(Debug, Default, Clone, Serialize)]
pub struct DerivationSummary {
    pub transactions_read: usize,
    pub relevant_transactions: usize,
    pub observations_produced: usize,
    pub rejected_malformed: usize,
    pub rejected_ambiguous: usize,
    pub rejected_unknown_target: usize,
    pub unique_observations: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reject {
    Malformed,
    Ambiguous,
    UnknownTarget,
    Irrelevant,
}

pub fn derive_prices(
    features: &[Value],
    transactions: &[Value],
) -> (Vec<PriceObservation>, DerivationSummary) {
    let targets = targets(features);
    let mut summary = DerivationSummary {
        transactions_read: transactions.len(),
        ..Default::default()
    };
    let mut observations = Vec::new();
    for transaction in transactions {
        match derive_one(&targets, transaction) {
            Ok(Some(observation)) => {
                summary.relevant_transactions += 1;
                observations.push(observation);
            }
            Ok(None) | Err(Reject::Irrelevant) => {}
            Err(Reject::Malformed) => summary.rejected_malformed += 1,
            Err(Reject::Ambiguous) => summary.rejected_ambiguous += 1,
            Err(Reject::UnknownTarget) => summary.rejected_unknown_target += 1,
        }
    }
    summary.observations_produced = observations.len();
    let observations = normalize_observations(observations);
    summary.unique_observations = observations.len();
    (observations, summary)
}

type Target = (String, String, String, i64);

fn targets(features: &[Value]) -> BTreeMap<String, Target> {
    features
        .iter()
        .filter_map(|feature| {
            Some((
                feature["launch_account"].as_str()?.to_owned(),
                (
                    feature["protocol"].as_str()?.to_owned(),
                    feature["base_mint"].as_str()?.to_owned(),
                    feature["quote_mint"].as_str()?.to_owned(),
                    timestamp(feature)?,
                ),
            ))
        })
        .collect()
}

fn derive_one(
    targets: &BTreeMap<String, Target>,
    tx: &Value,
) -> Result<Option<PriceObservation>, Reject> {
    if !tx["meta"]["err"].is_null() {
        return Err(Reject::Irrelevant);
    }
    let slot = tx["slot"].as_u64().ok_or(Reject::Malformed)?;
    let timestamp_unix = tx["blockTime"].as_i64().ok_or(Reject::Malformed)?;
    let source_tx_signature = tx
        .pointer("/transaction/signatures/0")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let account_keys = account_keys(tx).ok_or(Reject::Malformed)?;
    let swaps: Vec<_> = instructions(tx)
        .filter_map(|instruction| launchlab_swap(instruction, &account_keys))
        .collect();
    if swaps.is_empty() {
        return Ok(None);
    }
    if swaps.len() != 1 {
        return Err(Reject::Ambiguous);
    }
    let launch_account = &swaps[0];
    let Some((protocol, base_mint, quote_mint, _)) = targets.get(launch_account) else {
        return Err(Reject::UnknownTarget);
    };
    if protocol != "raydium_launchlab" {
        return Err(Reject::UnknownTarget);
    }
    let base = exchanged_amount(tx, base_mint)?;
    let quote = exchanged_amount(tx, quote_mint)?;
    let price_quote_per_base = quote / base;
    if !price_quote_per_base.is_finite() || price_quote_per_base <= 0.0 {
        return Err(Reject::Malformed);
    }
    Ok(Some(PriceObservation {
        protocol: protocol.clone(),
        launch_account: launch_account.clone(),
        base_mint: Some(base_mint.clone()),
        quote_mint: Some(quote_mint.clone()),
        timestamp_unix,
        slot: Some(slot),
        price_quote_per_base,
        source: "solana_transaction_export".into(),
        source_quality: "verified_launchlab_balance_deltas".into(),
        source_tx_signature,
        observed: true,
        derived_from_swaps: true,
    }))
}

fn account_keys(tx: &Value) -> Option<Vec<String>> {
    tx.pointer("/transaction/message/accountKeys")?
        .as_array()?
        .iter()
        .map(|key| {
            key.as_str()
                .or_else(|| key["pubkey"].as_str())
                .map(str::to_owned)
        })
        .collect()
}

fn instructions(tx: &Value) -> impl Iterator<Item = &Value> {
    tx.pointer("/transaction/message/instructions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .chain(
            tx.pointer("/meta/innerInstructions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .flat_map(|group| group["instructions"].as_array().into_iter().flatten()),
        )
}

fn launchlab_swap(instruction: &Value, account_keys: &[String]) -> Option<String> {
    let program = instruction["programId"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            instruction["programIdIndex"]
                .as_u64()
                .and_then(|index| account_keys.get(index as usize).cloned())
        })?;
    if program != LAUNCHLAB_PROGRAM {
        return None;
    }
    let data = bs58::decode(instruction["data"].as_str()?)
        .into_vec()
        .ok()?;
    if data.len() < 8 || !SWAP_DISCRIMINATORS.iter().any(|disc| data[..8] == *disc) {
        return None;
    }
    let accounts = instruction["accounts"].as_array()?;
    let target = accounts.get(4)?;
    target.as_str().map(str::to_owned).or_else(|| {
        target
            .as_u64()
            .and_then(|index| account_keys.get(index as usize).cloned())
    })
}

fn exchanged_amount(tx: &Value, mint: &str) -> Result<f64, Reject> {
    let pre = balances(tx, "/meta/preTokenBalances", mint)?;
    let post = balances(tx, "/meta/postTokenBalances", mint)?;
    if pre.len() != 2 || post.len() != 2 {
        return Err(Reject::Ambiguous);
    }
    let mut deltas = Vec::new();
    for (index, (before, decimals)) in pre {
        let Some((after, post_decimals)) = post.get(&index) else {
            return Err(Reject::Ambiguous);
        };
        if decimals != *post_decimals {
            return Err(Reject::Malformed);
        }
        deltas.push((after - before, decimals));
    }
    if deltas.len() != 2 || deltas[0].1 != deltas[1].1 {
        return Err(Reject::Ambiguous);
    }
    let positive: i128 = deltas
        .iter()
        .filter(|(delta, _)| *delta > 0)
        .map(|(delta, _)| *delta)
        .sum();
    let negative: i128 = deltas
        .iter()
        .filter(|(delta, _)| *delta < 0)
        .map(|(delta, _)| -*delta)
        .sum();
    if positive == 0 || positive != negative {
        return Err(Reject::Ambiguous);
    }
    Ok(positive as f64 / 10_f64.powi(deltas[0].1 as i32))
}

fn balances(tx: &Value, path: &str, mint: &str) -> Result<BTreeMap<u64, (i128, u8)>, Reject> {
    let rows = tx
        .pointer(path)
        .and_then(Value::as_array)
        .ok_or(Reject::Malformed)?;
    rows.iter()
        .filter(|row| row["mint"].as_str() == Some(mint))
        .map(|row| {
            let index = row["accountIndex"].as_u64().ok_or(Reject::Malformed)?;
            let amount = row["uiTokenAmount"]["amount"]
                .as_str()
                .ok_or(Reject::Malformed)?
                .parse()
                .map_err(|_| Reject::Malformed)?;
            let decimals = row["uiTokenAmount"]["decimals"]
                .as_u64()
                .and_then(|value| u8::try_from(value).ok())
                .ok_or(Reject::Malformed)?;
            Ok((index, (amount, decimals)))
        })
        .collect()
}
