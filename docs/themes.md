# Themes

Baboon's colours come from a theme. Three are built in: **Dark**
(`baboon-dark`), **Light** (`baboon-light`, the warm default) and **Classic
Light** (`baboon-light-classic`, the grey look). A theme of your own is a JSON
file in the themes folder.

## Choosing a theme

*File → Settings → Interface → Theme* lists **Use system setting** and every
theme, each with its appearance. Picking a theme uses it from then on.
**Use system setting** switches between two themes as the operating system
switches between light and dark; the two menus under it choose them, so the
system can switch to Classic Light or to a theme of your own.

- **Open Themes Folder** opens the folder, making it first if needed, and
  writes `baboon-theme.schema.json` into it.
- **Edit Theme...** opens the Theme Editor on the theme in use (also
  *View → Theme Editor...*; see below).
- **Export Current Theme** writes the theme in use into the folder as a
  complete file: the best place to start a theme.
- **Reload** reads the folder again.

A theme in use is read again within a second of its file changing, so you can
edit it in another editor and watch Baboon follow. A file that stops reading
correctly keeps the colours it last gave, and Settings says why.

## The Theme Editor

The editor lists every token of the theme in use, grouped as in the token
list below, and searches them by id, description or group. Each row shows its
colour, its value and where the colour comes from: **set** by the theme,
**inherited** from the theme it extends (or the built-in), or **follows**
another token by its rule. Click a swatch to pick a colour; Baboon draws the
change at once. **Reset** unsets a token, so it inherits or follows again.

- **Save** writes the changes back to the theme's file. Its other keys, its
  palette and its comments are kept; a token set from the palette stays
  `$name` until you pick a new colour for it. If the file changed on disk since
  the editor read it, Baboon asks before overwriting it.
- **Save As...** writes a new theme that extends the one being edited and sets
  only the colours you changed, then chooses it. A built-in is kept this way.
- **Cancel**, or closing the window, puts back the colours drawn when the
  editor opened, as far back as the last save.

### Contrast

The editor, and Settings for the theme in use, list colour pairs that are
hard to read: text and status colours under 4.5:1 on the backgrounds they are
drawn on, muted text under 3:1. They are warnings; the theme still applies.

### Gallery

*View → Theme Gallery...*, or **Gallery** in the editor, draws one of every
kind of widget, the text colours and a swatch of every token, in the theme
drawn now, edits included.

### Previewing

Resting the pointer on a theme in the Theme menu, or in either menu under Use
system setting, draws Baboon in it until the pointer leaves or the menu
closes.

## Where themes live

In the `themes` folder of Baboon's data directory: `%APPDATA%\Baboon\themes`
on Windows when installed, beside `Baboon.exe` in portable mode. A theme's
**id is its file name**, lowercased: `forerunner.json` is `forerunner`.
Renaming the theme inside the file changes nothing else. The three built-in
ids are reserved, and files larger than 1 MiB are skipped.

## The file

```jsonc
{
  "$schema": "./baboon-theme.schema.json", // completion in editors that read schemas
  "name": "Forerunner",                    // shown in Settings
  "appearance": "dark",                    // "dark" or "light"
  "extends": "baboon-dark",                // optional: another theme's id
  "palette": {                             // optional: names for reuse below
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
| `name` | yes | The name Settings shows. Missing, the file name is used |
| `appearance` | without `extends` | `dark` or `light`. It picks egui's base look and the built-in that fills unset colours. Inherited from the parent otherwise; missing, it is guessed from `background` |
| `extends` | no | A built-in's id or another file's. Chains may be up to eight links long |
| `palette` | no | Names (`a-z`, `0-9`, `_`, `-`) to colours |
| `colors` | no | Token ids to a colour or a `$name` from the palette |

Colours are `#RGB`, `#RGBA`, `#RRGGBB` or `#RRGGBBAA`. Alpha is straight, as
in CSS: `#FFFFFF1A` is white at 10%. `//` and `/* */` comments and trailing
commas are allowed.

### Extending a theme

A theme that `extends` another takes every colour it does not set from it. A
child that only changes palette entries recolours everything its parent
painted with them. "Baboon Dark with an orange accent" is:

