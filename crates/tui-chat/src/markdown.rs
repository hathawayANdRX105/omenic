//! Markdown rendering: convert markdown text to ratatui Lines.
//!
//! ponytail: minimal — handles code blocks, bold, inline code, headers, lists.
//! Not a full markdown renderer, just enough for chat output.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// Render markdown text into a vector of Lines for ratatui.
pub fn render(text: &str) -> Vec<Line<'static>> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);

    let parser = Parser::new_ext(text, options);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span> = Vec::new();
    let mut style_stack: Vec<Style> = vec![Style::default()];
    let mut in_code_block = false;
    let mut code_block_lines: Vec<String> = Vec::new();

    for event in parser {
        match event {
            // ===== Code blocks =====
            Event::Start(Tag::CodeBlock(_)) => {
                if !current_spans.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                }
                in_code_block = true;
                code_block_lines.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                for cl in code_block_lines.drain(..) {
                    lines.push(Line::from(vec![Span::styled(
                        cl,
                        Style::default().fg(Color::Cyan),
                    )]));
                }
                lines.push(Line::from(Span::raw("")));
            }

            // ===== Inline code: Event::Code, not a Tag =====
            Event::Code(c) => {
                let s = style_stack.last().copied().unwrap_or_default();
                current_spans.push(Span::styled(
                    format!("`{c}`"),
                    Style::default().fg(Color::Green),
                ));
                // suppress unused warning
                let _ = s;
            }

            // ===== Headings =====
            Event::Start(Tag::Heading { level, .. }) => {
                if !current_spans.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                }
                let s = match level {
                    HeadingLevel::H1 | HeadingLevel::H2 => Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                    _ => Style::default()
                        .fg(Color::Blue)
                        .add_modifier(Modifier::BOLD),
                };
                style_stack.push(s);
            }
            Event::End(TagEnd::Heading(_)) => {
                style_stack.pop();
                lines.push(Line::from(std::mem::take(&mut current_spans)));
            }

            // ===== Lists =====
            Event::Start(Tag::List(_)) => {}
            Event::End(TagEnd::List(_)) => {}
            Event::Start(Tag::Item) => {
                if !current_spans.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                }
                current_spans.push(Span::raw("  • "));
            }
            Event::End(TagEnd::Item) => {
                if !current_spans.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                }
            }

            // ===== Emphasis / Strong =====
            Event::Start(Tag::Emphasis) => {
                style_stack.push(
                    style_stack
                        .last()
                        .copied()
                        .unwrap_or_default()
                        .add_modifier(Modifier::ITALIC),
                );
            }
            Event::End(TagEnd::Emphasis) => {
                style_stack.pop();
            }
            Event::Start(Tag::Strong) => {
                style_stack.push(
                    style_stack
                        .last()
                        .copied()
                        .unwrap_or_default()
                        .add_modifier(Modifier::BOLD),
                );
            }
            Event::End(TagEnd::Strong) => {
                style_stack.pop();
            }

            // ===== Paragraph =====
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                if !current_spans.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                }
            }

            // ===== Text =====
            Event::Text(t) => {
                if in_code_block {
                    for line in t.lines() {
                        code_block_lines.push(line.to_string());
                    }
                } else {
                    let s = style_stack.last().copied().unwrap_or_default();
                    current_spans.push(Span::styled(t.to_string(), s));
                }
            }

            // ===== Line breaks =====
            Event::SoftBreak | Event::HardBreak => {
                if !in_code_block {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                }
            }

            _ => {}
        }
    }

    if !current_spans.is_empty() {
        lines.push(Line::from(current_spans));
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::raw("")));
    }

    lines
}
