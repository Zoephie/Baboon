//! Every colour Baboon paints that a theme can change: its id in a theme
//! file, where it appears, and its value in each built-in theme.
//!
//! Generated from the colours each accessor gave under the three built-in
//! looks before the registry existed; `golden.rs` keeps that capture and
//! checks the registry against it.

tokens! {
    group "Core" {
        Background "background" ["#282828", "#F2F1F0", "#E0E0DC"] "Editor panes, windows and the page behind tag fields.";
        Text "text" ["#F2F2F2", "#1C1815", "#191918"] "Body text and labels.";
        Accent "accent" ["#406C86", "#92C0E2", "#9EC0DE"] "Selection fill and other emphasis.";
        PanelBackground "panel.background" ["#1F1F1F", "#EBE5E0", "#EEEEEA"] "Side panels: the tag browser, tile bars and import panes." => dark(shade(Background, 0.225));
        RowBackground "row.background" ["#373737", "#ECE8E4", "#DBDBD8"] "Striped and faint rows, and the background of non-interactive widgets." => by_appearance(tint(Background, 0.07), shade(Background, 0.037));
        Border "border" ["#525252", "#DED6CF", "#B4B4AE"] "Separators and borders between panes." => dark(tint(Background, 0.195));
        Separator "separator" ["#3C3C3C", "#DED6CF", "#B4B4AE"] "egui's separator lines and the outline of non-interactive widgets." => dark(tint(Background, 0.093));
        TextMuted "text.muted" ["#A4A4A4", "#6E6660", "#52524E"] "Secondary text: hints, counts and disabled labels." => dark(mix(Background, Text, 0.614));
        TextStrong "text.strong" ["#FFFFFF", "#000000", "#000000"] "Text that must stand out at full contrast: white on dark, black on light.";
        AccentBorder "accent.border" ["#78AAC6", "#263F4E", "#263F4E"] "Outline of selected items and text selection.";
        AccentText "accent.text" ["#86B8D5", "#B4440C", "#0F2B40"] "Accent text and marks: highlighted names, links in tables, the animation playhead.";
        FocusBorder "focus.border" ["#A0A0A0", "#D0966A", "#4E78A0"] "Border of the focused tab or pane." => dark(tint(Background, 0.558));
        LinkText "link.text" ["#5AAAFF", "#009BFF", "#009BFF"] "Hyperlinks.";
        ModalBackdrop "modal.backdrop" ["#0000008C", "#0000008C", "#0000008C"] "Dimming laid over the app behind a modal dialog.";
    }
    group "Chrome" {
        MenuBarBackground "menu_bar.background" ["#323232", "#E5CCB2", "#DBDBD8"] "The menu bar, and bars across a pane such as Chimp's." => dark(tint(Background, 0.047));
        StatusBarBackground "status_bar.background" ["#323232", "#E5CCB2", "#DBDBD8"] "The status bar along the bottom of the window." => both(reference(MenuBarBackground));
        StatusBarText "status_bar.text" ["#F2F2F2", "#1C1815", "#191918"] "Text on the status bar." => both(reference(Text));
        MenuBarItemHover "menu_bar.item.hover" ["#FFFFFF1A", "#0000001A", "#0000001A"] "Fill of a hovered menu bar item.";
        ChromeBackground "chrome.background" ["#373737", "#E5CCB2", "#DBDBD8"] "Window chrome strips: tile title bars and menu panels." => dark(tint(Background, 0.07));
        ChromeBorder "chrome.border" ["#525252", "#D0966A", "#B4B4AE"] "Edge of the window chrome strips." => dark(tint(Background, 0.195));
        TabActiveBackground "tab.active.background" ["#484848", "#E5CCB2", "#A1A19D"] "The active document tab in Chimp." => dark(tint(Background, 0.149));
        TagTabActiveBackground "tab.tag.active.background" ["#282828", "#F2F1F0", "#E0E0DC"] "The active tag tab, joined to the page below it." => both(reference(Background));
        KitTabActiveBackground "tab.kit.active.background" ["#1F1F1F", "#EBE5E0", "#EEEEEA"] "The chosen kit tab, joined to the tag browser below it." => both(reference(PanelBackground));
        TabActiveText "tab.active.text" ["#F2F2F2", "#1C1815", "#191918"] "Text of an active tag or kit tab." => both(reference(Text));
        TabModifiedTint "tab.modified.tint" ["#B8860B", "#B8860B", "#B8860B"] "Tint mixed into a tab whose document has unsaved changes.";
        BrowserToolbarBackground "browser.toolbar.background" ["#444444", "#EEECEA", "#E2E2DE"] "The tag browser's toolbar." => by_appearance(tint(Background, 0.13), shade(Background, 0.021));
        BrowserToolbarActiveBackground "browser.toolbar.active.background" ["#497088", "#5D899E", "#5D899E"] "A toggled-on button in the tag browser toolbar." => dark(tint(Accent, 0.03));
        BrowserSearchBackground "browser.search.background" ["#00000066", "#EDD3B980", "#FFFFFF66"] "The tag browser's search box.";
        TreeFolderOpenIcon "tree.folder.open.icon" ["#22CD54", "#22CD54", "#22CD54"] "Disclosure triangle and icon of an open folder.";
        TreeFolderClosedIcon "tree.folder.closed.icon" ["#186FCD", "#186FCD", "#186FCD"] "Disclosure triangle of a closed folder.";
        BadgeBackground "badge.background" ["#303A42", "#E8EDF0", "#E8EDF0"] "Fill of the tag-group badge beside a tag in the tree." => by_appearance(mix(Background, Accent, 0.292), mix(Background, Accent, 0.093));
        BadgeBorder "badge.border" ["#4C5962", "#C4CED5", "#C4CED5"] "Outline of the tag-group badge.";
        BadgeText "badge.text" ["#E2EBF0", "#3A464F", "#3A464F"] "Text of the tag-group badge." => by_appearance(tint(Accent, 0.863), mix(Text, Accent, 0.27));
        BadgeReadOnlyBackground "badge.read_only.background" ["#D2913C", "#D2913C", "#D2913C"] "The read-only badge on a tag or layout.";
        BadgeReadOnlyIcon "badge.read_only.icon" ["#1E1E1E", "#1E1E1E", "#1E1E1E"] "The mark drawn on the read-only badge.";
        FavouriteIcon "favourite.icon" ["#F2C430", "#F2C430", "#F2C430"] "A favourite's filled star.";
        ClipboardCutBorder "clipboard.cut.border" ["#FFD138", "#FFD138", "#FFD138"] "Outline of rows cut to the clipboard.";
        ClipboardCopyBorder "clipboard.copy.border" ["#47AEEB", "#47AEEB", "#47AEEB"] "Outline of rows copied to the clipboard.";
        FieldHelpText "field.help.text" ["#3DA1CC", "#3DA1CC", "#3DA1CC"] "The question mark beside a field that has help.";
        WelcomePanelBackground "welcome.panel.background" ["#00000033", "#EFD2B950", "#D6D6D650"] "The welcome screen's side column.";
        ProgressBackground "progress.background" ["#1F1F1E", "#E2D0C2", "#D7D7D2"] "Track of a progress bar." => dark(shade(Background, 0.233));
        ProgressBar "progress.bar" ["#456F84", "#5B92AC", "#5B92AC"] "Filled part of a progress bar." => dark(tint(Accent, 0.01));
        LoadingPrimary "loading.primary" ["#BFBFBF", "#333333", "#333333"] "Brightest strokes of the loading animation." => by_appearance(tint(Background, 0.702), shade(Background, 0.788));
        LoadingSecondary "loading.secondary" ["#808080", "#4D4D4D", "#4D4D4D"] "Middle strokes of the loading animation." => by_appearance(tint(Background, 0.409), shade(Background, 0.68));
        LoadingLines "loading.lines" ["#404040", "#999999", "#999999"] "Faint lines of the loading animation." => by_appearance(tint(Background, 0.112), shade(Background, 0.365));
    }
    group "Controls" {
        ButtonBackground "button.background" ["#FFFFFF1A", "#FFFFFF80", "#0000001A"] "Fill of a button or combo box at rest.";
        ButtonHover "button.hover" ["#FFFFFF33", "#FFFFFFBF", "#00000033"] "Fill of a hovered or open button.";
        ButtonActive "button.active" ["#FFFFFF40", "#FFFFFF40", "#00000040"] "Fill of a pressed button.";
        ButtonBorder "button.border" ["#00000000", "#D6CDC5", "#00000000"] "Outline of a button at rest.";
        ButtonBorderHover "button.border.hover" ["#969696", "#BEB2A8", "#696969"] "Outline of a hovered button." => dark(tint(Background, 0.512));
        ButtonBorderActive "button.border.active" ["#FFFFFF", "#B0A297", "#000000"] "Outline of a pressed button.";
        ButtonBorderOpen "button.border.open" ["#3C3C3C", "#BEB2A8", "#A0A0A0"] "Outline of a button whose menu is open." => dark(tint(Background, 0.093));
        ButtonDestructive "button.destructive" ["#EB4747", "#EB4747", "#EB4747"] "Accent fill of a button for an action to take with care.";
        ButtonPrimaryBackground "button.primary.background" ["#287A3E", "#287A3E", "#287A3E"] "Fill of a primary, confirming button.";
        MenuHover "menu.hover" ["#FFFFFF1A", "#451D0A1A", "#0000001A"] "Fill of a hovered menu item or icon button.";
        ControlBackground "control.background" ["#444444", "#F9F8F8", "#C9C9C6"] "Fill of a control such as a slider or checkbox at rest." => by_appearance(tint(Background, 0.13), tint(Background, 0.524));
        ControlHover "control.hover" ["#525252", "#FCFBFB", "#B3B3B0"] "Fill of a hovered control." => by_appearance(tint(Background, 0.195), tint(Background, 0.739));
        ControlActive "control.active" ["#5C5C5C", "#F5F5F4", "#A8A8A5"] "Fill of a pressed control." => by_appearance(tint(Background, 0.242), reference(Background));
        InputBackground "input.background" ["#0000004D", "#EDD3B980", "#FFFFFF4D"] "Inside of a text input.";
        InputBorder "input.border" ["#545454", "#CC9966", "#70706C"] "Outline of a text input." => dark(tint(Background, 0.205));
        CardBackground "card.background" ["#FFFFFF0D", "#EDD3B780", "#0000000D"] "Documentation and note boxes.";
        SettingsCardBackground "settings.card.background" ["#FFFFFF0D", "#0000000D", "#0000000D"] "Cards in the Settings window.";
        SliderRailBackground "slider.rail.background" ["#0000005A", "#EDD3B980", "#FFFFFF4D"] "Rail of a slider.";
        SliderRailBorder "slider.rail.border" ["#00000000", "#CC9966", "#70706C"] "Outline of a slider's rail.";
        ControlWell "control.well" ["#000000", "#FFFFFF", "#FFFFFF"] "The well inside a checked checkbox or radio button.";
        CodeInlineBackground "code.inline.background" ["#404040", "#ECE8E4", "#DBDBD8"] "Background of inline code." => by_appearance(tint(Background, 0.112), shade(Background, 0.037));
    }
    group "Field editor" {
        GroupBackground "group.background" ["#2B2B2B", "#F5F4F3", "#ECECEA"] "Background of a group of fields." => both(reference(Background));
        GroupBorder "group.border" ["#525252", "#D8CFC7", "#989894"] "Outline of a group of fields, and of windows." => dark(tint(Background, 0.195));
        SectionHeaderBackground "section.header.background" ["#444444", "#F2E7DB", "#D6D6D2"] "Header bar of a section of fields." => dark(tint(Background, 0.13));
        BlockHeaderBackground "block.header.background" ["#484848", "#F0E2D4", "#CCCCC8"] "Header bar of a tag block, and of popup footers." => dark(tint(Background, 0.149));
        BlockHeaderText "block.header.text" ["#F2F2F2", "#1C1815", "#191918"] "Text on a tag block header." => both(reference(Text));
        BlockBorder "block.border" ["#5E5E5E", "#D6C8BC", "#9A9A95"] "Outline of a tag block." => dark(tint(Background, 0.251));
        BlockJumpIcon "block.jump.icon" ["#4DD0E1", "#4DD0E1", "#4DD0E1"] "Icon of the button that jumps to a block.";
        BlockJumpHighlight "block.jump.highlight" ["#FFD70026", "#FFD70026", "#FFD70026"] "Highlight on the block a jump landed on.";
        FieldLabelText "field.label.text" ["#F2F2F2", "#1C1815", "#191918"] "Names of tag fields, left of their values." => both(reference(Text));
        FlagsBackground "flags.background" ["#0000004D", "#EDD3B980", "#FFFFFF4D"] "Inside of a list of flags." => both(reference(InputBackground));
        FlagsBorder "flags.border" ["#545454", "#CC9966", "#70706C"] "Outline of a list of flags." => both(reference(InputBorder));
        FlagsText "flags.text" ["#F2F2F2", "#1C1815", "#191918"] "Names of the flags in a list of flags." => both(reference(Text));
        FlagsHover "flags.hover" ["#3A3A38", "#ECE8E4", "#E8E8E4"] "Fill of a hovered flag row." => by_appearance(tint(Background, 0.081), shade(Background, 0.037));
    }
    group "Status" {
        StatusError "status.error" ["#FF7C7C", "#A6232D", "#A6232D"] "Error text and icons, and warnings that something will be lost or overwritten.";
        StatusErrorBackground "status.error.background" ["#301E20", "#FFF0F0", "#FFF0F0"] "Fill of a field holding an invalid value.";
        StatusErrorBorder "status.error.border" ["#AB464C", "#C04B54", "#C04B54"] "Outline of a field holding an invalid value.";
        StatusWarning "status.warning" ["#F0BE5A", "#7A5000", "#7A5000"] "Warning text and icons.";
        StatusWarningBackground "status.warning.background" ["#362B18", "#FFF3C9", "#FFF7DC"] "Fill of a field holding a value that was not applied." => dark(shade(StatusWarning, 0.761));
        StatusSuccess "status.success" ["#7ECD85", "#10642A", "#10642A"] "Success text and icons: matches, accepted changes, add buttons.";
        ActionDestructive "action.destructive" ["#E25C5C", "#8B0000", "#8B0000"] "Destructive actions and removals: delete buttons, decreased or unused values.";
        SettingsCardErrorBackground "settings.card.error.background" ["#622824", "#FFDED8", "#FFDED8"] "A Settings card holding an invalid value.";
        BannerWarningBackground "banner.warning.background" ["#483816", "#FCF0CC", "#FCF0CC"] "Fill of a warning banner, such as a tag changed on disk.";
        BannerWarningBorder "banner.warning.border" ["#8C6820", "#DEB048", "#DEB048"] "Outline of a warning banner.";
        BannerWarningIcon "banner.warning.icon" ["#F2C430", "#B07800", "#B07800"] "Icon on a warning banner.";
        BannerErrorBackground "banner.error.background" ["#4E201E", "#FCE2DF", "#FCE2DF"] "Fill of an error banner, such as a tag deleted on disk.";
        BannerErrorBorder "banner.error.border" ["#9E3E38", "#E08076", "#E08076"] "Outline of an error banner.";
        BannerErrorIcon "banner.error.icon" ["#F05C50", "#C83026", "#C83026"] "Icon on an error banner.";
    }
    group "Diff" {
        DiffAdded "diff.added" ["#7EBA6C", "#7EBA6C", "#7EBA6C"] "Text of something added.";
        DiffRemoved "diff.removed" ["#D66A6A", "#D66A6A", "#D66A6A"] "Text of something removed.";
        DiffModified "diff.modified" ["#D6A82E", "#D6A82E", "#D6A82E"] "Text of something modified.";
        DiffUnchanged "diff.unchanged" ["#828282", "#828282", "#828282"] "Text of something unchanged.";
        DiffAddedBackground "diff.added.background" ["#1C2E20", "#1C2E20", "#1C2E20"] "Wash behind an added line.";
        DiffRemovedBackground "diff.removed.background" ["#341E1E", "#341E1E", "#341E1E"] "Wash behind a removed line.";
    }
    group "Shader editor and graphs" {
        ShaderPanelBackground "shader.panel.background" ["#3A3C38", "#F6F3F0", "#EEEEEB"] "The shader editor's panels." => by_appearance(tint(Background, 0.084), reference(Background));
        ShaderPanelBorder "shader.panel.border" ["#484A46", "#C4B8AE", "#A8A8A2"] "Outline of the shader editor's panels." => dark(tint(Background, 0.149));
        ShaderGrid "shader.grid" ["#363834", "#DAD2CB", "#C6C6C0"] "The shader editor's grid lines and hovered drag handles." => dark(tint(Background, 0.065));
        ShaderRowReferenceBackground "shader.row.reference.background" ["#1E3A28", "#A6CDA6", "#A6CDA6"] "A shader row holding a tag reference.";
        ShaderRowNumericBackground "shader.row.numeric.background" ["#3E2D27", "#E8BFAB", "#E8BFAB"] "A shader row holding a number.";
        ShaderRowDataBackground "shader.row.data.background" ["#2A2B29", "#E8E2DD", "#D8D8D8"] "A shader row holding other data." => dark(reference(Background));
        ShaderRowFunctionBackground "shader.row.function.background" ["#3A2F20", "#EFCD89", "#EFCD89"] "A shader row holding a function.";
        ShaderRowModifiedMarker "shader.row.modified.marker" ["#E09E3E", "#E09E3E", "#E09E3E"] "Stripe at the edge of a modified shader row.";
        ShaderInputBorder "shader.input.border" ["#52564E", "#AFA298", "#707070"] "Outline of an input in the shader editor." => dark(tint(Background, 0.195));
        ShaderText "shader.text" ["#E7E8E2", "#1C1815", "#141414"] "Text in the shader editor." => by_appearance(mix(Background, Text, 0.939), reference(Text));
        ShaderTextMuted "shader.text.muted" ["#9B9E96", "#6E6660", "#606060"] "Secondary text in the shader editor." => dark(mix(Background, Text, 0.566));
        GraphBackground "graph.background" ["#40403E", "#E6E0DB", "#CDCDCD"] "A function graph's plot." => by_appearance(tint(Background, 0.109), mix(Background, Text, 0.077));
        GraphGrid "graph.grid" ["#84847E", "#766C64", "#5C5C58"] "A function graph's grid lines." => dark(tint(Background, 0.419));
        GraphContainerBackground "graph.container.background" ["#00000040", "#000000", "#000000"] "The frame around a function graph.";
        GraphBandBackground "graph.band.background" ["#FFFFFF1C", "#FFFFFF1C", "#FFFFFF1C"] "The band between a function's two curves.";
        GraphBandBorder "graph.band.border" ["#FFFFFF78", "#FFFFFF78", "#FFFFFF78"] "Edges of the band between two curves.";
        GraphCurvePrimary "graph.curve.primary" ["#2ABE48", "#2ABE48", "#2ABE48"] "A function graph's first curve.";
        GraphCurveSecondary "graph.curve.secondary" ["#DC3737", "#DC3737", "#DC3737"] "A function graph's second curve.";
        GraphCurvePrimaryFill "graph.curve.primary.fill" ["#27BA451A", "#27BA451A", "#27BA451A"] "Fill under the first curve.";
        GraphCurveSecondaryFill "graph.curve.secondary.fill" ["#D83B3B1A", "#D83B3B1A", "#D83B3B1A"] "Fill under the second curve.";
    }
    group "Material editor" {
        MaterialRowDataBackground "material.row.data.background" ["#D8D8D8", "#D8D8D8", "#D8D8D8"] "A data row in the material editor.";
        MaterialRowFunctionBackground "material.row.function.background" ["#EFCD89", "#EFCD89", "#EFCD89"] "A function row in the material editor.";
        MaterialBoxBackground "material.box.background" ["#E0E0E0", "#E0E0E0", "#E0E0E0"] "A value box at its default in the material editor.";
        MaterialGrid "material.grid" ["#5C5C5C", "#5C5C5C", "#5C5C5C"] "The material editor's grid lines.";
        MaterialInputBorder "material.input.border" ["#707070", "#707070", "#707070"] "Outline of an input in the material editor and colour picker.";
        MaterialText "material.text" ["#141414", "#141414", "#141414"] "Text in the material editor.";
        MaterialTextMuted "material.text.muted" ["#606060", "#606060", "#606060"] "Secondary text in the material editor.";
    }
    group "Terminal, code and syntax" {
        TerminalBackground "terminal.background" ["#181817", "#181817", "#181817"] "The terminal.";
        TerminalText "terminal.text" ["#E8E8E4", "#E8E8E4", "#E8E8E4"] "Terminal text.";
        TerminalWarning "terminal.warning" ["#EEC45B", "#EEC45B", "#EEC45B"] "Warnings in the terminal.";
        TerminalError "terminal.error" ["#F46969", "#F46969", "#F46969"] "Errors in the terminal.";
        TerminalSuccess "terminal.success" ["#7BB889", "#7BB889", "#7BB889"] "Successes in the terminal.";
        CodeBackground "code.background" ["#181B1D", "#F7EEE7", "#EEF1F3"] "Code blocks in the help.";
        SyntaxPlain "syntax.plain" ["#D2D6DE", "#2D323A", "#2D323A"] "Plain text in a JSON view.";
        SyntaxKey "syntax.key" ["#F4BF5C", "#915B00", "#915B00"] "Keys in a JSON view.";
        SyntaxString "syntax.string" ["#BEEB7D", "#37700F", "#37700F"] "Strings in a JSON view.";
        SyntaxPath "syntax.path" ["#E89DDE", "#9A3488", "#9A3488"] "Tag paths in a JSON view.";
        SyntaxNumber "syntax.number" ["#F28B82", "#B93E30", "#B93E30"] "Numbers in a JSON view.";
        SyntaxLiteral "syntax.literal" ["#69BEFF", "#0068B2", "#0068B2"] "true, false and null in a JSON view.";
        SyntaxPunctuation "syntax.punctuation" ["#69D2E1", "#007080", "#007080"] "Punctuation in a JSON view.";
        SyntaxMarkupTag "syntax.markup.tag" ["#C592E6", "#792E9C", "#792E9C"] "Markup tags in field names and help.";
        SyntaxMarkupWarning "syntax.markup.warning" ["#F0B446", "#B06800", "#B06800"] "Markup the game will not read as written.";
        ChimpArchiveText "chimp.archive.text" ["#D29646", "#D29646", "#D29646"] "Entries inside an archive in Chimp's browser.";
        SearchMatchBackground "search.match.background" ["#FFDC46", "#FFDC46", "#FFDC46"] "Highlight behind a search match.";
        SearchMatchActiveBackground "search.match.active.background" ["#FFA826", "#FFA826", "#FFA826"] "Highlight behind the current search match.";
        SearchMatchText "search.match.text" ["#000000", "#000000", "#000000"] "Text of a highlighted search match.";
    }
}