```jsonc
{ "name": "Orange", "extends": "baboon-dark", "colors": { "accent": "#E8862A" } }
```

### Colours that follow

Many tokens are derived from a few others: a panel is the page a little
darker, a border the page a little lighter, a badge the page mixed toward the
accent. When a theme sets a token and leaves one derived from it unset, the
derived one follows, computed from the theme's colour by the same rule that
gives the built-in its value. Set `background` alone and the panels, rows,
borders and headers move with it. A token the theme sets is never changed.

The **Follows** column of the token list gives each rule, for dark and for
light themes: "background 0.2 toward white" is the page moved a fifth of the
way to white. "nothing" means the token keeps its built-in value until a
theme sets it; the material editor, the diff washes, the terminal and the
syntax colours are like this, as are the status colours and `accent`
themselves. Rules were fitted to the Dark and warm Light built-ins, each
within 4 of 255 per channel of the colour they replace.

### A theme of its own

A theme with no `extends` should set every token. One that leaves some unset
still applies: the unset tokens follow what it sets where a rule says so and
come from the built-in of its appearance otherwise, and Settings lists them, so you can watch the theme while you finish it. Export
Current Theme writes every token, which makes a complete starting file.

### Problems

Every problem is listed under the Theme menu in Settings, with its file and
key. None of them stops Baboon from drawing.

| Problem | Result |
|---|---|
| Not valid JSON | The theme cannot be applied; the current one is kept |
| `name` missing | The file name is used |
| `appearance` missing with no `extends` | Guessed from `background` |
| Unknown token id, unknown key | Ignored |
| A colour that does not parse, a `$name` not in the palette | That colour is ignored |
| `extends` names no theme | The built-in of the appearance is used |
| `extends` goes round in a circle, or is more than eight links deep | The theme cannot be applied |
| A built-in's id, or an id another file has | The later file is skipped |
| A token id that was renamed | Read as its new id |

A theme chosen in Settings that cannot be applied shows the built-in of its
appearance, and the status line says so.

### Renamed tokens

When a token is renamed, merged into another or removed, Baboon keeps reading
its old id as its new one, so a theme written for an older Baboon keeps every
colour. A theme written for a newer Baboon lists its newer tokens as unknown.

## Colours themes do not change

Colours read from tags (swatches, colour pickers, function gradients), the
bitmap viewer's channel overlays and alpha checkerboard, the model viewport's
axes, wireframe, markers and flat material palette, keyword pill colours,
outlines drawn over images, and the accent colours inside icon artwork.

## Tokens

Every colour a theme can set, by group, with its value in each built-in and
the rule it follows. Example themes are in
[docs/theme-examples](theme-examples/). This table is generated from Baboon's token table; a test keeps it current.

<!-- tokens:begin -->

### Core

| Token | Where it appears | Dark | Light | Classic Light | Follows (dark; light) |
|---|---|---|---|---|---|
| `background` | Editor panes, windows and the page behind tag fields. | `#282828` | `#F2F1F0` | `#E0E0DC` | nothing; nothing |
| `text` | Body text and labels. | `#F2F2F2` | `#1C1815` | `#191918` | nothing; nothing |
| `accent` | Selection fill and other emphasis. | `#406C86` | `#92C0E2` | `#9EC0DE` | nothing; nothing |
| `panel.background` | Side panels: the tag browser, tile bars and import panes. | `#1F1F1F` | `#EBE5E0` | `#EEEEEA` | background 0.225 toward black; nothing |
| `row.background` | Striped and faint rows, and the background of non-interactive widgets. | `#373737` | `#ECE8E4` | `#DBDBD8` | background 0.07 toward white; background 0.037 toward black |
| `border` | Separators and borders between panes. | `#525252` | `#DED6CF` | `#B4B4AE` | background 0.195 toward white; nothing |
| `separator` | egui's separator lines and the outline of non-interactive widgets. | `#3C3C3C` | `#DED6CF` | `#B4B4AE` | background 0.093 toward white; nothing |
| `text.muted` | Secondary text: hints, counts and disabled labels. | `#A4A4A4` | `#6E6660` | `#52524E` | background 0.614 toward text; nothing |
| `text.strong` | Text that must stand out at full contrast: white on dark, black on light. | `#FFFFFF` | `#000000` | `#000000` | nothing; nothing |
| `accent.border` | Outline of selected items and text selection. | `#78AAC6` | `#263F4E` | `#263F4E` | nothing; nothing |
| `accent.text` | Accent text and marks: highlighted names, links in tables, the animation playhead. | `#86B8D5` | `#B4440C` | `#0F2B40` | nothing; nothing |
| `focus.border` | Border of the focused tab or pane. | `#A0A0A0` | `#D0966A` | `#4E78A0` | background 0.558 toward white; nothing |
| `link.text` | Hyperlinks. | `#5AAAFF` | `#009BFF` | `#009BFF` | nothing; nothing |
| `modal.backdrop` | Dimming laid over the app behind a modal dialog. | `#0000008C` | `#0000008C` | `#0000008C` | nothing; nothing |

