use serde_json::Value;
// ============================================================
// CPMM DISCRIMINATORS
// ============================================================

const CPMM_INITIALIZE: [u8; 8] = [175, 175, 109, 31, 13, 152, 155, 237];

const CPMM_DEPOSIT: [u8; 8] = [242, 35, 198, 137, 82, 225, 242, 182];

const CPMM_WITHDRAW: [u8; 8] = [183, 18, 70, 156, 148, 109, 161, 34];

const CPMM_SWAP_BASE_INPUT: [u8; 8] = [143, 190, 90, 218, 196, 30, 51, 222];

const CPMM_SWAP_BASE_OUTPUT: [u8; 8] = [55, 217, 98, 86, 163, 74, 180, 173];

// ============================================================
// CLMM DISCRIMINATORS
// ============================================================

const CLMM_CREATE_POOL: [u8; 8] = [233, 146, 209, 142, 207, 104, 64, 188];

const CLMM_CREATE_CUSTOMIZABLE_POOL: [u8; 8] = [43, 68, 212, 167, 89, 47, 164, 1];

const CLMM_SWAP: [u8; 8] = [248, 198, 158, 145, 225, 117, 135, 200];

const CLMM_SWAP_V2: [u8; 8] = [43, 4, 237, 11, 26, 201, 30, 98];

const CLMM_INCREASE_LIQUIDITY_V2: [u8; 8] = [133, 29, 89, 223, 69, 238, 176, 10];

const CLMM_DECREASE_LIQUIDITY_V2: [u8; 8] = [58, 127, 188, 62, 79, 82, 196, 96];

const CLMM_OPEN_POSITION_V2: [u8; 8] = [77, 184, 74, 214, 112, 86, 241, 199];

#[derive(Debug, Clone)]
pub struct InstructionRecord {
    pub protocol: &'static str,
    pub name: String,
    pub discriminator: String,
    pub known: bool,
}

// ============================================================
// NEW POOL
// ============================================================

#[derive(Debug, Clone)]
pub struct NewPoolInfo {
    pub protocol: &'static str,
    pub instruction: &'static str,
    pub pool_state: String,
    pub token_mint_0: String,
    pub token_mint_1: String,
}

pub fn program_invoked_in_logs(logs: &[String], program_id: &str) -> bool {
    let prefix = format!("Program {} invoke", program_id);

    logs.iter().any(|log| log.starts_with(&prefix))
}

// ============================================================
// COUNT PROGRAM INVOCATIONS
// ============================================================

pub fn program_invoke_count(logs: &[String], program_id: &str) -> usize {
    let prefix = format!("Program {} invoke", program_id);

    logs.iter().filter(|log| log.starts_with(&prefix)).count()
}

// ============================================================
// DECODE RAW INSTRUCTION DATA
// ============================================================

fn decode_data(instruction: &Value) -> Option<Vec<u8>> {
    let data = instruction.get("data")?.as_str()?;

    bs58::decode(data).into_vec().ok()
}

// ============================================================
// HEX
// ============================================================

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{:02x}", byte)).collect()
}

// ============================================================
// GET ACCOUNT
// ============================================================

fn instruction_account(instruction: &Value, index: usize) -> Option<String> {
    let accounts = instruction.get("accounts")?.as_array()?;

    let account = accounts.get(index)?;

    if let Some(pubkey) = account.as_str() {
        return Some(pubkey.to_string());
    }

    account
        .get("pubkey")
        .and_then(Value::as_str)
        .map(String::from)
}

// ============================================================
// CLASSIFY ONE RAYDIUM INSTRUCTION
// ============================================================

