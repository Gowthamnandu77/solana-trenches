use dotenvy::dotenv;
use std::env;
const MAINNET_CPMM: &str = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";

const MAINNET_CLMM: &str = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK";
const MAINNET_LAUNCHLAB: &str = "LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj";

const DEVNET_CPMM: &str = "DRaycpLY18LhpbydsBWbVJtxpNv9oXPgjRSfpF2bWpYb";

const DEVNET_CLMM: &str = "DRayAUgENGQBKVaX8owNhgzkEDyoHTGVEGHVJT1E9pfH";

#[derive(Clone)]
pub struct Config {
    pub max_fetch_concurrency: usize,
    pub input_queue_capacity: usize,
    pub max_fetch_start_age_ms: u64,
    pub max_momentum_event_age_ms: u64,
    pub rpc_request_interval_ms: u64,
    pub cluster: String,
    pub http_url: String,
    pub ws_url: String,
    pub cpmm_program: &'static str,
    pub clmm_program: &'static str,
    pub launchlab_program: Option<&'static str>,
}

pub fn load_config() -> Result<Config, Box<dyn std::error::Error>> {
    dotenv().ok();

    let cluster = env::var("SOLANA_CLUSTER")
        .unwrap_or_else(|_| "devnet".to_string())
        .to_lowercase();

    if !matches!(cluster.as_str(), "mainnet" | "mainnet-beta" | "devnet") {
        return Err("SOLANA_CLUSTER must be mainnet, mainnet-beta, or devnet".into());
    }

    let mainnet = cluster == "mainnet" || cluster == "mainnet-beta";

    let http_url = env::var("SOLANA_HTTP_URL").unwrap_or_else(|_| {
        if mainnet {
            "https://api.mainnet-beta.solana.com".to_string()
        } else {
            "https://api.devnet.solana.com".to_string()
        }
    });

    let ws_url = env::var("SOLANA_WS_URL").unwrap_or_else(|_| {
        if mainnet {
            "wss://api.mainnet-beta.solana.com/".to_string()
        } else {
            "wss://api.devnet.solana.com/".to_string()
        }
    });

    let (cpmm_program, clmm_program) = if mainnet {
        (MAINNET_CPMM, MAINNET_CLMM)
    } else {
        (DEVNET_CPMM, DEVNET_CLMM)
    };

    Ok(Config {
        max_fetch_concurrency: setting(
            "FETCH_CONCURRENCY",
            setting("MAX_FETCH_CONCURRENCY", 8, 1, 64)?,
            1,
            64,
        )? as usize,
        input_queue_capacity: setting("INPUT_QUEUE_CAPACITY", 2000, 1, 100_000)? as usize,
        max_fetch_start_age_ms: setting("MAX_FETCH_START_AGE_MS", 5000, 1, 300_000)?,
        max_momentum_event_age_ms: setting("MAX_MOMENTUM_EVENT_AGE_MS", 5000, 1, 300_000)?,
        rpc_request_interval_ms: setting(
            "RPC_MIN_REQUEST_INTERVAL_MS",
            setting("RPC_REQUEST_INTERVAL_MS", 100, 10, 10_000)?,
            10,
            10_000,
        )?,
        cluster,
        http_url,
        ws_url,
        cpmm_program,
        clmm_program,
        launchlab_program: mainnet.then_some(MAINNET_LAUNCHLAB),
    })
}

fn setting(
    name: &str,
    default: u64,
    min: u64,
    max: u64,
) -> Result<u64, Box<dyn std::error::Error>> {
    parse_setting(env::var(name).ok().as_deref(), default, min, max)
        .ok_or_else(|| format!("{name} must be an integer from {min} to {max}").into())
}
fn parse_setting(value: Option<&str>, default: u64, min: u64, max: u64) -> Option<u64> {
    let n = match value {
        Some(v) => v.parse().ok()?,
        None => default,
    };
    (min..=max).contains(&n).then_some(n)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrency_limits_reject_zero_and_unbounded_values() {
        assert_eq!(parse_setting(None, 8, 1, 64), Some(8));
        for v in ["0", "65", "-1", "secret", "999999999999999999999"] {
            assert_eq!(parse_setting(Some(v), 8, 1, 64), None);
        }
    }
}
