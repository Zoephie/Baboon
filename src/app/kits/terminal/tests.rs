use super::*;

/// What a tool's argv is after cmd runs `line`: `/S /C` drops the first
/// and last quote, then the C runtime splits the rest (`\"` is a literal
/// quote, `"` toggles quoting, backslashes elsewhere are literal).
fn tool_argv(line: &str) -> Vec<String> {
    let body = line.strip_prefix("/S /C ").unwrap();
    let body = body.strip_prefix('"').unwrap();
    let body = &body[..body.rfind('"').unwrap()];
    let body = body.strip_suffix(" 2>&1").unwrap();
    let (mut args, mut current, mut quoted) = (Vec::new(), String::new(), false);
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'"') => current.push(chars.next().unwrap()),
            '"' => quoted = !quoted,
            ' ' if !quoted => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

#[test]
fn quoted_folders_with_spaces_reach_the_tool_whole() {
    let command = r#"tool -tags_dir "D:\Chelan_1\tags resolved" -data_dir "D:\Chelan_1\data vanilla" bitmaps "characters\x""#;
    assert_eq!(
        tool_argv(&windows_shell_command_line(command)),
        [
            "tool",
            "-tags_dir",
            r"D:\Chelan_1\tags resolved",
            "-data_dir",
            r"D:\Chelan_1\data vanilla",
            "bitmaps",
            r"characters\x",
        ]
    );
    // The line `Command::arg` built, quoting the argument and escaping the
    // command's quotes, is what split the folders in the reported failure.
    let escaped = format!("/S /C \"{} 2>&1\"", command.replace('"', "\\\""));
    let argv = tool_argv(&escaped);
    assert_eq!(argv[3], "resolved\"");
    assert_eq!(argv[6], "vanilla\"");
}

/// What a terminal command is launched as: program, arguments and folder.
#[test]
fn a_terminal_command_runs_in_the_platform_shell_in_its_folder() {
    let folder = std::env::temp_dir();
    let command = terminal_shell_command(r#"tool -tags_dir "D:\a b" x"#, &folder);
    let args: Vec<String> = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert_eq!(command.get_current_dir(), Some(folder.as_path()));
    #[cfg(windows)]
    {
        assert_eq!(command.get_program(), "cmd");
        assert_eq!(args, [r#"/S /C "tool -tags_dir "D:\a b" x 2>&1""#]);
    }
    #[cfg(not(windows))]
    {
        assert_eq!(command.get_program(), "sh");
        assert_eq!(args, ["-c", r#"tool -tags_dir "D:\a b" x"#]);
    }
}

/// Stop kills a running command (and on Windows its process tree) and
/// says so; the process is gone within seconds, and a second stop finds
/// nothing left to stop.
#[test]
fn stopping_a_terminal_command_ends_it() {
    #[cfg(windows)]
    let command = "ping -n 30 127.0.0.1";
    #[cfg(not(windows))]
    let command = "sleep 30";
    let child = terminal_shell_command(command, &std::env::temp_dir())
        .spawn()
        .expect("start the command");
    let process = TerminalProcess {
        child: Arc::new(Mutex::new(Some(child))),
        stop_requested: Arc::new(AtomicBool::new(false)),
    };
    // Still running before the stop: the stop is what ends it.
    assert!(
        process
            .child
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .try_wait()
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        stop_terminal_process(&process),
        Ok(TerminalStopResult::Stopped)
    ));
    let started = std::time::Instant::now();
    loop {
        let exited = process
            .child
            .lock()
            .unwrap()
            .as_mut()
            .map(|child| child.try_wait().unwrap().is_some())
            .unwrap_or(true);
        if exited {
            break;
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "the command was still running 5 s after Stop"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(matches!(
        stop_terminal_process(&process),
        Ok(TerminalStopResult::AlreadyExited)
    ));
}

const ARGV_CHILD: &str = "BABOON_ARGV_ECHO_CHILD";

/// The real thing behind `tool_argv`: `cmd` runs a command line naming
/// this test binary, which reports the argv its C runtime split out.
/// Quoted folders with spaces must arrive whole; the line `Command::arg`
/// would have built must not.
#[cfg(windows)]
#[test]
fn cmd_hands_a_program_its_quoted_arguments_whole() {
    let exe = std::env::current_exe().expect("test binary");
    // libtest reads every positional argument as a filter; with `--exact`
    // none of these match a test, so only the echo runs.
    // The echo's name as libtest filters it, from this module's own path: a
    // name written out by hand went stale when the module moved, and then
    // the child ran no test and echoed nothing.
    let module = module_path!().split_once("::").map_or(module_path!(), |(_, rest)| rest);
    let command = format!(
        r#""{}" {module}::argv_echo_child --exact --nocapture --test-threads=1 tags_dir "D:\Chelan 1\tags resolved" bitmaps "characters\x""#,
        exe.display()
    );
    let echoed = |mut command: std::process::Command| -> Option<Vec<String>> {
        let output = command.env(ARGV_CHILD, "1").output().expect("run cmd");
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        let json = text.lines().find_map(|line| line.split_once("ARGV ").map(|(_, json)| json.to_owned()))?;
        let argv: Vec<String> = serde_json::from_str(json.trim()).ok()?;
        Some(argv[argv.len().saturating_sub(4)..].to_vec())
    };
    let expected = vec![
        "tags_dir".to_owned(),
        r"D:\Chelan 1\tags resolved".to_owned(),
        "bitmaps".to_owned(),
        r"characters\x".to_owned(),
    ];
    assert_eq!(echoed(windows_shell_command(&command)), Some(expected.clone()));

    let mut escaped = background_command("cmd");
    escaped.args(["/S", "/C", &format!("{command} 2>&1")]);
    assert_ne!(echoed(escaped), Some(expected), "escaped quotes split the folders");
}

/// The echoing half: a no-op in a normal run.
#[test]
fn argv_echo_child() {
    if std::env::var_os(ARGV_CHILD).is_none() {
        return;
    }
    let argv: Vec<String> = std::env::args().collect();
    println!("ARGV {}", serde_json::to_string(&argv).unwrap());
}
