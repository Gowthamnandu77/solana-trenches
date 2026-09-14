use dotenvy::dotenv;
use std::env;
const MAINNET_CPMM: &str = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";

const MAINNET_CLMM: &str = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK";

const DEVNET_CPMM: &str = "DRaycpLY18LhpbydsBWbVJtxpNv9oXPgjRSfpF2bWpYb";

const DEVNET_CLMM: &str = "DRayAUgENGQBKVaX8owNhgzkEDyoHTGVEGHVJT1E9pfH";

#[derive(Clone)]
pub struct Config {
    pub cluster: String,
    pub http_url: String,
    pub ws_url: String,
    pub cpmm_program: &'static str,
    pub clmm_program: &'static str,
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
        cluster,
        http_url,
        ws_url,
        cpmm_program,
        clmm_program,
    })
}
