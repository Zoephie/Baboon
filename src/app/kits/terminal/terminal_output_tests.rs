
use super::*;

fn collect_terminal_output(input: &[u8]) -> Vec<(&'static str, String)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = egui::Context::default();
    let mut log_file = None;
    let mut log_error_reported = false;
    let result = stream_terminal_output(
        std::io::Cursor::new(input),
        &tx,
        &ctx,
        &mut log_file,
        &mut log_error_reported,
    );
    assert!(result.is_ok());
    drop(tx);

    rx.try_iter()
        .filter_map(|message| match message {
            WorkerMessage::TerminalLine(line) => Some(("line", line)),
            _ => None,
        })
        .collect()
}

#[test]
fn terminal_output_appends_carriage_return_progress() {
    let output = collect_terminal_output(
        b"building bsp3d children... 10%\rbuilding bsp3d children... 70%\rbuilding bsp3d children... 100%\nnext\n",
    );

    assert_eq!(
        output,
        vec![
            ("line", "building bsp3d children... 10%".to_owned()),
            ("line", "building bsp3d children... 70%".to_owned()),
            ("line", "building bsp3d children... 100%".to_owned()),
            ("line", "next".to_owned()),
        ]
    );
}

#[test]
fn terminal_output_treats_crlf_as_newline() {
    let output = collect_terminal_output(b"done\r\nnext");

    assert_eq!(
        output,
        vec![("line", "done".to_owned()), ("line", "next".to_owned()),]
    );
}

#[test]
fn terminal_output_full_log_keeps_carriage_return_progress() {
    let path = std::env::temp_dir().join(format!(
        "baboon-terminal-test-{}.log",
        terminal_log_timestamp()
    ));
    let file = std::fs::File::create(&path);
    assert!(file.is_ok());

    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = egui::Context::default();
    let mut log_file = file.ok();
    let mut log_error_reported = false;
    let result = stream_terminal_output(
        std::io::Cursor::new(b"building... 10%\rbuilding... 60%\rbuilding... 100%\n"),
        &tx,
        &ctx,
        &mut log_file,
        &mut log_error_reported,
    );
    assert!(result.is_ok());
    drop(log_file);
    drop(tx);

    let output: Vec<_> = rx
        .try_iter()
        .filter_map(|message| match message {
            WorkerMessage::TerminalLine(line) => Some(("line", line)),
            _ => None,
        })
        .collect();
    assert_eq!(
        output,
        vec![
            ("line", "building... 10%".to_owned()),
            ("line", "building... 60%".to_owned()),
            ("line", "building... 100%".to_owned()),
        ]
    );

    let text = std::fs::read_to_string(&path);
    assert!(text.is_ok());
    if let Ok(text) = text {
        assert!(text.contains("building... 10%\n"));
        assert!(text.contains("building... 60%\n"));
        assert!(text.contains("building... 100%\n"));
    }
    let _ = std::fs::remove_file(path);
}

#[test]
fn terminal_output_handles_heavy_carriage_return_progress() {
    let mut input = Vec::new();
    for index in 0..25_000 {
        input.extend_from_slice(format!("building bsp3d children... {index}%\r").as_bytes());
    }
    input.extend_from_slice(b"done\n");

    let output = collect_terminal_output(&input);

    assert_eq!(output.len(), 25_001);
    assert_eq!(
        output.first(),
        Some(&("line", "building bsp3d children... 0%".to_owned()))
    );
    assert_eq!(output.last(), Some(&("line", "done".to_owned())));
}

#[test]
fn terminal_line_severity_classifies_tool_markers() {
    assert!(matches!(
        TerminalLineEntry::new("-ERROR- bad connection".to_owned()).severity,
        TerminalLineSeverity::Error
    ));
    assert!(matches!(
        TerminalLineEntry::new("WARNING overlapping surfaces".to_owned()).severity,
        TerminalLineSeverity::Warning
    ));
    assert!(matches!(
        TerminalLineEntry::new("[exit 0]".to_owned()).severity,
        TerminalLineSeverity::Success
    ));
    assert!(matches!(
        TerminalLineEntry::new("[exit 2]".to_owned()).severity,
        TerminalLineSeverity::Error
    ));
    assert!(matches!(
        TerminalLineEntry::new("=== summary".to_owned()).severity,
        TerminalLineSeverity::Summary
    ));
}

#[test]
fn terminal_visible_lines_are_trimmed_to_limit() {
    let mut lines = Vec::new();
    for index in 0..(TERMINAL_VISIBLE_LINE_LIMIT + 10) {
        lines.push(TerminalLineEntry::new(format!("line {index}")));
    }

    trim_terminal_lines(&mut lines);

    assert_eq!(lines.len(), TERMINAL_VISIBLE_LINE_TRIM_TARGET);
    assert_eq!(
        lines.first().map(|line| line.text.as_str()),
        Some("line 2010")
    );
}
