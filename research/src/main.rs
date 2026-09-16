use research::{
    backtest, inventory, make_labeled, merge_observations, parse_features, read_features,
    read_jsonl_values, read_labeled, read_observations, read_price_export, read_prices,
    score_report, validate_price_export, write_jsonl, write_observations, write_serialized_jsonl,
    QualityDecision,
};
use std::{
    env,
    fs::File,
    io::{BufReader, ErrorKind},
    path::PathBuf,
    process::ExitCode,
};

fn main() -> ExitCode {
    let args: Vec<_> = env::args().collect();
    if matches!(
        args.get(1).map(String::as_str),
        Some("--help" | "-h" | "help")
    ) {
        return help_command();
    }
    if args.get(1).map(String::as_str) == Some("inventory") {
        return inventory_command(&args);
    }
    if args.get(1).map(String::as_str) == Some("backtest") {
        return backtest_command(&args);
    }
    if args.get(1).map(String::as_str) == Some("backfill") {
        return backfill_command(&args);
    }
    if args.get(1).map(String::as_str) == Some("validate-prices") {
        return validate_prices_command(&args);
    }
    if args.get(1).map(String::as_str) == Some("derive-prices") {
        return derive_prices_command(&args);
    }
    if args.get(1).map(String::as_str) == Some("fetch-transactions") {
        return fetch_transactions_command(&args);
    }
    if args.get(1).map(String::as_str) == Some("paper-trade") {
        return paper_trade_command(&args);
    }
    let features =
        argument(&args, "--features").unwrap_or_else(|| "ingestion/data/features_v15.jsonl".into());
    let prices = argument(&args, "--prices").map(PathBuf::from);
    let output = argument(&args, "--output")
        .unwrap_or_else(|| "research/data/labeled_launches.jsonl".into());
    let observations = argument(&args, "--observations")
        .unwrap_or_else(|| "research/data/price_observations.jsonl".into());
    let rows = match read_features(&PathBuf::from(features)) {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("features: {e}");
            return ExitCode::FAILURE;
        }
    };
    let price_rows = match prices {
        Some(path) => match read_prices(std::io::BufReader::new(match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) => {
                eprintln!("prices: {e}");
                return ExitCode::FAILURE;
            }
        })) {
            Ok(rows) => rows,
            Err(e) => {
                eprintln!("prices: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => Vec::new(),
    };
    if let Some(parent) = std::path::Path::new(&observations).parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("observations: {e}");
            return ExitCode::FAILURE;
        }
    }
    if let Err(e) = write_observations(std::path::Path::new(&observations), &price_rows) {
        eprintln!("observations: {e}");
        return ExitCode::FAILURE;
    }
    let labeled: Vec<_> = rows
        .into_iter()
        .map(|row| make_labeled(row, &price_rows))
        .collect();
    if let Some(parent) = std::path::Path::new(&output).parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("output: {e}");
            return ExitCode::FAILURE;
        }
    }
    if let Err(e) = write_jsonl(std::path::Path::new(&output), &labeled) {
        eprintln!("output: {e}");
        return ExitCode::FAILURE;
    }
    let rejected = labeled
        .iter()
        .filter(|r| QualityDecision::Reject == research::quality_decision(&r.features))
        .count();
    println!(
        "{}quality_rejected={} dataset={output}",
        score_report(&labeled),
        rejected
    );
    ExitCode::SUCCESS
}

fn help_command() -> ExitCode {
    println!(
        "Usage:\n  research [--features PATH] [--prices PATH] [--observations PATH] [--output PATH]\n  research inventory [--features PATH]\n  research validate-prices --features PATH --prices PATH\n  research derive-prices --features PATH --transactions PATH [--output PATH]\n  research fetch-transactions --features PATH [--target-index N] [--output-dir PATH] [--max-pages N] [--page-size N] [--signatures-only]\n  research backfill --features PATH --prices PATH [--observations PATH] [--output PATH]\n  research backtest --features PATH --prices PATH [--report PATH] [--fee-per-side RATE] [--slippage-per-side RATE]\n  research paper-trade --features PATH [--output PATH] [--metrics PATH] [--score-threshold N] [--holding-seconds 60|300|900|3600] [--starting-capital N] [--position-size N] [--fee-per-side RATE] [--slippage-per-side RATE]"
    );
    ExitCode::SUCCESS
}

