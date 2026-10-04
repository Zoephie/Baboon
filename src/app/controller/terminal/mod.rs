//! Terminal line retention, full-log persistence, and log-file launching.
//! It owns application actions and workflow coordination; widget layout and persistent state definitions belong elsewhere.

use super::*;

impl Baboon {
    /// Applies `WorkerMessage::TerminalLine` without changing receive-loop ordering.
    pub(super) fn handle_terminal_line(&mut self, line: String) -> bool {
        self.push_terminal_line(line);
        false
    }

    /// Applies `WorkerMessage::TerminalLogError` without changing receive-loop ordering.
    pub(super) fn handle_terminal_log_error(&mut self, error: String) -> bool {
        self.status = error;
        false
    }

    /// Applies `WorkerMessage::TerminalDone`; stale run IDs skip the rest of that loop iteration.
    pub(super) fn handle_terminal_done(&mut self, run_id: u64) -> bool {
        if self.terminal.running_id != Some(run_id) {
            return true;
        }
        self.terminal.running = false;
        self.terminal.running_id = None;
        self.terminal.running_command = None;
        self.terminal.process = None;
        self.terminal.scroll_to_bottom = true;
        self.terminal.refocus_input = true;
        false
    }
}

pub(super) enum TerminalStopResult {
    Stopped,
    AlreadyExited,
}

pub(super) fn stop_terminal_process(
    process: &TerminalProcess,
) -> Result<TerminalStopResult, String> {
    #[cfg(target_os = "windows")]
    {
        stop_terminal_process_windows(process)
    }
    #[cfg(not(target_os = "windows"))]
    {
        stop_terminal_process_unix(process)
    }
}

#[cfg(target_os = "windows")]
fn stop_terminal_process_windows(process: &TerminalProcess) -> Result<TerminalStopResult, String> {
    let pid = {
        let mut slot = process
            .child
            .lock()
            .map_err(|_| "terminal process lock was poisoned".to_owned())?;
        let Some(child) = slot.as_mut() else {
            return Ok(TerminalStopResult::AlreadyExited);
        };
        match child
            .try_wait()
            .map_err(|error| format!("could not query terminal process: {error}"))?
        {
            Some(_) => {
                *slot = None;
                return Ok(TerminalStopResult::AlreadyExited);
            }
            None => child.id(),
        }
    };
    let output = background_command("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .output()
        .map_err(|error| format!("could not launch taskkill: {error}"))?;
    if output.status.success() {
        return Ok(TerminalStopResult::Stopped);
    }
    if terminal_process_already_exited(process)? {
        return Ok(TerminalStopResult::AlreadyExited);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let detail = if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!("taskkill exited with {}", output.status)
    };
    Err(detail)
}

#[cfg(not(target_os = "windows"))]
fn stop_terminal_process_unix(process: &TerminalProcess) -> Result<TerminalStopResult, String> {
    let mut slot = process
        .child
        .lock()
        .map_err(|_| "terminal process lock was poisoned".to_owned())?;
    let Some(child) = slot.as_mut() else {
        return Ok(TerminalStopResult::AlreadyExited);
    };
    match child
        .try_wait()
        .map_err(|error| format!("could not query terminal process: {error}"))?
    {
        Some(_) => {
            *slot = None;
            Ok(TerminalStopResult::AlreadyExited)
        }
        None => {
            let pid = i32::try_from(child.id())
                .map_err(|_| format!("terminal process id {} is out of range", child.id()))?;
            let process_group = -pid;
            // The shell was spawned with `process_group(0)`, so its children
            // inherit the same process group. A negative pid targets the group.
            let result = unsafe { libc::kill(process_group, libc::SIGKILL) };
            if result == 0 {
                return Ok(TerminalStopResult::Stopped);
            }
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                if child.try_wait().ok().flatten().is_some() {
                    *slot = None;
                }
                return Ok(TerminalStopResult::AlreadyExited);
            }
            Err(format!(
                "could not kill terminal process group {pid}: {error}"
            ))
        }
    }
}

