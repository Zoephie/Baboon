//! Dropping a tag dragged out of Baboon onto an editing-kit tool window (Sapien, Guerilla): finding the tool under the cursor and handing it the tag file the way Explorer would.
//! It owns the Win32 side of the drop only; which palette a tag lands in belongs to `scenario_palettes`, and the drag's feedback and gating belong to the controller.
//!
//! Explorer hands these MFC tools a `WM_DROPFILES` message (H3EK's `sapien.exe`
//! and `guerilla.exe` import `DragAcceptFiles`/`DragQueryFile`; HREK's Sapien
//! too), and Sapien answers it by adding the file to the scenario's matching
//! palette. Baboon cannot start a real OLE drag from inside an egui frame: the
//! nested `DoDragDrop` message loop would re-enter winit's event handler, which
//! panics on re-entry. So the browser's own egui drag carries on past the
//! window edge (the window keeps mouse capture while a button is down) and,
//! when it ends over a tool's window, Baboon posts the message itself.

use super::*;

/// An editing-kit program that takes file drops the way Explorer gives them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum KitTool {
    Sapien,
    Guerilla,
}

impl KitTool {
    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            KitTool::Sapien => "Sapien",
            KitTool::Guerilla => "Guerilla",
        }
    }
}

/// A kit tool's window under the cursor, and where a drop on it would land.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct KitToolDropTarget {
    pub(in crate::app) tool: KitTool,
    /// The tool's executable, e.g. `D:\...\H3EK\sapien.exe`.
    pub(in crate::app) executable: PathBuf,
    /// The editing kit the tool runs from: the executable's directory.
    pub(in crate::app) kit_root: PathBuf,
    /// Whether the window that would receive the drop opted into file drops
    /// (`WS_EX_ACCEPTFILES`). Sapien's main window does; its Hierarchy,
    /// Properties and Output windows are separate top-level windows that do
    /// not.
    pub(in crate::app) accepts_files: bool,
    /// Whether any window of the tool's process takes file drops. False for
    /// Halo CE's and Halo 2's Sapien, which never opted in anywhere.
    pub(in crate::app) tool_accepts_files: bool,
    /// The window that receives the drop: the nearest accepting window from
    /// the cursor up, else the tool's top-level window. A raw `HWND`.
    window: isize,
    /// The cursor, in that window's client coordinates.
    client_point: (i32, i32),
}

impl KitToolDropTarget {
    #[cfg(all(test, windows))]
    fn for_window_in_tests(window: isize) -> Self {
        Self {
            tool: KitTool::Sapien,
            executable: PathBuf::from(r"C:\kit\sapien.exe"),
            kit_root: PathBuf::from(r"C:\kit"),
            accepts_files: true,
            tool_accepts_files: true,
            window,
            client_point: (10, 10),
        }
    }
}

/// The kit tool whose window is under the cursor right now, if any.
///
/// `None` over Baboon's own windows, the desktop, or any other program.
/// `executables` remembers each process looked at, by id, so a drag does not
/// query the same process every frame; empty it between drags.
pub(in crate::app) fn kit_tool_under_cursor(
    executables: &mut HashMap<u32, Option<PathBuf>>,
) -> Option<KitToolDropTarget> {
    platform::kit_tool_under_cursor(executables)
}

/// Hand `file` to the target as a file drop, in the path form Explorer uses.
///
/// Ownership of the drop memory passes to the system at post time; nothing
/// is freed here on success.
pub(in crate::app) fn deliver_file_drop(
    target: &KitToolDropTarget,
    file: &Path,
) -> Result<(), String> {
    platform::deliver_file_drop(target, &explorer_path(file))
}

/// Whether no mouse button is held right now, straight from the system.
///
/// egui learns of a release from the window's own messages, and egui-winit
/// drops one that arrives while it believes the pointer has left the window;
/// a drag ending over another program's window is exactly where that can
/// happen, so the drop asks the system as well.
pub(in crate::app) fn mouse_buttons_are_up() -> bool {
    platform::mouse_buttons_are_up()
}

