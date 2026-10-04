use super::*;

/// An overwrite confirmed for a workspace that closed before the frame
/// applied it writes nothing and leaves the focused workspace alone.
#[test]
fn an_overwrite_for_a_closed_workspace_writes_nothing() {
    let mut app = Baboon::for_test();
    let first = app.model.kits[0].id;
    let closed = app.add_kit();
    app.focus_navigation_kit(first);
    app.commands.send(ModsCommand::Overwrite {
        kit: closed,
        key: "file:objects/a.weapon".to_owned(),
        stop_asking: false,
    });
    app.remove_kit(closed);
    app.apply_commands(&egui::Context::default());
    assert_eq!(app.model.active_kit_id(), first);
    assert!(app.mods.container_write_leases.is_empty());
}

/// The same request for a workspace that is still open returns to it first,
/// since what it does acts on the active workspace.
#[test]
fn a_mods_command_returns_to_the_workspace_it_names() {
    let mut app = Baboon::for_test();
    let first = app.model.kits[0].id;
    let second = app.add_kit();
    app.focus_navigation_kit(first);
    app.commands.send(ModsCommand::ExportInstead { kit: second });
    app.apply_commands(&egui::Context::default());
    assert_eq!(app.model.active_kit_id(), second);
}
