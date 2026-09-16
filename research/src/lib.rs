pub mod derive;
pub mod paper;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    cmp::Ordering,
    collections::BTreeMap,
    fs::File,
    io::{self, BufRead, BufReader, Write},
    path::Path,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PriceObservation {
    pub protocol: String,
    pub launch_account: String,
    pub base_mint: Option<String>,
    pub quote_mint: Option<String>,
    pub timestamp_unix: i64,
    pub slot: Option<u64>,
    pub price_quote_per_base: f64,
    pub source: String,
    pub source_quality: String,
    pub observed: bool,
    pub derived_from_swaps: bool,
}

pub type PricePoint = PriceObservation;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Outcome {
    pub return_1m: Option<f64>,
    pub return_5m: Option<f64>,
    pub return_15m: Option<f64>,
    pub return_1h: Option<f64>,
    pub max_return_5m: Option<f64>,
    pub max_return_15m: Option<f64>,
    pub max_drawdown_5m: Option<f64>,
    pub max_drawdown_15m: Option<f64>,
    pub label_available: bool,
    pub price_data_source: Option<String>,
    pub price_source_quality: Option<String>,
    pub derived_from_swaps: bool,
    pub label_status: String,
    pub missing_data_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LabeledLaunch {
    pub protocol: String,
    pub launch_account: String,
    pub base_mint: Option<String>,
    pub quote_mint: Option<String>,
    pub creation_time_unix: Option<i64>,
    pub features: Value,
    pub momentum_score: f64,
    pub quality_flags: Value,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityDecision {
    Accept,
    Reject,
}

pub fn parse_features<R: BufRead>(reader: R) -> io::Result<Vec<Value>> {
    let mut rows = Vec::new();
    for (line_no, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(&line).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("feature line {}: {e}", line_no + 1),
            )
        })?;
        if value["schema_version"].as_u64() != Some(15) {
            continue;
        }
        rows.push(value);
    }
    Ok(rows)
}

pub fn read_jsonl_values<R: BufRead>(reader: R, kind: &str) -> io::Result<Vec<Value>> {
    let mut rows = Vec::new();
    for (line_no, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        rows.push(serde_json::from_str(&line).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{kind} line {}: {e}", line_no + 1),
            )
        })?);
    }
    Ok(rows)
}

pub fn read_features(path: &Path) -> io::Result<Vec<Value>> {
    let rows = parse_features(BufReader::new(File::open(path)?))?;
    let mut unique = BTreeMap::new();
    for row in rows {
        let key = row["launch_account"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let replace = unique
            .get(&key)
            .map(|old: &Value| {
                row["complete_window"].as_bool().unwrap_or(false)
                    && !old["complete_window"].as_bool().unwrap_or(false)
            })
            .unwrap_or(true);
        if replace {
            unique.insert(key, row);
        }
    }
    Ok(unique.into_values().collect())
}

pub fn read_prices<R: BufRead>(reader: R) -> io::Result<Vec<PriceObservation>> {
    Ok(normalize_observations(read_price_csv(reader, false)?))
}

pub fn read_price_export<R: BufRead>(reader: R) -> io::Result<Vec<PriceObservation>> {
    read_price_csv(reader, true)
}

fn read_price_csv<R: BufRead>(
    reader: R,
    normalized_only: bool,
) -> io::Result<Vec<PriceObservation>> {
    let mut points = Vec::new();
    for (line_no, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty()
            || line.starts_with("launch_account,")
            || line.starts_with("protocol,")
        {
            continue;
        }
        let fields: Vec<_> = line.split(',').map(str::trim).collect();
        if fields.len() != 4 && fields.len() != 9 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "price line {} needs 4 legacy or 9 normalized CSV fields",
                    line_no + 1
                ),
            ));
        }
        if normalized_only && fields.len() != 9 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("price line {} needs 9 normalized CSV fields", line_no + 1),
            ));
        }
        let normalized = if fields.len() == 9 {
            PriceObservation {
                protocol: fields[0].into(),
                launch_account: fields[1].into(),
                base_mint: (!fields[2].is_empty()).then(|| fields[2].into()),
                quote_mint: (!fields[3].is_empty()).then(|| fields[3].into()),
                timestamp_unix: fields[4].parse().map_err(invalid_price)?,
                slot: (!fields[5].is_empty())
                    .then(|| fields[5].parse())
                    .transpose()
                    .map_err(invalid_price)?,
                price_quote_per_base: fields[6].parse().map_err(invalid_price)?,
                source: fields[7].into(),
                source_quality: fields[8].into(),
                observed: true,
                derived_from_swaps: fields[8].contains("swap"),
            }
        } else {
            PriceObservation {
                protocol: "unknown".into(),
                launch_account: fields[0].into(),
                base_mint: None,
                quote_mint: None,
                timestamp_unix: fields[1].parse().map_err(invalid_price)?,
                slot: None,
                price_quote_per_base: fields[2].parse().map_err(invalid_price)?,
                source: fields[3].into(),
                source_quality: "unverified_import".into(),
                observed: true,
                derived_from_swaps: false,
            }
        };
        points.push(normalized);
    }
    Ok(points)
}

