use research::{
    make_labeled, read_features, read_prices, score_report, write_jsonl, write_observations,
    QualityDecision,
};
use std::{env, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    let args: Vec<_> = env::args().collect();
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

fn argument(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].clone())
}
