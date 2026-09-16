//! Offline, fail-closed LaunchLab transaction-export price derivation.

use crate::{normalize_observations, timestamp, PriceObservation};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const LAUNCHLAB_PROGRAM: &str = "LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj";
const BUY_EXACT_IN: [u8; 8] = [250, 234, 13, 123, 213, 156, 19, 236];
const SELL_EXACT_IN: [u8; 8] = [149, 39, 222, 155, 211, 124, 152, 26];
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
    let launch_account = &swaps[0].launch_account;
    let Some((protocol, base_mint, quote_mint, _creation_time)) = targets.get(launch_account)
    else {
        return Err(Reject::UnknownTarget);
    };
    if protocol != "raydium_launchlab" {
        return Err(Reject::UnknownTarget);
    }
    let payer = account_keys.first().ok_or(Reject::Malformed)?;
    let (base, quote, source_quality) =
        match transfer_price(tx, &swaps[0], base_mint, quote_mint, &account_keys)? {
            Some((base, quote)) => (
                base,
                quote,
                "verified_launchlab_instruction_scoped_token_transfers",
            ),
            None => {
                let base = exchanged_amount(tx, base_mint, &swaps[0].account_indices, payer)?;
                let quote = exchanged_amount(tx, quote_mint, &swaps[0].account_indices, payer)?;
                (
                    base,
                    quote,
                    "verified_launchlab_instruction_scoped_balance_deltas",
                )
            }
        };
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
        source_quality: source_quality.into(),
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

struct SwapReference {
    launch_account: String,
    account_indices: BTreeSet<u64>,
    discriminator: [u8; 8],
}

type Balance = (i128, u8, Option<String>);

fn launchlab_swap(instruction: &Value, account_keys: &[String]) -> Option<SwapReference> {
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
    let launch_account = target.as_str().map(str::to_owned).or_else(|| {
        target
            .as_u64()
            .and_then(|index| account_keys.get(index as usize).cloned())
    })?;
    Some(SwapReference {
        launch_account,
        account_indices: accounts
            .iter()
            .filter_map(|account| {
                account.as_u64().or_else(|| {
                    account
                        .as_str()
                        .and_then(|key| account_keys.iter().position(|candidate| candidate == key))
                        .map(|index| index as u64)
                })
            })
            .collect(),
        discriminator: data[..8].try_into().ok()?,
    })
}

fn transfer_price(
    tx: &Value,
    swap: &SwapReference,
    base_mint: &str,
    quote_mint: &str,
    account_keys: &[String],
) -> Result<Option<(f64, f64)>, Reject> {
    let input_mint = if swap.discriminator == BUY_EXACT_IN {
        quote_mint
    } else if swap.discriminator == SELL_EXACT_IN {
        base_mint
    } else {
        return Err(Reject::Ambiguous);
    };
    let base = transfer_amounts(tx, base_mint, &swap.account_indices, account_keys)?;
    let quote = transfer_amounts(tx, quote_mint, &swap.account_indices, account_keys)?;
    if base.is_empty() && quote.is_empty() {
        return Ok(None);
    }
    let data = instructions(tx)
        .find_map(|instruction| {
            launchlab_instruction_data(instruction, swap.discriminator, account_keys)
        })
        .ok_or(Reject::Malformed)?;
    let input_raw = u64::from_le_bytes(
        data.get(8..16)
            .ok_or(Reject::Malformed)?
            .try_into()
            .map_err(|_| Reject::Malformed)?,
    ) as u128;
    if input_raw == 0 {
        return Err(Reject::Ambiguous);
    }
    let input = if input_mint == base_mint {
        &base
    } else {
        &quote
    };
    let output = if input_mint == base_mint {
        &quote
    } else {
        &base
    };
    let input_matches: Vec<_> = input
        .iter()
        .filter(|(amount, _)| *amount == input_raw)
        .collect();
    if input_matches.is_empty() {
        return Err(Reject::Ambiguous);
    }
    let output_amount = unique_repeated_amount(output)?;
    let input_decimals = input_matches
        .iter()
        .map(|(_, decimals)| *decimals)
        .collect::<BTreeSet<_>>();
    if input_decimals.len() != 1 {
        return Err(Reject::Malformed);
    }
    let input_value = input_raw as f64 / 10_f64.powi(*input_decimals.iter().next().unwrap() as i32);
    let output_value = output_amount.0 as f64 / 10_f64.powi(output_amount.1 as i32);
    if input_value <= 0.0
        || output_value <= 0.0
        || !input_value.is_finite()
        || !output_value.is_finite()
    {
        return Err(Reject::Malformed);
    }
    if input_mint == base_mint {
        Ok(Some((input_value, output_value)))
    } else {
        Ok(Some((output_value, input_value)))
    }
}