pub fn validate_price_export(
    features: &[Value],
    observations: Vec<PriceObservation>,
) -> io::Result<Vec<PriceObservation>> {
    let mut targets = BTreeMap::new();
    for feature in features {
        let account = feature["launch_account"].as_str().unwrap_or_default();
        if account.is_empty() {
            continue;
        }
        targets.insert(
            account,
            (
                feature["protocol"].as_str().unwrap_or_default(),
                feature["base_mint"].as_str(),
                feature["quote_mint"].as_str(),
                timestamp(feature),
            ),
        );
    }
    for (row_no, observation) in observations.iter().enumerate() {
        let line_no = row_no + 2;
        if !observation.price_quote_per_base.is_finite() || observation.price_quote_per_base <= 0.0
        {
            return Err(invalid_export(
                line_no,
                "price_quote_per_base must be finite and > 0",
            ));
        }
        if observation.source.trim().is_empty() {
            return Err(invalid_export(line_no, "source must be non-empty"));
        }
        if observation.source_quality.trim().is_empty() {
            return Err(invalid_export(line_no, "source_quality must be non-empty"));
        }
        let Some((protocol, base_mint, quote_mint, creation_time)) =
            targets.get(observation.launch_account.as_str())
        else {
            return Err(invalid_export(
                line_no,
                "launch_account is not in the feature target set",
            ));
        };
        if observation.protocol != *protocol {
            return Err(invalid_export(
                line_no,
                "protocol does not match the feature target",
            ));
        }
        if observation.base_mint.as_deref() != *base_mint
            || observation.quote_mint.as_deref() != *quote_mint
        {
            return Err(invalid_export(
                line_no,
                "mints do not match the feature target",
            ));
        }
        if let Some(creation_time) = creation_time {
            if observation.timestamp_unix < *creation_time {
                return Err(invalid_export(
                    line_no,
                    "timestamp is before the launch creation baseline",
                ));
            }
        }
    }
    Ok(normalize_observations(observations))
}

fn invalid_export(line_no: usize, message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("price export line {line_no}: {message}"),
    )
}

pub fn read_observations<R: BufRead>(reader: R) -> io::Result<Vec<PriceObservation>> {
    let mut points = Vec::new();
    for (line_no, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        points.push(serde_json::from_str(&line).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("observation line {}: {e}", line_no + 1),
            )
        })?);
    }
    Ok(normalize_observations(points))
}

pub fn merge_observations(
    existing: Vec<PriceObservation>,
    imported: Vec<PriceObservation>,
) -> Vec<PriceObservation> {
    normalize_observations(existing.into_iter().chain(imported).collect())
}

pub fn normalize_observations(mut points: Vec<PriceObservation>) -> Vec<PriceObservation> {
    points.retain(|p| p.price_quote_per_base.is_finite() && p.price_quote_per_base > 0.0);
    points.sort_by_key(|p| (p.launch_account.clone(), p.timestamp_unix));
    points.dedup_by(|a, b| {
        a.launch_account == b.launch_account && a.timestamp_unix == b.timestamp_unix
    });
    points
}

fn invalid_price<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}

pub fn quality_decision(value: &Value) -> QualityDecision {
    let flags = &value["quality_flags"];
    let clean = flags["complete_window"].as_bool() == Some(true)
        && flags["scanner_drop_observed"].as_bool() == Some(false)
        && flags["stale_event_observed"].as_bool() == Some(false)
        && flags["rpc_rate_limited"].as_bool() == Some(false)
        && flags["partial_protocol_coverage"].as_bool() == Some(false);
    if clean {
        QualityDecision::Accept
    } else {
        QualityDecision::Reject
    }
}

pub(crate) fn timestamp(value: &Value) -> Option<i64> {
    value["block_time"]
        .as_i64()
        .or_else(|| value["creation_time_unix"].as_i64())
}

pub fn label_outcome(feature: &Value, prices: &[PricePoint]) -> Outcome {
    let account = feature["launch_account"].as_str().unwrap_or_default();
    let start = timestamp(feature);
    let mut series: Vec<_> = prices
        .iter()
        .filter(|p| p.launch_account == account)
        .collect();
    series.sort_by_key(|p| p.timestamp_unix);
    let Some(start) = start else {
        return unavailable(None, None, false, Some("missing_creation_timestamp"));
    };
    let Some(base) = series.iter().find(|p| p.timestamp_unix >= start) else {
        return unavailable(
            None,
            None,
            false,
            Some("no_price_observation_at_or_after_creation"),
        );
    };
    let source = Some(base.source.clone());
    let source_quality = Some(base.source_quality.clone());
    let horizon = |seconds: i64| -> Option<f64> {
        if !series.iter().any(|p| p.timestamp_unix >= start + seconds) {
            return None;
        }
        series
            .iter()
            .rev()
            .find(|p| p.timestamp_unix <= start + seconds)
            .map(|p| p.price_quote_per_base / base.price_quote_per_base - 1.0)
    };
    let window = |seconds: i64| -> Option<(f64, f64)> {
        let points: Vec<f64> = series
            .iter()
            .filter(|p| p.timestamp_unix >= start && p.timestamp_unix <= start + seconds)
            .map(|p| p.price_quote_per_base / base.price_quote_per_base - 1.0)
            .collect();
        if points.is_empty() {
            None
        } else {
            let mut peak: f64 = 0.0;
            let mut drawdown: f64 = 0.0;
            for value in points {
                peak = peak.max(value);
                drawdown = drawdown.min(value - peak);
            }
            Some((peak, drawdown))
        }
    };
    let w5 = horizon(300).and_then(|_| window(300));
    let w15 = horizon(900).and_then(|_| window(900));
    Outcome {
        return_1m: horizon(60),
        return_5m: horizon(300),
        return_15m: horizon(900),
        return_1h: horizon(3600),
        max_return_5m: w5.map(|w| w.0),
        max_return_15m: w15.map(|w| w.0),
        max_drawdown_5m: w5.map(|w| w.1),
        max_drawdown_15m: w15.map(|w| w.1),
        label_available: horizon(300).is_some(),
        price_data_source: source,
        price_source_quality: source_quality,
        derived_from_swaps: base.derived_from_swaps,
        label_status: if horizon(3600).is_some() {
            "complete"
        } else if horizon(300).is_some() {
            "partial"
        } else {
            "missing"
        }
        .into(),
        missing_data_reason: if horizon(300).is_some() && horizon(3600).is_none() {
            Some("future_series_ends_before_1h".into())
        } else {
            None
        },
    }
}

