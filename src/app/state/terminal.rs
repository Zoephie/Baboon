//! terminal application state.
//! It owns passive cross-frame state and operation messages; rendering and workflow execution belong to UI and controller modules.

use super::*;

pub(in crate::app) struct TerminalState {
    pub(in crate::app) input: String,
    pub(in crate::app) lines: Vec<TerminalLineEntry>,
    pub(in crate::app) history: Vec<String>,
    pub(in crate::app) history_cursor: Option<usize>,
    pub(in crate::app) refocus_input: bool,
    pub(in crate::app) running: bool,
    pub(in crate::app) running_id: Option<u64>,
    pub(in crate::app) next_run_id: u64,
    pub(in crate::app) running_command: Option<String>,
    pub(in crate::app) last_log_path: Option<PathBuf>,
    pub(in crate::app) process: Option<TerminalProcess>,
    pub(in crate::app) scroll_to_bottom: bool,
}

pub(in crate::app) struct TerminalLineEntry {
    pub(in crate::app) text: String,
    pub(in crate::app) severity: TerminalLineSeverity,
}

impl TerminalLineEntry {
    pub(in crate::app) fn new(text: String) -> Self {
        let severity = TerminalLineSeverity::classify(&text);
        Self { text, severity }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum TerminalLineSeverity {
    Normal,
    Warning,
    Error,
    Success,
    Summary,
}

impl TerminalLineSeverity {
    fn classify(line: &str) -> Self {
        let trimmed = line.trim_start();
        let lower = line.to_ascii_lowercase();
        if line.contains("-ERROR-")
            || lower.contains("[error]")
            || (trimmed.starts_with("[exit ")
                && !trimmed.starts_with("[exit 0]")
                && trimmed
                    .strip_prefix("[exit ")
                    .and_then(|rest| rest.strip_suffix(']'))
                    .and_then(|code| code.parse::<i32>().ok())
                    .is_some())
        {
            Self::Error
        } else if lower.contains("warning") {
            Self::Warning
        } else if trimmed.starts_with("[exit 0]") {
            Self::Success
        } else if trimmed.starts_with("===") {
            Self::Summary
        } else {
            Self::Normal
        }
    }
}

pub(in crate::app) struct TerminalProcess {
    pub(in crate::app) child: Arc<Mutex<Option<std::process::Child>>>,
    pub(in crate::app) stop_requested: Arc<AtomicBool>,
}
