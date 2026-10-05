//! Terminal line retention, full-log persistence, and log-file launching.
//! It owns application actions and workflow coordination; widget layout and persistent state definitions belong elsewhere.

use super::*;

pub(in crate::app) const TERMINAL_VISIBLE_LINE_LIMIT: usize = 20_000;
pub(in crate::app) const TERMINAL_VISIBLE_LINE_TRIM_TARGET: usize = 18_000;

impl Baboon {
    /// Applies `WorkerMessage::TerminalLine` without changing receive-loop ordering.
    pub(in crate::app) fn handle_terminal_line(&mut self, line: String) -> bool {
        self.push_terminal_line(line);
        false
    }

    /// Applies `WorkerMessage::TerminalLogError` without changing receive-loop ordering.
    pub(in crate::app) fn handle_terminal_log_error(&mut self, error: String) -> bool {
        self.model.status = error;
        false
    }

    /// Applies `WorkerMessage::TerminalDone`; stale run IDs skip the rest of that loop iteration.
    pub(in crate::app) fn handle_terminal_done(&mut self, run_id: u64) -> bool {
        if self.kit_tools.terminal.running_id != Some(run_id) {
            return true;
        }
        self.kit_tools.terminal.running = false;
        self.kit_tools.terminal.running_id = None;
        self.kit_tools.terminal.running_command = None;
        self.kit_tools.terminal.process = None;
        self.kit_tools.terminal.scroll_to_bottom = true;
        self.kit_tools.terminal.refocus_input = true;
        false
    }
}

pub(in crate::app) enum TerminalStopResult {
    Stopped,
    AlreadyExited,
}