fn unavailable(
    source: Option<String>,
    source_quality: Option<String>,
    derived_from_swaps: bool,
    reason: Option<&str>,
) -> Outcome {
    Outcome {
        return_1m: None,
        return_5m: None,
        return_15m: None,
        return_1h: None,
        max_return_5m: None,
        max_return_15m: None,
        max_drawdown_5m: None,
        max_drawdown_15m: None,
        label_available: false,
        price_data_source: source,
        price_source_quality: source_quality,
        derived_from_swaps,
        label_status: "missing".into(),
        missing_data_reason: reason.map(str::to_owned),
    }
}

pub fn bucket(score: f64) -> &'static str {
    match score.partial_cmp(&20.0).unwrap_or(Ordering::Less) {
        Ordering::Less => "0-20",
        _ => match score.partial_cmp(&40.0).unwrap_or(Ordering::Less) {
            Ordering::Less => "20-40",
            _ => match score.partial_cmp(&60.0).unwrap_or(Ordering::Less) {
                Ordering::Less => "40-60",
                _ => match score.partial_cmp(&80.0).unwrap_or(Ordering::Less) {
                    Ordering::Less => "60-80",
                    _ => "80-100",
                },
            },
        },
    }
}

pub fn strategy_a(value: &Value, threshold: f64) -> bool {
    quality_decision(value) == QualityDecision::Accept
        && value["momentum_score"].as_f64().unwrap_or(-1.0) >= threshold
}
pub fn strategy_b(value: &Value, threshold: f64, payers: u64) -> bool {
    strategy_a(value, threshold)
        && value["approximate_unique_fee_payers_10_30_60s"][2]
            .as_u64()
            .unwrap_or(0)
            >= payers
}
pub fn strategy_c(value: &Value, acceleration: f64) -> bool {
    value["transaction_acceleration_10_30_60s"][2]
        .as_f64()
        .unwrap_or(f64::NEG_INFINITY)
        > acceleration
        && quality_decision(value) == QualityDecision::Accept
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct MetricSummary {
    pub signals: usize,
    pub wins: usize,
    pub average_return: Option<f64>,
    pub median_return: Option<f64>,
    pub max_drawdown: Option<f64>,
    pub expectancy: Option<f64>,
}

pub fn metrics(returns: &mut [f64]) -> MetricSummary {
    if returns.is_empty() {
        return MetricSummary::default();
    }
    returns.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let wins = returns.iter().filter(|r| **r > 0.0).count();
    let average_return = Some(returns.iter().sum::<f64>() / returns.len() as f64);
    let median_return = Some(if returns.len().is_multiple_of(2) {
        (returns[returns.len() / 2 - 1] + returns[returns.len() / 2]) / 2.0
    } else {
        returns[returns.len() / 2]
    });
    MetricSummary {
        signals: returns.len(),
        wins,
        average_return,
        median_return,
        max_drawdown: Some(returns.iter().copied().fold(0.0, f64::min)),
        expectancy: average_return,
    }
}

pub fn correlation(xs: &[f64], ys: &[f64]) -> Option<f64> {
    if xs.len() < 2 || xs.len() != ys.len() {
        return None;
    }
    let mx = xs.iter().sum::<f64>() / xs.len() as f64;
    let my = ys.iter().sum::<f64>() / ys.len() as f64;
    let (mut n, mut dx, mut dy) = (0.0, 0.0, 0.0);
    for (x, y) in xs.iter().zip(ys) {
        let a = x - mx;
        let b = y - my;
        n += a * b;
        dx += a * a;
        dy += b * b;
    }
    (dx > 0.0 && dy > 0.0).then_some(n / (dx.sqrt() * dy.sqrt()))
}

pub fn make_labeled(value: Value, prices: &[PricePoint]) -> LabeledLaunch {
    LabeledLaunch {
        protocol: value["protocol"].as_str().unwrap_or("unknown").to_owned(),
        launch_account: value["launch_account"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        base_mint: value["base_mint"].as_str().map(str::to_owned),
        quote_mint: value["quote_mint"].as_str().map(str::to_owned),
        creation_time_unix: timestamp(&value),
        momentum_score: value["momentum_score"].as_f64().unwrap_or(0.0),
        quality_flags: value["quality_flags"].clone(),
        outcome: label_outcome(&value, prices),
        features: value,
    }
}

pub fn write_jsonl(path: &Path, rows: &[LabeledLaunch]) -> io::Result<()> {
    let mut file = File::create(path)?;
    for row in rows {
        serde_json::to_writer(&mut file, row).map_err(io::Error::other)?;
        file.write_all(b"\n")?;
    }
    Ok(())
}

pub fn read_labeled(path: &Path) -> io::Result<Vec<LabeledLaunch>> {
    let file = File::open(path)?;
    let mut rows = Vec::new();
    for (line_no, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        rows.push(serde_json::from_str(&line).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("labeled line {}: {e}", line_no + 1),
            )
        })?);
    }
    Ok(rows)
}

pub fn write_observations(path: &Path, rows: &[PriceObservation]) -> io::Result<()> {
    let mut file = File::create(path)?;
    for row in rows {
        serde_json::to_writer(&mut file, row).map_err(io::Error::other)?;
        file.write_all(b"\n")?;
    }
    Ok(())
}

