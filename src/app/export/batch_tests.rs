use super::*;

fn batch(written: usize, failures: usize) -> BatchExport {
    BatchExport {
        written,
        tags: written,
        failures: (0..failures)
            .map(|index| format!("objects/t{index}.bitmap: no images"))
            .collect(),
    }
}

fn finish(batch: BatchExport) -> anyhow::Result<String> {
    batch.finish("none found", "all failed", |written, tags| {
        format!("Wrote {written} from {tags}")
    })
}

/// A partly failed export says which tags failed, not only how many.
#[test]
fn a_batch_export_names_what_failed() {
    assert_eq!(finish(batch(4, 0)).unwrap(), "Wrote 4 from 4");
    assert_eq!(
        finish(batch(4, 2)).unwrap(),
        "Wrote 4 from 4; 2 failed: objects/t0.bitmap: no images; \
             objects/t1.bitmap: no images"
    );
    let many = finish(batch(1, 5)).unwrap();
    assert!(many.contains("objects/t2.bitmap") && !many.contains("objects/t3.bitmap"));
    assert!(many.ends_with("; and 2 more"), "{many}");

    let all = finish(batch(0, 2)).unwrap_err().to_string();
    assert!(all.starts_with("all failed: objects/t0.bitmap"), "{all}");
    assert_eq!(finish(batch(0, 0)).unwrap_err().to_string(), "none found");
}
