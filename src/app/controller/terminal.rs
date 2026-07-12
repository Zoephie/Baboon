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
    let output = Command::new("taskkill")
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

pub(super) fn run_terminal_command_for_reimport(
    command: &str,
    work_dir: &Path,
    tx: &Sender<WorkerMessage>,
    ctx: &egui::Context,
    mut log_file: Option<std::fs::File>,
) -> Result<(), String> {
    let mut log_error_reported = false;
    #[cfg(target_os = "windows")]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut c = std::process::Command::new("cmd");
        c.creation_flags(CREATE_NO_WINDOW);
        c.args(["/C", &format!("{command} 2>&1")]);
        c
    };
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
        let mut command = std::process::Command::new("cmd");
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
