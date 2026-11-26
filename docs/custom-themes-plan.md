# Custom themes — implementation plan

**Status (2026-11-25, branch `custom-themes`).** Every phase is built, one
local commit each, not yet pushed or in a pull request. Phase 0 is Paddy's
(#131, #133); Phase 1 is `269ce20e`, Phase 2 `e03b44bb`, Phase 3 `b8d6ad19`,
Phase 4 `317f54b6`, and Phase 5 the commit after it. Where the build differs
from the plan, the phase says so under **As built**. The running app has not
been tried: live reload with an outside editor, the editor's feel, preview on
hover, and everything on Windows are owed a check
([section 5](#5-verification-outside-the-test-suite)).

**Goal.** Let a user change Baboon's colours with a theme file, in the spirit
of VS Code's theme engine: named colours with documented defaults, themes
written from scratch or extending another and setting only what they change,
light and dark themes that follow the OS, live reload while authoring, and an
in-app editor.

**The limit.** VS Code has 971 colour ids. This plan has 113, measured from
the code. A theme written from scratch sets all of them (Export Current Theme
writes a complete starting file); a theme that extends another sets only what
it changes.

**Terms used throughout**

- **Token** — a named colour the UI asks for, such as `block.header.background`.
- **Appearance** — whether a theme is `dark` or `light`. It selects fallback
  values and the handful of code paths that differ between the two.
- **Rule** — how a token is computed from other tokens when a theme does not
  set it, for example "`block.header.background` is `background` mixed 15% toward white".
- **Built-in** — `baboon-dark`, `baboon-light` and `baboon-light-classic`,
  today's three looks (Dark, Light and Classic Light in the Theme menu).
- **Alias** — an accessor named for where it is used (`save_error_text`) that
  returns another token's colour in every look. Aliases stay as functions; they
  are not tokens.

---

## 1. Where the code is today

Measured on `main` at `750178f3`, after #131 and #133. Every value in
[Appendix A](#appendix-a--token-table) was read by calling each accessor under
each look, not copied from source.

| | |
|---|---|
| Theme state | Two `AtomicBool`s in `src/app/ui_kit/style.rs`: `DARK_MODE_ENABLED` and `CLASSIC_LIGHT`, the second behind a `LightTheme { Warm, Classic }` enum |
| Preferences | `dark_mode`, `follow_system_theme` and `light_theme` (`"warm"` or `"classic"`) in `GuiPrefs` (`src/app/prefs/state.rs`) |
| Theme menu | One menu, four choices: Use system setting, Light, Classic Light, Dark (`ThemeChoice` in `src/app/shell/settings.rs`). Following the system always gives the warm Light (`GuiPrefs::effective_light_theme`) |
| Central colours | `style.rs`, 2,268 lines: 121 colour accessors with no arguments, 6 with arguments, and private `*_for(dark)` helpers. 84 theme branches and 247 colour constructors, all inside this file |
| Distinct colours | The 127 public accessors give 100 distinct (dark, light, classic) triples, and 112 with the 15 egui visuals fields. 12 accessors alias the error colour, 7 the warning colour, 4 the success colour and 4 the destructive red |
| Colours outside `style.rs` | Kept out by `ui_kit/palette_gate.rs` (see Phase 0). 20 lines in 7 files carry a `// fixed:` reason; `model_preview/renderer/` and `model_preview/viewport/` are allowed as 3D-scene colours |
| `ui_kit/` outside `style.rs` | Not scanned by the gate. Theme reads in `close.rs`, `selection_art.rs`, `slider.rs`, `button_icons.rs` and `focus.rs`; literals in `button_icons.rs` (13) and one each in `close.rs`, `selection_art.rs` and `slider.rs` |
| egui visuals | Built by `foundation_visuals()` and set every frame (`src/app/shell/workspace.rs`). It sets `warn_fg_color` and `error_fg_color` from the status colours. The warm Light writes its own widget fills and strokes there; Classic Light uses `opaque_widget_fills` |
| Tests | A thread-local `with_theme(dark, light, f)` in `style.rs` lets a test draw in one look without touching the global |

### Findings that shape the plan

1. **The accessors are the seam.** `text_dark()`, `editor_bg()` and the rest
   take no arguments and are called everywhere. Their bodies can read a
   resolved theme while their call sites stay untouched.
2. **The scattered literals are gone** (Phase 0). Outside `style.rs` and the
   `// fixed:` lines, colours go through accessors, and a test keeps it so.
   The gaps left are the `ui_kit/` files the gate skips and the loading art
   (finding 5).
3. **Status colours are unified.** One error, one warning and one success
   colour per look, each tested at 4.5:1 or better on the page, panel, group
   and status-bar backgrounds of all three looks.
4. **Aliases.** #131 named many sites after their feature rather than their
   meaning: `save_error_text`, `bitmap_error_text`, `review_error_text` and
   eight more are the error colour in every look. The registry gives each
   meaning one token and keeps the feature names as aliases, which is how 142
   colour sources become 113 tokens.
5. **Light is behaviour as well as colour.** The warm Light's widget fills
   and strokes are written by hand in `foundation_visuals()`; Classic Light
   uses `opaque_widget_fills`; `bitmap_lightmode.svg` and `icon_backdrop()`
   darken icons in light looks; the loading art swaps hex strings inside its
   SVGs (`src/app/shell/loading.rs`, `LoadingPalette`), which neither the
   accessors nor the gate see. A custom theme has to declare its appearance
   so these keep working.
6. **Some surfaces are dark in every look.** The terminal, the read-only
   badge mark and the diff washes each have one value.
7. **Some colours are data and must never be themed.** Tag colour swatches,
   the bitmap viewer's channel overlays, axis colours, the alpha checkerboard,
   the flat material palette, and outlines drawn over arbitrary images. Most
   now carry a `// fixed:` reason.
8. **Recoloured SVGs are cached per colour.** Six thread-local caches in
   `close.rs`, `button_icons.rs` (two), `tag_icons.rs` (two) and
   `selection_art.rs` keep SVG strings keyed by colour and register a URI per
   colour. The button-icon cache sweeps entries unpainted for 600 passes; the
   others do not. Live theme editing would grow them without an explicit
   clear.
9. **Tests already have a per-thread theme.** `with_theme` does what the
   original plan's `ThemeGuard` was for, without a lock. Phase 1 widens it to
   take a resolved table.
10. **Hex with alpha already behaves like CSS.** In the pinned ecolor,
    `Color32::from_hex("#FFFFFF1A")` gives `[26, 26, 26, 26]`, the same value
    `neutral_button_fill` builds by hand. The built-in overlays can be written
    as hex without changing a pixel. Read from the source, not run; Phase 1's
    golden table confirms it.
11. **No new dependency is needed.** `serde_json` is in use and prefs are
    parsed by hand from `serde_json::Value`. There is no `toml`, `json5` or
    file-watching crate in the tree.
12. **The `light_theme` preference already exists**, holding `"warm"` or
    `"classic"`. Phase 2 reuses the key for a theme id and reads the two old
    values as the ids of the two light built-ins.

---

## 2. What the research says

Two research agents read primary sources (VS Code's repository and docs, and
the docs and source of the other projects). I have not re-checked each citation
myself; links are in [Appendix C](#appendix-c--sources).

| Idea | Seen in | Verdict |
|---|---|---|
| Registry of named colours with a description and per-appearance defaults | VS Code | Adopt |
| Defaults that derive from other colours | VS Code (about 100 transparency transforms, 13 lighten, 12 darken), Zed, Godot | Adopt, with three operations only |
| Themes set only some keys; the rest fall back by appearance | VS Code, Zed, JetBrains | Adopt |
| Single-parent inheritance | VS Code `include`, Helix `inherits`, JetBrains `parentTheme` | Adopt as `extends` |
| Named palette inside the theme file | Helix, JetBrains, Rerun | Adopt. VS Code lacks it; variables have been requested since 2018 (issue #56855) |
| Status triplets `x`, `x.background`, `x.border` | Zed | Adopt as the naming shape |
| Separate preferred dark and light themes that follow the OS | VS Code, Zed | Adopt |
| Keep the old theme when a reload fails | VS Code | Adopt |
| Export the current theme with every key | VS Code "Generate Color Theme From Current Settings" | Adopt |
| JSON schema for completion while authoring | VS Code | Adopt, generated from the registry |
| User override map layered on top of any theme | VS Code, Zed | Skip for now; see decision 2 |
| A few inputs generate the whole theme | Godot (base colour, accent, contrast) | Skip: a theme written from scratch sets every token, and rules only carry an override's dependents (decision 3) |
| Bad colour silently renders red; unknown key silently ignored | VS Code | Avoid: report both |
| Theme identified by its display name | VS Code (needs a rename-migration table) | Avoid: identify by file name |
| Hundreds of unlinked colours | Blender (939, then 300+ removed in 5.0) | Avoid: cap the count and link through rules |
| Dotted-prefix fallback (`a.b.c` falls back to `a.b`) | Helix | Skip: a missing `x.background` must not fall back to the foreground `x` |
| TextMate scopes and semantic tokens | VS Code | Skip: Baboon's few highlighters become about ten `syntax.*` tokens |
| egui's serde `Visuals` as the file format | egui | Skip: gaps fill with dark values, it covers 36 egui fields and none of Baboon's, and it changes with egui upgrades |
| Existing egui theme crates | catppuccin-egui (stops at egui 0.33), egui_colors | Skip: none reach Baboon's own colours |

---

---

## 3. Design

### 3.1 Token registry

One Rust table declares every token. It is the single source for the accessor
functions, the built-in values, the rules, the exported theme, the JSON schema
and the reference documentation.

Each row carries:

- an **id**, the string a theme file uses (`block.header.background`);
- a **description**, one sentence saying where the colour appears;
- the **built-in values**, one column per built-in: dark, light and classic
  light, today's hand-picked colours;
- a **rule** for themes that set nothing for it.

A sketch, not final syntax:

```rust
tokens! {
    //  variant       id                dark         light        classic      rule
    Background   "background"      "#282828"    "#F2F1F0"    "#E0E0DC"    root
        "Editor panes and windows.";
    Text         "text"            "#F2F2F2"    "#1C1815"    "#191918"    root
        "Body text and labels.";
    Accent       "accent"          "#406C86"    "#92C0E2"    "#9EC0DE"    root
        "Selection fill and other emphasis.";
    BlockHeader  "block.header.background"    "#484848"    "#F0E2D4"    "#CCCCC8"
        by_appearance(mix(Background, White, 0.149), mix(Background, Black, 0.089))
        "Header bar of a tag block.";
    SyntaxMarkupWarning "syntax.markup.warning" "#F0B446" "#B06800" "#B06800"
        reference(StatusWarning)
        "Underline under markup the game will not read as written.";
}
```

The macro generates `enum Token`, `Token::ALL`, `id()`, `description()`, the
built-in columns as `const` arrays, and the rule table.

**Aliases.** An accessor whose colour equals another token's in every
built-in stays as a function returning that token (`save_error_text()` returns
`theme::color(Token::StatusError)`). It is listed under its token in
Appendix A and in the authoring guide, so a theme sets the meaning once.

**Naming.** Lowercase, dotted, `group.role[.state]`: `tab.active.background`,
`status.error.border`. No camelCase. Ids are a public file format once a theme
exists in the wild, so they had a naming pass (decision 6), and any change
after Phase 2 ships goes through the rename table.

**Count.** [Appendix A](#appendix-a--token-table) has 113 tokens covering all
142 colour sources (127 accessors and 15 egui visuals fields). There is no cap
(decision 8): tokens are added as the UI needs them.

**Renames.** A token that is renamed, merged into another or removed gets a
row in a rename table, `old id → new id`, beside the registry. The loader
reads an old id as its new one and reports it as renamed, so a theme file
written against an older Baboon keeps every colour. Adding a token needs no
row: older files fill it from their parent or built-in. A test checks that
every rename points at a token that exists and that no rename shadows a live
id.

### 3.2 Rules

Three operations, each optionally different per appearance:

| Operation | Meaning |
|---|---|
| `reference(T)` | Same colour as token `T` |
| `alpha(T, a)` | Token `T` at opacity `a` |
| `mix(A, B, t)` | `A` moved fraction `t` toward `B`, per channel in sRGB |

`White` and `Black` are fixed operands. A rule may refer only to tokens declared
earlier in the table, so rules cannot form a cycle and resolve in one pass.

Rules decide only which tokens follow an override (section 3.5, step 6). A
token never takes a rule's value unless a token its rule depends on was set by
the theme; otherwise it keeps its fallback. Tokens with no rule are the roots
`background`, `text` and `accent`, the status colours, and any token Phase 3
finds no rule for and marks independent.

A rule has one value per appearance, not per built-in. The rules are fitted to
`baboon-dark` and to the warm `baboon-light` (decision 3). Classic Light keeps
its own column; an override on it moves dependents by the same light rules.

### 3.3 Theme file

JSON, with `//` comments and trailing commas tolerated by a small pre-pass
before `serde_json`.

```jsonc
{
  "name": "Forerunner",           // shown in Settings
  "appearance": "dark",           // "dark" or "light"
  "extends": "baboon-dark",       // optional: another theme's id
  "palette": {                    // optional: names for reuse below
    "steel": "#4B7D9B",
    "amber": "#F2C430"
  },
  "colors": {
    "accent": "$steel",
    "background": "#1E2227",
    "status.warning": "$amber"
  }
}
```

| Key | Required | Notes |
|---|---|---|
| `name` | yes | Display name. Not the identity |
| `appearance` | when there is no `extends` | Inherited from the parent otherwise |
| `extends` | no | The id of a built-in or of another file in the themes folder |
| `palette` | no | `name` to hex. Names match `[a-z0-9_-]+` |
| `colors` | no | Token id to a hex value or a `$name` from the palette |
| `$schema` | no | Ignored by Baboon; lets an external editor offer completion |

Colour values are `#RGB`, `#RGBA`, `#RRGGBB` or `#RRGGBBAA`, read by
`Color32::from_hex`. Alpha is straight, as in CSS.

The key is spelled `colors`, matching `Color32` and every theme format surveyed.

### 3.4 Identity, discovery and storage

- **Identity is the file name without `.json`**, lowercased: `forerunner.json`
  has the id `forerunner`. Changing `name` inside the file breaks nothing.
- **`baboon-dark`, `baboon-light` and `baboon-light-classic` are reserved.** A
  user file with one of those names is reported and skipped.
- **User themes live in `themes/` under the data directory**, reached through
  `core::storage::data_path("themes")`, so installed and portable modes both
  work. The folder is created on first use, not at startup.
- **Built-ins stay in Rust**, as the columns of the registry. They need no
  parsing at startup and cannot be broken by a bad file.
- **Discovery** reads the folder when Settings opens, when a selected theme is
  needed, and on reload. Files larger than 1 MiB are skipped with a report.

### 3.5 Resolution

Input: a theme id. Output: one colour per token.

1. **Build the chain.** Follow `extends` from the chosen theme to its root.
   Stop with an error on a cycle, a missing parent, or more than eight links.
2. **Merge.** Walk the chain root first. Later files replace earlier ones, per
   palette name and per token.
3. **Resolve references.** Each `$name` is looked up in the merged palette. A
   child that only overrides palette entries therefore recolours everything the
   parent painted with them.
4. **Choose the fallback.** The fallback is always a built-in. If the chain
   ends at one, it is that built-in. If the chain ends at a theme written
   from scratch, it is the built-in of that theme's appearance
   (`baboon-dark` or `baboon-light`), and every token the chain leaves unset
   is reported (section 3.9): such a theme applies, filled, while its author
   finishes it.
5. **Fill in.** Walk the registry in declaration order. A token takes its
   explicit value if it has one. If it has none, it takes the fallback, with
   one exception in step 6.
6. **Dependents follow.** When the fallback is a built-in value and a token
   this token's rule depends on was set explicitly, directly or through
   another token, the rule is used instead. Setting `accent` on top of
   `baboon-dark` therefore moves `accent.border`, `progress.bar` and
   `browser.toolbar.active.background` with it, while unrelated greys keep their
   hand-picked values.

Step 6 is the one subtle part of the resolver. Phase 3 carries its gate. It
applies to themes written from scratch as well: a missing token whose inputs
the theme set follows them rather than the built-in.

### 3.6 Runtime

- **Storage.** `DARK_MODE_ENABLED` and `CLASSIC_LIGHT` become one `AtomicU32`
  per token plus the appearance, initialised at compile time to
  `baboon-light`, which is what untouched flags mean today. A read costs what
  a flag read costs now: no lock, no allocation, no new dependency.
- **Accessors.** `text_dark()` and the others become one-line wrappers over
  `theme::color(Token::Text)`; aliases return their token. `is_dark_mode()`
  stays and reads the appearance. `light_theme()`, the `light()` helper and
  the `LightTheme` enum go: a built-in is a column, not a branch.
- **Installing a theme** writes all the atomics on the UI thread before any
  drawing, where `set_dark_mode` and `set_light_theme` are called today. A
  frame cannot see half of one theme and half of another.
- **egui visuals.** `foundation_visuals()` takes the resolved theme and is
  built once per theme change. The per-frame `set_visuals` call keeps its
  place and hands over the cached copy, so nothing that relies on visuals
  being reset each frame changes. The warm Light's hand-written widget fills
  and strokes become the `button.*` and `control.*` tokens of
  `baboon-light`; whether `opaque_widget_fills` survives for other light
  themes is settled in Phase 3.
- **egui's own theme.** `ctx.set_theme` follows the resolved theme's
  appearance, as it follows the flag today.
- **Icon caches.** Installing a theme clears the thread-local recoloured-SVG
  caches and forgets their URIs, and the loading art is recoloured from
  tokens instead of `LoadingPalette`. The caches hold bytes and URIs, never a
  bare `TextureId`, and the button-icon cache keeps its own handles and sweep,
  so the "icons disappear" failure fixed in PR #103 is not reopened.
- **Tests.** Resolution is a pure function from files to a table, so most
  tests never touch the global. `with_theme` is widened to take a resolved
  table, so a test draws in any theme on its own thread.

### 3.7 Selection and preferences

On 2026-11-23 you chose one Theme menu with four choices, and following the
system always gives the warm Light, with "revisit the slots if custom themes
arrive". On 2026-11-25 you chose two slots under one menu (decision 7):

| Key | Type | Default | Meaning |
|---|---|---|---|
| `follow_system_theme` | bool | existing | Follow the OS appearance |
| `dark_mode` | bool | existing | The appearance when not following, and the fallback when the OS reports none |
| `light_theme` | string | `"baboon-light"` | Existing key, widened from `"warm"`/`"classic"` to any theme id. The theme used when the appearance is light |
| `dark_theme` | string | `"baboon-dark"` | New. The theme used when the appearance is dark |

- **Existing prefs files load unchanged.** `"warm"` reads as `baboon-light`,
  `"classic"` as `baboon-light-classic`, and a missing `dark_theme` as
  `baboon-dark`. Saving writes the new ids.
- **An older Baboon reading newer prefs** finds a `light_theme` it doesn't
  know, falls back to Warm (as it does for any unknown value today) and
  ignores `dark_theme`.
- **A slot may hold any theme.** If the dark slot holds a light theme, the
  theme's own appearance decides the egui theme and the light-only branches.
- **A missing or broken selected theme** falls back to the built-in of the
  slot, keeps the preference as written, and says so in the status line.

**Settings, Interface tab.**

- "Theme" keeps one menu: Use system setting, then every theme (built-ins and
  user themes) with its appearance beside its name. Picking a theme sets the
  slot of its appearance and turns system-following off.
- While "Use system setting" is chosen, two rows appear under it, "Light
  theme" and "Dark theme", so the system can pick Classic Light or a user
  theme. Today it always picks Warm.
- Three buttons: Open Themes Folder, Export Current Theme, Reload.
- A problems line appears under the menu when the active theme reported
  anything, with a button that lists the reports.

First run keeps today's four-choice picker.

### 3.8 Reloading

- The active theme's file and each file in its chain are checked for a changed
  modified time or length at most once a second, beside the existing
  `persist_prefs_throttled` check.
- The check runs only on frames that are drawn anyway. Baboon does not wake
  itself to poll, except while the Settings window is open, when it asks for
  one repaint a second. A theme edited while Baboon sits idle is picked up
  when Baboon next draws.
- A reload that fails keeps the last good colours and reports the failure.
- The Reload button does the same check on demand.

### 3.9 Reporting

Every problem is reported with the file name and, where there is one, the key.
None of them stops Baboon from drawing.

| Problem | Result |
|---|---|
| File is not valid JSON | Theme is listed as broken and cannot be applied; the active theme is kept |
| `name` missing | File name is used |
| `appearance` missing with no `extends` | Guessed from the luminance of `background`, and reported |
| Unknown token id | Key ignored, reported with the id |
| Unparseable colour | Key ignored, reported with the value |
| `$name` not in the palette | Key ignored, reported |
| `extends` names no theme | Falls back to the built-in for the appearance, reported |
| `extends` cycle or too deep | Theme is listed as broken |
| Reserved or duplicate id | Later file skipped, reported |
| Theme written from scratch leaves tokens unset | Applied; the unset tokens take the built-in of its appearance, and one notification names the file and every unset token |
| Token id in the rename table | Read as its new id, reported as renamed with the new id |

A theme written for a newer Baboon loads in an older one with its new tokens
reported as unknown. A theme written for an older Baboon loads in a newer one
with the new tokens filled by fallback.

### 3.10 Export and schema

- **Export Current Theme** writes a complete theme file, the starting point
  for a theme written from scratch: `name`, `appearance`,
  and every token with its resolved value, grouped and commented with each
  token's description and its aliases. For a user theme, the tokens it did not
  set are written as comments, so the file records what was chosen and what
  was inherited.
- **`baboon-theme.schema.json`** is generated from the registry into the
  themes folder: every id, its description, and the colour pattern. A theme
  that names it in `$schema` gets completion and descriptions in VS Code and
  other schema-aware editors.
- **`docs/themes.md`** is the authoring guide. Its token reference table is
  generated from the registry and checked by a test, so it cannot drift.

### 3.11 Fixed colours

These stay in code and are listed in the authoring guide as not themable:

- colour values read from tags (swatches, colour pickers, function gradients);
- the bitmap viewer's red, green, blue and alpha channel overlays;
- the alpha checkerboard;
- axis colours, wireframe and markers in the model viewport, and the flat
  material palette (the two allowed `model_preview/` folders);
- outlines and markers drawn over arbitrary images, which pick black or white
  from the image under them;
- accent colours embedded in icon artwork (`#EB4747`, `#7EEB47` and others).

The always-dark surfaces from finding 6 are ordinary tokens whose built-in
values happen to match. A light theme may lighten its terminal.

### 3.12 Left out on purpose

- TextMate scopes, semantic tokens and per-language rules.
- Icon themes and recolouring of icon accents.
- Fonts, sizes, spacing and corner radii.
- A marketplace or an in-app theme download.
- A high-contrast appearance. It fits later as a third appearance value.
- A user override map in prefs (decision 2).
- Importing VS Code themes (decision 5).
- Colour functions in theme files. Rules live in the registry; files hold
  values.

---

## 4. Phases

One feature, one PR, one commit per step, following the repo's convention.

Every gate below names a case that must make it fail, so no gate can pass by
agreeing with itself.

### Phase 0 — Consolidate the scattered colours: DONE

Built by Paddy in #131 (merged 2026-11-20) and #133 (merged 2026-11-23).
Checked against the code at `750178f3`:

| Step | State |
|---|---|
| 1–2. Inventory and name the clusters | Done. Every themable colour outside `style.rs` became a named accessor; fixed ones carry `// fixed:` with a reason |
| 3. Move the theme branches | Done outside `ui_kit/`. Five `ui_kit/` files still read the theme themselves (section 1) |
| 4. Split double meanings | Done. `material_delete_text` is only called inside `style.rs`; the green triangle only paints open folders |
| 5. Unify status colours | Done, in all three looks, with egui's `warn_fg_color` and `error_fg_color` set from them. Unlike the plan, this changed dark-mode pixels too; you accepted that on 2026-11-23 |

| Gate | State |
|---|---|
| Source scan | `palette_gate.rs`: fails on a colour constructor or theme read outside `ui_kit/`, the two `model_preview/` folders and `// fixed:` lines. Passes |
| Contrast | `status_colours_read_on_every_theme_background`: error, warning and success at 4.5:1 or better on four backgrounds in all three looks. Passes |

Two things Phase 0 did beyond the plan: #131 restyled the light look (the
warm Light, peach chrome and orange accents), and #133 kept the old grey look
as Classic Light. Both are now built-ins.

### Phase 1 — Registry behind the accessors: DONE (`269ce20e`)

**As built.** The module is `ui_kit/themes` (egui exports a `theme`). The
registry has 136 tokens, not 113: Phase 1 found 24 colours the measurement
missed (`separator`, `loading.*`, `button.border.open`, `button.destructive`,
`slider.rail.border`, `control.well`, `code.inline.background`,
`status.error.border`, `material.*` and `syntax.*`), and merged
`tab.focus.border` into `focus.border`. The golden gate compares every
built-in's colours and its whole `Visuals` against the values captured before.

**Purpose.** Replace the two flags and the `LightTheme` enum with resolved
colours while all three built-ins stay pixel-identical.

**Work**

1. Capture a golden table on the commit before this phase: every accessor's
   value under each of the three built-ins, and `foundation_visuals()` for
   each. Appendix A was produced this way; the gate keeps the dump as a test.
2. Add `ui_kit/theme/` with the registry macro, the 113-token table, the
   atomics, and `with_theme` widened to take a table.
3. Turn each accessor into a wrapper over its token, and each alias into a
   wrapper over its token. Remove `light()`, `light_theme()`,
   `CLASSIC_LIGHT` and the `*_for(dark)` helpers.
4. Route the theme reads and literals left in `close.rs`, `selection_art.rs`,
   `slider.rs`, `button_icons.rs` and `focus.rs` through tokens or mark them
   fixed, and widen the palette gate to scan `ui_kit/` except `style.rs` and
   `theme/`.
5. Build `foundation_visuals()` from the resolved theme and cache it, with the
   warm Light's hand-written widget values coming from its tokens.
6. Recolour the loading art from tokens, and clear the recoloured-SVG caches
   on install.

**Gates**

- Every token equals the golden table under all three built-ins, and the
  three `Visuals` compare equal to the captured ones. *Must fail when* any
  registry value differs; shown by a test that alters one entry in a copy of
  the table and expects the comparison to name it.
- Every alias equals its token under all three built-ins. *Must fail when* an
  alias is given its own value; shown on a fixture.
- Token ids are unique and well formed, and every rule refers only to earlier
  tokens. *Must fail when* a rule refers forward; shown on a two-row fixture.
- The widened palette gate passes. *Must fail when* a literal is added to a
  `ui_kit/` file other than `style.rs` and `theme/`; shown on a fixture
  string.
- The existing `theme_switches_preserve_font_sizes_spacing_and_widget_dimensions`
  test passes unchanged.

**Risk.** Low. No user-visible change is intended, and the golden table says
whether there is one. The loading art is the one piece checked by eye.

### Phase 2 — Theme files: DONE (`e03b44bb`)

**Purpose.** The feature a user sees: drop a file in a folder, pick it, edit it
live.

**Ids:** this phase makes Appendix A's ids a file format. They were reviewed
in the naming pass (decision 6); any change after this phase ships goes
through the rename table.

**Work**

1. Parser with the comment pre-pass, the rename table, and the reports of
   section 3.9.
2. Discovery, identity and the chain of section 3.5, steps 1 to 5.
3. `light_theme` widened to theme ids with the two legacy values mapped, and
   `dark_theme` added, in `GuiPrefs`, `prefs_from_value` and `prefs_to_value`.
4. The Theme menu, the system rows, the three buttons and the problems line
   (section 3.7).
5. Reload check.
6. Export, the schema file, `docs/themes.md`, and updates to the README
   preferences paragraph, `docs/help.json` and `ARCHITECTURE.md`.

**Gates**

- Export then load gives back the same table, for all three built-ins and for
  a theme with a parent. *Must fail when* the exporter drops or misnames a
  token; shown by removing one line from an exported file.
- A corpus of malformed files (truncated, wrong types, bad hex, unknown keys,
  self-parent, two-file cycle, nine-deep chain) never panics and produces the
  exact report expected for each. *Must fail when* a report is missing; each
  file's expected report is asserted, not merely the absence of a panic.
- A palette-only child recolours its parent's tokens. *Must fail when*
  references are resolved before merging.
- A file using a renamed id loads its colour into the new token and reports
  the rename; every rename points at a live token and none shadows one.
  *Must fail when* a rename points at a missing token; shown on a fixture.
- A failed reload leaves every atomic unchanged. *Must fail when* install
  happens before validation.
- Legacy prefs select what they select today: only `dark_mode`, neither key,
  `light_theme: "warm"`, `light_theme: "classic"`, and an unknown
  `light_theme`. *Must fail when* `"classic"` is not mapped; shown by
  dropping the mapping in a fixture.

**Risk.** Medium. This is new file handling on a user-writable folder. Path
handling and the Open Folder button need a Windows check that cannot be done
on this machine; they reuse the code paths prefs already use.

### Phase 3 — Rules: DONE (`b8d6ad19`)

**As built.** 43 tokens carry a rule; rules may only read tokens declared
before them, so they resolve in one pass. The rule forms are reference, tint,
shade, mix and alpha (alpha is unused by the table so far). The material
editor, diff, terminal and syntax groups were left without rules on purpose:
their built-in values are hand-picked per surface and a fitted rule would move
them when a theme changes the page. `opaque_widget_fills` needed no decision:
it already keys on the appearance, so every light theme gets it. The examples
are in `docs/theme-examples`.

**Purpose.** An override carries its dependents with it: "Baboon Dark with an
orange accent" is one line.

**Work**

1. Fit each token's rule to `baboon-dark` and to the warm `baboon-light`;
   mark the tokens no rule fits as independent.
2. Enable step 6 of section 3.5.
3. Decide whether `opaque_widget_fills` stays for light themes other than
   the built-ins.
4. Ship two or three example themes, one written from scratch and the rest
   extending a built-in with a palette and a handful of tokens, as worked
   examples in the themes documentation.

**Gates**

- For `baboon-dark` and `baboon-light`, the rule-derived value of every token
  is within a set channel tolerance of the hand-picked one, so an override
  that changes nothing changes nothing visible. Tokens that cannot meet it are
  marked independent and listed. *Must fail when* a rule is wrong; shown by a
  fixture rule with the mix fraction doubled.
- Setting only `accent` over `baboon-dark` changes exactly the tokens whose
  rules depend on it and leaves every other token equal to the built-in.
  *Must fail when* an unrelated token moves.
- A theme written from scratch with tokens missing applies, fills them from
  the built-in of its appearance (or by rule where step 6 applies), and
  reports exactly the missing ids. *Must fail when* one missing id is left
  out of the report.

**Risk.** Medium. Which tokens follow an override is a matter of looking at
them. It needs your eye on the example themes in the running app.

### Phase 4 — Theme editor: DONE (`317f54b6`)

**As built.** Search is by id, description and group, not alias: aliases are
accessor names in `style.rs`, which the registry does not record. The window
is `shell/theme_editor.rs`, the draft and its saves `ui_kit/themes/draft.rs`.
Save edits the colour entries in place and writes the file whole only when its
layout does not allow that, then checks the result reads back as the draft.
Save As chooses the new theme. The editor opens on the theme in use; to edit
another, choose it first.

**Purpose.** Edit a theme inside Baboon and see it live (decision 4).

**Work**

1. A Theme Editor window, opened from Settings and from the Theme menu: the
   tokens grouped as in Appendix A, with search by id, description or alias.
2. Each row shows its id, a swatch using the existing colour picker, whether
   the value is set, inherited from the parent, or following a dependency
   (step 6), and a reset to the inherited value.
3. Live apply: each edit installs the edited theme; Cancel reinstalls the
   theme that was active when the editor opened.
4. Save writes the edited user theme back to its file, keeping its `extends`,
   palette and comments it can keep, and writing only the tokens it sets.
   Save As writes a new user theme that extends the theme being edited and
   holds only what changed. Editing a built-in always goes through Save As.
5. Saving while the file has changed on disk asks first, as tag saves do.

**Gates**

- Save As then load gives the table the editor showed. *Must fail when* a
  changed token is not written; shown by dropping one from a fixture save.
- Save As writes only changed tokens. *Must fail when* an inherited value is
  written as set.
- Cancel restores every atomic to its value before the editor opened. *Must
  fail when* one live edit survives Cancel.
- Reset on a row makes it inherit again and removes the key on save.

**Risk.** Medium. It is new UI over the Phase 2 machinery; the file round trip
is the part tests can cover, the editing feel is the part only the running app
shows.

### Phase 5 — Optional: DONE

**As built.** All three. The gallery is `shell/theme_gallery.rs` (View menu
and the editor's Gallery button). Contrast pairs are `ui_kit/themes/contrast.rs`:
text and status colours at 4.5:1, muted text at 3:1, shown in the editor and
in Settings. `link.text` and `field.help.text` are left out, because the light
built-ins draw them at 2.2 to 2.6:1 on the page; that is worth a look on its
own. Preview on hover covers the Theme menu and the two system menus.

Each item is independent and none is needed for the feature to be useful.

- **Theme gallery.** A window that draws one of everything, grouped by token,
  for authors and for screenshots.
- **Contrast warnings.** The registry declares foreground and background
  pairs; the editor and the problems line show pairs under their threshold.
  Warnings only.
- **Preview on hover** in the theme menu, reverting when the menu closes
  without a choice.

---

## 5. Verification outside the test suite

- **Panel screenshots.** `run_ui_test` returns shapes, and the model preview's
  headless renderer (`model_preview/harness/shot.rs`) draws only the 3D view.
  There is no headless rasteriser for ordinary egui panels, so before and
  after shots for Phase 1's loading art and for Phase 3 come from the running
  app unless one is added.
- **Shots cover all three built-ins**, a close-up of dense field rows, the
  shader grid, and one custom theme.
- **Windows.** The themes folder path, Open Themes Folder and modified-time
  checks cannot be exercised here. They are verified by equivalence with the
  prefs code they share, and owed a check on a Windows machine.
- **App test.** Reload while editing a file in an external editor, and the
  editor's live apply, Cancel and Save As, are behaviours only the running app
  shows.

---

## Decisions

All made on 2026-11-25 unless dated otherwise.

1. **Status colours unified.** Done in #131 and #133, in all three looks,
   accepted on 2026-11-23.
2. **Themes are written from scratch or extend another**, with full support
   for both. No override map in prefs.
3. **Unset tokens.** A theme that extends another takes unset tokens from its
   parent; a theme written from scratch should set every token, and when it
   does not it still applies, with the unset tokens taken from the built-in of
   its appearance and a notification naming each one. Either way, dependents
   follow an override (section 3.5, step 6). Rules are fitted to `baboon-dark`
   and the warm `baboon-light`.
4. **The in-app editor is part of the feature**, as Phase 4.
5. **No VS Code theme import.**
6. **Token names: naming pass done and signed off.** Every id ends in what it
   colours (`.background`, `.border`, `.text`, `.icon`, `.hover`) except the
   roots and the Zed-style status triplets, and names its meaning rather than
   the feature that first used it. 42 of the 113 ids were renamed; Appendix A
   shows the result. Nothing had shipped, so none needs a rename-table row.
7. **Two slots, one menu.** `light_theme` holds a theme id (`"warm"` and
   `"classic"` mapped), `dark_theme` joins it, and two rows under "Use system
   setting" pick each appearance's theme. This replaces "System always gives
   Warm" (2026-11-23).
8. **No token cap.** The registry grows and shrinks freely; renamed, merged
   and removed ids go in a rename table so old theme files keep loading.

---

## Appendix A — Token table

*This is the table as planned. The registry as built, with 136 tokens, their
values and their rules, is generated into [docs/themes.md](themes.md); Phase 1
says what changed.*

Every colour source in the code, measured at `750178f3` by calling each
accessor and reading `foundation_visuals()` under each built-in. The built-ins
are these values exactly. "Light" is the warm Light, the default.

Roots are marked ★. "Source" is the accessor or visuals field that holds the
value now; "+n" lists the aliases that return the same colour in all three
looks, checked when the table was generated.

### Core

| Token | Source | Dark | Light | Classic Light |
|---|---|---|---|---|
| `background` ★ | `editor_bg` | `#282828` | `#F2F1F0` | `#E0E0DC` |
| `text` ★ | `text_dark` | `#F2F2F2` | `#1C1815` | `#191918` |
| `accent` ★ | `visuals.selection.bg_fill` | `#406C86` | `#92C0E2` | `#9EC0DE` |
| `panel.background` | `left_panel` | `#1F1F1F` | `#EBE5E0` | `#EEEEEA` |
| `row.background` | `row_type` | `#373737` | `#ECE8E4` | `#DBDBD8` |
| `border` | `grid_line` | `#525252` | `#DED6CF` | `#B4B4AE` |
| `text.muted` | `subtle_dark` | `#A4A4A4` | `#6E6660` | `#52524E` |
| `text.strong` | `text_strong` | `#FFFFFF` | `#000000` | `#000000` |
| `accent.border` | `visuals.selection.stroke` | `#78AAC6` | `#263F4E` | `#263F4E` |
| `accent.text` | `foundation_blue` (+1: `playhead`) | `#86B8D5` | `#B4440C` | `#0F2B40` |
| `focus.border` | `focused_tab_border(true)` | `#A0A0A0` | `#A0A0A0` | `#A0A0A0` |
| `link.text` | `visuals.hyperlink_color` | `#5AAAFF` | `#009BFF` | `#009BFF` |
| `modal.backdrop` | `modal_scrim` | `#0000008C` | `#0000008C` | `#0000008C` |

### Chrome

| Token | Source | Dark | Light | Classic Light |
|---|---|---|---|---|
| `menu_bar.background` | `menu_bar` | `#323232` | `#E5CCB2` | `#DBDBD8` |
| `chrome.background` | `chrome_bar` | `#373737` | `#E5CCB2` | `#DBDBD8` |
| `chrome.border` | `chrome_edge` | `#525252` | `#D0966A` | `#B4B4AE` |
| `tab.active.background` | `active_tab` | `#484848` | `#E5CCB2` | `#A1A19D` |
| `tab.focus.border` | `focused_tab_border(false)` | `#D0966A` | `#D0966A` | `#4E78A0` |
| `tab.modified.tint` | `dirty_tab_tint` | `#B8860B` | `#B8860B` | `#B8860B` |
| `browser.toolbar.background` | `browser_toolbar_bg` | `#444444` | `#EEECEA` | `#E2E2DE` |
| `browser.toolbar.active.background` | `browser_toolbar_active` | `#497088` | `#5D899E` | `#5D899E` |
| `browser.search.background` | `browser_search_bg` | `#00000066` | `#EDD3B980` | `#FFFFFF66` |
| `tree.folder.open.icon` | `disclosure_triangle_green` (+1: `open_folder_icon`) | `#22CD54` | `#22CD54` | `#22CD54` |
| `tree.folder.closed.icon` | `disclosure_triangle_blue` | `#186FCD` | `#186FCD` | `#186FCD` |
| `badge.background` | `group_badge_fill` | `#303A42` | `#E8EDF0` | `#E8EDF0` |
| `badge.border` | `group_badge_edge` | `#4C5962` | `#C4CED5` | `#C4CED5` |
| `badge.text` | `group_badge_text` | `#E2EBF0` | `#3A464F` | `#3A464F` |
| `badge.read_only.background` | `read_only_badge` | `#D2913C` | `#D2913C` | `#D2913C` |
| `badge.read_only.icon` | `read_only_badge_mark` | `#1E1E1E` | `#1E1E1E` | `#1E1E1E` |
| `favourite.icon` | `favourite_star` | `#F2C430` | `#F2C430` | `#F2C430` |
| `clipboard.cut.border` | `clipboard_cut_mark` | `#FFD138` | `#FFD138` | `#FFD138` |
| `clipboard.copy.border` | `clipboard_copy_mark` | `#47AEEB` | `#47AEEB` | `#47AEEB` |
| `field.help.text` | `help_cue` | `#3DA1CC` | `#3DA1CC` | `#3DA1CC` |
| `welcome.panel.background` | `welcome_side_panel` | `#00000033` | `#EFD2B950` | `#D6D6D650` |
| `progress.background` | `progress_track` | `#1F1F1E` | `#E2D0C2` | `#D7D7D2` |
| `progress.bar` | `progress_fill` | `#456F84` | `#5B92AC` | `#5B92AC` |

### Controls

| Token | Source | Dark | Light | Classic Light |
|---|---|---|---|---|
| `button.background` | `visuals.inactive.weak_bg_fill` | `#FFFFFF1A` | `#FFFFFF80` | `#0000001A` |
| `button.hover` | `visuals.hovered.weak_bg_fill` (+1: `visuals.open.weak_bg_fill`) | `#FFFFFF33` | `#FFFFFFBF` | `#00000033` |
| `button.active` | `visuals.active.weak_bg_fill` | `#FFFFFF40` | `#FFFFFF40` | `#00000040` |
| `button.border` | `visuals.inactive.bg_stroke` | `#00000000` | `#D6CDC5` | `#00000000` |
| `button.border.hover` | `visuals.hovered.bg_stroke` | `#969696` | `#BEB2A8` | `#696969` |
| `button.border.active` | `visuals.active.bg_stroke` | `#FFFFFF` | `#B0A297` | `#000000` |
| `button.primary.background` | `import_button_fill` | `#287A3E` | `#287A3E` | `#287A3E` |
| `menu.hover` | `context_menu_hover` | `#FFFFFF1A` | `#451D0A1A` | `#0000001A` |
| `menu_bar.item.hover` | `menu_row_hover` | `#FFFFFF1A` | `#0000001A` | `#0000001A` |
| `control.background` | `visuals.inactive.bg_fill` | `#444444` | `#F9F8F8` | `#C9C9C6` |
| `control.hover` | `visuals.hovered.bg_fill` | `#525252` | `#FCFBFB` | `#B3B3B0` |
| `control.active` | `visuals.active.bg_fill` | `#5C5C5C` | `#F5F5F4` | `#A8A8A5` |
| `input.background` | `foundation_input` (+1: `foundation_input_read_only`) | `#0000004D` | `#EDD3B980` | `#FFFFFF4D` |
| `input.border` | `foundation_input_edge` | `#545454` | `#CC9966` | `#70706C` |
| `card.background` | `foundation_documentation_bg` | `#FFFFFF0D` | `#EDD3B780` | `#0000000D` |
| `settings.card.background` | `settings_card_fill` | `#FFFFFF0D` | `#0000000D` | `#0000000D` |
| `slider.rail.background` | `slider_rail_dark` | `#0000005A` | `#0000005A` | `#0000005A` |

### Field editor

| Token | Source | Dark | Light | Classic Light |
|---|---|---|---|---|
| `group.background` | `foundation_group_bg` | `#2B2B2B` | `#F5F4F3` | `#ECECEA` |
| `group.border` | `foundation_group_edge` | `#525252` | `#D8CFC7` | `#989894` |
| `section.header.background` | `foundation_section_bar` | `#444444` | `#F2E7DB` | `#D6D6D2` |
| `block.header.background` | `foundation_block_bar` | `#484848` | `#F0E2D4` | `#CCCCC8` |
| `block.header.text` | `foundation_block_text` | `#F2F2F2` | `#1C1815` | `#191918` |
| `block.border` | `foundation_block_edge` | `#5E5E5E` | `#D6C8BC` | `#9A9A95` |
| `block.jump.icon` | `foundation_jump_cyan` | `#4DD0E1` | `#4DD0E1` | `#4DD0E1` |
| `block.jump.highlight` | `jump_glow` | `#FFD70026` | `#FFD70026` | `#FFD70026` |
| `flags.hover` | `foundation_flag_hover` | `#3A3A38` | `#ECE8E4` | `#E8E8E4` |

### Status

| Token | Source | Dark | Light | Classic Light |
|---|---|---|---|---|
| `status.error` | `invalid_value_text` (+12: `error_text`, `replace_warning_text`, `chimp_error_text`, `save_error_text`, `installed_overwrite_text`, `panel_error_text`, `bitmap_error_text`, `chimp_browser_error_text`, `review_error_text`, `first_run_error_text`, `reference_missing_text`, `visuals.error_fg_color`) | `#FF7C7C` | `#A6232D` | `#A6232D` |
| `status.error.background` | `invalid_value_fill` | `#301E20` | `#FFF0F0` | `#FFF0F0` |
| `settings.card.error.background` | `settings_card_invalid_fill` | `#622824` | `#FFDED8` | `#FFDED8` |
| `status.warning` | `unapplied_value_text` (+7: `import_warning_text`, `header_notice_text`, `filter_warning_text`, `settings_warning_text`, `expert_mode_text`, `chimp_raw_value_text`, `visuals.warn_fg_color`) | `#F0BE5A` | `#7A5000` | `#7A5000` |
| `status.warning.background` | `unapplied_value_fill` | `#362B18` | `#FFF3C9` | `#FFF7DC` |
| `status.success` | `good_news` (+3: `match_tick`, `accept_text`, `add_icon`) | `#7ECD85` | `#10642A` | `#10642A` |
| `action.destructive` | `material_delete_text` (+3: `value_decreased`, `destructive_icon`, `unused_value_text`) | `#E25C5C` | `#8B0000` | `#8B0000` |
| `banner.warning.background` | `disk_banner_fill` | `#483816` | `#FCF0CC` | `#FCF0CC` |
| `banner.warning.border` | `disk_banner_edge` | `#8C6820` | `#DEB048` | `#DEB048` |
| `banner.warning.icon` | `disk_banner_icon` | `#F2C430` | `#B07800` | `#B07800` |
| `banner.error.background` | `disk_missing_fill` | `#4E201E` | `#FCE2DF` | `#FCE2DF` |
| `banner.error.border` | `disk_missing_edge` | `#9E3E38` | `#E08076` | `#E08076` |
| `banner.error.icon` | `disk_missing_icon` | `#F05C50` | `#C83026` | `#C83026` |

### Diff

| Token | Source | Dark | Light | Classic Light |
|---|---|---|---|---|
| `diff.added` | `added_text` | `#7EBA6C` | `#7EBA6C` | `#7EBA6C` |
| `diff.removed` | `removed_text` | `#D66A6A` | `#D66A6A` | `#D66A6A` |
| `diff.modified` | `modified_text` | `#D6A82E` | `#D6A82E` | `#D6A82E` |
| `diff.unchanged` | `unchanged_text` | `#828282` | `#828282` | `#828282` |
| `diff.added.background` | `added_wash` | `#1C2E20` | `#1C2E20` | `#1C2E20` |
| `diff.removed.background` | `removed_wash` | `#341E1E` | `#341E1E` | `#341E1E` |

### Shader editor and graphs

| Token | Source | Dark | Light | Classic Light |
|---|---|---|---|---|
| `shader.panel.background` | `material_panel` | `#3A3C38` | `#F6F3F0` | `#EEEEEB` |
| `shader.panel.border` | `material_panel_edge` | `#484A46` | `#C4B8AE` | `#A8A8A2` |
| `shader.grid` | `material_grid_light` | `#363834` | `#DAD2CB` | `#C6C6C0` |
| `shader.row.reference.background` | `material_ref_row` | `#1E3A28` | `#A6CDA6` | `#A6CDA6` |
| `shader.row.numeric.background` | `material_numeric_row` | `#3E2D27` | `#E8BFAB` | `#E8BFAB` |
| `shader.row.data.background` | `material_data_row` | `#2A2B29` | `#E8E2DD` | `#D8D8D8` |
| `shader.row.function.background` | `material_function_row` | `#3A2F20` | `#EFCD89` | `#EFCD89` |
| `shader.input.border` | `material_input_edge` | `#52564E` | `#AFA298` | `#707070` |
| `shader.text` | `material_text` | `#E7E8E2` | `#1C1815` | `#141414` |
| `shader.text.muted` | `material_muted_text` | `#9B9E96` | `#6E6660` | `#606060` |
| `shader.row.modified.marker` | `shader_modified_accent` | `#E09E3E` | `#E09E3E` | `#E09E3E` |
| `graph.background` | `function_plot_bg` | `#40403E` | `#E6E0DB` | `#CDCDCD` |
| `graph.grid` | `function_grid_line` | `#84847E` | `#766C64` | `#5C5C58` |
| `graph.container.background` | `graph_container_fill` | `#00000040` | `#000000` | `#000000` |
| `graph.band.background` | `graph_band_fill` | `#FFFFFF1C` | `#FFFFFF1C` | `#FFFFFF1C` |
| `graph.band.border` | `graph_band_edge` | `#FFFFFF78` | `#FFFFFF78` | `#FFFFFF78` |
| `graph.curve.primary` | `graph_curve(0)` | `#2ABE48` | `#2ABE48` | `#2ABE48` |
| `graph.curve.secondary` | `graph_curve(1)` | `#DC3737` | `#DC3737` | `#DC3737` |
| `graph.curve.primary.fill` | `graph_curve_fill(0)` | `#27BA451A` | `#27BA451A` | `#27BA451A` |
| `graph.curve.secondary.fill` | `graph_curve_fill(1)` | `#D83B3B1A` | `#D83B3B1A` | `#D83B3B1A` |

### Terminal, code and syntax

| Token | Source | Dark | Light | Classic Light |
|---|---|---|---|---|
| `terminal.background` | `terminal_bg` | `#181817` | `#181817` | `#181817` |
| `terminal.text` | `terminal_text` | `#E8E8E4` | `#E8E8E4` | `#E8E8E4` |
| `terminal.warning` | `terminal_warning_text` | `#EEC45B` | `#EEC45B` | `#EEC45B` |
| `terminal.error` | `terminal_error_text` | `#F46969` | `#F46969` | `#F46969` |
| `terminal.success` | `terminal_success_text` | `#7BB889` | `#7BB889` | `#7BB889` |
| `code.background` | `help_code_bg` | `#181B1D` | `#F7EEE7` | `#EEF1F3` |
| `syntax.markup.tag` | `markup_tag_text` | `#C592E6` | `#792E9C` | `#792E9C` |
| `syntax.markup.warning` | `markup_warning_text` | `#F0B446` | `#B06800` | `#B06800` |
| `chimp.archive.text` | `chimp_archive_text` | `#D29646` | `#D29646` | `#D29646` |
| `search.match.background` | `search_match_bg` | `#FFDC46` | `#FFDC46` | `#FFDC46` |
| `search.match.active.background` | `search_active_match_bg` | `#FFA826` | `#FFA826` | `#FFA826` |
| `search.match.text` | `search_match_text` | `#000000` | `#000000` | `#000000` |

---

## Appendix B — Phase 0 record

What #131 and #133 did with the original plan's literal clusters, for history.

| Original cluster | Now |
|---|---|
| Warning text and its other shades | `status.warning`, with six feature-named aliases |
| Error text and its other shades | `status.error`, with eleven feature-named aliases |
| Success, accept and "identical" ticks | `status.success` (`good_news`, `match_tick`, `accept_text`, `add_icon`) |
| Destructive red | `action.destructive` (`material_delete_text`, `destructive_icon`, `value_decreased`, `unused_value_text`) |
| Modified accent, dirty tab tint | `shader.row.modified.marker`, `tab.modified.tint` |
| Favourite star, cut and copy outlines | `favourite.icon`, `clipboard.cut.border`, `clipboard.copy.border` |
| Function curves and fills | `graph.curve.*` |
| Invalid settings card | `settings.card.error.background` |
| White or black by appearance | `text.strong` |
| Neutral hover overlay | `menu_bar.item.hover` |
| Loading art | Not done: still `LoadingPalette` hex strings in `shell/loading.rs` (Phase 1, step 6) |
| Chimp JSON syntax palette | Gone from the code; the old plan's `syntax.*` tokens other than the two markup ones are dropped |

---

## Appendix C — Sources

Read by the research agents on 2026-11-16. I have not re-checked them myself.

**VS Code**

- Colour registry and transforms: `src/vs/platform/theme/common/colorUtils.ts` and `colors/` beside it, <https://github.com/microsoft/vscode/tree/main/src/vs/platform/theme/common>
- Theme loading, `include`, precedence, storage: `src/vs/workbench/services/themes/common/colorThemeData.ts`, <https://github.com/microsoft/vscode/blob/main/src/vs/workbench/services/themes/common/colorThemeData.ts>
- Preferred dark and light themes: `themeConfiguration.ts` in the same folder
- Picker preview and theme export: <https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/themes/browser/themes.contribution.ts>
- Theme colour reference (971 ids): <https://code.visualstudio.com/api/references/theme-color>
- Variables in themes, open since 2018: <https://github.com/microsoft/vscode/issues/56855>

**Other systems**

- Zed themes and schema: <https://zed.dev/docs/extensions/themes>, <https://zed.dev/schema/themes/v0.2.0.json>
- Helix themes, palette and `inherits`: <https://docs.helix-editor.com/themes.html>
- JetBrains themes: <https://plugins.jetbrains.com/docs/intellij/themes-customize.html>
- Sublime Text colour schemes: <https://www.sublimetext.com/docs/color_schemes.html>
- Blender 5.0 theme consolidation: <https://developer.blender.org/docs/release_notes/5.0/user_interface/>
- Godot editor theme settings: <https://github.com/godotengine/godot/blob/master/editor/settings/editor_settings.cpp>
- Rerun design tokens: <https://github.com/rerun-io/rerun/tree/main/crates/ui_tools/re_ui/data>
- WCAG contrast: <https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html>

**Read directly in this repo and its pinned dependencies**

- Everything in section 1 and both appendices.
- egui `Visuals`, `Context::set_theme`, `forget_image`, and ecolor's
  `Color32::from_hex` and alpha handling, at the pinned revision `ce0307b`.