### Chrome

| Token | Where it appears | Dark | Light | Classic Light | Follows (dark; light) |
|---|---|---|---|---|---|
| `menu_bar.background` | The menu bar and status bar. | `#323232` | `#E5CCB2` | `#DBDBD8` | background 0.047 toward white; nothing |
| `menu_bar.item.hover` | Fill of a hovered menu bar item. | `#FFFFFF1A` | `#0000001A` | `#0000001A` | nothing; nothing |
| `chrome.background` | Window chrome strips: tile title bars and menu panels. | `#373737` | `#E5CCB2` | `#DBDBD8` | background 0.07 toward white; nothing |
| `chrome.border` | Edge of the window chrome strips. | `#525252` | `#D0966A` | `#B4B4AE` | background 0.195 toward white; nothing |
| `tab.active.background` | The active document tab. | `#484848` | `#E5CCB2` | `#A1A19D` | background 0.149 toward white; nothing |
| `tab.modified.tint` | Tint mixed into a tab whose document has unsaved changes. | `#B8860B` | `#B8860B` | `#B8860B` | nothing; nothing |
| `browser.toolbar.background` | The tag browser's toolbar. | `#444444` | `#EEECEA` | `#E2E2DE` | background 0.13 toward white; background 0.021 toward black |
| `browser.toolbar.active.background` | A toggled-on button in the tag browser toolbar. | `#497088` | `#5D899E` | `#5D899E` | accent 0.03 toward white; nothing |
| `browser.search.background` | The tag browser's search box. | `#00000066` | `#EDD3B980` | `#FFFFFF66` | nothing; nothing |
| `tree.folder.open.icon` | Disclosure triangle and icon of an open folder. | `#22CD54` | `#22CD54` | `#22CD54` | nothing; nothing |
| `tree.folder.closed.icon` | Disclosure triangle of a closed folder. | `#186FCD` | `#186FCD` | `#186FCD` | nothing; nothing |
| `badge.background` | Fill of the tag-group badge beside a tag in the tree. | `#303A42` | `#E8EDF0` | `#E8EDF0` | background 0.292 toward accent; background 0.093 toward accent |
| `badge.border` | Outline of the tag-group badge. | `#4C5962` | `#C4CED5` | `#C4CED5` | nothing; nothing |
| `badge.text` | Text of the tag-group badge. | `#E2EBF0` | `#3A464F` | `#3A464F` | accent 0.863 toward white; text 0.27 toward accent |
| `badge.read_only.background` | The read-only badge on a tag or layout. | `#D2913C` | `#D2913C` | `#D2913C` | nothing; nothing |
| `badge.read_only.icon` | The mark drawn on the read-only badge. | `#1E1E1E` | `#1E1E1E` | `#1E1E1E` | nothing; nothing |
| `favourite.icon` | A favourite's filled star. | `#F2C430` | `#F2C430` | `#F2C430` | nothing; nothing |
| `clipboard.cut.border` | Outline of rows cut to the clipboard. | `#FFD138` | `#FFD138` | `#FFD138` | nothing; nothing |
| `clipboard.copy.border` | Outline of rows copied to the clipboard. | `#47AEEB` | `#47AEEB` | `#47AEEB` | nothing; nothing |
| `field.help.text` | The question mark beside a field that has help. | `#3DA1CC` | `#3DA1CC` | `#3DA1CC` | nothing; nothing |
| `welcome.panel.background` | The welcome screen's side column. | `#00000033` | `#EFD2B950` | `#D6D6D650` | nothing; nothing |
| `progress.background` | Track of a progress bar. | `#1F1F1E` | `#E2D0C2` | `#D7D7D2` | background 0.233 toward black; nothing |
| `progress.bar` | Filled part of a progress bar. | `#456F84` | `#5B92AC` | `#5B92AC` | accent 0.01 toward white; nothing |
| `loading.primary` | Brightest strokes of the loading animation. | `#BFBFBF` | `#333333` | `#333333` | background 0.702 toward white; background 0.788 toward black |
| `loading.secondary` | Middle strokes of the loading animation. | `#808080` | `#4D4D4D` | `#4D4D4D` | background 0.409 toward white; background 0.68 toward black |
| `loading.lines` | Faint lines of the loading animation. | `#404040` | `#999999` | `#999999` | background 0.112 toward white; background 0.365 toward black |

