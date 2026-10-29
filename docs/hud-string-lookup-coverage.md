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

## H2A: saved-tag verification and list grouping

The installed H2A MP kit was extracted and its 17,611 eligible tags parsed
without errors. The census examined 1,154,001 nonempty string ID instances.
Its saved globals resolve multiplayer UI globals, then HUD globals, then
six `string references[*]/string list` tags: `ui/strings/ingame`,
`ingame_campaign`, `ingame_interact`, `ingame_pve`, `ingame_pvp`, and
`ingame_film`. These hold 1,788 ID entries, or 1,766 distinct IDs. The MP kit
retains references to absent main-menu/SP/firefight globals; its multiplayer
route is verified. Do not hardcode the old tooltip path `ui/hud/hud_messages`.

The following seventeen fields are annotated in both H2A and H4. Counts are
H2A matches / nonempty field instances:

| Definition / fields | Matches / nonempty |
| --- | ---: |
| Biped assassination CHUD text | 3/3 |
| Vehicle flip message | 12/12 |
| Control action / action-denied / MP-team-denied strings | 5/5, 1/1, 1/1 |
| Seat enter / grenade / melee / in-seat / selection strings | 63/63, 6/6, 7/7, 1/1, 10/10 |
| Item pickup / swap / dual pickup / dual swap | 30/31, 31/32, 8/8, 8/8 |
| Item switch-to / switch-to-from-AI / notify-empty messages | 23/23, 23/23, 22/22 |

The unmatched item IDs are `ord_equip_damageboost` and `grenade_frag_swap`.
Manual editing remains available. The 516 crate pickup fields are blank;
training/terminal/dispenser fields provide no populated-value evidence in
this MP kit and are not annotated by this pass.

The resolver collects distinct referenced lists, preserving their order and
individual previews. Twenty-two IDs occur in more than one list; eight have
differing English text. The picker groups by list before optional prefix
groups. Search expands matching entries beneath source headings, and source
navigation lets the user choose a list. Known IDs remain valid if another
list cannot load; unknown IDs then have unavailable validation, not a false
missing warning. A loaded empty collection remains a known empty source.

Do not extend the existing shared-CHUD-skin rule to collect conflicting
skins: collection is a separate operation. H2A multiplayer event display
strings use a separate game-engine text reference (0/116 in HUD lists;
82/116 in the referenced list). Status messages resolve the multiplayer
in-game list (22/22 each); blocked-teleporter text occurs there and in HUD
lists. Item picked-up and H2A weapon-spawn fields include IDs from
`ui/strings/weapons`, so neither receives a HUD-only annotation.

## H4: schema-backed coverage, pending corpus verification

All seventeen annotated fields have the same string-ID fields and owning
struct GUIDs as H2A. Both schemas provide UI globals → HUD globals → a block
of Unicode-list references. H4 follows available singleplayer, multiplayer,
and firefight routes, collecting distinct lists and retaining their actual
paths; it does not assume H2A's six list paths. Missing mode roots are
expected in mode-specific kits; at least one root must resolve. Configured
leaf lists that cannot load retain an unavailable reason.

H4 tags are not installed. The annotations and shared route are supported
by the matching definitions and synthetic tests, not H4 saved-value
verification. Verify the populated mode routes and IDs when its kit becomes
available. Terminal page/name strings retain their own referenced source.
