use super::app_icon;

#[test]
fn bundled_application_icon_decodes_for_the_native_window() {
    let icon = app_icon().expect("bundled Baboon icon should decode");
    assert!(icon.width >= 32);
    assert!(icon.height >= 32);
    assert_eq!(
        icon.rgba.len(),
        icon.width as usize * icon.height as usize * 4
    );
}

#[cfg(windows)]
#[test]
fn windows_process_uses_baboon_taskbar_identity() {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn GetCurrentProcessExplicitAppUserModelID(app_id: *mut *mut u16) -> i32;
    }
    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoTaskMemFree(memory: *mut std::ffi::c_void);
    }

    super::set_windows_app_user_model_id();
    let mut app_id = std::ptr::null_mut();
    assert_eq!(
        unsafe { GetCurrentProcessExplicitAppUserModelID(&mut app_id) },
        0
    );
    assert!(!app_id.is_null());
    let length = unsafe {
        (0..)
            .find(|offset| *app_id.add(*offset) == 0)
            .expect("AppUserModelID should be null terminated")
    };
    let value = unsafe { OsString::from_wide(std::slice::from_raw_parts(app_id, length)) };
    unsafe { CoTaskMemFree(app_id.cast()) };
    assert_eq!(value, "Zoephie.Baboon");
}