### Controls

| Token | Where it appears | Dark | Light | Classic Light | Follows (dark; light) |
|---|---|---|---|---|---|
| `button.background` | Fill of a button or combo box at rest. | `#FFFFFF1A` | `#FFFFFF80` | `#0000001A` | nothing; nothing |
| `button.hover` | Fill of a hovered or open button. | `#FFFFFF33` | `#FFFFFFBF` | `#00000033` | nothing; nothing |
| `button.active` | Fill of a pressed button. | `#FFFFFF40` | `#FFFFFF40` | `#00000040` | nothing; nothing |
| `button.border` | Outline of a button at rest. | `#00000000` | `#D6CDC5` | `#00000000` | nothing; nothing |
| `button.border.hover` | Outline of a hovered button. | `#969696` | `#BEB2A8` | `#696969` | background 0.512 toward white; nothing |
| `button.border.active` | Outline of a pressed button. | `#FFFFFF` | `#B0A297` | `#000000` | nothing; nothing |
| `button.border.open` | Outline of a button whose menu is open. | `#3C3C3C` | `#BEB2A8` | `#A0A0A0` | background 0.093 toward white; nothing |
| `button.destructive` | Accent fill of a button for an action to take with care. | `#EB4747` | `#EB4747` | `#EB4747` | nothing; nothing |
| `button.primary.background` | Fill of a primary, confirming button. | `#287A3E` | `#287A3E` | `#287A3E` | nothing; nothing |
| `menu.hover` | Fill of a hovered menu item or icon button. | `#FFFFFF1A` | `#451D0A1A` | `#0000001A` | nothing; nothing |
| `control.background` | Fill of a control such as a slider or checkbox at rest. | `#444444` | `#F9F8F8` | `#C9C9C6` | background 0.13 toward white; background 0.524 toward white |
| `control.hover` | Fill of a hovered control. | `#525252` | `#FCFBFB` | `#B3B3B0` | background 0.195 toward white; background 0.739 toward white |
| `control.active` | Fill of a pressed control. | `#5C5C5C` | `#F5F5F4` | `#A8A8A5` | background 0.242 toward white; background |
| `input.background` | Inside of a text input. | `#0000004D` | `#EDD3B980` | `#FFFFFF4D` | nothing; nothing |
| `input.border` | Outline of a text input. | `#545454` | `#CC9966` | `#70706C` | background 0.205 toward white; nothing |
| `card.background` | Documentation and note boxes. | `#FFFFFF0D` | `#EDD3B780` | `#0000000D` | nothing; nothing |
| `settings.card.background` | Cards in the Settings window. | `#FFFFFF0D` | `#0000000D` | `#0000000D` | nothing; nothing |
| `slider.rail.background` | Rail of a slider. | `#0000005A` | `#EDD3B980` | `#FFFFFF4D` | nothing; nothing |
| `slider.rail.border` | Outline of a slider's rail. | `#00000000` | `#CC9966` | `#70706C` | nothing; nothing |
| `control.well` | The well inside a checked checkbox or radio button. | `#000000` | `#FFFFFF` | `#FFFFFF` | nothing; nothing |
| `code.inline.background` | Background of inline code. | `#404040` | `#ECE8E4` | `#DBDBD8` | background 0.112 toward white; background 0.037 toward black |