fn fetch_transactions_command(args: &[String]) -> ExitCode {
    let feature_path =
        argument(args, "--features").unwrap_or_else(|| "ingestion/data/features_v15.jsonl".into());
    let target_index = argument(args, "--target-index")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let max_pages = argument(args, "--max-pages")
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| (1..=20).contains(value))
        .unwrap_or(8);
    let page_size = argument(args, "--page-size")
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| (1..=100).contains(value))
        .unwrap_or(50);
    let fetch_transactions = !args.iter().any(|arg| arg == "--signatures-only");
    let output_dir =
        argument(args, "--output-dir").unwrap_or_else(|| "research/data/raw_transactions".into());
    let features = match read_features(PathBuf::from(feature_path).as_path()) {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("features: {e}");
            return ExitCode::FAILURE;
        }
    };
    let targets: Vec<_> = features
        .iter()
        .filter(|row| row["protocol"].as_str() == Some("raydium_launchlab"))
        .filter_map(|row| {
            Some((
                row["launch_account"].as_str()?,
                row["block_time"]
                    .as_i64()
                    .or_else(|| row["creation_time_unix"].as_i64())?,
            ))
        })
        .collect();
    let Some((account, creation_time)) = targets.get(target_index).copied() else {
        eprintln!("fetch-transactions target-index is outside the LaunchLab target set");
        return ExitCode::FAILURE;
    };
    let url = match research::fetch::configured_rpc_url() {
        Ok(url) => url,
        Err(e) => {
            eprintln!("fetch-transactions: {e}");
            return ExitCode::FAILURE;
        }
    };
    let output = PathBuf::from(output_dir).join(format!("{account}.jsonl"));
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(_) => {
            eprintln!("fetch-transactions: could not start read-only runtime");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(research::fetch::fetch_target(
        url,
        account,
        creation_time,
        &output,
        research::fetch::FetchOptions {
            max_pages,
            page_size,
            fetch_transactions,
        },
    )) {
        Ok(summary) => {
            println!("{}", serde_json::to_string(&summary).unwrap());
            if summary.rpc_failures == 0 && summary.window_complete {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("fetch-transactions: {e}");
            ExitCode::FAILURE
        }
    }
}

fn derive_prices_command(args: &[String]) -> ExitCode {
    let feature_path =
        argument(args, "--features").unwrap_or_else(|| "ingestion/data/features_v15.jsonl".into());
    let Some(transaction_path) = argument(args, "--transactions") else {
        eprintln!("derive-prices requires --transactions PATH");
        return ExitCode::FAILURE;
    };
    let output = argument(args, "--output")
        .unwrap_or_else(|| "research/data/derived_price_observations.jsonl".into());
    let features = match read_features(PathBuf::from(feature_path).as_path()) {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("features: {e}");
            return ExitCode::FAILURE;
        }
    };
    let transactions = match File::open(transaction_path)
        .map(BufReader::new)
        .and_then(|reader| read_jsonl_values(reader, "transaction"))
    {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("transactions: {e}");
            return ExitCode::FAILURE;
        }
    };
    let (observations, summary) = research::derive::derive_prices(&features, &transactions);
    if let Some(parent) = std::path::Path::new(&output).parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("output: {e}");
            return ExitCode::FAILURE;
        }
    }
    if let Err(e) = write_observations(std::path::Path::new(&output), &observations) {
        eprintln!("output: {e}");
        return ExitCode::FAILURE;
    }
    println!("{}", serde_json::to_string(&summary).unwrap());
    ExitCode::SUCCESS
}

