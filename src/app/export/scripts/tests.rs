use super::*;

#[test]
fn source_text_cuts_at_the_first_terminator() {
    assert_eq!(source_text(b"(script foo)\0"), b"(script foo)");
    assert_eq!(source_text(b"(script foo)\0\0\0"), b"(script foo)");
    // Everything past the first NUL is padding the engine never reads, so
    // it must not reach the text file either.
    assert_eq!(source_text(b"a\0b\0"), b"a");
    assert_eq!(source_text(b"a\0\0b"), b"a");
    // No NUL at all (a blob written by something else) is still just text.
    assert_eq!(source_text(b"(script foo)"), b"(script foo)");
    assert_eq!(source_text(b""), b"");
    assert_eq!(source_text(b"\0\0"), b"");
}

/// Whatever the blob held, the bytes written to disk are NUL-free.
#[test]
fn extracted_text_never_contains_a_nul() {
    for blob in [
        b"(script foo)\0".as_slice(),
        b"a\0b\0".as_slice(),
        b"\0".as_slice(),
        b"plain".as_slice(),
    ] {
        assert!(
            !source_text(blob).contains(&0),
            "{blob:?} produced text with a NUL"
        );
    }
}

/// A `.hsc` that already carries a terminator must not get a second one.
#[test]
fn import_does_not_double_the_terminator() {
    let dir = scratch(
        "term",
        &[
            ("plain.hsc", b"(script foo)".as_slice()),
            ("already.hsc", b"(script bar)\0".as_slice()),
        ],
    );
    let mut tag = TagFile::new(test_definition_path("haloce_evolved/scenario.json")).unwrap();
    replace_scenario_scripts(&mut tag, &dir).expect("import");
    let files = read_source_files(&tag).unwrap();
    assert_eq!(files.len(), 2);
    for (name, body) in &files {
        assert_eq!(body.last(), Some(&0), "{name} should be terminated");
        assert_eq!(
            body.iter().filter(|byte| **byte == 0).count(),
            1,
            "{name} should carry exactly one terminator, got {body:?}"
        );
    }
    // Both bodies are the same script text plus one terminator, so the file
    // that arrived with a terminator and the one without land identically.
    assert_eq!(files[0].1, b"(script bar)\0");
    assert_eq!(files[1].1, b"(script foo)\0");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn file_names_are_sanitised_and_never_empty() {
    assert_eq!(hsc_file_name("a30", 0), "a30.hsc");
    // A path separator in a source-file name must not escape the folder.
    assert_eq!(hsc_file_name("solo/a30", 3), "solo_a30.hsc");
    assert_eq!(hsc_file_name("   ", 7), "source_7.hsc");
    assert_eq!(hsc_file_name("", 2), "source_2.hsc");
}

#[test]
fn only_scenario_tags_are_accepted() {
    assert!(is_scenario_group(u32::from_be_bytes(*b"scnr")));
    assert!(!is_scenario_group(u32::from_be_bytes(*b"bipd")));
}

/// Build a scratch folder of `.hsc` files under a unique name.
fn scratch(tag: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("baboon_hsc_test_{tag}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for (name, body) in files {
        fs::write(dir.join(name), body).unwrap();
    }
    dir
}

/// The always-runs gate: import a folder of `.hsc` into a real (schema-built)
/// CE scenario, extract it back out, and require the files to come back byte
/// for byte — then require a perturbed file to be detected.
///
/// Built from the shipped schema rather than a scenario corpus so this runs
/// everywhere. The corpus test below is an *additional* check, and it skips
/// when the corpus is absent — which is why this one must not.
#[test]
fn hsc_folder_round_trips_through_a_scenario_tag() {
    let mut tag = TagFile::new(test_definition_path("haloce_evolved/scenario.json"))
        .expect("CE scenario schema builds a default tag");

    // Content chosen to break naive handling: CRLF, a trailing newline, a
    // `;*  *;` block comment, and a body that is already NUL-free.
    let source = scratch(
        "in",
        &[
            (
                "a30.hsc",
                b"(script static void foo\r\n\t(print \"hi\")\r\n)\r\n".as_slice(),
            ),
            (
                "b_globals.hsc",
                b";* commented\nout *;\n(global short g 1)\n".as_slice(),
            ),
            ("c_empty_ish.hsc", b"; only a comment\n".as_slice()),
            ("ignored.txt", b"not haloscript".as_slice()),
        ],
    );

    let message = replace_scenario_scripts(&mut tag, &source).expect("import");
    assert!(message.contains("Imported 3 script file(s)"), "{message}");
    let imported = read_source_files(&tag).expect("source files");
    assert_eq!(imported.len(), 3, "the .txt must not be imported");
    // Sorted by name, and each body carries exactly one terminator.
    assert_eq!(
        imported
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["a30", "b_globals", "c_empty_ish"]
    );
    for (name, body) in &imported {
        assert_eq!(body.last(), Some(&0), "{name} should be NUL-terminated");
        assert_eq!(
            body.iter().filter(|byte| **byte == 0).count(),
            1,
            "{name} should carry exactly one terminator"
        );
    }

    let out = std::env::temp_dir().join("baboon_hsc_test_out");
    let _ = fs::remove_dir_all(&out);
    write_source_files(&tag, &out, "fixture").expect("extract");

    // Every .hsc must come back byte-identical to what went in.
    for name in ["a30.hsc", "b_globals.hsc", "c_empty_ish.hsc"] {
        let before = fs::read(source.join(name)).unwrap();
        let after = fs::read(out.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(after, before, "{name} did not round-trip");
    }
    assert!(!out.join("ignored.txt").exists());

    // Re-importing the extracted folder must land on the same block.
    replace_scenario_scripts(&mut tag, &out).expect("re-import");
    assert_eq!(
        read_source_files(&tag).unwrap(),
        imported,
        "second pass diverged"
    );

    // Negative control: the comparison above has to be capable of failing.
    fs::write(out.join("a30.hsc"), b"(script static void changed)\n").unwrap();
    replace_scenario_scripts(&mut tag, &out).expect("import perturbed");
    assert_ne!(
        read_source_files(&tag).unwrap(),
        imported,
        "a changed .hsc produced an identical block — this test proves nothing"
    );

    let _ = fs::remove_dir_all(&source);
    let _ = fs::remove_dir_all(&out);
}

/// Same round-trip against real shipped scenarios. Additional coverage over
/// the schema-built fixture above; skips loudly when the corpus is absent.
#[test]
fn extract_then_import_round_trips_shipped_scenarios() {
    let Ok(dir) = std::env::var("CE_SCENARIO_DIR") else {
        eprintln!("skipping: set CE_SCENARIO_DIR to a folder of .scenario tags");
        return;
    };
    let scenarios: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("CE_SCENARIO_DIR is readable")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            (path.extension()?.eq_ignore_ascii_case("scenario")).then_some(path)
        })
        .collect();
    assert!(!scenarios.is_empty(), "no .scenario tags in {dir}");

    for path in &scenarios {
        let label = path.display().to_string();
        let mut tag = TagFile::read(path).expect("scenario reads");
        let before = read_source_files(&tag).expect("has source files");
        if before.is_empty() {
            continue;
        }

        let out = std::env::temp_dir().join(format!(
            "baboon_hsc_{}",
            path.file_stem().unwrap().to_string_lossy()
        ));
        let _ = fs::remove_dir_all(&out);
        write_source_files(&tag, &out, &label).expect("extract");
        replace_scenario_scripts(&mut tag, &out).expect("import");
        let after = read_source_files(&tag).expect("still has source files");

        // Extraction skips empty bodies, so compare against what was written.
        let expected: Vec<(String, Vec<u8>)> = {
            let mut kept: Vec<(String, Vec<u8>)> = before
                .iter()
                .enumerate()
                .filter(|(_, (_, body))| !source_text(body).is_empty())
                .map(|(index, (name, body))| {
                    let mut normalised = source_text(body).to_vec();
                    normalised.push(0);
                    // The name survives only as far as the file name does.
                    let stem = hsc_file_name(name, index)
                        .trim_end_matches(".hsc")
                        .to_owned();
                    (stem, normalised)
                })
                .collect();
            kept.sort_by(|(a, _), (b, _)| a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()));
            kept
        };
        assert_eq!(after.len(), expected.len(), "{label}: element count");
        for (index, ((got_name, got), (want_name, want))) in
            after.iter().zip(expected.iter()).enumerate()
        {
            assert_eq!(got_name, want_name, "{label}: name of element {index}");
            assert_eq!(got, want, "{label}: body of element {index}");
        }

        // Negative control: the comparison above must be capable of failing.
        // Perturb one byte on disk and confirm the re-import no longer matches.
        let first = fs::read_dir(&out)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|x| x == "hsc"))
            .expect("at least one .hsc was written");
        let mut perturbed = fs::read(&first).unwrap();
        perturbed.extend_from_slice(b"\n; control\n");
        fs::write(&first, &perturbed).unwrap();
        replace_scenario_scripts(&mut tag, &out).expect("re-import");
        let control = read_source_files(&tag).expect("source files");
        assert_ne!(
            control, expected,
            "{label}: control did not diverge — the round-trip check proves nothing"
        );

        let _ = fs::remove_dir_all(&out);
    }
}