### Field editor

| Token | Where it appears | Dark | Light | Classic Light | Follows (dark; light) |
|---|---|---|---|---|---|
| `group.background` | Background of a group of fields. | `#2B2B2B` | `#F5F4F3` | `#ECECEA` | background; background |
| `group.border` | Outline of a group of fields, and of windows. | `#525252` | `#D8CFC7` | `#989894` | background 0.195 toward white; nothing |
| `section.header.background` | Header bar of a section of fields. | `#444444` | `#F2E7DB` | `#D6D6D2` | background 0.13 toward white; nothing |
| `block.header.background` | Header bar of a tag block, and of popup footers. | `#484848` | `#F0E2D4` | `#CCCCC8` | background 0.149 toward white; nothing |
| `block.header.text` | Text on a tag block header. | `#F2F2F2` | `#1C1815` | `#191918` | text; text |
| `block.border` | Outline of a tag block. | `#5E5E5E` | `#D6C8BC` | `#9A9A95` | background 0.251 toward white; nothing |
| `block.jump.icon` | Icon of the button that jumps to a block. | `#4DD0E1` | `#4DD0E1` | `#4DD0E1` | nothing; nothing |
| `block.jump.highlight` | Highlight on the block a jump landed on. | `#FFD70026` | `#FFD70026` | `#FFD70026` | nothing; nothing |
| `flags.hover` | Fill of a hovered flag row. | `#3A3A38` | `#ECE8E4` | `#E8E8E4` | background 0.081 toward white; background 0.037 toward black |

### Status

| Token | Where it appears | Dark | Light | Classic Light | Follows (dark; light) |
|---|---|---|---|---|---|
| `status.error` | Error text and icons, and warnings that something will be lost or overwritten. | `#FF7C7C` | `#A6232D` | `#A6232D` | nothing; nothing |
| `status.error.background` | Fill of a field holding an invalid value. | `#301E20` | `#FFF0F0` | `#FFF0F0` | nothing; nothing |
| `status.error.border` | Outline of a field holding an invalid value. | `#AB464C` | `#C04B54` | `#C04B54` | nothing; nothing |
| `status.warning` | Warning text and icons. | `#F0BE5A` | `#7A5000` | `#7A5000` | nothing; nothing |
| `status.warning.background` | Fill of a field holding a value that was not applied. | `#362B18` | `#FFF3C9` | `#FFF7DC` | status.warning 0.761 toward black; nothing |
| `status.success` | Success text and icons: matches, accepted changes, add buttons. | `#7ECD85` | `#10642A` | `#10642A` | nothing; nothing |
| `action.destructive` | Destructive actions and removals: delete buttons, decreased or unused values. | `#E25C5C` | `#8B0000` | `#8B0000` | nothing; nothing |
| `settings.card.error.background` | A Settings card holding an invalid value. | `#622824` | `#FFDED8` | `#FFDED8` | nothing; nothing |
| `banner.warning.background` | Fill of a warning banner, such as a tag changed on disk. | `#483816` | `#FCF0CC` | `#FCF0CC` | nothing; nothing |
| `banner.warning.border` | Outline of a warning banner. | `#8C6820` | `#DEB048` | `#DEB048` | nothing; nothing |
| `banner.warning.icon` | Icon on a warning banner. | `#F2C430` | `#B07800` | `#B07800` | nothing; nothing |
| `banner.error.background` | Fill of an error banner, such as a tag deleted on disk. | `#4E201E` | `#FCE2DF` | `#FCE2DF` | nothing; nothing |
| `banner.error.border` | Outline of an error banner. | `#9E3E38` | `#E08076` | `#E08076` | nothing; nothing |
| `banner.error.icon` | Icon on an error banner. | `#F05C50` | `#C83026` | `#C83026` | nothing; nothing |

