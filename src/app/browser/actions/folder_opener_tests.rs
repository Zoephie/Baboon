use super::*;

/// Open Folder did nothing but say "only available on Windows" on macOS
/// and Linux. Every platform now launches its file manager on the folder.
#[test]
fn open_folder_launches_the_platform_file_manager() {
    let folder = std::env::temp_dir();
    let mut app = Baboon::for_test();
    let mut launched = None;
    app.open_folder_with(folder.clone(), "Tag", |command| {
        launched = Some((
            command.get_program().to_owned(),
            command.get_args().map(ToOwned::to_owned).collect::<Vec<_>>(),
        ));
        Ok(())
    });
    let expected = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    assert_eq!(
        launched,
        Some((expected.into(), vec![folder.clone().into_os_string()]))
    );
    assert!(app.model.status.starts_with("Opened Tag folder"), "{}", app.model.status);

    // A folder that is not there launches nothing.
    let mut launched = false;
    app.open_folder_with(folder.join("baboon-no-such-folder"), "Tag", |_| {
        launched = true;
        Ok(())
    });
    assert!(!launched);
    assert!(app.model.status.contains("not found"), "{}", app.model.status);
}