/// Which kit tool an executable file stem names (`sapien`, `sapien_play`,
/// `guerilla`, ...), case-insensitively.
pub(in crate::app) fn kit_tool_for_executable(stem: &str) -> Option<KitTool> {
    let stem = stem.to_ascii_lowercase();
    let names = |tool: &str| stem == tool || stem.starts_with(&format!("{tool}_"));
    if names("sapien") {
        Some(KitTool::Sapien)
    } else if names("guerilla") {
        Some(KitTool::Guerilla)
    } else {
        None
    }
}

/// Whether `tag_file` lives under `kit_root\tags`, compared component by
/// component and case-insensitively, without touching the disk. A verbatim
/// `\\?\D:\` prefix (what `canonicalize` hands back) counts as `D:\`.
pub(in crate::app) fn tag_within_kit(tag_file: &Path, kit_root: &Path) -> bool {
    let mut tag = tag_file.components();
    for root in kit_root.components() {
        match tag.next() {
            Some(component) if same_component(component, root) => {}
            _ => return false,
        }
    }
    let under_tags = tag
        .next()
        .is_some_and(|component| component.as_os_str().eq_ignore_ascii_case("tags"));
    under_tags && tag.next().is_some()
}

fn same_component(a: std::path::Component<'_>, b: std::path::Component<'_>) -> bool {
    use std::path::Component;
    match (a, b) {
        (Component::Prefix(a), Component::Prefix(b)) => same_prefix(a.kind(), b.kind()),
        _ => a.as_os_str().eq_ignore_ascii_case(b.as_os_str()),
    }
}

fn same_prefix(a: std::path::Prefix<'_>, b: std::path::Prefix<'_>) -> bool {
    use std::path::Prefix::{DeviceNS, Disk, UNC, Verbatim, VerbatimDisk, VerbatimUNC};
    match (a, b) {
        (Disk(a) | VerbatimDisk(a), Disk(b) | VerbatimDisk(b)) => a.eq_ignore_ascii_case(&b),
        (
            UNC(server_a, share_a) | VerbatimUNC(server_a, share_a),
            UNC(server_b, share_b) | VerbatimUNC(server_b, share_b),
        ) => server_a.eq_ignore_ascii_case(server_b) && share_a.eq_ignore_ascii_case(share_b),
        (Verbatim(a), Verbatim(b)) | (DeviceNS(a), DeviceNS(b)) => a.eq_ignore_ascii_case(b),
        _ => false,
    }
}