### Diff

| Token | Where it appears | Dark | Light | Classic Light | Follows (dark; light) |
|---|---|---|---|---|---|
| `diff.added` | Text of something added. | `#7EBA6C` | `#7EBA6C` | `#7EBA6C` | nothing; nothing |
| `diff.removed` | Text of something removed. | `#D66A6A` | `#D66A6A` | `#D66A6A` | nothing; nothing |
| `diff.modified` | Text of something modified. | `#D6A82E` | `#D6A82E` | `#D6A82E` | nothing; nothing |
| `diff.unchanged` | Text of something unchanged. | `#828282` | `#828282` | `#828282` | nothing; nothing |
| `diff.added.background` | Wash behind an added line. | `#1C2E20` | `#1C2E20` | `#1C2E20` | nothing; nothing |
| `diff.removed.background` | Wash behind a removed line. | `#341E1E` | `#341E1E` | `#341E1E` | nothing; nothing |

### Shader editor and graphs

| Token | Where it appears | Dark | Light | Classic Light | Follows (dark; light) |
|---|---|---|---|---|---|
| `shader.panel.background` | The shader editor's panels. | `#3A3C38` | `#F6F3F0` | `#EEEEEB` | background 0.084 toward white; background |
| `shader.panel.border` | Outline of the shader editor's panels. | `#484A46` | `#C4B8AE` | `#A8A8A2` | background 0.149 toward white; nothing |
| `shader.grid` | The shader editor's grid lines and hovered drag handles. | `#363834` | `#DAD2CB` | `#C6C6C0` | background 0.065 toward white; nothing |
| `shader.row.reference.background` | A shader row holding a tag reference. | `#1E3A28` | `#A6CDA6` | `#A6CDA6` | nothing; nothing |
| `shader.row.numeric.background` | A shader row holding a number. | `#3E2D27` | `#E8BFAB` | `#E8BFAB` | nothing; nothing |
| `shader.row.data.background` | A shader row holding other data. | `#2A2B29` | `#E8E2DD` | `#D8D8D8` | background; nothing |
| `shader.row.function.background` | A shader row holding a function. | `#3A2F20` | `#EFCD89` | `#EFCD89` | nothing; nothing |
| `shader.row.modified.marker` | Stripe at the edge of a modified shader row. | `#E09E3E` | `#E09E3E` | `#E09E3E` | nothing; nothing |
| `shader.input.border` | Outline of an input in the shader editor. | `#52564E` | `#AFA298` | `#707070` | background 0.195 toward white; nothing |
| `shader.text` | Text in the shader editor. | `#E7E8E2` | `#1C1815` | `#141414` | background 0.939 toward text; text |
| `shader.text.muted` | Secondary text in the shader editor. | `#9B9E96` | `#6E6660` | `#606060` | background 0.566 toward text; nothing |
| `graph.background` | A function graph's plot. | `#40403E` | `#E6E0DB` | `#CDCDCD` | background 0.109 toward white; background 0.077 toward text |
| `graph.grid` | A function graph's grid lines. | `#84847E` | `#766C64` | `#5C5C58` | background 0.419 toward white; nothing |
| `graph.container.background` | The frame around a function graph. | `#00000040` | `#000000` | `#000000` | nothing; nothing |
| `graph.band.background` | The band between a function's two curves. | `#FFFFFF1C` | `#FFFFFF1C` | `#FFFFFF1C` | nothing; nothing |
| `graph.band.border` | Edges of the band between two curves. | `#FFFFFF78` | `#FFFFFF78` | `#FFFFFF78` | nothing; nothing |
| `graph.curve.primary` | A function graph's first curve. | `#2ABE48` | `#2ABE48` | `#2ABE48` | nothing; nothing |
| `graph.curve.secondary` | A function graph's second curve. | `#DC3737` | `#DC3737` | `#DC3737` | nothing; nothing |
| `graph.curve.primary.fill` | Fill under the first curve. | `#27BA451A` | `#27BA451A` | `#27BA451A` | nothing; nothing |
| `graph.curve.secondary.fill` | Fill under the second curve. | `#D83B3B1A` | `#D83B3B1A` | `#D83B3B1A` | nothing; nothing |

