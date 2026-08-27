//! UI rendering: layout, message list, input box.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};

use crate::app::{App, Role};
use crate::markdown;

pub fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),    // messages
            Constraint::Length(3), // input
            Constraint::Length(1), // status bar
        ])
        .split(f.area());

    draw_messages(f, app, chunks[0]);
    draw_input(f, app, chunks[1]);
    draw_status(f, app, chunks[2]);
}

fn draw_messages(f: &mut Frame, app: &App, area: Rect) {
    let mut items: Vec<ListItem> = Vec::new();

    for msg in &app.messages {
        let role_label = match msg.role {
            Role::User => Span::styled(
                " You",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::LightBlue)
                    .add_modifier(Modifier::BOLD),
            ),
            Role::Assistant => Span::styled(
                " AI",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD),
            ),
        };

        // Render the message text as markdown lines
        let md_lines = markdown::render(&msg.text);

        // First line gets the role label prefix
        for (i, line) in md_lines.iter().enumerate() {
            let mut spans = Vec::new();
            if i == 0 {
                spans.push(role_label.clone());
                spans.push(Span::raw(" "));
            } else {
                spans.push(Span::raw("     "));
            }
            spans.extend(line.spans.iter().cloned());
            items.push(ListItem::new(Line::from(spans)));
        }

        // Empty line between messages
        items.push(ListItem::new(Line::from(Span::raw(""))));
    }

    // Show a placeholder if no messages
    if items.is_empty() {
        items.push(ListItem::new(Line::from(vec![Span::styled(
            " 开始输入消息，按 Enter 发送。Ctrl+C 或 Esc 退出。",
            Style::default().fg(Color::DarkGray),
        )])));
    }

    // Streaming indicator: add a pulsing dot
    if app.streaming {
        let dot = if app.cursor_tick % 10 < 5 {
            "●"
        } else {
            "○"
        };
        items.push(ListItem::new(Line::from(vec![Span::styled(
            format!(" {dot} streaming..."),
            Style::default().fg(Color::Yellow),
        )])));
    }

    let title = format!(" tui-chat — {} messages ", app.messages.len());
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .style(Style::default());

    f.render_widget(list, area);
}

fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let input_style = if app.streaming {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::White)
    };

    let title = if app.streaming {
        " waiting for response... (Ctrl+C to abort) "
    } else {
        " input (Enter to send) "
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(if app.streaming {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::DarkGray)
        });

    // Show cursor
    let display = if app.streaming {
        app.input.clone()
    } else {
        let cursor = if app.cursor_tick % 20 < 10 {
            "│"
        } else {
            " "
        };
        format!("{}{}", app.input, cursor)
    };

    let paragraph = Paragraph::new(display)
        .style(input_style)
        .block(block)
        .wrap(Wrap { trim: false });

    f.render_widget(paragraph, area);
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let status = if app.streaming {
        " ● streaming"
    } else {
        " ● ready"
    };

    let model_info = format!("model: {} ", app.model.model);

    let line = Line::from(vec![
        Span::styled(
            status,
            if app.streaming {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::Green)
            },
        ),
        Span::raw("  │  "),
        Span::styled(model_info, Style::default().fg(Color::DarkGray)),
        Span::raw("  │  "),
        Span::styled(
            "Ctrl+C quit · Esc quit/abort · ↑↓ scroll",
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let block = Block::default().style(Style::default().bg(Color::Black));
    f.render_widget(Paragraph::new(line).block(block), area);
}
