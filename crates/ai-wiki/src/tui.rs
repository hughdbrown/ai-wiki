use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::prelude::*;
use ratatui::widgets::*;

use ai_wiki_core::config::Config;
use ai_wiki_core::queue::{ItemStatus, Queue, QueueItem};

pub fn run(config: &Config) -> anyhow::Result<()> {
    let queue = Queue::open(&config.paths.database_path)
        .map_err(|e| anyhow::anyhow!("failed to open queue: {}", e))?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, queue);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, queue: Queue) -> anyhow::Result<()> {
    let mut table_state = TableState::default();
    let mut items: Vec<QueueItem> = vec![];
    let mut counts: Vec<(String, u64)> = vec![];
    let mut last_refresh = Instant::now() - Duration::from_secs(3); // force immediate refresh

    loop {
        // Refresh data every 2 seconds
        if last_refresh.elapsed() >= Duration::from_secs(2) {
            items = queue
                .list_items(None)
                .map_err(|e| anyhow::anyhow!("failed to list items: {}", e))?;
            counts = queue
                .count_by_status()
                .map_err(|e| anyhow::anyhow!("failed to count by status: {}", e))?;
            last_refresh = Instant::now();

            // Clamp selection to valid range
            if let Some(sel) = table_state.selected() {
                if items.is_empty() {
                    table_state.select(None);
                } else if sel >= items.len() {
                    table_state.select(Some(items.len() - 1));
                }
            }
        }

        terminal.draw(|f| draw(f, &items, &counts, &mut table_state))?;

        // Poll for events with 250ms timeout for responsiveness
        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('r') => {
                        // Force immediate refresh
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
                    _ => {}
                }
            }
        }
    }
}

fn draw(
    f: &mut Frame,
    items: &[QueueItem],
    counts: &[(String, u64)],
    table_state: &mut TableState,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(f.area());

    // ── Status bar ────────────────────────────────────────────────────────────
    let status_text = format_counts(counts);
    let status_block = Block::default()
        .borders(Borders::ALL)
        .title("Queue Status");
    let status_para = Paragraph::new(status_text)
        .block(status_block)
        .style(Style::default().fg(Color::White));
    f.render_widget(status_para, chunks[0]);

    // ── Queue table ───────────────────────────────────────────────────────────
    let header_cells = ["ID", "File", "Type", "Status", "Started", "Parent", "Wiki Page"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow).bold()));
    let header = Row::new(header_cells).height(1).bottom_margin(0);

    let rows: Vec<Row> = items.iter().map(|item| {
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

        let parent = item
            .parent_id
            .map(|id| id.to_string())
            .unwrap_or_default();

        let wiki_page = item.wiki_page_path.clone().unwrap_or_default();

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
    }).collect();

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

    // ── Help line ─────────────────────────────────────────────────────────────
    let help = Paragraph::new(" q: quit | ↑↓: scroll | r: refresh")
        .style(Style::default().fg(Color::Cyan));
    f.render_widget(help, chunks[2]);
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
