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

#[derive(Debug, Clone, Serialize)]
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
    points.retain(|p| p.price_quote_per_base.is_finite() && p.price_quote_per_base > 0.0);
    points.sort_by_key(|p| (p.launch_account.clone(), p.timestamp_unix));
    points.dedup_by(|a, b| {
        a.launch_account == b.launch_account && a.timestamp_unix == b.timestamp_unix
    });
    Ok(points)
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

fn timestamp(value: &Value) -> Option<i64> {
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
    let w5 = window(300);
    let w15 = window(900);
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

pub fn write_observations(path: &Path, rows: &[PriceObservation]) -> io::Result<()> {
    let mut file = File::create(path)?;
    for row in rows {
        serde_json::to_writer(&mut file, row).map_err(io::Error::other)?;
        file.write_all(b"\n")?;
    }
    Ok(())
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
}
