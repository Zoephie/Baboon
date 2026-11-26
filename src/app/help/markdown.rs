//! A small egui presentation layer over the CommonMark parser.
//! Keep authoring in Markdown; string examples share the editor's highlighter.
use super::*;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

pub(super) fn draw(ui: &mut Ui, source: &str) {
    let mut job = egui::text::LayoutJob::default();
    let mut strong = 0usize;
    let mut emphasis = 0usize;
    let mut strike = 0usize;
    let mut heading = None;
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut code: Option<(bool, String)> = None;
    for event in Parser::new_ext(source, Options::ENABLE_STRIKETHROUGH) {
        match event {
            Event::Start(Tag::Strong) => strong += 1,
            Event::End(TagEnd::Strong) => strong = strong.saturating_sub(1),
            Event::Start(Tag::Emphasis) => emphasis += 1,
            Event::End(TagEnd::Emphasis) => emphasis = emphasis.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => strike += 1,
            Event::End(TagEnd::Strikethrough) => strike = strike.saturating_sub(1),
            Event::Start(Tag::Heading { level, .. }) => {
                flush(ui, &mut job);
                heading = Some(level as u8);
            }
            Event::Start(Tag::List(start)) => {
                flush(ui, &mut job);
                lists.push(start);
            }
            Event::End(TagEnd::List(_)) => { flush(ui, &mut job); lists.pop(); }
            Event::Start(Tag::Item) => {
                flush(ui, &mut job);
                let mut prefix = "    ".repeat(lists.len().saturating_sub(1));
                match lists.last_mut() {
                    Some(Some(number)) => { prefix.push_str(&format!("{number}. ")); *number += 1; }
                    _ => prefix.push_str("• "),
                }
                append(ui, &mut job, RichText::new(prefix).color(subtle_dark()));
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                flush(ui, &mut job);
                let strings = matches!(kind, CodeBlockKind::Fenced(info) if info.as_ref() == "strings-halo3");
                code = Some((strings, String::new()));
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((strings, text)) = code.take() {
                    Frame::NONE.fill(ui.visuals().code_bg_color).inner_margin(8).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let mut example = if strings {
                            crate::app::editor::string_list::highlighted_example(text.trim_end_matches('\n'), &TextStyle::Monospace.resolve(ui.style()))
                        } else {
                            let mut example = egui::text::LayoutJob::default();
                            append(ui, &mut example, RichText::new(text.trim_end_matches('\n')).monospace().color(text_dark()));
                            example
                        };
                        example.wrap.max_width = ui.available_width();
                        ui.add(egui::Label::new(example).wrap());
                    });
                    ui.add_space(4.0);
                }
            }
            Event::Text(text) if code.is_some() => code.as_mut().unwrap().1.push_str(&text),
            Event::Text(ref text) | Event::Code(ref text) => {
                let inline_code = matches!(event, Event::Code(_));
                let mut rich = RichText::new(text.as_ref());
                if inline_code { rich = rich.code(); }
                if strong > 0 || heading.is_some() { rich = rich.strong(); }
                else { rich = rich.color(text_dark()); }
                if emphasis > 0 { rich = rich.italics(); }
                if strike > 0 { rich = rich.strikethrough(); }
                if let Some(level) = heading { rich = rich.size((20.0 - f32::from(level) * 1.5).max(14.0)).color(foundation_blue()); }
                append(ui, &mut job, rich);
            }
            Event::SoftBreak => append(ui, &mut job, RichText::new(" ")),
            Event::HardBreak => append(ui, &mut job, RichText::new("\n")),
            Event::End(TagEnd::Heading(_)) => { flush(ui, &mut job); heading = None; }
            Event::End(TagEnd::Paragraph | TagEnd::Item | TagEnd::BlockQuote(_)) => flush(ui, &mut job),
            Event::Rule => { flush(ui, &mut job); ui.separator(); }
            // Raw HTML remains literal text; documentation never executes it.
            Event::Html(text) | Event::InlineHtml(text) => append(ui, &mut job, RichText::new(text.as_ref()).color(text_dark())),
            _ => {}
        }
    }
    flush(ui, &mut job);
}

