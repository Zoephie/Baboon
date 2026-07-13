use super::*;

#[test]
fn parses_functions_overloads_metadata_and_external_globals() {
    let parsed = parse_document(
        "; AVAILABLE FUNCTIONS:\n\n(<void> wake <script>)\n; wakes a script.\n; NETWORK SAFE: Yes\n\n(<void> wake <script> [<boolean>])\n; alternate form.\n\n; AVAILABLE EXTERNAL GLOBALS:\n\n(<boolean> debug_enabled)\n; debug toggle.\n",
    );
    assert_eq!(parsed.functions.len(), 2);
    assert_eq!(parsed.functions[0].name, "wake");
    assert_eq!(parsed.functions[0].return_type, "void");
    assert_eq!(parsed.functions[0].network_safe.as_deref(), Some("Yes"));
    assert_eq!(parsed.functions[1].types, ["void", "script", "boolean"]);
    assert_eq!(parsed.globals.len(), 1);
    assert_eq!(parsed.globals[0].name, "debug_enabled");
    assert_eq!(parsed.globals[0].value_type, "boolean");
}

#[test]
fn extracts_lisp_and_c_style_examples_and_ignores_comments() {
    let documented = ["ai_place", "thread", "print"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    let text = "; (ai_place ignored)\n(script static void test\n (ai_place squad)\n)\n// thread(ignored());\nthread(run_loop());\nprint(\"hello\");\n";
    let examples = extract_examples(text, "sample.hsc", &documented);
    assert!(examples.iter().any(|example| {
        example.function_name == "ai_place" && example.code == "(ai_place squad)"
    }));
    assert!(
        examples
            .iter()
            .any(|example| example.function_name == "thread")
    );
    assert!(
        examples
            .iter()
            .any(|example| example.function_name == "print")
    );
    assert!(
        examples
            .iter()
            .all(|example| !example.code.contains("ignored"))
    );
}

#[test]
fn malformed_or_oversized_calls_are_not_examples() {
    let documented = ["print"].into_iter().map(str::to_owned).collect();
    let oversized = format!("(print {})", "x ".repeat(400));
    let text = format!("(print \"unterminated\"\n{oversized}");
    assert!(extract_examples(&text, "bad.hsc", &documented).is_empty());
}

#[test]
fn sha256_is_stable() {
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn committed_database_contains_every_game_and_valid_sources() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/script_docs.sqlite3");
    let connection = Connection::open(path).unwrap();
    let version: i64 = connection
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM metadata WHERE key='schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    let game_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM games", [], |row| row.get(0))
        .unwrap();
    assert_eq!(game_count, 7);
    for (_, game, _) in GAMES {
        let counts: (i64, i64, i64) = connection
            .query_row(
                "SELECT (SELECT COUNT(*) FROM functions WHERE game_id=?1),
                        (SELECT COUNT(*) FROM globals WHERE game_id=?1),
                        (SELECT COUNT(*) FROM types WHERE game_id=?1)",
                [game],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert!(counts.0 > 0, "{game} has no functions");
        assert!(counts.1 > 0, "{game} has no globals");
        assert!(counts.2 > 0, "{game} has no types");
    }
    let invalid_hashes: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM source_files WHERE length(sha256) != 64",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(invalid_hashes, 0);
    let orphan_examples: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM examples e WHERE NOT EXISTS(
                 SELECT 1 FROM source_files s WHERE s.game_id=e.game_id AND s.path=e.source_file AND s.kind='example')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(orphan_examples, 0);
}