fn paper_trade_command(args: &[String]) -> ExitCode {
    let Some(feature_path) = argument(args, "--features") else {
        eprintln!("paper-trade requires --features PATH to labeled_launches.jsonl");
        return ExitCode::FAILURE;
    };
    let parse = |flag: &str, default: f64| {
        argument(args, flag)
            .and_then(|value| value.parse().ok())
            .unwrap_or(default)
    };
    let holding_seconds = argument(args, "--holding-seconds")
        .and_then(|value| value.parse().ok())
        .unwrap_or(300);
    if !matches!(holding_seconds, 60 | 300 | 900 | 3600) {
        eprintln!("paper-trade holding-seconds must be 60, 300, 900, or 3600");
        return ExitCode::FAILURE;
    }
    let config = research::paper::PaperConfig {
        strategy_version: "score_quality_horizon_v1".into(),
        score_threshold: parse("--score-threshold", 60.0),
        holding_seconds,
        starting_capital: parse("--starting-capital", 10_000.0),
        position_size: parse("--position-size", 100.0),
        fee_per_side: parse("--fee-per-side", 0.0),
        slippage_per_side: parse("--slippage-per-side", 0.0),
    };
    if let Err(e) = config.validate() {
        eprintln!("paper-trade {e}");
        return ExitCode::FAILURE;
    }
    let rows = match read_labeled(std::path::Path::new(&feature_path)) {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("features: {e}");
            return ExitCode::FAILURE;
        }
    };
    let output =
        argument(args, "--output").unwrap_or_else(|| "research/data/paper_trades.jsonl".into());
    let metrics_path =
        argument(args, "--metrics").unwrap_or_else(|| "research/data/paper_metrics.jsonl".into());
    let (trades, metrics) = match research::paper::simulate(&rows, &config) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("paper-trade {e}");
            return ExitCode::FAILURE;
        }
    };
    for path in [&output, &metrics_path] {
        if let Some(parent) = std::path::Path::new(path).parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("output: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    if let Err(e) = write_serialized_jsonl(std::path::Path::new(&output), &trades) {
        eprintln!("output: {e}");
        return ExitCode::FAILURE;
    }
    if let Err(e) = write_serialized_jsonl(
        std::path::Path::new(&metrics_path),
        std::slice::from_ref(&metrics),
    ) {
        eprintln!("metrics: {e}");
        return ExitCode::FAILURE;
    }
    println!(
        "PAPER SIMULATION ONLY — NO LIVE TRADES\n{}",
        serde_json::to_string(&metrics).unwrap()
    );
    ExitCode::SUCCESS
}

fn validate_prices_command(args: &[String]) -> ExitCode {
    let feature_path =
        argument(args, "--features").unwrap_or_else(|| "ingestion/data/features_v15.jsonl".into());
    let Some(price_path) = argument(args, "--prices") else {
        eprintln!("validate-prices requires --prices PATH");
        return ExitCode::FAILURE;
    };
    let features = match read_features(PathBuf::from(feature_path).as_path()) {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("features: {e}");
            return ExitCode::FAILURE;
        }
    };
    let imported = match File::open(&price_path) {
        Ok(file) => match if price_path.ends_with(".jsonl") {
            read_observations(BufReader::new(file))
        } else {
            read_price_export(BufReader::new(file))
        } {
            Ok(rows) => rows,
            Err(e) => {
                eprintln!("prices: {e}");
                return ExitCode::FAILURE;
            }
        },
        Err(e) => {
            eprintln!("prices: {e}");
            return ExitCode::FAILURE;
        }
    };
    let imported_count = imported.len();
    match validate_price_export(&features, imported) {
        Ok(validated) => {
            println!(
                "validated_price_observations={} normalized_unique_observations={} target_launches={}",
                imported_count,
                validated.len(),
                features.len()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("prices: {e}");
            ExitCode::FAILURE
        }
    }
}

