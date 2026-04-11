use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::prelude::*;
use ratatui::widgets::*;

use anyhow::Context;

use ai_wiki_core::config::Config;
use ai_wiki_core::queue::{ItemStatus, Queue, QueueItem};

/// Upper bound on detail-view scroll offset. Prevents unbounded scrolling
/// when exact content length is not tracked. 500 lines covers typical detail views.
const MAX_DETAIL_SCROLL: u16 = 500;

pub fn run(config: &Config) -> anyhow::Result<()> {
    let queue = Queue::open(&config.paths.database_path)
        .with_context(|| format!("failed to open queue at {}", config.paths.database_path.display()))?;

    // Install panic hook to restore terminal on crash
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
        original_hook(info);
    }));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, queue, config);

    // Restore terminal (normal exit path)
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    // Restore original panic hook
    let _ = std::panic::take_hook();

    result
}

/// Whether we're showing the main table or a detail overlay.
enum View {
    Table,
    Detail(QueueItem),
}

fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    queue: Queue,
    config: &Config,
) -> anyhow::Result<()> {
    let mut table_state = TableState::default();
    let mut items: Vec<QueueItem> = vec![];
    let mut counts: Vec<(String, u64)> = vec![];
    let mut last_refresh = Instant::now() - Duration::from_secs(3);
    let mut view = View::Table;
    let mut detail_scroll: u16 = 0;
    let mut status_msg: Option<(String, Instant)> = None;

    loop {
        // Refresh data every 2 seconds (only in table view)
        if matches!(view, View::Table) && last_refresh.elapsed() >= Duration::from_secs(2) {
            items = queue
                .list_items(None)
                .map_err(|e| anyhow::anyhow!("failed to list items: {}", e))?;
            counts = queue
                .count_by_status()
                .map_err(|e| anyhow::anyhow!("failed to count by status: {}", e))?;
            last_refresh = Instant::now();

            if let Some(sel) = table_state.selected() {
                if items.is_empty() {
                    table_state.select(None);
                } else if sel >= items.len() {
                    table_state.select(Some(items.len() - 1));
                }
            }
        }

        // Expire status message after 5 seconds
        if let Some((_, ts)) = &status_msg
            && ts.elapsed() >= Duration::from_secs(5)
        {
            status_msg = None;
        }

        let status_text = status_msg.as_ref().map(|(msg, _)| msg.as_str());
        terminal.draw(|f| match &view {
            View::Table => draw_table(f, &items, &counts, &mut table_state, status_text),
            View::Detail(item) => draw_detail(f, item, config, detail_scroll),
        })?;

        if event::poll(Duration::from_millis(250))?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match &view {
                View::Table => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('r') => {
                        last_refresh = Instant::now() - Duration::from_secs(3);
                    }
                    KeyCode::Up => {
                        let sel = match table_state.selected() {
                            Some(0) | None => 0,
                            Some(i) => i - 1,
                        };
                        if !items.is_empty() {
                            table_state.select(Some(sel));
                        }
                    }
                    KeyCode::Down => {
                        if !items.is_empty() {
                            let sel = match table_state.selected() {
                                None => 0,
                                Some(i) => (i + 1).min(items.len() - 1),
                            };
                            table_state.select(Some(sel));
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(sel) = table_state.selected()
                            && let Some(item) = items.get(sel)
                        {
                            // Only open detail for terminal states
                            match item.status {
                                ItemStatus::Complete | ItemStatus::Error | ItemStatus::Rejected => {
                                    detail_scroll = 0;
                                    view = View::Detail(item.clone());
                                }
                                _ => {} // no action for queued/in_progress
                            }
                        }
                    }
                    KeyCode::Char('R') => {
                        // Retry: requeue errored/rejected item from table view
                        if let Some(sel) = table_state.selected()
                            && let Some(item) = items.get(sel)
                            && matches!(item.status, ItemStatus::Error | ItemStatus::Rejected)
                        {
                            match queue.requeue_item(item.id) {
                                Ok(()) => {
                                    status_msg = Some((
                                        format!("Item {} requeued", item.id),
                                        Instant::now(),
                                    ));
                                }
                                Err(e) => {
                                    status_msg = Some((
                                        format!("Retry failed for item {}: {e}", item.id),
                                        Instant::now(),
                                    ));
                                }
                            }
                            last_refresh = Instant::now() - Duration::from_secs(3);
                        }
                    }
                    _ => {}
                },
                View::Detail(item) => match key.code {
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                        view = View::Table;
                        last_refresh = Instant::now() - Duration::from_secs(3);
                    }
                    KeyCode::Char('R') => {
                        if matches!(item.status, ItemStatus::Error | ItemStatus::Rejected) {
                            match queue.requeue_item(item.id) {
                                Ok(()) => {
                                    status_msg = Some((
                                        format!("Item {} requeued", item.id),
                                        Instant::now(),
                                    ));
                                }
                                Err(e) => {
                                    status_msg = Some((
                                        format!("Retry failed for item {}: {e}", item.id),
                                        Instant::now(),
                                    ));
                                }
                            }
                            view = View::Table;
                            last_refresh = Instant::now() - Duration::from_secs(3);
                        }
                    }
                    KeyCode::Up => {
                        detail_scroll = detail_scroll.saturating_sub(1);
                    }
                    KeyCode::Down => {
                        detail_scroll = detail_scroll.saturating_add(1).min(MAX_DETAIL_SCROLL);
                    }
                    _ => {}
                },
            }
        }
    }
}

