//! Starting helper programs without flashing a console window.

use std::process::Command;

/// A command for a helper program Baboon runs out of sight: `git`,
/// `taskkill`, PowerShell, `cmd`.
///
/// On Windows it starts with `CREATE_NO_WINDOW`. A release build is a
/// GUI-subsystem program with no console of its own, so Windows gives every
/// console program it starts a new console window, which flashes up over
/// Baboon. Elsewhere this is `Command::new` exactly.
pub(crate) fn background_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        /// `CREATE_NO_WINDOW` from the Windows process creation flags.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}