fn classify_raw_instruction(
    instruction: &Value,
    cpmm_program: &str,
    clmm_program: &str,
) -> Option<InstructionRecord> {
    let program_id = instruction.get("programId")?.as_str()?;

    let protocol = if program_id == cpmm_program {
        "raydium_cpmm"
    } else if program_id == clmm_program {
        "raydium_clmm"
    } else {
        return None;
    };

    let Some(decoded) = decode_data(instruction) else {
        return Some(InstructionRecord {
            protocol,
            name: format!(
                "{}_no_raw_data",
                if protocol == "raydium_cpmm" {
                    "cpmm"
                } else {
                    "clmm"
                }
            ),
            discriminator: String::new(),
            known: false,
        });
    };

    if decoded.len() < 8 {
        return Some(InstructionRecord {
            protocol,
            name: format!(
                "{}_short_data",
                if protocol == "raydium_cpmm" {
                    "cpmm"
                } else {
                    "clmm"
                }
            ),
            discriminator: bytes_to_hex(&decoded),
            known: false,
        });
    }

    let disc = &decoded[..8];

    let hex = bytes_to_hex(disc);

    // ========================================================
    // CPMM
    // ========================================================

    if protocol == "raydium_cpmm" {
        let name = if disc == CPMM_INITIALIZE {
            Some("cpmm_initialize")
        } else if disc == CPMM_DEPOSIT {
            Some("cpmm_deposit")
        } else if disc == CPMM_WITHDRAW {
            Some("cpmm_withdraw")
        } else if disc == CPMM_SWAP_BASE_INPUT {
            Some("cpmm_swap_base_input")
        } else if disc == CPMM_SWAP_BASE_OUTPUT {
            Some("cpmm_swap_base_output")
        } else {
            None
        };

        if let Some(name) = name {
            return Some(InstructionRecord {
                protocol,
                name: name.to_string(),
                discriminator: hex,
                known: true,
            });
        }

        return Some(InstructionRecord {
            protocol,
            name: format!("cpmm_unknown_{}", hex),
            discriminator: hex,
            known: false,
        });
    }

    // ========================================================
    // CLMM
    // ========================================================

    let name = if disc == CLMM_CREATE_POOL {
        Some("clmm_create_pool")
    } else if disc == CLMM_CREATE_CUSTOMIZABLE_POOL {
        Some("clmm_create_customizable_pool")
    } else if disc == CLMM_SWAP {
        Some("clmm_swap")
    } else if disc == CLMM_SWAP_V2 {
        Some("clmm_swap_v2")
    } else if disc == CLMM_INCREASE_LIQUIDITY_V2 {
        Some("clmm_increase_liquidity_v2")
    } else if disc == CLMM_DECREASE_LIQUIDITY_V2 {
        Some("clmm_decrease_liquidity_v2")
    } else if disc == CLMM_OPEN_POSITION_V2 {
        Some("clmm_open_position_v2")
    } else {
        None
    };

    if let Some(name) = name {
        return Some(InstructionRecord {
            protocol,
            name: name.to_string(),
            discriminator: hex,
            known: true,
        });
    }

    Some(InstructionRecord {
        protocol,
        name: format!("clmm_unknown_{}", hex),
        discriminator: hex,
        known: false,
    })
}

// ============================================================
// COLLECT OUTER + INNER INSTRUCTIONS
// ============================================================

pub fn collect_instruction_records(
    tx_json: &Value,
    cpmm_program: &str,
    clmm_program: &str,
) -> Vec<PoolInstruction> {
    let mut records = Vec::new();

    // Outer
    if let Some(instructions) = tx_json
        .pointer("/transaction/message/instructions")
        .and_then(Value::as_array)
    {
        for instruction in instructions {
            if let Some(record) = classify_instruction(instruction, cpmm_program, clmm_program) {
                records.push(record);
            }
        }
    }

    // Inner
    if let Some(groups) = tx_json
        .pointer("/meta/innerInstructions")
        .and_then(Value::as_array)
    {
        for group in groups {
            let Some(instructions) = group.get("instructions").and_then(Value::as_array) else {
                continue;
            };

            for instruction in instructions {
                if let Some(record) = classify_instruction(instruction, cpmm_program, clmm_program)
                {
                    records.push(record);
                }
            }
        }
    }

    records
}

