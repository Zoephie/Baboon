use super::*;

fn database_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/script_docs.sqlite3")
}

#[test]
fn opens_database_read_only_and_rejects_missing_database() {
    assert!(open_database(&database_path()).is_ok());
    let missing = std::env::temp_dir().join(format!(
        "baboon-missing-script-docs-{}.sqlite3",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&missing);
    assert!(open_database(&missing).is_err());
}

#[test]
fn function_search_prioritizes_exact_and_prefix_names() {
    let connection = open_database(&database_path()).unwrap();
    let rows = query_rows(
        &connection,
        "halo3_mcc",
        ScriptDocCategory::Functions,
        ScriptDocNetworkFilter::All,
        "sleep",
    )
    .unwrap();
    assert!(!rows.is_empty());
    assert_eq!(rows[0].name, "sleep");
}

#[test]
fn every_category_returns_rows_and_details() {
    let connection = open_database(&database_path()).unwrap();
    for category in [
        ScriptDocCategory::Functions,
        ScriptDocCategory::Globals,
        ScriptDocCategory::Types,
    ] {
        let rows = query_rows(
            &connection,
            "haloce_mcc",
            category,
            ScriptDocNetworkFilter::All,
            "",
        )
        .unwrap();
        assert!(!rows.is_empty());
        assert!(
            query_detail(&connection, "haloce_mcc", category, &rows[0].key)
                .unwrap()
                .is_some()
        );
    }
}

#[test]
fn network_safety_filter_separates_yes_unknown_and_no() {
    let connection = open_database(&database_path()).unwrap();
    for filter in [
        ScriptDocNetworkFilter::Yes,
        ScriptDocNetworkFilter::Unknown,
        ScriptDocNetworkFilter::No,
    ] {
        let rows = query_rows(
            &connection,
            "halo3_mcc",
            ScriptDocCategory::Functions,
            filter,
            "",
        )
        .unwrap();
        assert!(!rows.is_empty(), "{filter:?} returned no functions");
    }
}

#[test]
fn corrupt_or_unsupported_schema_is_reported() {
    let path = std::env::temp_dir().join(format!(
        "baboon-invalid-script-docs-{}.sqlite3",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    {
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);
                 INSERT INTO metadata VALUES('schema_version','999');",
            )
            .unwrap();
    }
    let error = open_database(&path).err().unwrap();
    assert!(error.contains("Unsupported script documentation schema"));
    let _ = std::fs::remove_file(path);
}