fn launchlab_instruction_data(
    instruction: &Value,
    discriminator: [u8; 8],
    account_keys: &[String],
) -> Option<Vec<u8>> {
    let program = instruction["programId"].as_str().or_else(|| {
        instruction["programIdIndex"]
            .as_u64()
            .and_then(|index| account_keys.get(index as usize).map(String::as_str))
    })?;
    if program != LAUNCHLAB_PROGRAM {
        return None;
    }
    let data = bs58::decode(instruction["data"].as_str()?)
        .into_vec()
        .ok()?;
    (data.get(..8) == Some(discriminator.as_slice())).then_some(data)
}

fn transfer_amounts(
    tx: &Value,
    mint: &str,
    account_indices: &BTreeSet<u64>,
    account_keys: &[String],
) -> Result<Vec<(u128, u8)>, Reject> {
    let mut amounts = Vec::new();
    for instruction in instructions(tx) {
        if instruction["program"] != "spl-token" {
            continue;
        }
        let Some(info) = instruction["parsed"]["info"].as_object() else {
            continue;
        };
        if instruction["parsed"]["type"].as_str() != Some("transferChecked")
            || info.get("mint").and_then(Value::as_str) != Some(mint)
        {
            continue;
        }
        let source = info["source"].as_str().and_then(|value| {
            account_keys
                .iter()
                .position(|key| key == value)
                .map(|index| index as u64)
        });
        let destination = info["destination"].as_str().and_then(|value| {
            account_keys
                .iter()
                .position(|key| key == value)
                .map(|index| index as u64)
        });
        if !source.is_some_and(|index| account_indices.contains(&index))
            && !destination.is_some_and(|index| account_indices.contains(&index))
        {
            continue;
        }
        let amount = info["tokenAmount"]["amount"]
            .as_str()
            .ok_or(Reject::Malformed)?
            .parse()
            .map_err(|_| Reject::Malformed)?;
        let decimals = info["tokenAmount"]["decimals"]
            .as_u64()
            .and_then(|value| u8::try_from(value).ok())
            .ok_or(Reject::Malformed)?;
        if amount == 0 {
            return Err(Reject::Ambiguous);
        }
        amounts.push((amount, decimals));
    }
    Ok(amounts)
}

fn unique_repeated_amount(amounts: &[(u128, u8)]) -> Result<(u128, u8), Reject> {
    let mut counts = BTreeMap::new();
    for amount in amounts {
        *counts.entry(*amount).or_insert(0usize) += 1;
    }
    let Some((&best, &count)) = counts.iter().max_by_key(|(_, count)| **count) else {
        return Err(Reject::Ambiguous);
    };
    if counts.values().filter(|value| **value == count).count() != 1 {
        return Err(Reject::Ambiguous);
    }
    Ok(best)
}

fn exchanged_amount(
    tx: &Value,
    mint: &str,
    account_indices: &BTreeSet<u64>,
    payer: &str,
) -> Result<f64, Reject> {
    let pre = balances(tx, "/meta/preTokenBalances", mint, account_indices)?;
    let post = balances(tx, "/meta/postTokenBalances", mint, account_indices)?;
    let indices: BTreeSet<_> = pre.keys().chain(post.keys()).copied().collect();
    if indices.len() < 2 {
        return Err(Reject::Ambiguous);
    }
    let user_indices: Vec<_> = indices
        .iter()
        .filter(|index| {
            pre.get(index).and_then(|(_, _, owner)| owner.as_deref()) == Some(payer)
                || post.get(index).and_then(|(_, _, owner)| owner.as_deref()) == Some(payer)
        })
        .collect();
    if user_indices.len() != 1 {
        return Err(Reject::Ambiguous);
    }
    let index = *user_indices[0];
    let (before, decimals, pre_owner) = match pre.get(&index) {
        Some((amount, decimals, owner)) => (*amount, *decimals, owner.clone()),
        None => {
            let Some((_, decimals, _)) = post.get(&index) else {
                return Err(Reject::Malformed);
            };
            (0, *decimals, None)
        }
    };
    let (after, post_decimals, post_owner) = match post.get(&index) {
        Some((amount, decimals, owner)) => (*amount, *decimals, owner.clone()),
        None => (0, decimals, None),
    };
    if decimals != post_decimals || pre_owner.as_deref().or(post_owner.as_deref()) != Some(payer) {
        return Err(Reject::Malformed);
    }
    let delta = after - before;
    if delta == 0 {
        return Err(Reject::Ambiguous);
    }
    Ok(delta.unsigned_abs() as f64 / 10_f64.powi(decimals as i32))
}

fn balances(
    tx: &Value,
    path: &str,
    mint: &str,
    account_indices: &BTreeSet<u64>,
) -> Result<BTreeMap<u64, Balance>, Reject> {
    let rows = tx
        .pointer(path)
        .and_then(Value::as_array)
        .ok_or(Reject::Malformed)?;
    rows.iter()
        .filter(|row| {
            row["mint"].as_str() == Some(mint)
                && row["accountIndex"]
                    .as_u64()
                    .is_some_and(|index| account_indices.contains(&index))
        })
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
            Ok((
                index,
                (amount, decimals, row["owner"].as_str().map(str::to_owned)),
            ))
        })
        .collect()
}