// ─── Table View ──────────────────────────────────────────────────────────────

fn draw_table(
    f: &mut Frame,
    items: &[QueueItem],
    counts: &[(String, u64)],
    table_state: &mut TableState,
    status_msg: Option<&str>,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(f.area());

    // Status bar: show status message if present, otherwise show counts
    let status_text = if let Some(msg) = status_msg {
        msg.to_owned()
    } else {
        format_counts(counts)
    };
    let status_block = Block::default().borders(Borders::ALL).title("Queue Status");
    let status_para = Paragraph::new(status_text)
        .block(status_block)
        .style(Style::default().fg(Color::White));
    f.render_widget(status_para, chunks[0]);

    // Queue table
    let header_cells = [
        "ID",
        "File",
        "Type",
        "Status",
        "Started",
        "Parent",
        "Wiki Page",
    ]
    .iter()
    .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow).bold()));
    let header = Row::new(header_cells).height(1).bottom_margin(0);

    let rows: Vec<Row> = items
        .iter()
        .map(|item| {
            let row_style = match &item.status {
                ItemStatus::Queued => Style::default().fg(Color::DarkGray),
                ItemStatus::InProgress => Style::default().fg(Color::Yellow),
                ItemStatus::Complete => Style::default().fg(Color::Green),
                ItemStatus::Rejected | ItemStatus::Error => Style::default().fg(Color::Red),
            };

            let started = item
                .started_at
                .map(|dt| dt.format("%H:%M:%S").to_string())
                .unwrap_or_default();
            let parent = item.parent_id.map(|id| id.to_string()).unwrap_or_default();
            let wiki_page = item.wiki_page_path.as_deref().unwrap_or("");
            let file_name = item
                .file_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| item.file_path.to_string_lossy().into_owned());

            Row::new([
                Cell::from(item.id.to_string()),
                Cell::from(file_name),
                Cell::from(item.file_type.as_str()),
                Cell::from(item.status.as_str()),
                Cell::from(started),
                Cell::from(parent),
                Cell::from(wiki_page),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(6),
        Constraint::Min(20),
        Constraint::Length(10),
        Constraint::Length(12),
        Constraint::Length(10),
        Constraint::Length(8),
        Constraint::Min(15),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title("Queue Items"))
        .row_highlight_style(Style::default().bg(Color::DarkGray).bold())
        .highlight_symbol(">> ");

    f.render_stateful_widget(table, chunks[1], table_state);

    // Help line
    let help =
        Paragraph::new(" q: quit | ↑↓: scroll | Enter: details | r: refresh | R: retry errored")
            .style(Style::default().fg(Color::Cyan));
    f.render_widget(help, chunks[2]);
}

// ─── Detail View ─────────────────────────────────────────────────────────────

fn draw_detail(f: &mut Frame, item: &QueueItem, config: &Config, scroll: u16) {
    let area = f.area();

    let status_color = match &item.status {
        ItemStatus::Complete => Color::Green,
        ItemStatus::Error => Color::Red,
        ItemStatus::Rejected => Color::Red,
        _ => Color::Yellow,
    };

    let title = format!(
        " Item {} — {} ",
        item.id,
        item.status.as_str().to_uppercase()
    );

    let mut lines: Vec<Line> = Vec::new();

    // Header info
    lines.push(Line::from(vec![
        Span::styled("File: ", Style::default().bold()),
        Span::raw(item.file_path.to_string_lossy().into_owned()),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Type: ", Style::default().bold()),
        Span::raw(item.file_type.as_str()),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Status: ", Style::default().bold()),
        Span::styled(item.status.as_str(), Style::default().fg(status_color)),
    ]));
    if let Some(parent) = item.parent_id {
        lines.push(Line::from(vec![
            Span::styled("Parent ID: ", Style::default().bold()),
            Span::raw(parent.to_string()),
        ]));
    }
    if let Some(ref wp) = item.wiki_page_path {
        lines.push(Line::from(vec![
            Span::styled("Wiki Page: ", Style::default().bold()),
            Span::raw(wp.clone()),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled("Created: ", Style::default().bold()),
        Span::raw(item.created_at.format("%Y-%m-%d %H:%M:%S").to_string()),
    ]));
    if let Some(dt) = item.started_at {
        lines.push(Line::from(vec![
            Span::styled("Started: ", Style::default().bold()),
            Span::raw(dt.format("%Y-%m-%d %H:%M:%S").to_string()),
        ]));
    }
    if let Some(dt) = item.completed_at {
        lines.push(Line::from(vec![
            Span::styled("Completed: ", Style::default().bold()),
            Span::raw(dt.format("%Y-%m-%d %H:%M:%S").to_string()),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "─".repeat(area.width.saturating_sub(4) as usize),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));

    // Status-specific content
    match &item.status {
        ItemStatus::Error => {
            lines.push(Line::from(Span::styled(
                "ERROR DETAILS",
                Style::default().fg(Color::Red).bold(),
            )));
            lines.push(Line::from(""));
            if let Some(ref msg) = item.error_message {
                for line in msg.lines() {
                    lines.push(Line::from(Span::styled(
                        line.to_string(),
                        Style::default().fg(Color::Red),
                    )));
                }
            } else {
                lines.push(Line::from("No error message recorded."));
            }
        }
        ItemStatus::Rejected => {
            lines.push(Line::from(Span::styled(
                "REJECTION REASON",
                Style::default().fg(Color::Red).bold(),
            )));
            lines.push(Line::from(""));
            if let Some(ref msg) = item.error_message {
                for line in msg.lines() {
                    lines.push(Line::from(line.to_string()));
                }
            } else {
                lines.push(Line::from("No rejection reason recorded."));
            }
        }
        ItemStatus::Complete => {
            lines.push(Line::from(Span::styled(
                "SOURCE SUMMARY",
                Style::default().fg(Color::Green).bold(),
            )));
            lines.push(Line::from(""));

            // Try to read the wiki page content
            if let Some(ref wiki_path) = item.wiki_page_path {
                let full_path = config.paths.wiki_dir.join(wiki_path);
                match std::fs::read_to_string(&full_path) {
                    Ok(content) => {
                        for line in content.lines() {
                            lines.push(Line::from(line.to_string()));
                        }
                    }
                    Err(e) => {
                        lines.push(Line::from(format!(
                            "Could not read wiki page {}: {}",
                            wiki_path, e
                        )));
                    }
                }
            } else {
                lines.push(Line::from("No wiki page path recorded."));
            }
        }
        _ => {}
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " Esc/Enter/q: back to list | ↑↓: scroll",
        Style::default().fg(Color::Cyan),
    )));

    let para = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .title_style(Style::default().fg(status_color).bold()),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    f.render_widget(para, area);
}

fn format_counts(counts: &[(String, u64)]) -> String {
    let get = |status: &str| -> u64 {
        counts
            .iter()
            .find(|(s, _)| s == status)
            .map(|(_, n)| *n)
            .unwrap_or(0)
    };

    format!(
        "Queued: {} | In Progress: {} | Complete: {} | Rejected: {} | Error: {}",
        get("queued"),
        get("in_progress"),
        get("complete"),
        get("rejected"),
        get("error"),
    )
}