pub(in crate::app) fn stop_terminal_process(
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
pub(in crate::app) fn windows_shell_command(command: &str) -> std::process::Command {
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
pub(in crate::app) fn terminal_shell_command(command: &str, work_dir: &Path) -> std::process::Command {
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

pub(in crate::app) fn run_terminal_command_for_reimport(
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

pub(in crate::app) fn stream_terminal_output<R: std::io::Read>(
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

pub(in crate::app) fn send_terminal_line(
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

pub(in crate::app) fn trim_terminal_lines(lines: &mut Vec<TerminalLineEntry>) {
    if lines.len() > TERMINAL_VISIBLE_LINE_LIMIT {
        let remove = lines.len() - TERMINAL_VISIBLE_LINE_TRIM_TARGET;
        lines.drain(..remove);
    }
}

pub(in crate::app) fn create_terminal_log_file(
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

pub(in crate::app) fn write_terminal_log_line(
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

pub(in crate::app) fn append_terminal_log_path(path: &Path, line: &str) -> Result<(), String> {
    use std::io::Write as _;

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|error| format!("Could not open terminal log {}: {error}", path.display()))?;
    writeln!(file, "{line}")
        .map_err(|error| format!("Could not write terminal log {}: {error}", path.display()))
}

pub(in crate::app) fn terminal_log_timestamp() -> String {
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

impl Baboon {
    pub(in crate::app) fn push_terminal_line(&mut self, line: String) {
        self.kit_tools.terminal.lines.push(TerminalLineEntry::new(line));
        trim_terminal_lines(&mut self.kit_tools.terminal.lines);
        self.kit_tools.terminal.scroll_to_bottom = true;
    }

    pub(in crate::app) fn begin_terminal_command(&mut self, ctx: egui::Context) {
        let command = self.kit_tools.terminal.input.trim().to_owned();
        if command.is_empty() {
            return;
        }
        self.submit_terminal_command(command, ctx);
    }

    pub(in crate::app) fn submit_terminal_command(&mut self, command: String, ctx: egui::Context) {
        if self.kit_tools.terminal.history.last() != Some(&command) {
            self.kit_tools.terminal.history.push(command.clone());
        }
        self.kit_tools.terminal.history_cursor = None;
        self.kit_tools.terminal.input.clear();
        self.kit_tools.terminal.refocus_input = true;
        self.spawn_terminal_command(command, ctx);
    }

    /// Run `command` in the editing-kit root, streaming output to the terminal
    /// panel. Shared by the terminal input and the geometry Import button.
    /// Starts the configured command without blocking frame rendering.
    /// Output and completion return through ordered worker messages for the active run id.
    pub(in crate::app) fn spawn_terminal_command(&mut self, command: String, ctx: egui::Context) {
        if self.refuse_read_only_edit(self.model.active) {
            return;
        }
        if self.kit_tools.terminal.running {
            self.model.status = "A command is already running".to_owned();
            return;
        }
        let Some(work_dir) = self.views[self.model.kits[self.model.active].id].terminal.work_dir.clone() else {
            self.model.status = "Run requires a loaded editing-kit folder".to_owned();
            return;
        };
        // Rewritten before it is echoed, so the terminal shows what really ran.
        let command = with_tool_folder_options(&command, &self.model.active_kit_tool_folder_options());
        self.views[self.model.kits[self.model.active].id].terminal.open = true;
        self.kit_tools.terminal
            .lines
            .push(TerminalLineEntry::new(format!("> {command}")));
        trim_terminal_lines(&mut self.kit_tools.terminal.lines);
        self.kit_tools.terminal.scroll_to_bottom = true;
        self.kit_tools.terminal.refocus_input = true;
        self.kit_tools.terminal.running = true;
        let run_id = self.kit_tools.terminal.next_run_id;
        self.kit_tools.terminal.next_run_id = self.kit_tools.terminal.next_run_id.wrapping_add(1).max(1);
        self.kit_tools.terminal.running_id = Some(run_id);
        self.kit_tools.terminal.running_command = Some(command.clone());
        let mut log_file = match create_terminal_log_file(run_id, &command) {
            Ok((path, file)) => {
                self.kit_tools.terminal.last_log_path = Some(path);
                Some(file)
            }
            Err(error) => {
                self.model.status = format!("Terminal full log unavailable: {error}");
                self.kit_tools.terminal.last_log_path = None;
                None
            }
        };
        let tx = self.tx.clone();
        let child_slot: Arc<Mutex<Option<std::process::Child>>> = Arc::new(Mutex::new(None));
        let stop_requested = Arc::new(AtomicBool::new(false));
        self.kit_tools.terminal.process = Some(TerminalProcess {
            child: Arc::clone(&child_slot),
            stop_requested: Arc::clone(&stop_requested),
        });
        // The run settles with `TerminalDone` however it ends, a panic
        // included, so the terminal never stays "running".
        let (worker_tx, worker_ctx) = (tx.clone(), ctx.clone());
        spawn_worker(&tx, &ctx, move || {
            let (tx, ctx) = (worker_tx, worker_ctx);
            let mut log_error_reported = false;
            let mut cmd = terminal_shell_command(&command, &work_dir);
            match cmd.spawn() {
                Err(e) => {
                    send_terminal_line(
                        &tx,
                        &ctx,
                        &mut log_file,
                        &mut log_error_reported,
                        format!("[error] {e}"),
                    );
                    WorkerMessage::TerminalDone { run_id }
                }
                Ok(child) => {
                    let stdout = match child_slot.lock() {
                        Ok(mut slot) => {
                            *slot = Some(child);
                            slot.as_mut().and_then(|child| child.stdout.take())
                        }
                        Err(_) => {
                            send_terminal_line(
                                &tx,
                                &ctx,
                                &mut log_file,
                                &mut log_error_reported,
                                "[error] terminal process lock was poisoned".to_owned(),
                            );
                            return WorkerMessage::TerminalDone { run_id };
                        }
                    };
                    if let Some(stdout) = stdout {
                        let _ = stream_terminal_output(
                            stdout,
                            &tx,
                            &ctx,
                            &mut log_file,
                            &mut log_error_reported,
                        );
                    }
                    let exit = match child_slot.lock() {
                        Ok(mut slot) => {
                            if let Some(mut child) = slot.take() {
                                child.wait().ok()
                            } else {
                                None
                            }
                        }
                        Err(_) => {
                            send_terminal_line(
                                &tx,
                                &ctx,
                                &mut log_file,
                                &mut log_error_reported,
                                "[error] terminal process lock was poisoned".to_owned(),
                            );
                            None
                        }
                    };
                    if let Some(code) = exit.and_then(|status| status.code())
                        && !stop_requested.load(Ordering::SeqCst)
                    {
                        send_terminal_line(
                            &tx,
                            &ctx,
                            &mut log_file,
                            &mut log_error_reported,
                            format!("[exit {code}]"),
                        );
                    }
                    WorkerMessage::TerminalDone { run_id }
                }
            }
        }, move |_| WorkerMessage::TerminalDone { run_id });
    }

    pub(in crate::app) fn stop_terminal_command(&mut self) {
        if !self.kit_tools.terminal.running {
            self.model.status = "No terminal command is running".to_owned();
            return;
        }
        let Some(process) = self.kit_tools.terminal.process.as_ref() else {
            self.model.status = "No tracked terminal process to stop".to_owned();
            return;
        };

        process.stop_requested.store(true, Ordering::SeqCst);
        let command = self
            .kit_tools.terminal
            .running_command
            .clone()
            .unwrap_or_else(|| "command".to_owned());
        match stop_terminal_process(process) {
            Ok(TerminalStopResult::Stopped) => {
                let line = format!("[stopped] {command} stopped by user");
                let mut log_status = None;
                if let Some(path) = self.kit_tools.terminal.last_log_path.as_ref()
                    && let Err(error) = append_terminal_log_path(path, &line)
                {
                    log_status = Some(error);
                }
                self.kit_tools.terminal.lines.push(TerminalLineEntry::new(line));
                trim_terminal_lines(&mut self.kit_tools.terminal.lines);
                self.finish_stopped_terminal_command();
                self.model.status = log_status.unwrap_or_else(|| "Terminal command stopped".to_owned());
            }
            Ok(TerminalStopResult::AlreadyExited) => {
                self.finish_stopped_terminal_command();
                self.model.status = "Terminal command had already exited".to_owned();
            }
            Err(error) => {
                let line = format!("[error] could not stop terminal command: {error}");
                let mut log_status = None;
                if let Some(path) = self.kit_tools.terminal.last_log_path.as_ref()
                    && let Err(log_error) = append_terminal_log_path(path, &line)
                {
                    log_status = Some(log_error);
                }
                self.kit_tools.terminal.lines.push(TerminalLineEntry::new(line));
                trim_terminal_lines(&mut self.kit_tools.terminal.lines);
                self.kit_tools.terminal.scroll_to_bottom = true;
                self.model.status = log_status
                    .unwrap_or_else(|| format!("Could not stop terminal command: {error}"));
            }
        }
    }

    pub(in crate::app) fn finish_stopped_terminal_command(&mut self) {
        self.kit_tools.terminal.running = false;
        self.kit_tools.terminal.running_id = None;
        self.kit_tools.terminal.running_command = None;
        self.kit_tools.terminal.process = None;
        self.kit_tools.terminal.scroll_to_bottom = true;
        self.kit_tools.terminal.refocus_input = true;
    }

    /// Record the current terminal-open state against the loaded game so it
    /// is restored next time that editing kit is opened.
    pub(in crate::app) fn remember_terminal_open_for_game(&mut self) {
        let Some(game) = self.model.source().and_then(|s| s.game.clone()) else {
            return;
        };
        if self.views[self.model.kits[self.model.active].id].terminal.open {
            self.kit_tools.terminal_open_games.insert(game.as_str().to_owned());
        } else {
            self.kit_tools.terminal_open_games.remove(game.as_str());
        }
    }
}

/// Where this kit's terminal is: whether its panel is open and the directory
/// its commands run in.
#[derive(Default)]
pub(in crate::app) struct KitTerminal {
    pub(in crate::app) open: bool,
    /// Working directory for terminal commands (game kit root, parent of tags/).
    pub(in crate::app) work_dir: Option<PathBuf>,
}

impl TerminalState {
    /// Step the input through the command history: `delta` back (-1) or
    /// forward (1), past the newest back to an empty line.
    pub(in crate::app) fn recall_history(&mut self, delta: i32) {
        let len = self.history.len();
        if len == 0 {
            return;
        }

        let next = match self.history_cursor {
            Some(index) => index as i32 + delta,
            None if delta < 0 => len as i32 - 1,
            None => return,
        };

        if next < 0 {
            self.history_cursor = Some(0);
            self.input = self.history[0].clone();
        } else if next >= len as i32 {
            self.history_cursor = None;
            self.input.clear();
        } else {
            let next = next as usize;
            self.history_cursor = Some(next);
            self.input = self.history[next].clone();
        }
    }
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
}