// ============================================================
// NEW POOL DETECTION
// ============================================================

pub fn detect_pool_instruction(
    instruction: &Value,
    cpmm_program: &str,
    clmm_program: &str,
) -> Option<NewPoolInfo> {
    let program_id = instruction.get("programId")?.as_str()?;

    let decoded = decode_data(instruction)?;

    if decoded.len() < 8 {
        return None;
    }

    let disc = &decoded[..8];

    // CPMM initialize
    if program_id == cpmm_program && disc == CPMM_INITIALIZE {
        return Some(NewPoolInfo {
            protocol: "raydium_cpmm",

            instruction: "initialize",

            pool_state: instruction_account(instruction, 3)?,

            token_mint_0: instruction_account(instruction, 4)?,

            token_mint_1: instruction_account(instruction, 5)?,
        });
    }

    // CLMM create_pool
    if program_id == clmm_program && disc == CLMM_CREATE_POOL {
        return Some(NewPoolInfo {
            protocol: "raydium_clmm",

            instruction: "create_pool",

            pool_state: instruction_account(instruction, 2)?,

            token_mint_0: instruction_account(instruction, 3)?,

            token_mint_1: instruction_account(instruction, 4)?,
        });
    }

    // CLMM customizable pool
    if program_id == clmm_program && disc == CLMM_CREATE_CUSTOMIZABLE_POOL {
        return Some(NewPoolInfo {
            protocol: "raydium_clmm",

            instruction: "create_customizable_pool",

            pool_state: instruction_account(instruction, 2)?,

            token_mint_0: instruction_account(instruction, 3)?,

            token_mint_1: instruction_account(instruction, 4)?,
        });
    }

    None
}

// ============================================================
// FIND NEW POOL
// ============================================================

