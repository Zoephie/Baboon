use super::*;

fn outcome(path: &str, input: &str, result: Result<(), String>) -> FieldEditOutcome {
    FieldEditOutcome {
        path: path.to_owned(),
        input: input.to_owned(),
        result,
    }
}

/// The summary counts cells that applied, and names what failed. It used
/// to count every cell it tried.
#[test]
fn a_tsv_paste_reports_the_cells_that_failed() {
    let outcomes = [
        outcome("regions[0]/name", "hull", Ok(())),
        outcome(
            "regions[1]/lod",
            "high",
            Err("expected i16 value".to_owned()),
        ),
    ];
    let summary = tsv_paste_summary(&outcomes, 2, 0, 2);
    assert_eq!(
        summary,
        "Pasted 1 of 2 cell(s) across 2 row(s) — 1 failed; first: regions[1]/lod = \"high\": expected i16 value"
    );
    let clean = tsv_paste_summary(&outcomes[..1], 1, 3, 1);
    assert_eq!(
        clean,
        "Pasted 1 cell(s) across 1 row(s) — 3 extra row(s) ignored (block has 1 elements; add more first)"
    );
}
