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

    // Hand-rolled rather than clap: this is a build helper with six flags and
    // no dependency budget worth spending on it.
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut index = 0;
    while index < argv.len() {
        let flag = argv[index].clone();
        index += 1;
        let mut value = || {
            let Some(value) = argv.get(index).cloned().filter(|v| !v.starts_with("--")) else {
                eprintln!("{flag} needs a value");
                std::process::exit(2);
            };
            index += 1;
            value
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
            // The group name is optional, so a reviewer working one group at a
            // time is not handed the whole corpus.
            "--suggest-drops" => {
                let group = argv.get(index).cloned().filter(|v| !v.starts_with("--"));
                if group.is_some() {
                    index += 1;
                }
                suggest = Some(group);
            }
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
