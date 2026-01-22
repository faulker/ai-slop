use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, Paragraph},
    Frame,
};
use crate::app::{App, AppState};

pub fn ui(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(
            [
                Constraint::Length(3), // Header
                Constraint::Length(3), // Progress
                Constraint::Min(0),    // List
                Constraint::Length(3), // Footer
            ]
            .as_ref(),
        )
        .split(f.size());

    // Header
    let header = Block::default()
        .borders(Borders::ALL)
        .title("Bookmark Cleaner TUI")
        .style(Style::default().fg(Color::Cyan));
    f.render_widget(header, chunks[0]);

    // Progress
    let gauge = Gauge::default()
        .block(Block::default().title("Scan Progress").borders(Borders::ALL))
        .gauge_style(Style::default().fg(Color::Green))
        .percent((app.scan_progress * 100.0) as u16);
    f.render_widget(gauge, chunks[1]);

    // List
    let items: Vec<ListItem> = app
        .dead_links
        .iter()
        .enumerate()
        .map(|(i, (idx, reason))| {
            let bookmark = &app.bookmarks[*idx];
            let is_kept = app.bookmarks_to_keep.contains(idx);
            let is_highlighted = app.list_state.selected() == Some(i);
            
            let (prefix, checkbox_style) = if is_kept {
                ("[KEEP] ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
            } else {
                ("[DEL ] ", Style::default().fg(Color::Red))
            };
            
            let folder_color = if is_highlighted {
                Color::White
            } else {
                Color::DarkGray
            };

            ListItem::new(Line::from(vec![
                Span::styled(prefix, checkbox_style),
                Span::raw(format!("{} ", bookmark.url)),
                Span::styled(format!("({}) ", bookmark.folder_path.join("/")), Style::default().fg(folder_color)),
                Span::styled(format!("- {}", reason), Style::default().fg(Color::Yellow)),
            ]))
        })
        .collect();

    let list_title = match app.state {
        AppState::Scanning => "Scanning... (Results will appear below)",
        AppState::Finished | AppState::Saved | AppState::Error(_) => "Dead Links (Space to toggle, Enter to save & quit)",
    };

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(list_title))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");

    f.render_stateful_widget(list, chunks[2], &mut app.list_state);
    
    // Footer
    let footer_text = match app.state {
        AppState::Scanning => "Scanning... Please wait.",
        AppState::Finished => "Up/Down: Navigate | Space: Toggle | k: Keep All | d: Delete All | Enter: Save | q: Quit",
        AppState::Saved => "Done. Press any key to exit.",
        AppState::Error(_) => "Error occurred. Press any key to exit.",
    };
    let footer = Block::default().borders(Borders::ALL).title(footer_text);
    f.render_widget(footer, chunks[3]);

    // Popups
    match &app.state {
        AppState::Saved => {
            let block = Block::default()
                .title("Success")
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::Green).add_modifier(Modifier::BOLD));
            
            let path = app.output_path.as_deref().unwrap_or("unknown file");
            let text = vec![
                Line::from(""),
                Line::from("Cleaned bookmarks saved to:"),
                Line::from(Span::styled(path, Style::default().fg(Color::Yellow))),
                Line::from(""),
                Line::from("Press any key to exit"),
            ];
            
            let paragraph = Paragraph::new(text)
                .block(block)
                .alignment(Alignment::Center);

            let area = centered_rect(60, 20, f.size());
            f.render_widget(Clear, area);
            f.render_widget(paragraph, area);
        },
        AppState::Error(msg) => {
            let block = Block::default()
                .title("Error")
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD));
            
            let text = vec![
                Line::from(""),
                Line::from(Span::styled(msg, Style::default().fg(Color::White))),
                Line::from(""),
                Line::from("Press any key to exit"),
            ];
            
            let paragraph = Paragraph::new(text)
                .block(block)
                .alignment(Alignment::Center);

            let area = centered_rect(60, 20, f.size());
            f.render_widget(Clear, area);
            f.render_widget(paragraph, area);
        },
        _ => {}
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
