use crate::raydium::{InstructionRecord, NewPoolInfo};
use serde_json::{json, Value};

/// Stable scanner-facing event shape. Optional fields stay empty when the
/// instruction does not identify them without account-state lookups.
pub struct LaunchEvent {
    pub protocol: &'static str,
    pub event_type: &'static str,
    pub signature: String,
    pub slot: u64,
    pub notification_slot: u64,
    pub block_time: Option<i64>,
    pub detected_at: String,
    pub processing_lag_ms: u64,
    pub launch_account: Option<String>,
    pub base_mint: Option<String>,
    pub quote_mint: Option<String>,
    pub creator: Option<String>,
    pub source_program: &'static str,
    pub instruction: String,
    pub decode_status: &'static str,
    pub notification_received_unix_ms: i64,
    pub fetch_started_unix_ms: i64,
    pub fetch_completed_unix_ms: i64,
    pub source_invoke_count: usize,
}

pub struct EventContext {
    pub signature: String,
    pub slot: u64,
    pub notification_slot: u64,
    pub block_time: Option<i64>,
    pub detected_at: String,
    pub processing_lag_ms: u64,
    pub source_program: &'static str,
    pub notification_received_unix_ms: i64,
    pub fetch_started_unix_ms: i64,
    pub fetch_completed_unix_ms: i64,
    pub source_invoke_count: usize,
}

impl LaunchEvent {
    pub fn from_record(
        record: &InstructionRecord,
        pool: Option<&NewPoolInfo>,
        context: EventContext,
    ) -> Self {
        Self {
            protocol: record.protocol,
            event_type: record.event_type,
            signature: context.signature,
            slot: context.slot,
            notification_slot: context.notification_slot,
            block_time: context.block_time,
            detected_at: context.detected_at,
            processing_lag_ms: context.processing_lag_ms,
            launch_account: pool.map(|p| p.pool_state.clone()),
            base_mint: pool.map(|p| p.token_mint_0.clone()),
            quote_mint: pool.map(|p| p.token_mint_1.clone()),
            creator: pool.and_then(|p| p.creator.clone()),
            source_program: context.source_program,
            instruction: record.name.clone(),
            decode_status: if record.known { "decoded" } else { "unknown" },
            notification_received_unix_ms: context.notification_received_unix_ms,
            fetch_started_unix_ms: context.fetch_started_unix_ms,
            fetch_completed_unix_ms: context.fetch_completed_unix_ms,
            source_invoke_count: context.source_invoke_count,
        }
    }

    pub fn to_value(&self, cluster: &str) -> Value {
        json!({
            "schema_version": 14,
            "cluster": cluster,
            "protocol": self.protocol,
            "event_type": self.event_type,
            "signature": self.signature,
            "slot": self.slot,
            "notification_slot": self.notification_slot,
            "block_time": self.block_time,
            "detected_at": self.detected_at,
            "processing_lag_ms": self.processing_lag_ms,
            "launch_account": self.launch_account,
            "base_mint": self.base_mint,
            "quote_mint": self.quote_mint,
            "creator": self.creator,
            "source_program": self.source_program,
            "instruction": self.instruction,
            "decode_status": self.decode_status,
            "notification_received_unix_ms": self.notification_received_unix_ms,
            "fetch_started_unix_ms": self.fetch_started_unix_ms,
            "fetch_completed_unix_ms": self.fetch_completed_unix_ms,
            "source_invoke_count": self.source_invoke_count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalized_event_keeps_verified_launch_fields() {
        let record = InstructionRecord {
            protocol: "raydium_launchlab",
            name: "launchlab_initialize_v2".into(),
            discriminator: "".into(),
            known: true,
            event_type: "launch_created",
        };
        let pool = NewPoolInfo {
            protocol: "raydium_launchlab",
            instruction: "initialize_v2",
            pool_state: "launch".into(),
            token_mint_0: "base".into(),
            token_mint_1: "quote".into(),
            creator: Some("creator".into()),
        };
        let event = LaunchEvent::from_record(
            &record,
            Some(&pool),
            EventContext {
                signature: "signature".into(),
                slot: 42,
                notification_slot: 41,
                block_time: Some(1),
                detected_at: "now".into(),
                processing_lag_ms: 12,
                source_program: "launch-program",
                notification_received_unix_ms: 100,
                fetch_started_unix_ms: 101,
                fetch_completed_unix_ms: 102,
                source_invoke_count: 1,
            },
        );
        let value = event.to_value("mainnet");
        assert_eq!(value["schema_version"], 14);
        assert_eq!(value["event_type"], "launch_created");
        assert_eq!(value["launch_account"], "launch");
        assert_eq!(value["creator"], "creator");
    }
}