fn backfill_command(args: &[String]) -> ExitCode {
    let feature_path =
        argument(args, "--features").unwrap_or_else(|| "ingestion/data/features_v15.jsonl".into());
    let Some(price_path) = argument(args, "--prices") else {
        eprintln!("backfill requires --prices PATH");
        return ExitCode::FAILURE;
    };
    let observations = argument(args, "--observations")
        .unwrap_or_else(|| "research/data/price_observations.jsonl".into());
    let output =
        argument(args, "--output").unwrap_or_else(|| "research/data/labeled_launches.jsonl".into());
    let features = match read_features(PathBuf::from(feature_path).as_path()) {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("features: {e}");
            return ExitCode::FAILURE;
        }
    };
    let imported = match File::open(&price_path) {
        Ok(file) => match if price_path.ends_with(".jsonl") {
            read_observations(BufReader::new(file))
        } else {
            read_price_export(BufReader::new(file))
        } {
            Ok(rows) => rows,
            Err(e) => {
                eprintln!("prices: {e}");
                return ExitCode::FAILURE;
            }
        },
        Err(e) => {
            eprintln!("prices: {e}");
            return ExitCode::FAILURE;
        }
    };
    let existing = match File::open(&observations) {
        Ok(file) => match read_observations(BufReader::new(file)) {
            Ok(rows) => rows,
            Err(e) => {
                eprintln!("observations: {e}");
                return ExitCode::FAILURE;
            }
        },
        Err(e) if e.kind() == ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            eprintln!("observations: {e}");
            return ExitCode::FAILURE;
        }
    };
    let imported_count = imported.len();
    let imported = match validate_price_export(&features, imported) {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("prices: {e}");
            return ExitCode::FAILURE;
        }
    };
    let existing_count = existing.len();
    let merged = merge_observations(existing, imported);
    if let Some(parent) = std::path::Path::new(&observations).parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("observations: {e}");
            return ExitCode::FAILURE;
        }
    }
    if let Err(e) = write_observations(std::path::Path::new(&observations), &merged) {
        eprintln!("observations: {e}");
        return ExitCode::FAILURE;
    }
    let labeled: Vec<_> = features
        .into_iter()
        .map(|row| make_labeled(row, &merged))
        .collect();
    if let Some(parent) = std::path::Path::new(&output).parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("output: {e}");
            return ExitCode::FAILURE;
        }
    }
    if let Err(e) = write_jsonl(std::path::Path::new(&output), &labeled) {
        eprintln!("output: {e}");
        return ExitCode::FAILURE;
    }
    let complete = labeled
        .iter()
        .filter(|row| row.outcome.label_status == "complete")
        .count();
    let partial = labeled
        .iter()
        .filter(|row| row.outcome.label_status == "partial")
        .count();
    let missing = labeled.len() - complete - partial;
    println!(
        "backfill imported_observations={imported_count} existing_observations={existing_count} merged_unique_observations={} labeled_launches={} complete={complete} partial={partial} missing={missing}",
        merged.len(),
        labeled.len()
    );
    ExitCode::SUCCESS
}

fn inventory_command(args: &[String]) -> ExitCode {
    let path =
        argument(args, "--features").unwrap_or_else(|| "ingestion/data/features_v15.jsonl".into());
    let rows = match parse_features(BufReader::new(match File::open(&path) {
        Ok(file) => file,
        Err(e) => {
            eprintln!("features: {e}");
            return ExitCode::FAILURE;
        }
    })) {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("features: {e}");
            return ExitCode::FAILURE;
        }
    };
    let labeled =
        research::read_labeled(std::path::Path::new("research/data/labeled_launches.jsonl")).ok();
    println!(
        "{}",
        serde_json::to_string_pretty(&inventory(&rows, labeled.as_deref())).unwrap()
    );
    ExitCode::SUCCESS
}

fn backtest_command(args: &[String]) -> ExitCode {
    let feature_path =
        argument(args, "--features").unwrap_or_else(|| "ingestion/data/features_v15.jsonl".into());
    let price_path = argument(args, "--prices");
    let report_path =
        argument(args, "--report").unwrap_or_else(|| "research/data/backtest_report.json".into());
    let fee = argument(args, "--fee-per-side")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let slippage = argument(args, "--slippage-per-side")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let features = match read_features(PathBuf::from(feature_path).as_path()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("features: {e}");
            return ExitCode::FAILURE;
        }
    };
    let prices = match price_path {
        Some(path) => {
            let file = match File::open(&path) {
                Ok(file) => file,
                Err(e) => {
                    eprintln!("prices: {e}");
                    return ExitCode::FAILURE;
                }
            };
            match if path.ends_with(".jsonl") {
                read_observations(BufReader::new(file))
            } else {
                read_prices(BufReader::new(file))
            } {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("prices: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        None => Vec::new(),
    };
    let mut labels: Vec<_> = features
        .into_iter()
        .map(|row| make_labeled(row, &prices))
        .collect();
    labels.sort_by_key(|row| row.creation_time_unix);
    let report = backtest(&labels, fee, slippage);
    if let Some(parent) = std::path::Path::new(&report_path).parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("report: {e}");
            return ExitCode::FAILURE;
        }
    }
    match std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()) {
        Ok(()) => {
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("report: {e}");
            ExitCode::FAILURE
        }
    }
}

fn argument(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].clone())
}
