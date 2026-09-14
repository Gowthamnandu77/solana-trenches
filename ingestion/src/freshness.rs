//! Local age uses monotonic time; chain age uses optional second-resolution block time.
pub fn stale_reason(local_ms: u64, block_ms: Option<u64>, limit_ms: u64) -> Option<&'static str> {
    if local_ms > limit_ms {
        Some("local_pipeline_age")
    } else if block_ms.is_some_and(|age| age > limit_ms) {
        Some("block_time_age")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_momentum_and_age_boundary() {
        assert_eq!(stale_reason(5000, Some(5000), 5000), None);
        assert_eq!(stale_reason(5001, None, 5000), Some("local_pipeline_age"));
        assert_eq!(stale_reason(1, Some(60_000), 5000), Some("block_time_age"));
        assert_eq!(stale_reason(1, None, 5000), None);
    }
}