#[cfg(target_os = "windows")]
fn terminal_process_already_exited(process: &TerminalProcess) -> Result<bool, String> {
    let mut slot = process
        .child
        .lock()
        .map_err(|_| "terminal process lock was poisoned".to_owned())?;
    let Some(child) = slot.as_mut() else {
        return Ok(true);
    };
    match child
        .try_wait()
        .map_err(|error| format!("could not query terminal process: {error}"))?
    {
        Some(_) => {
            *slot = None;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// The `cmd /S /C "<command> 2>&1"` line a terminal command runs as on
/// Windows. It is passed raw: `Command::arg` would escape the command's own
/// quotes as `\"`, which cmd doesn't read but the tool's C runtime does, so a
/// quoted path with a space (`-tags_dir "D:\tags resolved"`) reached the tool
/// split in two with a literal `"` on each half. `/S` strips just the outer
/// pair of quotes and leaves the command as typed.
#[cfg(any(target_os = "windows", test))]
fn windows_shell_command_line(command: &str) -> String {
    format!("/S /C \"{command} 2>&1\"")
}

/// `cmd` running `command` as typed, with no console window; see
/// [`windows_shell_command_line`].
#[cfg(target_os = "windows")]
pub(super) fn windows_shell_command(command: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut c = background_command("cmd");
    c.raw_arg(windows_shell_command_line(command));
    c
}

/// The shell a terminal command runs in, in `work_dir`, its output piped
/// and its input closed.
///
/// On Windows that is `cmd` with the command line passed raw (see
/// [`windows_shell_command_line`]). Elsewhere it is `sh -c` in a new process
/// group, so stopping the command kills whatever it started as well.
pub(super) fn terminal_shell_command(command: &str, work_dir: &Path) -> std::process::Command {
    #[cfg(target_os = "windows")]
    let mut cmd = windows_shell_command(command);
    #[cfg(not(target_os = "windows"))]
    let mut cmd = {
        #[cfg(unix)]
        use std::os::unix::process::CommandExt;
        let mut c = std::process::Command::new("sh");
        c.args(["-c", command]);
        #[cfg(unix)]
        c.process_group(0);
        c
    };
    cmd.current_dir(work_dir)
        .stdout(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null());
    cmd
}

pub(super) fn run_terminal_command_for_reimport(
    command: &str,
    work_dir: &Path,
    tx: &Sender<WorkerMessage>,
    ctx: &egui::Context,
    mut log_file: Option<std::fs::File>,
) -> Result<(), String> {
    let mut log_error_reported = false;
    #[cfg(target_os = "windows")]
    let mut cmd = windows_shell_command(command);
    #[cfg(not(target_os = "windows"))]
    let mut cmd = {
        let mut c = std::process::Command::new("sh");
        c.args(["-c", command]);
        c
    };
    cmd.current_dir(work_dir)
        .stdout(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null());
    let mut child = cmd.spawn().map_err(|error| {
        let message = format!("[error] {error}");
        send_terminal_line(
            tx,
            ctx,
            &mut log_file,
            &mut log_error_reported,
            message.clone(),
        );
        message
    })?;
    if let Some(stdout) = child.stdout.take() {
        if let Err(error) =
            stream_terminal_output(stdout, tx, ctx, &mut log_file, &mut log_error_reported)
        {
            let message = format!("Could not read tool output: {error}");
            send_terminal_line(
                tx,
                ctx,
                &mut log_file,
                &mut log_error_reported,
                format!("[error] {message}"),
            );
            return Err(message);
        }
    }
    let status = child
        .wait()
        .map_err(|error| format!("Could not wait for tool: {error}"))?;
    if let Some(code) = status.code() {
        send_terminal_line(
            tx,
            ctx,
            &mut log_file,
            &mut log_error_reported,
            format!("[exit {code}]"),
        );
    }
    if status.success() {
        Ok(())
    } else {
        Err(status
            .code()
            .map(|code| format!("tool exited with code {code}"))
            .unwrap_or_else(|| "tool exited without a status code".to_owned()))
    }
}

pub(super) fn stream_terminal_output<R: std::io::Read>(
    mut reader: R,
    tx: &Sender<WorkerMessage>,
    ctx: &egui::Context,
    log_file: &mut Option<std::fs::File>,
    log_error_reported: &mut bool,
) -> std::io::Result<()> {
    let mut buffer = [0_u8; 4096];
    let mut line = Vec::new();
    let mut pending_cr = false;

    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        for &byte in &buffer[..read] {
            if pending_cr {
                if byte == b'\n' {
                    emit_terminal_output_line(&line, tx, ctx, log_file, log_error_reported);
                    line.clear();
                    pending_cr = false;
                    continue;
                }
                emit_terminal_output_line(&line, tx, ctx, log_file, log_error_reported);
                line.clear();
                pending_cr = false;
            }
            match byte {
                b'\r' => pending_cr = true,
                b'\n' => {
                    emit_terminal_output_line(&line, tx, ctx, log_file, log_error_reported);
                    line.clear();
                }
                _ => line.push(byte),
            }
        }
    }
    if pending_cr {
        emit_terminal_output_line(&line, tx, ctx, log_file, log_error_reported);
        line.clear();
    }
    if !line.is_empty() {
        emit_terminal_output_line(&line, tx, ctx, log_file, log_error_reported);
    }
    Ok(())
}

fn emit_terminal_output_line(
    line: &[u8],
    tx: &Sender<WorkerMessage>,
    ctx: &egui::Context,
    log_file: &mut Option<std::fs::File>,
    log_error_reported: &mut bool,
) {
    let line = String::from_utf8_lossy(line).into_owned();
    write_terminal_log_line(log_file, tx, line.as_str(), log_error_reported);
    let _ = tx.send(WorkerMessage::TerminalLine(line));
    ctx.request_repaint();
}

pub(super) fn send_terminal_line(
    tx: &Sender<WorkerMessage>,
    ctx: &egui::Context,
    log_file: &mut Option<std::fs::File>,
    log_error_reported: &mut bool,
    line: String,
) {
    write_terminal_log_line(log_file, tx, &line, log_error_reported);
    let _ = tx.send(WorkerMessage::TerminalLine(line));
    ctx.request_repaint();
}

pub(super) fn trim_terminal_lines(lines: &mut Vec<TerminalLineEntry>) {
    if lines.len() > TERMINAL_VISIBLE_LINE_LIMIT {
        let remove = lines.len() - TERMINAL_VISIBLE_LINE_TRIM_TARGET;
        lines.drain(..remove);
    }
}

pub(super) fn create_terminal_log_file(
    run_id: u64,
    command: &str,
) -> Result<(PathBuf, std::fs::File), String> {
    use std::io::Write as _;

    let dir = terminal_logs_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    let path = dir.join(format!(
        "terminal-run-{}-{run_id}.log",
        terminal_log_timestamp()
    ));
    let mut file = std::fs::File::create(&path)
        .map_err(|error| format!("could not create {}: {error}", path.display()))?;
    writeln!(file, "> {command}")
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    Ok((path, file))
}

pub(super) fn write_terminal_log_line(
    log_file: &mut Option<std::fs::File>,
    tx: &Sender<WorkerMessage>,
    line: &str,
    log_error_reported: &mut bool,
) {
    use std::io::Write as _;

    let Some(file) = log_file.as_mut() else {
        return;
    };
    if let Err(error) = writeln!(file, "{line}") {
        *log_file = None;
        if !*log_error_reported {
            *log_error_reported = true;
            let _ = tx.send(WorkerMessage::TerminalLogError(format!(
                "Terminal full log disabled: {error}"
            )));
        }
    }
}

pub(super) fn append_terminal_log_path(path: &Path, line: &str) -> Result<(), String> {
    use std::io::Write as _;

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|error| format!("Could not open terminal log {}: {error}", path.display()))?;
    writeln!(file, "{line}")
        .map_err(|error| format!("Could not write terminal log {}: {error}", path.display()))
}

pub(super) fn terminal_log_timestamp() -> String {
    let seconds = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration.as_secs(),
        Err(_) => 0,
    };
    let days = (seconds / 86_400) as i64;
    let day_seconds = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = day_seconds / 3_600;
    let minute = (day_seconds % 3_600) / 60;
    let second = day_seconds % 60;
    format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}")
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year as i32, m as u32, d as u32)
}

pub(in crate::app) fn open_terminal_log(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = background_command("cmd");
        command.arg("/C").arg("start").arg("").arg(path);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command.arg(path);
        command
    };
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(path);
        command
    };

    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not open terminal log {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
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
        let command = format!(
            r#""{}" app::controller::terminal::tests::argv_echo_child --exact --nocapture --test-threads=1 tags_dir "D:\Chelan 1\tags resolved" bitmaps "characters\x""#,
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
}