#[derive(Debug, Clone)]
pub struct PoolInstruction {
    pub record: InstructionRecord,
    pub accounts: Vec<String>,
    pub swap_pool: Option<String>,
}
impl std::ops::Deref for PoolInstruction {
    type Target = InstructionRecord;
    fn deref(&self) -> &Self::Target {
        &self.record
    }
}
fn classify_instruction(ix: &Value, cpmm: &str, clmm: &str) -> Option<PoolInstruction> {
    let record = classify_raw_instruction(ix, cpmm, clmm)?;
    // Verified against Raydium Swap / SwapSingle / SwapSingleV2 account structs.
    let index = match record.name.as_str() {
        "cpmm_swap_base_input" | "cpmm_swap_base_output" => Some(3),
        "clmm_swap" | "clmm_swap_v2" => Some(2),
        _ => None,
    };
    let swap_pool = index.and_then(|i| instruction_account(ix, i));
    let accounts = ix
        .get("accounts")
        .and_then(Value::as_array)
        .map(|a| {
            (0..a.len())
                .filter_map(|i| instruction_account(ix, i))
                .collect()
        })
        .unwrap_or_default();
    Some(PoolInstruction {
        record,
        accounts,
        swap_pool,
    })
}
pub fn instructions(tx: &Value) -> impl Iterator<Item = &Value> {
    tx.pointer("/transaction/message/instructions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .chain(
            tx.pointer("/meta/innerInstructions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .flat_map(|g| {
                    g.get("instructions")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                }),
        )
}
pub fn detect_new_pools(tx: &Value, cpmm: &str, clmm: &str) -> Vec<NewPoolInfo> {
    instructions(tx)
        .filter_map(|ix| detect_pool_instruction(ix, cpmm, clmm))
        .collect()
}
/// JSON-parsed account zero is the fee payer; require explicit signer metadata.
pub fn fee_payer(tx: &Value) -> Option<String> {
    let first = tx.pointer("/transaction/message/accountKeys/0")?;
    (first.get("signer")?.as_bool()?
        && first.get("source").and_then(Value::as_str) != Some("lookupTable"))
    .then(|| first.get("pubkey")?.as_str().map(str::to_owned))
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn ix(program: &str, disc: [u8; 8], accounts: Vec<&str>) -> Value {
        json!({"programId":program,"data":bs58::encode(disc).into_string(),"accounts":accounts})
    }
    #[test]
    fn preserves_all_v10_discriminators() {
        for (program, disc, name) in [
            ("cp", CPMM_INITIALIZE, "cpmm_initialize"),
            ("cp", CPMM_DEPOSIT, "cpmm_deposit"),
            ("cp", CPMM_WITHDRAW, "cpmm_withdraw"),
            ("cp", CPMM_SWAP_BASE_INPUT, "cpmm_swap_base_input"),
            ("cp", CPMM_SWAP_BASE_OUTPUT, "cpmm_swap_base_output"),
            ("cl", CLMM_CREATE_POOL, "clmm_create_pool"),
            (
                "cl",
                CLMM_CREATE_CUSTOMIZABLE_POOL,
                "clmm_create_customizable_pool",
            ),
            ("cl", CLMM_SWAP, "clmm_swap"),
            ("cl", CLMM_SWAP_V2, "clmm_swap_v2"),
            (
                "cl",
                CLMM_INCREASE_LIQUIDITY_V2,
                "clmm_increase_liquidity_v2",
            ),
            (
                "cl",
                CLMM_DECREASE_LIQUIDITY_V2,
                "clmm_decrease_liquidity_v2",
            ),
            ("cl", CLMM_OPEN_POSITION_V2, "clmm_open_position_v2"),
        ] {
            let r = classify_instruction(&ix(program, disc, vec![]), "cp", "cl").unwrap();
            assert!(r.known);
            assert_eq!(r.name, name);
        }
    }
    #[test]
    fn outer_inner_routes_and_precise_swap_pool() {
        let tx = json!({"transaction":{"message":{"instructions":[ix("cp",CPMM_SWAP_BASE_INPUT,vec!["payer","authority","config","poolA"])]}},
            "meta":{"innerInstructions":[{"instructions":[ix("cl",CLMM_SWAP_V2,vec!["payer","config","poolB"])]}]}});
        let records = collect_instruction_records(&tx, "cp", "cl");
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].swap_pool.as_deref(), Some("poolA"));
        assert_eq!(records[1].swap_pool.as_deref(), Some("poolB"));
    }
    #[test]
    fn multiple_pool_creation_and_missing_accounts() {
        let tx = json!({"transaction":{"message":{"instructions":[ix("cp",CPMM_INITIALIZE,vec!["creator","config","authority","a","m0","m1"])]}},
            "meta":{"innerInstructions":[{"instructions":[ix("cl",CLMM_CREATE_POOL,vec!["creator","config","b","n0","n1"])]}]}});
        let pools = detect_new_pools(&tx, "cp", "cl");
        assert_eq!(pools.len(), 2);
        assert_eq!(pools[0].pool_state, "a");
        assert_eq!(pools[1].token_mint_1, "n1");
        assert!(detect_pool_instruction(&ix("cp", CPMM_INITIALIZE, vec![]), "cp", "cl").is_none());
    }
    #[test]
    fn malformed_unknown_and_fee_payer_validation() {
        let short = json!({"programId":"cp","data":"1"});
        assert!(!classify_instruction(&short, "cp", "cl").unwrap().known);
        assert!(
            !classify_instruction(&ix("cp", [0; 8], vec![]), "cp", "cl")
                .unwrap()
                .known
        );
        assert!(classify_instruction(&ix("other", CLMM_SWAP, vec![]), "cp", "cl").is_none());
        let mut tx = json!({"transaction":{"message":{"accountKeys":[{"pubkey":"payer","signer":true,"source":"transaction"}]}}});
        assert_eq!(fee_payer(&tx).as_deref(), Some("payer"));
        tx["transaction"]["message"]["accountKeys"][0]["signer"] = json!(false);
        assert!(fee_payer(&tx).is_none());
        assert!(fee_payer(&json!({})).is_none());
    }
}
