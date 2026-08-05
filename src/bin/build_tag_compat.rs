#[path = "../tag_compat_build.rs"]
mod tag_compat_build;

use std::path::PathBuf;

fn main() {
    let (mut definitions, mut mappings, mut output, mut csv) = tag_compat_build::default_paths();
    let mut pairs: Vec<(String, String)> = tag_compat_build::DEFAULT_PAIRS
        .iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect();
    let mut suggest: Option<Option<String>> = None;

    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || {
            args.next().unwrap_or_else(|| {
                eprintln!("{flag} needs a value");
                std::process::exit(2);
            })
        };
        match flag.as_str() {
            "--definitions" => definitions = PathBuf::from(value()),
            "--mappings" => mappings = PathBuf::from(value()),
            "--out" => output = PathBuf::from(value()),
            "--csv" => csv = PathBuf::from(value()),
            "--pairs" => {
                pairs = value()
                    .split(',')
                    .filter_map(|pair| pair.split_once(':'))
                    .map(|(a, b)| (a.trim().to_owned(), b.trim().to_owned()))
                    .collect();
            }
            // Optional group filter, so a reviewer working one group at a time
            // is not handed the whole corpus.
            "--suggest-drops" => suggest = Some(std::env::args().nth(2).filter(|a| !a.starts_with("--"))),
            "--help" | "-h" => {
                eprintln!(
                    "usage: build_tag_compat [--definitions DIR] [--mappings FILE] \
                     [--out SQLITE] [--csv FILE] [--pairs a:b,c:d] [--suggest-drops [GROUP]]"
                );
                return;
            }
            other => {
                eprintln!("unknown flag {other}");
                std::process::exit(2);
            }
        }
    }

    if pairs.is_empty() {
        eprintln!("no profile pairs to compare");
        std::process::exit(2);
    }

    let reports = match tag_compat_build::build_database(&definitions, &mappings, &pairs, &output) {
        Ok(reports) => reports,
        Err(error) => {
            eprintln!("tag compatibility build failed: {error}");
            std::process::exit(1);
        }
    };

    if let Some(group) = suggest {
        print!("{}", tag_compat_build::suggest_drops(&reports, group.as_deref()));
        return;
    }

    if let Err(error) = tag_compat_build::write_csv(&reports, &csv) {
        eprintln!("csv export failed: {error}");
        std::process::exit(1);
    }
    println!("wrote {} and {}", output.display(), csv.display());
}
