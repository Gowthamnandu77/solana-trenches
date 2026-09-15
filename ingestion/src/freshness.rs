//! Momentum windows begin when this live process receives a notification.  Use
//! monotonic notification-to-processing age for admission: confirmed `blockTime`
//! is second-resolution and can precede its notification by more than the
//! pipeline limit.
pub fn stale_reason(local_ms: u64, limit_ms: u64) -> Option<&'static str> {
    if local_ms > limit_ms {
        Some("local_pipeline_age")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_momentum_and_age_boundary() {
        assert_eq!(stale_reason(5000, 5000), None);
        assert_eq!(stale_reason(5001, 5000), Some("local_pipeline_age"));
    }
}
