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
        WNDCLASSW, WS_EX_TOPMOST, WS_OVERLAPPEDWINDOW, WS_POPUP, WS_VISIBLE,
    };
    use windows::core::w;

    const CHILD_FLAG: &str = "BABOON_DROP_RECEIVER_CHILD";
    const VISIBLE_FLAG: &str = "BABOON_DROP_RECEIVER_VISIBLE";
    /// Where the visible receiver sits: x, y, width, height on screen.
    const VISIBLE_RECT: (i32, i32, i32, i32) = (100, 100, 300, 200);
    const DROPPED: &str = r"C:\kit\tags\objects\weapons\rifle\rifle.weapon";

    /// The lookup half, end to end: a window of a process whose executable
    /// is named `sapien.exe`, found under a screen point, is reported as
    /// Sapien in the kit it runs from, opted into drops, and a drop handed
    /// to what the lookup returned reaches it. The child is a copy of this
    /// test binary named `sapien.exe`, showing its receiver window.
    #[test]
    fn a_kit_tool_window_is_found_under_a_point_and_takes_the_drop() {
        let kit = std::env::temp_dir().join(format!(
            "baboon-hit-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&kit).expect("kit folder");
        let sapien = kit.join("sapien.exe");
        std::fs::copy(std::env::current_exe().expect("test binary"), &sapien)
            .expect("copy the test binary as sapien.exe");
        let child = Command::new(&sapien)
            .args([
                "app::kit_tool_drop::tests::cross_process::drop_receiver_child",
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_FLAG, "1")
            .env(VISIBLE_FLAG, "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn the receiver");
        let child = Arc::new(Mutex::new(child));
        let watchdog = Arc::clone(&child);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(30));
            let _ = watchdog.lock().unwrap().kill();
        });
        let stdout = child.lock().unwrap().stdout.take().expect("piped stdout");
        let mut transcript = Vec::new();
        let mut found = None;
        let mut received = None;
        for line in BufReader::new(stdout).lines() {
            let line = line.expect("read the receiver");
            transcript.push(line.clone());
            if let Some((_, handle)) = line.split_once("HWND ") {
                let window: isize = handle.trim().parse().expect("a window handle");
                let (x, y, width, height) = VISIBLE_RECT;
                let centre = windows::Win32::Foundation::POINT {
                    x: x + width / 2,
                    y: y + height / 2,
                };
                // Nothing else of ours is a kit tool, so a miss is a miss.
                let target = super::super::platform::kit_tool_at(centre, &mut HashMap::new());
                found = Some((window, target.clone()));
                let Some(target) = target else { break };
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
        let transcript = format!("receiver said:\n{}\n{output}", transcript.join("\n"));
        let (window, target) = found.unwrap_or_else(|| panic!("no window; {transcript}"));
        let target = target.unwrap_or_else(|| panic!("no kit tool under the point; {transcript}"));
        assert_eq!(target.tool, KitTool::Sapien);
        assert_eq!(target.window, window, "the receiver's own window");
        assert!(target.accepts_files && target.tool_accepts_files);
        assert_eq!(
            std::fs::canonicalize(&target.kit_root).unwrap(),
            std::fs::canonicalize(&kit).unwrap(),
            "the kit is the folder the tool runs from"
        );
        assert_eq!(received.as_deref(), Some(DROPPED), "{transcript}");
        let _ = std::fs::remove_dir_all(&kit);
    }

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
                let target = KitToolDropTarget::for_tests(
                    KitTool::Sapien,
                    Path::new(r"C:\kit"),
                    true,
                    true,
                    window,
                );
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
            // Shown topmost at a known place when the hit test needs to
            // find it under a point; hidden otherwise.
            let visible = std::env::var_os(VISIBLE_FLAG).is_some();
            let (ex_style, style, rect) = if visible {
                (WS_EX_TOPMOST, WS_POPUP | WS_VISIBLE, VISIBLE_RECT)
            } else {
                (WINDOW_EX_STYLE::default(), WS_OVERLAPPEDWINDOW, (0, 0, 200, 100))
            };
            let Ok(window) = CreateWindowExW(
                ex_style,
                class_name,
                w!("Baboon drop receiver"),
                style,
                rect.0,
                rect.1,
                rect.2,
                rect.3,
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