/// The path in the form Explorer hands over: one backslash between parts, a
/// plain `D:` or `\\server\share` prefix rather than the verbatim `\\?\` form
/// `canonicalize` produces, and no `.` parts. A kit root typed with forward
/// slashes in Settings otherwise reaches Sapien with mixed separators, which
/// its own prefix matching against its tags folder need not survive.
pub(in crate::app) fn explorer_path(file: &Path) -> PathBuf {
    use std::ffi::OsString;
    use std::path::{Component, Prefix};
    let mut joined = OsString::new();
    let separate = |joined: &mut OsString| {
        if !joined.is_empty() && joined.as_encoded_bytes().last() != Some(&b'\\') {
            joined.push(r"\");
        }
    };
    for component in file.components() {
        match component {
            Component::Prefix(prefix) => match prefix.kind() {
                Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
                    joined.push(format!("{}:", char::from(drive)));
                }
                Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                    joined.push(r"\\");
                    joined.push(server);
                    joined.push(r"\");
                    joined.push(share);
                }
                _ => joined.push(prefix.as_os_str()),
            },
            Component::RootDir => joined.push(r"\"),
            Component::CurDir => {}
            Component::ParentDir => {
                separate(&mut joined);
                joined.push("..");
            }
            Component::Normal(part) => {
                separate(&mut joined);
                joined.push(part);
            }
        }
    }
    PathBuf::from(joined)
}

/// The bytes of a `DROPFILES` block carrying one wide path: the 20-byte
/// header (`pFiles = 20`, the client point, `fNC = 0`, `fWide = 1`) followed
/// by the UTF-16LE path and the list's double terminator.
pub(in crate::app) fn encode_dropfiles(file: &Path, client_point: (i32, i32)) -> Vec<u8> {
    const HEADER_LEN: u32 = 20;
    const NOT_IN_NONCLIENT_AREA: i32 = 0;
    const WIDE: i32 = 1;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&HEADER_LEN.to_le_bytes());
    bytes.extend_from_slice(&client_point.0.to_le_bytes());
    bytes.extend_from_slice(&client_point.1.to_le_bytes());
    bytes.extend_from_slice(&NOT_IN_NONCLIENT_AREA.to_le_bytes());
    bytes.extend_from_slice(&WIDE.to_le_bytes());
    for unit in wide_path(file).chain([0u16, 0u16]) {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

#[cfg(windows)]
fn wide_path(file: &Path) -> impl Iterator<Item = u16> + '_ {
    use std::os::windows::ffi::OsStrExt;
    file.as_os_str().encode_wide()
}

#[cfg(not(windows))]
fn wide_path(file: &Path) -> impl Iterator<Item = u16> + '_ {
    file.to_string_lossy()
        .encode_utf16()
        .collect::<Vec<_>>()
        .into_iter()
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::ffi::{OsString, c_void};
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, GlobalFree, HANDLE, HWND, LPARAM, POINT, WPARAM,
    };
    use windows::Win32::Graphics::Gdi::ScreenToClient;
    use windows::Win32::System::Memory::{GHND, GlobalAlloc, GlobalLock, GlobalUnlock};
    use windows::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, EnumWindows, GA_PARENT, GA_ROOT, GWL_EXSTYLE, GetAncestor, GetCursorPos,
        GetDesktopWindow, GetWindowLongPtrW, GetWindowThreadProcessId, PostMessageW,
        WINDOW_EX_STYLE, WM_DROPFILES, WS_EX_ACCEPTFILES, WindowFromPoint,
    };
    use windows::core::{BOOL, PWSTR};

    struct OwnedHandle(HANDLE);

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    pub(super) fn kit_tool_under_cursor(
        executables: &mut HashMap<u32, Option<PathBuf>>,
    ) -> Option<KitToolDropTarget> {
        let mut cursor = POINT::default();
        unsafe { GetCursorPos(&mut cursor) }.ok()?;
        let hit = unsafe { WindowFromPoint(cursor) };
        if hit.is_invalid() {
            return None;
        }
        let mut process_id = 0u32;
        unsafe { GetWindowThreadProcessId(hit, Some(&mut process_id)) };
        if process_id == 0 || process_id == unsafe { GetCurrentProcessId() } {
            return None;
        }
        let executable = executables
            .entry(process_id)
            .or_insert_with(|| process_executable(process_id))
            .clone()?;
        let tool = kit_tool_for_executable(executable.file_stem()?.to_str()?)?;
        let kit_root = executable.parent()?.to_path_buf();
        let (window, accepts_files) = match accepting_ancestor(hit) {
            Some(window) => (window, true),
            None => (unsafe { GetAncestor(hit, GA_ROOT) }, false),
        };
        if window.is_invalid() {
            return None;
        }
        let tool_accepts_files = accepts_files || process_accepts_files(process_id);
        let mut client_point = cursor;
        if !unsafe { ScreenToClient(window, &mut client_point) }.as_bool() {
            return None;
        }
        Some(KitToolDropTarget {
            tool,
            executable,
            kit_root,
            accepts_files,
            tool_accepts_files,
            window: window.0 as isize,
            client_point: (client_point.x, client_point.y),
        })
    }

    pub(super) fn mouse_buttons_are_up() -> bool {
        // The high bit is the button's current state.
        [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON]
            .into_iter()
            .all(|button| unsafe { GetAsyncKeyState(i32::from(button.0)) } >= 0)
    }

    fn accepts_files(window: HWND) -> bool {
        let style = WINDOW_EX_STYLE(unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) } as u32);
        style.contains(WS_EX_ACCEPTFILES)
    }

    /// The first window from `hit` up to (not including) the desktop whose
    /// extended style says it takes file drops. That is the window Explorer's
    /// drop would reach too.
    fn accepting_ancestor(hit: HWND) -> Option<HWND> {
        let desktop = unsafe { GetDesktopWindow() };
        let mut window = hit;
        while !window.is_invalid() && window != desktop {
            if accepts_files(window) {
                return Some(window);
            }
            window = unsafe { GetAncestor(window, GA_PARENT) };
        }
        None
    }

    /// Whether any window of the process, top-level or child, takes file
    /// drops. Tells a tool window that happens not to (Sapien's Hierarchy
    /// view) from a tool that never does (Halo CE's Sapien).
    fn process_accepts_files(process_id: u32) -> bool {
        struct Search {
            process_id: u32,
            found: bool,
        }

        unsafe extern "system" fn visit_top_level(window: HWND, lparam: LPARAM) -> BOOL {
            let search = unsafe { &mut *(lparam.0 as *mut Search) };
            let mut owner = 0u32;
            unsafe { GetWindowThreadProcessId(window, Some(&mut owner)) };
            if owner == search.process_id {
                if accepts_files(window) {
                    search.found = true;
                } else {
                    unsafe {
                        let _ = EnumChildWindows(Some(window), Some(visit_child), lparam);
                    }
                }
            }
            BOOL::from(!search.found)
        }

        unsafe extern "system" fn visit_child(window: HWND, lparam: LPARAM) -> BOOL {
            let search = unsafe { &mut *(lparam.0 as *mut Search) };
            if accepts_files(window) {
                search.found = true;
            }
            BOOL::from(!search.found)
        }

        let mut search = Search {
            process_id,
            found: false,
        };
        // Stopping the walk early makes EnumWindows report a failure, which
        // is the found case, so its result says nothing.
        unsafe {
            let _ = EnumWindows(
                Some(visit_top_level),
                LPARAM(&mut search as *mut Search as isize),
            );
        }
        search.found
    }

    fn process_executable(process_id: u32) -> Option<PathBuf> {
        let process = OwnedHandle(
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }.ok()?,
        );
        let mut buffer = vec![0u16; 32 * 1024];
        let mut length = buffer.len() as u32;
        unsafe {
            QueryFullProcessImageNameW(
                process.0,
                PROCESS_NAME_WIN32,
                PWSTR(buffer.as_mut_ptr()),
                &mut length,
            )
        }
        .ok()?;
        Some(PathBuf::from(OsString::from_wide(
            &buffer[..length as usize],
        )))
    }

    pub(super) fn deliver_file_drop(target: &KitToolDropTarget, file: &Path) -> Result<(), String> {
        let tool = target.tool.label();
        let bytes = encode_dropfiles(file, target.client_point);
        let memory = unsafe { GlobalAlloc(GHND, bytes.len()) }
            .map_err(|error| format!("Could not allocate the drop for {tool}: {error}"))?;
        unsafe {
            let block = GlobalLock(memory);
            if block.is_null() {
                let _ = GlobalFree(Some(memory));
                return Err(format!("Could not fill the drop for {tool}"));
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), block as *mut u8, bytes.len());
            // GlobalUnlock reports failure with NO_ERROR once the block is
            // unlocked, which is the outcome wanted here, so its result is
            // not a verdict on anything.
            let _ = GlobalUnlock(memory);
        }
        let window = HWND(target.window as *mut c_void);
        // The system takes the block as the message is posted: once
        // PostMessageW returns, GlobalSize on it already reports 0 and
        // GlobalFree fails (measured), so freeing it here would be a double
        // free. The receiver's DragFinish frees the copy it was handed.
        match unsafe {
            PostMessageW(
                Some(window),
                WM_DROPFILES,
                WPARAM(memory.0 as usize),
                LPARAM(0),
            )
        } {
            Ok(()) => Ok(()),
            Err(error) => {
                unsafe {
                    let _ = GlobalFree(Some(memory));
                }
                if error.code() == ERROR_ACCESS_DENIED.to_hresult() {
                    Err(format!(
                        "{tool} is running with higher privileges than Baboon, so Windows blocks the drop (run both as the same user, or neither as administrator)"
                    ))
                } else {
                    Err(format!("Could not hand the file to {tool}: {error}"))
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;

    pub(super) fn kit_tool_under_cursor(
        _executables: &mut HashMap<u32, Option<PathBuf>>,
    ) -> Option<KitToolDropTarget> {
        None
    }

    pub(super) fn mouse_buttons_are_up() -> bool {
        false
    }

    pub(super) fn deliver_file_drop(
        _target: &KitToolDropTarget,
        _file: &Path,
    ) -> Result<(), String> {
        Err("Dropping tags onto Sapien is only available on Windows".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layout Explorer's targets read with `DragQueryFile`: a 20-byte
    /// header pointing past itself, then the wide path, then two zero units.
    #[test]
    fn a_dropfiles_block_is_the_header_then_the_wide_path_then_the_list_end() {
        let path = Path::new(r"C:\kit\tags\a.weapon");
        let bytes = encode_dropfiles(path, (7, -3));
        let mut header = Vec::new();
        header.extend_from_slice(&20u32.to_le_bytes());
        header.extend_from_slice(&7i32.to_le_bytes());
        header.extend_from_slice(&(-3i32).to_le_bytes());
        header.extend_from_slice(&0i32.to_le_bytes());
        header.extend_from_slice(&1i32.to_le_bytes());
        assert_eq!(&bytes[..20], &header[..]);
        let mut wide: Vec<u8> = r"C:\kit\tags\a.weapon"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        wide.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(&bytes[20..], &wide[..]);
        assert_eq!(bytes.len(), 20 + 2 * (r"C:\kit\tags\a.weapon".len() + 2));
    }

    #[test]
    fn only_sapien_and_guerilla_stems_name_a_kit_tool() {
        assert_eq!(kit_tool_for_executable("sapien"), Some(KitTool::Sapien));
        assert_eq!(kit_tool_for_executable("Sapien"), Some(KitTool::Sapien));
        assert_eq!(
            kit_tool_for_executable("sapien_play"),
            Some(KitTool::Sapien)
        );
        assert_eq!(kit_tool_for_executable("guerilla"), Some(KitTool::Guerilla));
        assert_eq!(
            kit_tool_for_executable("guerilla_play"),
            Some(KitTool::Guerilla)
        );
        for other in [
            "tool",
            "foundation",
            "explorer",
            "sapienx",
            "halo3_tag_test",
            "",
        ] {
            assert_eq!(kit_tool_for_executable(other), None, "{other}");
        }
    }

    #[test]
    fn a_tag_is_within_a_kit_only_under_its_tags_folder() {
        let kit = Path::new("/kits/h3ek");
        assert!(tag_within_kit(
            Path::new("/kits/h3ek/tags/objects/x.weapon"),
            kit
        ));
        assert!(tag_within_kit(
            Path::new("/kits/H3EK/TAGS/objects/x.weapon"),
            kit
        ));
        assert!(!tag_within_kit(
            Path::new("/kits/hrek/tags/objects/x.weapon"),
            kit
        ));
        assert!(!tag_within_kit(Path::new("/kits/h3ek/tags"), kit));
        assert!(!tag_within_kit(
            Path::new("/kits/h3ek/data/objects/x.jms"),
            kit
        ));
        assert!(!tag_within_kit(
            Path::new("/kits/h3ek2/tags/objects/x.weapon"),
            kit
        ));
    }

    /// What reaches Sapien is the shape Explorer would give it, whatever
    /// shape Baboon's settings or `canonicalize` gave the path.
    #[cfg(windows)]
    #[test]
    fn a_path_is_handed_over_in_explorer_form() {
        let expected = Path::new(r"D:\H3EK\tags\objects\x.weapon");
        assert_eq!(
            explorer_path(Path::new(r"\\?\D:\H3EK\tags\objects\x.weapon")),
            expected
        );
        assert_eq!(
            explorer_path(Path::new("D:/H3EK\\tags/objects/x.weapon")),
            expected
        );
        assert_eq!(
            explorer_path(Path::new(r"D:\H3EK\.\tags\objects\x.weapon")),
            expected
        );
        assert_eq!(explorer_path(expected), expected);
        assert_eq!(
            explorer_path(Path::new(r"\\?\UNC\server\share\tags\x.weapon")),
            Path::new(r"\\server\share\tags\x.weapon")
        );
        assert_eq!(
            explorer_path(Path::new(r"objects\x.weapon")),
            Path::new(r"objects\x.weapon")
        );
    }

    /// Drive letters compare regardless of case and of the verbatim prefix
    /// `canonicalize` adds, and either separator will do.
    #[cfg(windows)]
    #[test]
    fn a_windows_kit_root_matches_across_case_prefix_and_separator() {
        let kit = Path::new(r"D:\H3EK");
        assert!(tag_within_kit(
            Path::new(r"d:\h3ek\TAGS\objects\x.weapon"),
            kit
        ));
        assert!(tag_within_kit(
            Path::new(r"\\?\D:\H3EK\tags\objects\x.weapon"),
            kit
        ));
        assert!(tag_within_kit(
            Path::new("D:/H3EK/tags/objects/x.weapon"),
            kit
        ));
        assert!(!tag_within_kit(
            Path::new(r"E:\H3EK\tags\objects\x.weapon"),
            kit
        ));
        assert!(!tag_within_kit(
            Path::new(r"D:\H3EK2\tags\objects\x.weapon"),
            kit
        ));
    }

    /// The whole mechanism, end to end, against a real second process: a
    /// `WM_DROPFILES` posted with a `GlobalAlloc`'d `DROPFILES` block arrives
    /// in another process intact, and `DragQueryFile` there reads the path
    /// back. The child is this test binary running only the receiver test.
    #[cfg(windows)]
    mod cross_process {
        use super::super::*;
        use std::ffi::c_void;
        use std::io::{BufRead, BufReader, Write};
        use std::process::{Child, Command, Stdio};
        use std::sync::{Arc, Mutex};
        use std::time::Duration;
        use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG, PostQuitMessage,
            RegisterClassW, SetTimer, TranslateMessage, WINDOW_EX_STYLE, WM_DROPFILES, WM_TIMER,
            WNDCLASSW, WS_OVERLAPPEDWINDOW,
        };
        use windows::core::w;

        const CHILD_FLAG: &str = "BABOON_DROP_RECEIVER_CHILD";
        const DROPPED: &str = r"C:\kit\tags\objects\weapons\rifle\rifle.weapon";

        #[test]
        fn a_posted_drop_reaches_another_process() {
            let child = Command::new(std::env::current_exe().expect("test binary"))
                .args([
                    "app::kit_tool_drop::tests::cross_process::drop_receiver_child",
                    "--exact",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(CHILD_FLAG, "1")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn the receiver");
            let child = Arc::new(Mutex::new(child));
            // If the receiver never answers, its own timer quits it; this is
            // the backstop for a receiver that hangs before even that.
            let watchdog = Arc::clone(&child);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(30));
                let _ = watchdog.lock().unwrap().kill();
            });
            let stdout = child.lock().unwrap().stdout.take().expect("piped stdout");
            let mut transcript = Vec::new();
            let mut received = None;
            // The receiver's first report shares a line with libtest's own
            // "test ... " prefix, so the markers are looked for mid-line.
            for line in BufReader::new(stdout).lines() {
                let line = line.expect("read the receiver");
                transcript.push(line.clone());
                if let Some((_, handle)) = line.split_once("HWND ") {
                    let window: isize = handle.trim().parse().expect("a window handle");
                    let target = KitToolDropTarget::for_window_in_tests(window);
                    deliver_file_drop(&target, Path::new(DROPPED)).expect("post the drop");
                } else if let Some((_, path)) = line.split_once("FILE ") {
                    received = Some(path.to_owned());
                    break;
                }
            }
            let output = {
                let mut child = child.lock().unwrap();
                let _ = child.kill();
                child.wait_with_output_in_place()
            };
            assert_eq!(
                received.as_deref(),
                Some(DROPPED),
                "receiver said:\n{}\n{output}",
                transcript.join("\n")
            );
        }

        trait WaitInPlace {
            fn wait_with_output_in_place(&mut self) -> String;
        }

        impl WaitInPlace for Child {
            fn wait_with_output_in_place(&mut self) -> String {
                let _ = self.wait();
                let mut stderr = String::new();
                if let Some(mut pipe) = self.stderr.take() {
                    use std::io::Read;
                    let _ = pipe.read_to_string(&mut stderr);
                }
                stderr
            }
        }

        /// The receiving half. A no-op in a normal test run; in the child
        /// process it opens a hidden window that takes file drops, reports
        /// the window, and reports the first file dropped on it.
        #[test]
        fn drop_receiver_child() {
            if std::env::var_os(CHILD_FLAG).is_none() {
                return;
            }
            unsafe {
                let Ok(module) = GetModuleHandleW(None) else {
                    println!("FAIL no module handle");
                    return;
                };
                let class_name = w!("BaboonDropReceiver");
                let class = WNDCLASSW {
                    lpfnWndProc: Some(receiver_wndproc),
                    hInstance: module.into(),
                    lpszClassName: class_name,
                    ..Default::default()
                };
                if RegisterClassW(&class) == 0 {
                    println!("FAIL RegisterClassW");
                    return;
                }
                let Ok(window) = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    class_name,
                    w!("Baboon drop receiver"),
                    WS_OVERLAPPEDWINDOW,
                    0,
                    0,
                    200,
                    100,
                    None,
                    None,
                    Some(module.into()),
                    None,
                ) else {
                    println!("FAIL CreateWindowExW");
                    return;
                };
                DragAcceptFiles(window, true);
                SetTimer(Some(window), 1, 15_000, None);
                println!("HWND {}", window.0 as isize);
                let _ = std::io::stdout().flush();
                let mut message = MSG::default();
                while GetMessageW(&mut message, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }

        unsafe extern "system" fn receiver_wndproc(
            window: HWND,
            message: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            unsafe {
                match message {
                    WM_DROPFILES => {
                        let drop = HDROP(wparam.0 as *mut c_void);
                        let length = DragQueryFileW(drop, 0, None) as usize;
                        let mut buffer = vec![0u16; length + 1];
                        let copied = DragQueryFileW(drop, 0, Some(&mut buffer)) as usize;
                        let path = String::from_utf16_lossy(&buffer[..copied]);
                        DragFinish(drop);
                        println!("FILE {path}");
                        let _ = std::io::stdout().flush();
                        PostQuitMessage(0);
                        LRESULT(0)
                    }
                    WM_TIMER => {
                        println!("FAIL no drop arrived in time");
                        let _ = std::io::stdout().flush();
                        PostQuitMessage(0);
                        LRESULT(0)
                    }
                    _ => DefWindowProcW(window, message, wparam, lparam),
                }
            }
        }
    }
}