### Material editor

| Token | Where it appears | Dark | Light | Classic Light | Follows (dark; light) |
|---|---|---|---|---|---|
| `material.row.data.background` | A data row in the material editor. | `#D8D8D8` | `#D8D8D8` | `#D8D8D8` | nothing; nothing |
| `material.row.function.background` | A function row in the material editor. | `#EFCD89` | `#EFCD89` | `#EFCD89` | nothing; nothing |
| `material.box.background` | A value box at its default in the material editor. | `#E0E0E0` | `#E0E0E0` | `#E0E0E0` | nothing; nothing |
| `material.grid` | The material editor's grid lines. | `#5C5C5C` | `#5C5C5C` | `#5C5C5C` | nothing; nothing |
| `material.input.border` | Outline of an input in the material editor and colour picker. | `#707070` | `#707070` | `#707070` | nothing; nothing |
| `material.text` | Text in the material editor. | `#141414` | `#141414` | `#141414` | nothing; nothing |
| `material.text.muted` | Secondary text in the material editor. | `#606060` | `#606060` | `#606060` | nothing; nothing |

### Terminal, code and syntax

| Token | Where it appears | Dark | Light | Classic Light | Follows (dark; light) |
|---|---|---|---|---|---|
| `terminal.background` | The terminal. | `#181817` | `#181817` | `#181817` | nothing; nothing |
| `terminal.text` | Terminal text. | `#E8E8E4` | `#E8E8E4` | `#E8E8E4` | nothing; nothing |
| `terminal.warning` | Warnings in the terminal. | `#EEC45B` | `#EEC45B` | `#EEC45B` | nothing; nothing |
| `terminal.error` | Errors in the terminal. | `#F46969` | `#F46969` | `#F46969` | nothing; nothing |
| `terminal.success` | Successes in the terminal. | `#7BB889` | `#7BB889` | `#7BB889` | nothing; nothing |
| `code.background` | Code blocks in the help. | `#181B1D` | `#F7EEE7` | `#EEF1F3` | nothing; nothing |
| `syntax.plain` | Plain text in a JSON view. | `#D2D6DE` | `#2D323A` | `#2D323A` | nothing; nothing |
| `syntax.key` | Keys in a JSON view. | `#F4BF5C` | `#915B00` | `#915B00` | nothing; nothing |
| `syntax.string` | Strings in a JSON view. | `#BEEB7D` | `#37700F` | `#37700F` | nothing; nothing |
| `syntax.path` | Tag paths in a JSON view. | `#E89DDE` | `#9A3488` | `#9A3488` | nothing; nothing |
| `syntax.number` | Numbers in a JSON view. | `#F28B82` | `#B93E30` | `#B93E30` | nothing; nothing |
| `syntax.literal` | true, false and null in a JSON view. | `#69BEFF` | `#0068B2` | `#0068B2` | nothing; nothing |
| `syntax.punctuation` | Punctuation in a JSON view. | `#69D2E1` | `#007080` | `#007080` | nothing; nothing |
| `syntax.markup.tag` | Markup tags in field names and help. | `#C592E6` | `#792E9C` | `#792E9C` | nothing; nothing |
| `syntax.markup.warning` | Markup the game will not read as written. | `#F0B446` | `#B06800` | `#B06800` | nothing; nothing |
| `chimp.archive.text` | Entries inside an archive in Chimp's browser. | `#D29646` | `#D29646` | `#D29646` | nothing; nothing |
| `search.match.background` | Highlight behind a search match. | `#FFDC46` | `#FFDC46` | `#FFDC46` | nothing; nothing |
| `search.match.active.background` | Highlight behind the current search match. | `#FFA826` | `#FFA826` | `#FFA826` | nothing; nothing |
| `search.match.text` | Text of a highlighted search match. | `#000000` | `#000000` | `#000000` | nothing; nothing |
<!-- tokens:end -->
