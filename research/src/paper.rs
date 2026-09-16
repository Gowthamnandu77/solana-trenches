//! Deterministic paper simulation. This module has no wallet, RPC, or signing code.

use crate::{net_return, quality_decision, LabeledLaunch, QualityDecision};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaperConfigError {
    NonFinite(&'static str),
    Negative(&'static str),
}

impl std::fmt::Display for PaperConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonFinite(field) => write!(f, "{field} must be finite"),
            Self::Negative(field) => write!(f, "{field} must be non-negative"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PaperConfig {
    pub strategy_version: String,
    pub score_threshold: f64,
    pub holding_seconds: u64,
    pub starting_capital: f64,
    pub position_size: f64,
    pub fee_per_side: f64,
    pub slippage_per_side: f64,
}

impl PaperConfig {
    pub fn validate(&self) -> Result<(), PaperConfigError> {
        for (field, value, non_negative) in [
            ("score_threshold", self.score_threshold, false),
            ("starting_capital", self.starting_capital, true),
            ("position_size", self.position_size, true),
            ("fee_per_side", self.fee_per_side, true),
            ("slippage_per_side", self.slippage_per_side, true),
        ] {
            if !value.is_finite() {
                return Err(PaperConfigError::NonFinite(field));
            }
            if non_negative && value < 0.0 {
                return Err(PaperConfigError::Negative(field));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StrategyDecision {
    pub launch_account: String,
    pub accepted: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PaperTrade {
    pub launch_account: String,
    pub protocol: String,
    pub decision_timestamp_unix: Option<i64>,
    pub holding_seconds: u64,
    pub gross_return: f64,
    pub fee_per_side: f64,
    pub slippage_per_side: f64,
    pub net_return: f64,
    pub position_size: f64,
    pub simulated_pnl: f64,
    pub ending_capital: f64,
    pub strategy_version: String,
    pub quality_accepted: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PaperMetrics {
    pub candidate_signals: usize,
    pub accepted_signals: usize,
    pub trades: usize,
    pub wins: usize,
    pub losses: usize,
    pub gross_pnl: f64,
    pub costs: f64,
    pub net_pnl: f64,
    pub ending_capital: f64,
    pub max_drawdown: f64,
}

pub fn decision(row: &LabeledLaunch, config: &PaperConfig) -> StrategyDecision {
    let accepted = quality_decision(&row.features) == QualityDecision::Accept
        && row.momentum_score >= config.score_threshold
        && return_at_horizon(row, config.holding_seconds).is_some();
    let reason = if quality_decision(&row.features) != QualityDecision::Accept {
        "quality_rejected"
    } else if row.momentum_score < config.score_threshold {
        "score_below_threshold"
    } else if return_at_horizon(row, config.holding_seconds).is_none() {
        "missing_horizon_return"
    } else {
        "accepted"
    };
    StrategyDecision {
        launch_account: row.launch_account.clone(),
        accepted,
        reason: reason.into(),
    }
}

pub fn simulate(
    rows: &[LabeledLaunch],
    config: &PaperConfig,
) -> Result<(Vec<PaperTrade>, PaperMetrics), PaperConfigError> {
    config.validate()?;
    let mut ordered: Vec<_> = rows.iter().collect();
    ordered.sort_by_key(|row| row.creation_time_unix);
    let mut metrics = PaperMetrics {
        ending_capital: config.starting_capital,
        ..Default::default()
    };
    let mut seen = BTreeSet::new();
    let mut peak = config.starting_capital;
    let mut trades = Vec::new();
    for row in ordered {
        metrics.candidate_signals += 1;
        let signal = decision(row, config);
        if !signal.accepted || !seen.insert(row.launch_account.clone()) {
            continue;
        }
        metrics.accepted_signals += 1;
        let Some(gross_return) = return_at_horizon(row, config.holding_seconds) else {
            continue;
        };
        let position_size = config.position_size.min(metrics.ending_capital).max(0.0);
        if position_size == 0.0 {
            continue;
        }
        let net = net_return(gross_return, config.fee_per_side, config.slippage_per_side);
        let pnl = position_size * net;
        metrics.gross_pnl += position_size * gross_return;
        metrics.net_pnl += pnl;
        metrics.costs += position_size * (gross_return - net);
        metrics.ending_capital += pnl;
        peak = peak.max(metrics.ending_capital);
        metrics.max_drawdown = metrics.max_drawdown.min(metrics.ending_capital - peak);
        if pnl > 0.0 {
            metrics.wins += 1;
        } else {
            metrics.losses += 1;
        }
        trades.push(PaperTrade {
            launch_account: row.launch_account.clone(),
            protocol: row.protocol.clone(),
            decision_timestamp_unix: row.creation_time_unix,
            holding_seconds: config.holding_seconds,
            gross_return,
            fee_per_side: config.fee_per_side,
            slippage_per_side: config.slippage_per_side,
            net_return: net,
            position_size,
            simulated_pnl: pnl,
            ending_capital: metrics.ending_capital,
            strategy_version: config.strategy_version.clone(),
            quality_accepted: true,
        });
    }
    metrics.trades = trades.len();
    Ok((trades, metrics))
}

fn return_at_horizon(row: &LabeledLaunch, seconds: u64) -> Option<f64> {
    match seconds {
        60 => row.outcome.return_1m,
        300 => row.outcome.return_5m,
        900 => row.outcome.return_15m,
        3600 => row.outcome.return_1h,
        _ => None,
    }
}
