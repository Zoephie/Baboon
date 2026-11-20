//! A script tag's two tools: open its `.hsc` in VS Code, and recompile the
//! scenario that lists it. What the files and scenarios are is
//! `core::script_tags`'s; this runs the programs.

use super::*;
use crate::app::shell::frame::{is_file_cached, recheck_cached};
use crate::app::export::string_list_window::StringToolReload;
use crate::core::script_tags::{self, data_file, listed_scripts, scenario_folder};

/// What the header's VS Code button and the row menu's Open in VS Code item
/// say when they're greyed out because VS Code isn't installed.
const NO_VSCODE: &str = "Baboon couldn't find VS Code. Install it, or put its code command on \
                         your PATH, to open scripts in it.";

/// Why Open in VS Code can't open the script tag at `display_path` in the
/// kit whose data folder is `data_root`, or `None` when it can: VS Code isn't
/// installed, or the script has no `.hsc` data file. The header and the row
/// menu ask every frame, so both are re-checked at most once a second.
pub(in crate::app) fn vscode_unavailable(
    ctx: &egui::Context,
    data_root: Option<&Path>,
    display_path: &str,
) -> Option<String> {
    let installed = recheck_cached(ctx, "vscode_program", || vscode_program().is_some());
    let file = data_root.map(|root| data_file(root, display_path));
    let exists = file.as_deref().is_some_and(|file| is_file_cached(ctx, file));
    unavailable_reason(installed, file.as_deref(), exists)
}

/// [`vscode_unavailable`]'s answer from what it found.
fn unavailable_reason(installed: bool, file: Option<&Path>, exists: bool) -> Option<String> {
    if !installed {
        return Some(NO_VSCODE.to_owned());
    }
    match file {
        None => Some("Baboon couldn't find this kit's data folder, which holds the script's .hsc file.".to_owned()),
        Some(file) if !exists => Some(format!(
            "This script has no data file to open. Baboon looked for {}.",
            file.display()
        )),
        Some(_) => None,
    }
}

impl Baboon {
    /// Open in VS Code: open the `.hsc` the active kit's script tag at `key`
    /// compiles from, where a HaloScript extension such as Atlas edits it.
    /// The header and menu grey the command out when VS Code or the file is
    /// missing; this says why again if either went missing since.
    pub(in crate::app) fn open_script_in_vscode(&mut self, key: &str) {
        let (entry, data_root) = match self.script_tag(key) {
            Ok(found) => found,
            Err(error) => {
                self.model.status = error;
                return;
            }
        };
        let file = data_root.map(|root| data_file(&root, &entry.display_path));
        let exists = file.as_deref().is_some_and(Path::is_file);
        if let Some(reason) = unavailable_reason(vscode_program().is_some(), file.as_deref(), exists) {
            self.model.status = reason;
            return;
        }
        let Some(file) = file else { return };
        self.model.status = match launch_vscode(&file) {
            Ok(()) => format!("Opened {} in VS Code", file.display()),
            Err(error) => format!("Couldn't start VS Code ({error}). The script is {}", file.display()),
        };
    }

    /// Recompile: compile the scenario that lists the active kit's script
    /// tag at `key` from its data files: with Tool's
    /// `rebuild-scenario-scripts` in the terminal on Halo 2, or by opening it
    /// in Sapien, which compiles as it loads, on Halo 3, ODST and Reach,
    /// whose tools have no such command.
    pub(in crate::app) fn recompile_script(&mut self, key: &str, ctx: &egui::Context) {
        let (entry, _) = match self.script_tag(key) {
            Ok(found) => found,
            Err(error) => {
                self.model.status = error;
                return;
            }
        };
        let scenario = match self.scenario_listing(&entry) {
            Ok(scenario) => scenario,
            Err(error) => {
                self.model.status = error;
                return;
            }
        };
        if self.model.source().and_then(|source| source.game) == Some(GameId::Halo2) {
            // Tool writes the compiled scripts into the scenario tag, which
            // an open, edited copy would then save over.
            let active = self.model.active;
            let open = self.model.kits[active].parsed_tags.get(&scenario.key);
            if open.is_some_and(|document| document.dirty.is_set()) {
                self.model.status = "Save the scenario before recompiling its scripts".to_owned();
                return;
            }
            let documents = open
                .map(|document| vec![(scenario.key.clone(), document.id, document.dirty.revision())])
                .unwrap_or_default();
            let (kit, source_session) = (self.model.kits[active].id, self.model.kits[active].source_session);
            let path = tag_path::without_extension(&scenario.display_path);
            // A run already going refuses this one; its id isn't ours.
            let busy = self.kit_tools.terminal.running;
            self.spawn_terminal_command(format!("tool rebuild-scenario-scripts \"{path}\""), ctx.clone());
            // Once Tool is done, the open scenario shows what it wrote.
            if let Some(run_id) = self.kit_tools.terminal.running_id
                && !busy
                && !documents.is_empty()
            {
                self.export.pending_string_tool_reload =
                    Some(StringToolReload::new(run_id, kit, source_session, documents, "scenarios"));
            }
        } else {
            self.launch_scenario_in_sapien(&scenario.key);
        }
    }