pub fn write_serialized_jsonl<T: Serialize>(path: &Path, rows: &[T]) -> io::Result<()> {
    let mut file = File::create(path)?;
    for row in rows {
        serde_json::to_writer(&mut file, row).map_err(io::Error::other)?;
        file.write_all(b"\n")?;
    }
    Ok(())
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DatasetInventory {
    pub total_feature_records: usize,
    pub unique_launch_accounts: usize,
    pub records_by_protocol: BTreeMap<String, usize>,
    pub complete_windows: usize,
    pub partial_windows: usize,
    pub scanner_quality_flag_counts: BTreeMap<String, usize>,
    pub records_with_usable_mints: usize,
    pub records_ready_for_labeling: usize,
    pub labeled: usize,
    pub partially_labeled: usize,
    pub unlabeled: usize,
}

pub fn inventory(features: &[Value], labels: Option<&[LabeledLaunch]>) -> DatasetInventory {
    let mut result = DatasetInventory {
        total_feature_records: features.len(),
        ..Default::default()
    };
    let mut accounts = BTreeMap::new();
    for row in features {
        let account = row["launch_account"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        accounts.insert(account, true);
        *result
            .records_by_protocol
            .entry(row["protocol"].as_str().unwrap_or("unknown").to_owned())
            .or_default() += 1;
        if row["complete_window"].as_bool() == Some(true) {
            result.complete_windows += 1;
        } else {
            result.partial_windows += 1;
        }
        for flag in [
            "scanner_drop_observed",
            "stale_event_observed",
            "rpc_rate_limited",
            "approximate_trader_identity",
            "partial_protocol_coverage",
        ] {
            if row["quality_flags"][flag].as_bool() == Some(true) {
                *result
                    .scanner_quality_flag_counts
                    .entry(flag.to_owned())
                    .or_default() += 1;
            }
        }
        let mints = row["base_mint"]
            .as_str()
            .filter(|v| !v.is_empty())
            .is_some()
            && row["quote_mint"]
                .as_str()
                .filter(|v| !v.is_empty())
                .is_some();
        if mints {
            result.records_with_usable_mints += 1;
        }
        if mints && quality_decision(row) == QualityDecision::Accept {
            result.records_ready_for_labeling += 1;
        }
    }
    result.unique_launch_accounts = accounts.len();
    if let Some(labels) = labels {
        for row in labels {
            match row.outcome.label_status.as_str() {
                "complete" => result.labeled += 1,
                "partial" => result.partially_labeled += 1,
                _ => result.unlabeled += 1,
            }
        }
    }
    result
}

#[derive(Debug, Clone, Serialize)]
pub struct StrategyResult {
    pub signals: usize,
    pub wins: usize,
    pub win_rate: Option<f64>,
    pub average_return: Option<f64>,
    pub median_return: Option<f64>,
    pub expectancy: Option<f64>,
    pub max_drawdown: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BacktestReport {
    pub methodology_version: String,
    pub dataset_size: usize,
    pub labeled_samples: usize,
    pub usable_samples: usize,
    pub rejected_quality_samples: usize,
    pub complete_samples: usize,
    pub partial_samples: usize,
    pub unlabeled_samples: usize,
    pub chronological_train_samples: usize,
    pub chronological_holdout_samples: usize,
    pub score_bucket_counts: BTreeMap<String, usize>,
    pub return_5m_by_score_bucket: BTreeMap<String, MetricSummary>,
    pub correlations_return_5m: BTreeMap<String, Option<f64>>,
    pub strategies: BTreeMap<String, StrategyResult>,
    pub fee_per_side: f64,
    pub slippage_per_side: f64,
    pub warnings: Vec<String>,
}

pub fn net_return(gross: f64, fee: f64, slippage: f64) -> f64 {
    (1.0 + gross) * (1.0 - fee - slippage).powi(2) - 1.0
}

fn strategy_result(
    rows: &[&LabeledLaunch],
    predicate: impl Fn(&LabeledLaunch) -> bool,
    fee: f64,
    slip: f64,
) -> StrategyResult {
    let mut returns: Vec<f64> = rows
        .iter()
        .filter(|r| predicate(r))
        .filter_map(|r| r.outcome.return_5m)
        .map(|r| net_return(r, fee, slip))
        .collect();
    let summary = metrics(&mut returns);
    StrategyResult {
        signals: summary.signals,
        wins: summary.wins,
        win_rate: (summary.signals > 0).then_some(summary.wins as f64 / summary.signals as f64),
        average_return: summary.average_return,
        median_return: summary.median_return,
        expectancy: summary.expectancy,
        max_drawdown: summary.max_drawdown,
    }
}

pub fn backtest(rows: &[LabeledLaunch], fee: f64, slippage: f64) -> BacktestReport {
    let labeled: Vec<_> = rows.iter().filter(|r| r.outcome.label_available).collect();
    let mut usable: Vec<_> = labeled
        .iter()
        .filter(|r| quality_decision(&r.features) == QualityDecision::Accept)
        .copied()
        .collect();
    usable.sort_by_key(|row| row.creation_time_unix);
    let mut buckets = BTreeMap::new();
    let mut grouped: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for row in &usable {
        let key = bucket(row.momentum_score).to_owned();
        *buckets.entry(key.clone()).or_default() += 1;
        if let Some(r) = row.outcome.return_5m {
            grouped
                .entry(key)
                .or_default()
                .push(net_return(r, fee, slippage));
        }
    }
    let mut bucket_metrics = BTreeMap::new();
    for (key, mut values) in grouped {
        bucket_metrics.insert(key, metrics(&mut values));
    }
    let mut warnings = Vec::new();
    if usable.len() < 30 {
        warnings.push("fewer than 30 usable samples: exploratory only".into());
    } else if usable.len() < 100 {
        warnings.push("30-100 usable samples: still weak evidence".into());
    } else {
        warnings.push("100+ samples: preliminary evaluation, not proof".into());
    }
    let split = if usable.len() >= 30 {
        usable.len() * 70 / 100
    } else {
        usable.len()
    };
    let evaluation = &usable[split..];
    if usable.len() < 30 {
        warnings.push(
            "chronological train/test holdout skipped because sample size is insufficient".into(),
        );
    }
    let mut correlations = BTreeMap::new();
    let mut scores = Vec::new();
    let mut outcomes = Vec::new();
    for row in &usable {
        if let Some(outcome) = row.outcome.return_5m {
            scores.push(row.momentum_score);
            outcomes.push(outcome);
        }
    }
    correlations.insert("momentum_score".into(), correlation(&scores, &outcomes));
    for (name, values) in [
        (
            "swap_rate",
            usable
                .iter()
                .filter_map(|r| r.features["swap_rates_10_30_60s"][2].as_f64())
                .collect::<Vec<_>>(),
        ),
        (
            "tx_rate",
            usable
                .iter()
                .filter_map(|r| r.features["transaction_rates_10_30_60s"][2].as_f64())
                .collect::<Vec<_>>(),
        ),
        (
            "acceleration",
            usable
                .iter()
                .filter_map(|r| r.features["transaction_acceleration_10_30_60s"][2].as_f64())
                .collect::<Vec<_>>(),
        ),
        (
            "unique_payers",
            usable
                .iter()
                .filter_map(|r| r.features["approximate_unique_fee_payers_10_30_60s"][2].as_f64())
                .collect::<Vec<_>>(),
        ),
    ] {
        correlations.insert(name.into(), correlation(&values, &outcomes));
    }
    let mut strategies = BTreeMap::new();
    for (name, threshold) in [
        ("momentum_ge_40", 40.0),
        ("momentum_ge_60", 60.0),
        ("momentum_ge_80", 80.0),
    ] {
        strategies.insert(
            name.into(),
            strategy_result(evaluation, |r| r.momentum_score >= threshold, fee, slippage),
        );
    }
    strategies.insert(
        "momentum_ge_40_payers_ge_3".into(),
        strategy_result(
            evaluation,
            |r| strategy_b(&r.features, 40.0, 3),
            fee,
            slippage,
        ),
    );
    strategies.insert(
        "positive_acceleration_clean".into(),
        strategy_result(evaluation, |r| strategy_c(&r.features, 0.0), fee, slippage),
    );
    BacktestReport {
        methodology_version: "p3-exploratory-v1".into(),
        dataset_size: rows.len(),
        labeled_samples: labeled.len(),
        usable_samples: usable.len(),
        rejected_quality_samples: labeled.len() - usable.len(),
        complete_samples: usable
            .iter()
            .filter(|r| r.outcome.label_status == "complete")
            .count(),
        partial_samples: usable
            .iter()
            .filter(|r| r.outcome.label_status == "partial")
            .count(),
        unlabeled_samples: rows.len() - labeled.len(),
        chronological_train_samples: split,
        chronological_holdout_samples: usable.len() - split,
        score_bucket_counts: buckets,
        return_5m_by_score_bucket: bucket_metrics,
        correlations_return_5m: correlations,
        strategies,
        fee_per_side: fee,
        slippage_per_side: slippage,
        warnings,
    }
}

pub fn score_report(rows: &[LabeledLaunch]) -> String {
    let labeled: Vec<_> = rows.iter().filter(|r| r.outcome.label_available).collect();
    let usable: Vec<_> = labeled
        .iter()
        .filter(|r| quality_decision(&r.features) == QualityDecision::Accept)
        .collect();
    let rejected = rows
        .iter()
        .filter(|r| quality_decision(&r.features) == QualityDecision::Reject)
        .count();
    let mut grouped: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    let mut score_values = Vec::new();
    let mut outcome_values = Vec::new();
    let mut feature_values: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for row in &usable {
        if let Some(v) = row.outcome.return_5m {
            score_values.push(row.momentum_score);
            outcome_values.push(v);
            for (name, path) in [
                ("swap_rate", ["swap_rates_10_30_60s", "2"]),
                ("tx_rate", ["transaction_rates_10_30_60s", "2"]),
                ("acceleration", ["transaction_acceleration_10_30_60s", "2"]),
                (
                    "unique_payers",
                    ["approximate_unique_fee_payers_10_30_60s", "2"],
                ),
            ] {
                if let Some(number) =
                    row.features[path[0]][path[1].parse::<usize>().unwrap()].as_f64()
                {
                    feature_values.entry(name).or_default().push(number);
                }
            }
            grouped
                .entry(bucket(row.momentum_score))
                .or_default()
                .push(v);
        }
    }
    let mut out = format!(
        "total_samples={} usable_samples={} rejected_quality={} unlabeled={}\n",
        rows.len(),
        usable.len(),
        rejected,
        rows.len() - labeled.len()
    );
    out.push_str(&format!("score_distribution_min={:?} score_distribution_max={:?} correlation_score_return_5m={:?}\n", score_values.iter().copied().reduce(f64::min), score_values.iter().copied().reduce(f64::max), correlation(&score_values, &outcome_values)));
    for (name, values) in feature_values {
        out.push_str(&format!(
            "correlation_{}_return_5m={:?}\n",
            name,
            correlation(&values, &outcome_values)
        ));
    }
    for (name, mut values) in grouped {
        let summary = metrics(&mut values);
        out.push_str(&format!("bucket={} signals={} win_rate={:.3} average_return={:.6} median_return={:.6} max_drawdown={:.6}\n", name, summary.signals, summary.wins as f64 / summary.signals as f64, summary.average_return.unwrap_or(0.0), summary.median_return.unwrap_or(0.0), summary.max_drawdown.unwrap_or(0.0)));
    }
    out.push_str("returns are gross/unadjusted; fees and slippage are not modeled; correlation is omitted when sample coverage is insufficient\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn feature() -> Value {
        serde_json::json!({"schema_version":15,"protocol":"raydium_cpmm","launch_account":"pool","base_mint":"base","quote_mint":"quote","block_time":1000,"momentum_score":42.0,"approximate_unique_fee_payers_10_30_60s":[1,2,3],"transaction_acceleration_10_30_60s":[0.0,0.0,0.5],"quality_flags":{"complete_window":true,"scanner_drop_observed":false,"stale_event_observed":false,"rpc_rate_limited":false,"partial_protocol_coverage":false}})
    }

    fn launchlab_feature() -> Value {
        serde_json::json!({"schema_version":15,"protocol":"raydium_launchlab","launch_account":"launch","base_mint":"base","quote_mint":"quote","block_time":1000,"momentum_score":80.0,"complete_window":true,"approximate_unique_fee_payers_10_30_60s":[1,2,3],"transaction_acceleration_10_30_60s":[0.0,0.0,0.5],"quality_flags":{"complete_window":true,"scanner_drop_observed":false,"stale_event_observed":false,"rpc_rate_limited":false,"partial_protocol_coverage":false}})
    }

    fn token_balance(index: u64, mint: &str, amount: &str) -> Value {
        serde_json::json!({"accountIndex":index,"mint":mint,"uiTokenAmount":{"amount":amount,"decimals":6}})
    }

    fn launchlab_transaction(program: &str, launch: &str) -> Value {
        let data = bs58::encode([250, 234, 13, 123, 213, 156, 19, 236]).into_string();
        serde_json::json!({
            "slot":7,"blockTime":1060,
            "transaction":{"message":{"accountKeys":["payer","base_user","base_vault","quote_user","quote_vault","launch"],"instructions":[{"programId":program,"accounts":["payer","a","b","c",launch],"data":data}]}},
            "meta":{"err":null,
                "preTokenBalances":[token_balance(1,"base","2000000"),token_balance(2,"base","0"),token_balance(3,"quote","0"),token_balance(4,"quote","10000000")],
                "postTokenBalances":[token_balance(1,"base","0"),token_balance(2,"base","2000000"),token_balance(3,"quote","10000000"),token_balance(4,"quote","0")]}
        })
    }

    #[test]
    fn derives_launchlab_price_from_verified_balance_deltas() {
        let transaction =
            launchlab_transaction("LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj", "launch");
        let (observations, summary) = derive::derive_prices(&[launchlab_feature()], &[transaction]);
        assert_eq!(summary.transactions_read, 1);
        assert_eq!(summary.unique_observations, 1);
        assert_eq!(observations[0].slot, Some(7));
        assert_eq!(observations[0].timestamp_unix, 1060);
        assert!((observations[0].price_quote_per_base - 5.0).abs() < 1e-12);
        assert_eq!(
            observations[0].source_quality,
            "verified_launchlab_balance_deltas"
        );
    }

    #[test]
    fn derivation_rejects_wrong_failed_unknown_missing_zero_and_ambiguous_transactions() {
        let program = "LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj";
        let wrong = launchlab_transaction("unrelated", "launch");
        let (_, summary) = derive::derive_prices(&[launchlab_feature()], &[wrong]);
        assert_eq!(summary.observations_produced, 0);

        let mut failed = launchlab_transaction(program, "launch");
        failed["meta"]["err"] = serde_json::json!("failed");
        assert_eq!(
            derive::derive_prices(&[launchlab_feature()], &[failed])
                .0
                .len(),
            0
        );

        let unknown = launchlab_transaction(program, "unknown");
        assert_eq!(
            derive::derive_prices(&[launchlab_feature()], &[unknown])
                .1
                .rejected_unknown_target,
            1
        );

        let mut missing_decimals = launchlab_transaction(program, "launch");
        missing_decimals["meta"]["preTokenBalances"][0]["uiTokenAmount"]
            .as_object_mut()
            .unwrap()
            .remove("decimals");
        assert_eq!(
            derive::derive_prices(&[launchlab_feature()], &[missing_decimals])
                .1
                .rejected_malformed,
            1
        );

        let mut zero = launchlab_transaction(program, "launch");
        zero["meta"]["postTokenBalances"] = zero["meta"]["preTokenBalances"].clone();
        assert_eq!(
            derive::derive_prices(&[launchlab_feature()], &[zero])
                .1
                .rejected_ambiguous,
            1
        );

        let mut ambiguous = launchlab_transaction(program, "launch");
        ambiguous["meta"]["preTokenBalances"]
            .as_array_mut()
            .unwrap()
            .push(token_balance(8, "base", "1"));
        ambiguous["meta"]["postTokenBalances"]
            .as_array_mut()
            .unwrap()
            .push(token_balance(8, "base", "2"));
        assert_eq!(
            derive::derive_prices(&[launchlab_feature()], &[ambiguous])
                .1
                .rejected_ambiguous,
            1
        );
    }

    #[test]
    fn derivation_deduplicates_same_launch_timestamp() {
        let transaction =
            launchlab_transaction("LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj", "launch");
        let (observations, summary) =
            derive::derive_prices(&[launchlab_feature()], &[transaction.clone(), transaction]);
        assert_eq!(summary.observations_produced, 2);
        assert_eq!(summary.unique_observations, 1);
        assert_eq!(observations.len(), 1);
    }

    #[test]
    fn paper_simulation_is_costed_quality_gated_and_idempotent() {
        let mut accepted = make_labeled(feature(), &[]);
        accepted.outcome.return_5m = Some(0.1);
        let config = paper::PaperConfig {
            strategy_version: "test".into(),
            score_threshold: 40.0,
            holding_seconds: 300,
            starting_capital: 1_000.0,
            position_size: 100.0,
            fee_per_side: 0.01,
            slippage_per_side: 0.0,
        };
        let (trades, metrics) =
            paper::simulate(&[accepted.clone(), accepted.clone()], &config).unwrap();
        assert_eq!(trades.len(), 1);
        assert!((trades[0].gross_return - 0.1).abs() < 1e-12);
        assert!(trades[0].net_return < trades[0].gross_return);
        assert!(
            (metrics.ending_capital - (1_000.0 + 100.0 * net_return(0.1, 0.01, 0.0))).abs() < 1e-12
        );

        let mut rejected = accepted.clone();
        rejected.features["quality_flags"]["stale_event_observed"] = true.into();
        let (_, rejected_metrics) = paper::simulate(&[rejected], &config).unwrap();
        assert_eq!(rejected_metrics.trades, 0);
        let mut below = accepted;
        below.momentum_score = 39.0;
        let (_, below_metrics) = paper::simulate(&[below], &config).unwrap();
        assert_eq!(below_metrics.trades, 0);

        let mut non_finite = config;
        non_finite.score_threshold = f64::NAN;
        assert!(matches!(
            paper::simulate(&[], &non_finite),
            Err(paper::PaperConfigError::NonFinite("score_threshold"))
        ));
    }
    #[test]
    fn parses_and_filters_features() {
        let rows = parse_features("{\"schema_version\":15}\n{\"schema_version\":14}\n".as_bytes())
            .unwrap();
        assert_eq!(rows.len(), 1);
    }
    #[test]
    fn parses_normalized_prices_and_deduplicates_timestamps() {
        let csv = "protocol,launch_account,base_mint,quote_mint,timestamp_unix,slot,price_quote_per_base,source,source_quality\nraydium_cpmm,pool,base,quote,1000,7,10.0,onchain,derived_from_swaps\nraydium_cpmm,pool,base,quote,1000,7,11.0,onchain,derived_from_swaps\n";
        let rows = read_prices(csv.as_bytes()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].slot, Some(7));
        assert!(rows[0].derived_from_swaps);
    }

    #[test]
    fn observation_cache_merge_is_sorted_and_idempotent() {
        let path = std::env::temp_dir().join(format!(
            "trenches-observations-{}-{}.jsonl",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let existing = vec![PriceObservation {
            protocol: "raydium_cpmm".into(),
            launch_account: "pool".into(),
            base_mint: Some("base".into()),
            quote_mint: Some("quote".into()),
            timestamp_unix: 1000,
            slot: Some(1),
            price_quote_per_base: 10.0,
            source: "cached_fixture".into(),
            source_quality: "observed".into(),
            observed: true,
            derived_from_swaps: false,
        }];
        write_observations(&path, &existing).unwrap();
        let cached = read_observations(BufReader::new(File::open(&path).unwrap())).unwrap();
        let imported = read_prices("protocol,launch_account,base_mint,quote_mint,timestamp_unix,slot,price_quote_per_base,source,source_quality\n\
raydium_cpmm,pool,base,quote,1060,2,11.0,imported_fixture,observed\n\
raydium_cpmm,pool,base,quote,1000,1,99.0,imported_fixture,observed\n".as_bytes()).unwrap();

        let merged = merge_observations(cached, imported.clone());
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].timestamp_unix, 1000);
        assert_eq!(merged[0].price_quote_per_base, 10.0);
        assert_eq!(merged[1].timestamp_unix, 1060);

        let repeated = merge_observations(merged.clone(), imported);
        assert_eq!(
            serde_json::to_string(&repeated).unwrap(),
            serde_json::to_string(&merged).unwrap()
        );
        assert_eq!(
            serde_json::to_string(&make_labeled(feature(), &repeated)).unwrap(),
            serde_json::to_string(&make_labeled(feature(), &merged)).unwrap()
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn validated_price_export_flows_from_target_to_cache_to_label() {
        let csv = "protocol,launch_account,base_mint,quote_mint,timestamp_unix,slot,price_quote_per_base,source,source_quality\n\
raydium_cpmm,pool,base,quote,1000,1,100.0,verified_fixture,observed\n\
raydium_cpmm,pool,base,quote,1000,1,999.0,verified_fixture,observed\n\
raydium_cpmm,pool,base,quote,1060,2,110.0,verified_fixture,observed\n\
raydium_cpmm,pool,base,quote,1300,3,120.0,verified_fixture,observed\n\
raydium_cpmm,pool,base,quote,1900,4,90.0,verified_fixture,observed\n\
raydium_cpmm,pool,base,quote,4600,5,150.0,verified_fixture,observed\n";
        let imported = read_price_export(csv.as_bytes()).unwrap();
        let validated = validate_price_export(&[feature()], imported).unwrap();
        assert_eq!(validated.len(), 5);
        assert_eq!(validated[0].price_quote_per_base, 100.0);

        let cached = merge_observations(Vec::new(), validated);
        let labeled = make_labeled(feature(), &cached);
        assert_eq!(labeled.outcome.label_status, "complete");
        assert_eq!(
            labeled.outcome.price_data_source.as_deref(),
            Some("verified_fixture")
        );
    }

    #[test]
    fn price_export_validation_rejects_unknown_invalid_and_unprovenanced_rows() {
        let valid = PriceObservation {
            protocol: "raydium_cpmm".into(),
            launch_account: "pool".into(),
            base_mint: Some("base".into()),
            quote_mint: Some("quote".into()),
            timestamp_unix: 1000,
            slot: Some(1),
            price_quote_per_base: 10.0,
            source: "fixture".into(),
            source_quality: "observed".into(),
            observed: true,
            derived_from_swaps: false,
        };
        let mut unknown = valid.clone();
        unknown.launch_account = "unknown".into();
        assert!(validate_price_export(&[feature()], vec![unknown]).is_err());

        let mut invalid_price = valid.clone();
        invalid_price.price_quote_per_base = 0.0;
        assert!(validate_price_export(&[feature()], vec![invalid_price]).is_err());

        let mut missing_source = valid.clone();
        missing_source.source.clear();
        assert!(validate_price_export(&[feature()], vec![missing_source]).is_err());

        let mut missing_quality = valid.clone();
        missing_quality.source_quality.clear();
        assert!(validate_price_export(&[feature()], vec![missing_quality]).is_err());
    }

    #[test]
    fn labels_only_observations_at_or_before_horizons() {
        let prices = vec![
            PricePoint {
                protocol: "raydium_cpmm".into(),
                launch_account: "pool".into(),
                base_mint: None,
                quote_mint: None,
                timestamp_unix: 1000,
                slot: None,
                price_quote_per_base: 10.0,
                source: "onchain_fixture".into(),
                source_quality: "derived_from_swaps".into(),
                observed: true,
                derived_from_swaps: true,
            },
            PricePoint {
                protocol: "raydium_cpmm".into(),
                launch_account: "pool".into(),
                base_mint: None,
                quote_mint: None,
                timestamp_unix: 1060,
                slot: None,
                price_quote_per_base: 12.0,
                source: "onchain_fixture".into(),
                source_quality: "derived_from_swaps".into(),
                observed: true,
                derived_from_swaps: true,
            },
            PricePoint {
                protocol: "raydium_cpmm".into(),
                launch_account: "pool".into(),
                base_mint: None,
                quote_mint: None,
                timestamp_unix: 1300,
                slot: None,
                price_quote_per_base: 9.0,
                source: "onchain_fixture".into(),
                source_quality: "derived_from_swaps".into(),
                observed: true,
                derived_from_swaps: true,
            },
        ];
        let outcome = label_outcome(&feature(), &prices);
        assert!((outcome.return_1m.unwrap() - 0.2).abs() < 1e-12);
        assert!((outcome.return_5m.unwrap() + 0.1).abs() < 1e-12);
        assert!((outcome.max_return_5m.unwrap() - 0.2).abs() < 1e-12);
        assert!((outcome.max_drawdown_5m.unwrap() + 0.3).abs() < 1e-12);
        assert_eq!(outcome.label_status, "partial");
        assert_eq!(
            outcome.missing_data_reason.as_deref(),
            Some("future_series_ends_before_1h")
        );
    }

    #[test]
    fn feature_prices_label_and_backtest_form_one_complete_flow() {
        let csv = "protocol,launch_account,base_mint,quote_mint,timestamp_unix,slot,price_quote_per_base,source,source_quality\n\
raydium_cpmm,pool,base,quote,1000,1,100.0,fixture,observed\n\
raydium_cpmm,pool,base,quote,1060,2,110.0,fixture,observed\n\
raydium_cpmm,pool,base,quote,1300,3,120.0,fixture,observed\n\
raydium_cpmm,pool,base,quote,1900,4,90.0,fixture,observed\n\
raydium_cpmm,pool,base,quote,4600,5,150.0,fixture,observed\n";
        let prices = read_prices(csv.as_bytes()).unwrap();
        let labeled = make_labeled(feature(), &prices);

        assert_eq!(labeled.outcome.label_status, "complete");
        assert!((labeled.outcome.return_1m.unwrap() - 0.10).abs() < 1e-12);
        assert!((labeled.outcome.return_5m.unwrap() - 0.20).abs() < 1e-12);
        assert!((labeled.outcome.return_15m.unwrap() + 0.10).abs() < 1e-12);
        assert!((labeled.outcome.return_1h.unwrap() - 0.50).abs() < 1e-12);

        let report = backtest(&[labeled], 0.0, 0.0);
        assert_eq!(report.dataset_size, 1);
        assert_eq!(report.complete_samples, 1);
        assert_eq!(report.chronological_train_samples, 1);
        assert_eq!(report.chronological_holdout_samples, 0);
        let bucket = &report.return_5m_by_score_bucket["40-60"];
        assert_eq!(bucket.signals, 1);
        assert!((bucket.average_return.unwrap() - 0.20).abs() < 1e-12);
    }

    #[test]
    fn bucketing_and_strategies_are_deterministic() {
        let f = feature();
        assert_eq!(bucket(20.0), "20-40");
        assert_eq!(bucket(100.0), "80-100");
        assert!(strategy_a(&f, 40.0));
        assert!(strategy_b(&f, 40.0, 3));
        assert!(strategy_c(&f, 0.1));
    }
    #[test]
    fn quality_filter_and_metrics_reject_bad_or_empty_data() {
        let mut f = feature();
        assert_eq!(quality_decision(&f), QualityDecision::Accept);
        f["quality_flags"]["stale_event_observed"] = true.into();
        assert_eq!(quality_decision(&f), QualityDecision::Reject);
        let mut values = vec![-0.2, 0.1, 0.3];
        let m = metrics(&mut values);
        assert_eq!(m.signals, 3);
        assert_eq!(m.wins, 2);
        assert_eq!(metrics(&mut []).signals, 0);
    }
    #[test]
    fn inventory_and_cost_adjustment_are_stable() {
        let mut first = feature();
        first["complete_window"] = true.into();
        let mut duplicate = feature();
        duplicate["complete_window"] = false.into();
        let rows = vec![first, duplicate];
        let report = inventory(&rows, None);
        assert_eq!(report.total_feature_records, 2);
        assert_eq!(report.unique_launch_accounts, 1);
        assert_eq!(report.complete_windows, 1);
        assert!((net_return(0.0, 0.01, 0.01) + 0.0396).abs() < 1e-12);
    }
    #[test]
    fn backtest_report_serializes_and_warns_on_small_sample() {
        let row = make_labeled(feature(), &[]);
        let report = backtest(&[row], 0.0, 0.0);
        assert_eq!(report.usable_samples, 0);
        assert_eq!(report.chronological_holdout_samples, 0);
        assert!(report.warnings.iter().any(|w| w.contains("exploratory")));
        serde_json::to_string(&report).unwrap();
    }
}