fn append(ui: &Ui, job: &mut egui::text::LayoutJob, text: RichText) {
    text.append_to(job, ui.style(), egui::FontSelection::Default, egui::Align::Center);
}

fn flush(ui: &mut Ui, job: &mut egui::text::LayoutJob) {
    if !job.text.is_empty() {
        ui.add(egui::Label::new(std::mem::take(job)).wrap().selectable(false));
        ui.add_space(4.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(source: &str, visuals: egui::Visuals, width: f32) -> Vec<egui::text::LayoutJob> {
        let ctx = egui::Context::default();
        ctx.set_visuals(visuals);
        let output = crate::app::run_ui_test(&ctx, egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(width, 900.0))),
            ..Default::default()
        }, |ui| draw(ui, source));
        output.shapes.into_iter().filter_map(|shape| match shape.shape {
            egui::Shape::Text(text) => Some(text.galley.job.as_ref().clone()),
            _ => None,
        }).collect()
    }

    #[test]
    fn markdown_renders_emphasis_lists_headings_and_literal_code() {
        for visuals in [egui::Visuals::dark(), egui::Visuals::light()] {
            let jobs = rendered("## Heading\n\n**Bold** and *italic* with `&a_button`.\n\n- First\n- Second\n\n1. Start\n2. Finish", visuals, 520.0);
            let text = jobs.iter().map(|job| job.text.as_str()).collect::<Vec<_>>().join("\n");
            assert!(text.contains("Heading") && !text.contains("##"));
            assert!(text.contains("Bold and italic with &a_button."));
            assert!(text.contains("• First") && text.contains("• Second"));
            assert!(text.contains("1. Start") && text.contains("2. Finish"));
            assert!(jobs.iter().flat_map(|job| &job.sections).any(|section| section.format.italics));
            assert!(jobs.iter().flat_map(|job| &job.sections).any(|section| section.format.font_id.family == egui::FontFamily::Monospace));
            let plain = jobs.iter().find(|job| job.text.starts_with("Bold")).unwrap();
            assert_ne!(plain.sections[0].format.color, plain.sections[1].format.color, "strong text uses native egui emphasis");
        }
    }

    #[test]
    fn string_examples_render_editor_colours_in_both_themes() {
        // In a theme of this thread's own, so a test installing another
        // meanwhile cannot change the colours between rendering and reading.
        for (visuals, built_in) in [
            (egui::Visuals::dark(), crate::app::ui_kit::themes::BuiltIn::Dark),
            (egui::Visuals::light(), crate::app::ui_kit::themes::BuiltIn::Light),
        ] {
          crate::app::ui_kit::style::with_built_in(built_in, || {
            let jobs = rendered("```strings-halo3\n&a_button &U+E453\\r\\n\n```", visuals, 520.0);
            let job = jobs.iter().find(|job| job.text.contains("&a_button")).unwrap();
            let colour = |token: &str| {
                let start = job.text.find(token).unwrap();
                job.sections.iter().find(|section| section.byte_range.start.0 <= start && start < section.byte_range.end.0).unwrap().format.color
            };
            assert_eq!(colour("&a_button"), foundation_blue());
            assert_eq!(colour("&U+E453"), good_news());
            assert_eq!(colour("\\r"), markup_warning_text());
            assert_eq!(colour("\\n"), markup_warning_text());
          });
        }
    }

    #[test]
    fn packaged_string_guide_renders_at_narrow_and_wide_widths() {
        for width in [300.0, 900.0] {
            let jobs = rendered(include_str!("../../../docs/strings-editor.md"), egui::Visuals::dark(), width);
            assert!(jobs.iter().any(|job| job.text == "Reading string text"));
            assert!(jobs.iter().any(|job| job.text.contains("&U+E453")));
            assert!(jobs.iter().all(|job| job.wrap.max_width.is_finite()));
        }
        let HelpDocsState::Loaded(docs) = HelpDocsState::load() else { panic!("packaged documentation must load"); };
        let section = docs.tab("doc").unwrap().sections.iter().find(|section| section.title == STRING_EDITOR_HELP_SECTION).unwrap();
        assert!(matches!(&section.blocks[0], HelpDocBlock::Markdown { text, .. } if text.contains("## Reading string text")));
    }
}