    /// The active kit's script tag at `key`, and its data folder from the
    /// kit's chosen layout, when its game's script tags can be opened and
    /// recompiled.
    fn script_tag(&self, key: &str) -> Result<(TagEntry, Option<PathBuf>), String> {
        let game = self.model.source().and_then(|source| source.game);
        if !script_tags::supports_script_tags(game) {
            return Err("Opening and recompiling scripts works for Halo 2, Halo 3, ODST and Reach".to_owned());
        }
        if self.model.loaded_tags_root_for(self.model.active).is_none() {
            return Err("Opening and recompiling scripts needs an editing kit".to_owned());
        }
        let Some(entry) = self.model.entry_for_key(key).cloned() else {
            return Err("The script tag is no longer in this kit".to_owned());
        };
        if &entry.group_tag.to_be_bytes() != b"hsc*" {
            return Err("Only script tags can be opened or recompiled".to_owned());
        }
        let data_root = self.model.kit_layout_for(self.model.active).map(|layout| layout.data);
        Ok((entry, data_root))
    }

    /// The one scenario that lists the script tag `script`: among the
    /// scenarios beside its `scripts` folder, read from their open documents
    /// or their files. A script several scenarios list (a shared one, such
    /// as `global_scripts`) has no one scenario to recompile.
    fn scenario_listing(&self, script: &TagEntry) -> Result<TagEntry, String> {
        let source = self.model.source().ok_or("No kit is loaded")?;
        let folder = scenario_folder(&script.display_path);
        let kit = &self.model.kits[self.model.active];
        let listing: Vec<TagEntry> = source
            .full_entry_set()
            .iter()
            .filter(|entry| {
                tag_path::split_extension(&entry.display_path).is_some_and(|(stem, group)| {
                    group.eq_ignore_ascii_case("scenario")
                        && tag_path::parent(stem).unwrap_or("").eq_ignore_ascii_case(folder)
                })
            })
            .filter(|entry| {
                let listed = match kit.parsed_tags.get(&entry.key) {
                    Some(document) => listed_scripts(&document.tag),
                    None => read_entry(&source.source, entry).map(|tag| listed_scripts(&tag)).unwrap_or_default(),
                };
                listed.iter().any(|listed| listed.eq_ignore_ascii_case(&script.display_path))
            })
            .cloned()
            .collect();
        match listing.as_slice() {
            [scenario] => Ok(scenario.clone()),
            [] => Err(format!(
                "No scenario in {} lists this script, so there's nothing to recompile. Recompile from the scenario instead.",
                if folder.is_empty() { "the tags folder" } else { folder }
            )),
            _ => Err("Several scenarios list this script. Recompile from one of the scenarios instead.".to_owned()),
        }
    }
}

/// Open `file` in VS Code. Tests run everything up to here, but start
/// nothing.
fn launch_vscode(file: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    {
        let _ = file;
        Ok(())
    }
    #[cfg(not(test))]
    {
        let program = vscode_program().ok_or_else(|| std::io::Error::other(NO_VSCODE))?;
        // Out of sight: `code.cmd` runs through cmd, whose console window
        // would otherwise flash up over Baboon. VS Code's own window shows.
        crate::core::process::background_command(program).arg(file).spawn().map(drop)
    }
}

/// VS Code's program: where its installers put it, else its `code` command
/// on the path. `None` when neither is there.
fn vscode_program() -> Option<PathBuf> {
    #[cfg(windows)]
    let (installed, command) = (
        [
            std::env::var_os("LOCALAPPDATA").map(|root| PathBuf::from(root).join("Programs")),
            std::env::var_os("ProgramFiles").map(PathBuf::from),
            std::env::var_os("ProgramFiles(x86)").map(PathBuf::from),
        ]
        .into_iter()
        .flatten()
        .map(|root| root.join("Microsoft VS Code").join("Code.exe"))
        .find(|exe| exe.is_file()),
        // A batch file, which Rust runs through cmd for us.
        "code.cmd",
    );
    // The app bundle's `code`, in the Applications folder or the user's own.
    #[cfg(target_os = "macos")]
    let (installed, command) = (
        [Some(PathBuf::from("/Applications")), std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Applications"))]
            .into_iter()
            .flatten()
            .map(|root| {
                root.join("Visual Studio Code.app")
                    .join("Contents")
                    .join("Resources")
                    .join("app")
                    .join("bin")
                    .join("code")
            })
            .find(|program| program.is_file()),
        "code",
    );
    #[cfg(not(any(windows, target_os = "macos")))]
    let (installed, command) = (None, "code");
    installed.or_else(|| {
        std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|folder| folder.join(command))
            .find(|program| program.is_file())
    })
}

