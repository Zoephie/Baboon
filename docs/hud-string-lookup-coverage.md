# HUD string lookup coverage audit

Audit date: 2026-10-29. Branch: `feature/string-id-lookups`.

Read-only comparison of installed editing-kit tags with the saved HUD-message
list resolved by each game's existing `hud_message` rule. No tag data was edited.
The audit read 650 candidate field values, 400 nonempty, with no tag parse errors.
391 nonempty values were present in the resolved HUD list.

## Verified additions

Counts below are nonempty values matching the resolved HUD list / nonempty values
examined. Blank fields were also inspected but are omitted from these counts.

| Fields | H2 | H3 | ODST | Reach |
| --- | --- | --- | --- | --- |
| HUD training `display string` | 29/29 | 32/32 | 29/32 | 28/32 |
| HUD training `display string2` / `display string3` | 10/10 | — | — | — |
| Vehicle `flip message` | 13/13 | 26/26 | 23/23 | 30/30 |
| Device control `action string` | 15/16 | 26/26 | 49/50 | 26/26 |
| Terminal `action string` | — | 7/7 | 7/7 | 26/26 |
| Biped `assassination chud text` | — | — | — | 15/15 |

All four games resolved `ui\hud\hud_messages`, following actual references:

- H2: `globals\globals` → `interface tags[0]/hud globals` → `new globals/hud text`.
- H3/ODST/Reach: `globals\globals` → `interface tags[0]/chud globals` →
  the shared `skins[*]/chud text` reference. Existing rules reject differing skin
  targets rather than choosing one arbitrarily.

Examples: H2's `ui\hud\default.hud_globals` uses `training_flashlight` and its
  grouped/ungrouped training fields use `training_looklong_group` and
  `training_looklong_ungroup`. H3 terminal tags use `device_terminal`;
  Reach terminal tags use `device_pda`; Reach bipeds use `finishing_move`.

The nine unmatched values remain visible as missing-ID warnings, with manual
editing allowed: H2 `jail_kiosk_action`, ODST `device_fakeout`, and seven old
training references across ODST and Reach (`training_dualhold`,
`training_leftfire`, `training_reload_left`, `training_stealth`). Matching values
in the same fields and their schema documentation establish the HUD source;
these exceptions are not evidence for selecting a different string list.

Terminal names and `pages[*]/text` are deliberately excluded: Reach terminals
have their own `strings` reference. A regression test uses different HUD/page
lists to ensure the action lookup cannot silently use the page list.

## H4 and H2A: pending real-tag verification

Neither tag corpus is installed locally. The user confirmed that H4/H2A tags
are unavailable. No HUD annotations or guessed source rules were added for them.
Campaign Evolved's parallel candidates were also left unchanged by this pass.

The H4/H2A schemas establish that the earlier CHUD route cannot be reused:

- `globals/interface tags[*]` has `singleplayer ui globals`, `multiplayer ui
  globals`, `firefight ui globals`, and `mainmenu ui globals` (`wgtz`), not
  `chud globals`.
- `user_interface_globals_definition` has `HUD globals` (`uihg`) and `shared
  globals` (`wigl`) references. UI shared globals expose several different
  Unicode lists, including a general UI list and a damage-reporting list.
- `user_interface_hud_globals_definition` training `display string` says it
  comes from HUD text globals, but this alone does not identify the saved list.
- `crate/grab pickup string` explicitly names `ui/hud/hud_messages`; that is
  strong schema evidence, but its saved values and availability still need a
  real-tag check before adding coverage.

When either corpus is available, inspect its globals and UI-globals references,
compare singleplayer/multiplayer/firefight sources, and check nonempty values in
items, unit seats, training, flip/action/denied strings, assassination text and
crate pickup fields. Determine whether HUD messages use a fixed tag, a shared
referenced list, or multiple lists before extending the resolver. Never use a
terminal's page list as the HUD source. Test missing and conflicting source
references as well as the successful route.
