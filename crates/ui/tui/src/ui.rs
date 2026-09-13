//! UI rendering: sidebar session list + message area + input box.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};

use crate::app::{App, Focus, Role};
use crate::markdown;

pub fn draw(f: &mut Frame, app: &App) {
    // Outer horizontal split: sidebar | chat area
    let outer = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(22), // sidebar
            Constraint::Min(1),     // chat area
        ])
        .split(f.area());

    draw_sidebar(f, app, outer[0]);

    // Chat area: vertical split messages | input | status
    let chat = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),    // messages
            Constraint::Length(3), // input
            Constraint::Length(1), // status bar
        ])
        .split(outer[1]);

    draw_messages(f, app, chat[0]);
    draw_input(f, app, chat[1]);
    draw_status(f, app, chat[2]);
}

fn draw_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let mut items: Vec<ListItem> = Vec::new();

    for (i, session) in app.sessions.iter().enumerate() {
        let is_active = i == app.active;
        let is_selected = i == app.session_scroll as usize && app.focus == Focus::Sessions;

        let prefix = if is_active { "▶ " } else { "  " };
        let title_text = format!("{prefix}{}", session.title);

        let style = if is_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else if is_active {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };

        items.push(ListItem::new(Line::from(vec![Span::styled(
            title_text, style,
        )])));
    }

    let border_color = if app.focus == Focus::Sessions {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let title = format!(" sessions ({}) ", app.sessions.len());
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(border_color)),
    );

    f.render_widget(list, area);

    // Hint footer at bottom of sidebar
    let hints = " n:new  d:del  ↵:open ";
    let hint_area = Rect {
        x: area.x,
        y: area.bottom().saturating_sub(1),
        width: area.width,
        height: 1,
    };
    f.render_widget(
        Paragraph::new(hints).style(Style::default().fg(Color::DarkGray)),
        hint_area,
    );
}

fn draw_messages(f: &mut Frame, app: &App, area: Rect) {
    let mut items: Vec<ListItem> = Vec::new();
    let messages = &app.active_session().messages;

    for msg in messages {
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
            " 开始输入消息，按 Enter 发送。Tab 切换会话列表。",
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

    let title = format!(" chat — {} messages ", messages.len());
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
        " input (Enter to send, Tab for sessions) "
    };

    let border_color = if app.streaming {
        Color::Yellow
    } else if app.focus == Focus::Input {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border_color));

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
            "Tab:switch  Ctrl+C:quit  Esc:abort/quit  ↑↓:scroll",
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let block = Block::default().style(Style::default().bg(Color::Black));
    f.render_widget(Paragraph::new(line).block(block), area);
}
