use crate::{listener, metrics::Metrics};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;

pub fn apply_quality_flags(value: &mut Value, launchlab_disabled: bool, metrics: &Metrics) {
    let drops = listener::DROPPED_LOGS.load(Ordering::Relaxed) > 0
        || metrics.notifications_dropped.load(Ordering::Relaxed) > 0;
    let stale = metrics.stale_before_momentum.load(Ordering::Relaxed) > 0;
    let rate_limited = metrics.rate_limits.load(Ordering::Relaxed) > 0;
    let partial = launchlab_disabled || drops;
    let complete_window = value["complete_window"].clone();
    value["scanner_drop_observed"] = json!(drops);
    value["stale_event_observed"] = json!(stale);
    value["rpc_rate_limited"] = json!(rate_limited);
    value["approximate_trader_identity"] = json!(true);
    value["partial_protocol_coverage"] = json!(partial);
    value["quality_flags"] = json!({
        "complete_window": complete_window,
        "scanner_drop_observed": drops,
        "stale_event_observed": stale,
        "rpc_rate_limited": rate_limited,
        "approximate_trader_identity": true,
        "partial_protocol_coverage": partial
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quality_flags_are_explicit_and_serializable() {
        let metrics = Metrics::default();
        let mut value = json!({"complete_window": true});
        apply_quality_flags(&mut value, true, &metrics);
        assert_eq!(value["quality_flags"]["complete_window"], true);
        assert_eq!(value["quality_flags"]["partial_protocol_coverage"], true);
        assert_eq!(value["approximate_trader_identity"], true);
        serde_json::to_string(&value).unwrap();
    }
}
