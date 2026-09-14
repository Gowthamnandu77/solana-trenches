use research::{
    backtest, inventory, make_labeled, parse_features, read_features, read_prices, score_report,
    write_jsonl, write_observations, QualityDecision,
};
use std::{env, fs::File, io::BufReader, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    let args: Vec<_> = env::args().collect();
    if args.get(1).map(String::as_str) == Some("inventory") {
        return inventory_command(&args);
    }
    if args.get(1).map(String::as_str) == Some("backtest") {
        return backtest_command(&args);
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
        Some(path) => match read_prices(BufReader::new(match File::open(path) {
            Ok(file) => file,
            Err(e) => {
                eprintln!("prices: {e}");
                return ExitCode::FAILURE;
            }
        })) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("prices: {e}");
                return ExitCode::FAILURE;
            }
        },
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
