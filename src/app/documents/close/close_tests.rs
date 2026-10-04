use std::path::PathBuf;


#[test]
fn only_workspace_close_actions_wait_for_chimp_documents() {
    assert!(crate::app::documents::close::close_action_includes_chimp(
        &super::PendingCloseAction::CloseApp
    ));
    assert!(crate::app::documents::close::close_action_includes_chimp(
        &super::PendingCloseAction::CloseKit(super::KitId(1))
    ));
    assert!(!crate::app::documents::close::close_action_includes_chimp(
        &super::PendingCloseAction::CloseAllTabs
    ));
    assert!(!crate::app::documents::close::close_action_includes_chimp(
        &super::PendingCloseAction::CloseTab("tag".to_owned())
    ));
}

/// The close prompt's Save sends container tags to the container writers
/// and only file tags to the file save.
#[test]
fn the_close_prompt_saves_container_tags_through_the_containers() {
    use super::TagEntryLocation;
    use crate::app::documents::close::{ClosePromptSave, close_prompt_save_route};
    let route = |location: TagEntryLocation| close_prompt_save_route(Some(&location));
    assert_eq!(
        route(TagEntryLocation::NewContainer {
            template: crate::core::source::NewContainerTemplate::Derived {
                group: "camera_track".to_owned(),
            },
            package: "/Game/Tags/objects/foo/bar-camera_track".to_owned(),
            group_tag: u32::from_be_bytes(*b"trak"),
        }),
        ClosePromptSave::NewContainer
    );
    assert_eq!(
        route(TagEntryLocation::Container {
            container: 0,
            rel_path: "Meteorite/Content/Tags/objects/a-biped.ubulk".to_owned(),
        }),
        ClosePromptSave::ContainerInPlace
    );
    assert_eq!(
        route(TagEntryLocation::LooseFile(PathBuf::from("/kit/tags/a.biped"))),
        ClosePromptSave::File
    );
    assert_eq!(
        route(TagEntryLocation::Monolithic {
            name: r"objects\a".to_owned(),
            group_tag: u32::from_be_bytes(*b"bipd"),
        }),
        ClosePromptSave::File
    );
    assert_eq!(close_prompt_save_route(None), ClosePromptSave::File);
}
