//! A script tag's two tools: open its `.hsc` in VS Code, and recompile the
//! scenario that lists it. What the files and scenarios are is
//! `core::script_tags`'s; this runs the programs.

use super::*;
use crate::core::script_tags::{self, data_file, listed_scripts, script_text, scenario_folder};

impl Baboon {
    /// Open in VS Code: open the `.hsc` the active kit's script tag at `key`
    /// compiles from, where a HaloScript extension such as Atlas edits it.
    /// A data file that doesn't exist yet is written from the copy the tag
    /// holds first, and the status line says so.
    pub(in crate::app) fn open_script_in_vscode(&mut self, key: &str) {
        let (entry, tags_root) = match self.script_tag(key) {
            Ok(found) => found,
            Err(error) => {
                self.model.status = error;
                return;
            }
        };
        let Some(file) = data_file(&tags_root, &entry.display_path) else {
            self.model.status = "Couldn't find this kit's data folder".to_owned();
            return;
        };
        let mut wrote = false;
        if !file.exists() {
            let text = match self.model.kits[self.model.active].parsed_tags.get(key) {
                Some(document) => script_text(&document.tag),
                None => self
                    .model
                    .source()
                    .and_then(|source| read_entry(&source.source, &entry).ok())
                    .and_then(|tag| script_text(&tag)),
            };
            let Some(text) = text else {
                self.model.status =
                    format!("{} doesn't exist, and the tag holds no script to write it from", file.display());
                return;
            };
            let written = file
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&file, text.as_bytes()));
            if let Err(error) = written {
                self.model.status = format!("Couldn't write {}: {error}", file.display());
                return;
            }
            wrote = true;
        }
        self.model.status = match launch_vscode(&file) {
            Ok(_) if wrote => format!("Wrote {} from the tag's copy and opened it in VS Code", file.display()),
            Ok(_) => format!("Opened {} in VS Code", file.display()),
            Err(error) => format!("Couldn't start VS Code ({error}). Is it installed? The script is {}", file.display()),
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
            let path = tag_path::without_extension(&scenario.display_path).to_owned();
            let path = if path.contains(' ') { format!("\"{path}\"") } else { path };
            let id = self.model.kits[self.model.active].id;
            self.views[id].terminal.open = true;
            self.submit_terminal_command(format!("tool rebuild-scenario-scripts {path}"), ctx.clone());
        } else {
            self.launch_scenario_in_sapien(&scenario.key);
        }
    }

    /// The active kit's script tag at `key`, and its tags folder, when its
    /// game's script tags can be opened and recompiled.
    fn script_tag(&self, key: &str) -> Result<(TagEntry, PathBuf), String> {
        let game = self.model.source().and_then(|source| source.game);
        if !script_tags::supports_script_tags(game) {
            return Err("Opening and recompiling scripts works for Halo 2, Halo 3, ODST and Reach".to_owned());
        }
        let Some(tags_root) = self.model.loaded_tags_root_for(self.model.active) else {
            return Err("Opening and recompiling scripts needs an editing kit".to_owned());
        };
        let Some(entry) = self.model.entry_for_key(key).cloned() else {
            return Err("The script tag is no longer in this kit".to_owned());
        };
        if &entry.group_tag.to_be_bytes() != b"hsc*" {
            return Err("Only script tags can be opened or recompiled".to_owned());
        }
        Ok((entry, tags_root))
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
        vscode_command().arg(file).spawn().map(drop)
    }
}

/// VS Code's program: where its installers put it, else `code` on the path.
#[cfg_attr(test, allow(dead_code))]
fn vscode_command() -> std::process::Command {
    #[cfg(windows)]
    {
        let installed = [
            std::env::var_os("LOCALAPPDATA").map(|root| PathBuf::from(root).join("Programs")),
            std::env::var_os("ProgramFiles").map(PathBuf::from),
            std::env::var_os("ProgramFiles(x86)").map(PathBuf::from),
        ]
        .into_iter()
        .flatten()
        .map(|root| root.join("Microsoft VS Code").join("Code.exe"))
        .find(|exe| exe.is_file());
        match installed {
            Some(exe) => std::process::Command::new(exe),
            // `code` on the path is a batch file, which only cmd runs.
            None => {
                let mut command = std::process::Command::new("cmd");
                command.args(["/C", "code"]);
                command
            }
        }
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("code")
    }
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
    fn opening_a_script_with_no_data_file_writes_it_from_the_tag() {
        let kit = LooseKit::new("script_tools_vscode", "halo3_mcc");
        script(&kit, r"levels\a\scripts\a_mission");
        let mut app = app();
        kit.install(&mut app);
        app.open_script_in_vscode(&kit.key(r"levels\a\scripts\a_mission.scenario_hs_source_file"));
        let file = kit.root.parent().unwrap().join("data").join("levels").join("a").join("scripts").join("a_mission.hsc");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "(script startup a)");
        assert!(app.model.status.contains("Wrote") || app.model.status.contains("VS Code"), "{}", app.model.status);
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