#[cfg(test)]
mod tests {
    use crate::app::loose_fixture::{LooseKit, app, ctx};
    use blam_tags::TagFieldData;

    /// A scenario listing `scripts`, written into `kit` at `rel`.
    fn scenario(kit: &LooseKit, rel: &str, scripts: &[&str]) {
        kit.write_mcc(rel, "scenario", |tag| {
            {
                let mut root = tag.root_mut();
                let mut resources = root.field_mut("scenario resources").unwrap();
                let mut resources = resources.as_block_mut().unwrap();
                let index = resources.add_element();
                let mut element = resources.element_mut(index).unwrap();
                let mut sources = element.field_mut("script source").unwrap();
                let mut sources = sources.as_block_mut().unwrap();
                for _ in scripts {
                    sources.add_element();
                }
            }
            for (index, script) in scripts.iter().enumerate() {
                tag.root_mut()
                    .field_path_mut(&format!("scenario resources[0]/script source[{index}]/reference"))
                    .unwrap()
                    .set(TagFieldData::TagReference(blam_tags::TagReferenceData {
                        group_tag_and_name: Some((u32::from_be_bytes(*b"hsc*"), (*script).to_owned())),
                    }))
                    .unwrap();
            }
        });
    }

    fn script(kit: &LooseKit, rel: &str) {
        kit.write_mcc(rel, "scenario_hs_source_file", |tag| {
            tag.root_mut()
                .field_path_mut("source")
                .unwrap()
                .set(TagFieldData::Data(b"(script startup a)\0".to_vec()))
                .unwrap();
        });
    }

    #[test]
    fn a_script_finds_the_one_scenario_that_lists_it() {
        let kit = LooseKit::new("script_tools_scenario", "halo3_mcc");
        script(&kit, r"levels\a\scripts\a_mission");
        script(&kit, r"levels\a\scripts\a_shared");
        scenario(&kit, r"levels\a\a", &[r"levels\a\scripts\a_mission", r"levels\a\scripts\a_shared"]);
        scenario(&kit, r"levels\a\a_cinematics", &[r"levels\a\scripts\a_shared"]);
        let mut app = app();
        kit.install(&mut app);
        let entry_of = |app: &crate::app::Baboon, rel: &str| app.model.entry_for_key(&kit.key(rel)).cloned().unwrap();

        let mission = entry_of(&app, r"levels\a\scripts\a_mission.scenario_hs_source_file");
        assert_eq!(
            app.scenario_listing(&mission).map(|entry| entry.display_path),
            Ok(r"levels\a\a.scenario".to_owned())
        );
        let shared = entry_of(&app, r"levels\a\scripts\a_shared.scenario_hs_source_file");
        assert!(app.scenario_listing(&shared).err().is_some_and(|error| error.contains("Several scenarios")));

        // Recompiling a shared script says why it can't, and runs nothing.
        app.recompile_script(&kit.key(r"levels\a\scripts\a_shared.scenario_hs_source_file"), &ctx());
        assert!(app.model.status.contains("Several scenarios"), "{}", app.model.status);
    }

    #[test]
    fn open_in_vscode_says_why_it_cant_open_a_script() {
        use super::unavailable_reason;
        let file = std::path::Path::new(r"C:\kit\data\levels\a\scripts\a_mission.hsc");
        assert!(unavailable_reason(false, Some(file), true).unwrap().contains("couldn't find VS Code"));
        assert!(unavailable_reason(true, None, false).unwrap().contains("data folder"));
        let missing = unavailable_reason(true, Some(file), false).unwrap();
        assert!(missing.contains("no data file") && missing.contains("a_mission.hsc"), "{missing}");
        assert_eq!(unavailable_reason(true, Some(file), true), None);
    }

    #[test]
    fn a_script_with_no_data_file_isnt_written_or_opened() {
        let kit = LooseKit::new("script_tools_vscode", "halo3_mcc");
        script(&kit, r"levels\a\scripts\a_mission");
        let mut app = app();
        kit.install(&mut app);
        app.open_script_in_vscode(&kit.key(r"levels\a\scripts\a_mission.scenario_hs_source_file"));
        let file = kit.root.parent().unwrap().join("data").join("levels").join("a").join("scripts").join("a_mission.hsc");
        assert!(!file.exists());
        // Says why: the missing data file, or first that VS Code is missing
        // on a machine without it.
        let status = &app.model.status;
        if super::vscode_program().is_some() {
            assert!(status.starts_with("This script has no data file to open"), "{status}");
            assert!(status.contains("a_mission.hsc"), "{status}");
        } else {
            assert_eq!(status, super::NO_VSCODE);
        }
    }

    #[test]
    fn halo_ce_has_no_script_tags_to_open() {
        let kit = LooseKit::new("script_tools_ce", "haloce_mcc");
        let mut app = app();
        kit.install(&mut app);
        app.open_script_in_vscode("missing");
        assert!(app.model.status.contains("Halo 2, Halo 3, ODST and Reach"), "{}", app.model.status);
    }
}
