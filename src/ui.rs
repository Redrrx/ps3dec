use crate::{
    args::Ps3decargs,
    logging::LogBuffer,
    queue::{Job, REFRESH_INTERVAL},
};
use indicatif::{HumanBytes, HumanDuration};
use ratatui::{
    Frame,
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout},
    style::{Color, Style, Stylize},
    text::Line,
    widgets::{Block, BorderType, Cell, Paragraph, Row, Table, TableState},
};
use std::{io, time::Instant};

pub(crate) fn run(
    args: &Ps3decargs,
    queue: &crate::queue::Queue,
    logs: &LogBuffer,
) -> io::Result<()> {
    let mut selection = TableState::default();
    let mut selected_id = None;
    let result = (|| {
        let mut terminal = ratatui::try_init()?;
        let mut next_scan = Instant::now();
        loop {
            let stopping = queue.shutting_down();
            if !stopping && Instant::now() >= next_scan {
                queue.discover(args)?;
                next_scan = Instant::now() + REFRESH_INTERVAL;
            }
            let rows = queue.snapshot();
            let selected = selected_id
                .and_then(|id| rows.iter().position(|job| job.id == id))
                .or_else(|| {
                    (!rows.is_empty())
                        .then(|| selection.selected().unwrap_or(0).min(rows.len() - 1))
                });
            selection.select(selected);
            selected_id = selected.map(|index| rows[index].id);
            let done = queue.idle();
            terminal.draw(|frame| render(frame, args, queue, &rows, logs, &mut selection))?;
            if done
                && (stopping
                    || (args.skip
                        && !args.iso.is_empty()
                        && !rows.iter().any(|job| job.active() || job.paused())))
            {
                return Ok(());
            }
            let timeout = if stopping {
                REFRESH_INTERVAL
            } else {
                next_scan.saturating_duration_since(Instant::now())
            };
            if !event::poll(timeout)? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind == KeyEventKind::Release {
                continue;
            }
            let quit = matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                || (key.code == KeyCode::Char('c')
                    && key.modifiers.contains(KeyModifiers::CONTROL));
            if quit {
                if queue.idle() {
                    return Ok(());
                }
                queue.shutdown();
                continue;
            }
            if key.code == KeyCode::Enter && queue.idle() {
                return Ok(());
            }
            if let Some(current) = selected {
                let id = rows[current].id;
                match key.code {
                    KeyCode::Up => {
                        let index = current.saturating_sub(1);
                        selection.select(Some(index));
                        selected_id = Some(rows[index].id);
                    }
                    KeyCode::Down => {
                        let index = (current + 1).min(rows.len() - 1);
                        selection.select(Some(index));
                        selected_id = Some(rows[index].id);
                    }
                    KeyCode::Char('s') if !stopping => queue.start(id),
                    KeyCode::Char('p') if !stopping => queue.toggle_pause(id),
                    KeyCode::Char('c') => queue.cancel(id),
                    KeyCode::Char('+') | KeyCode::Char('=') if !stopping => {
                        queue.move_job(id, true)
                    }
                    KeyCode::Char('-') if !stopping => queue.move_job(id, false),
                    _ => {}
                }
            }
        }
    })();
    let restored = ratatui::try_restore();
    result.and(restored)
}

fn render(
    frame: &mut Frame,
    args: &Ps3decargs,
    queue: &crate::queue::Queue,
    rows: &[Job],
    logs: &LogBuffer,
    selection: &mut TableState,
) {
    let directory = &queue.directory;
    let stopping = queue.shutting_down();
    let done = queue.idle();
    let area = frame.area();
    if area.width < 80 || area.height < 12 {
        frame.render_widget(
            Paragraph::new("Resize to at least 80 × 12. q / Ctrl+C stops safely.").cyan(),
            area,
        );
        return;
    }
    let log_height = (area.height / 3).clamp(4, 10);
    let [title, files, log_area, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(4),
        Constraint::Length(log_height),
        Constraint::Length(2),
    ])
    .areas(area);
    let completed = rows
        .iter()
        .filter(|job| matches!(job.status().as_str(), "Done" | "Already done"))
        .count();
    let failed = rows.iter().filter(|job| job.status() == "Failed").count();
    let jobs = if args.sequential { 1 } else { args.jobs };
    let heading = Line::from(format!(
        " PS3DEC  ·  {} ISOs  ·  {jobs} jobs  ·  {} CPU threads  ·  {completed} done / {failed} failed",
        rows.len(), args.tc,
    )).cyan().bold();
    frame.render_widget(
        Paragraph::new(vec![
            heading,
            Line::from(format!(" {}", directory.display())).dark_gray(),
        ]),
        title,
    );

    let name_width = usize::from(files.width).saturating_sub(60);
    let table_rows = rows.iter().enumerate().map(|(index, job)| {
        let input = &job.input;
        let bar = &job.progress;
        let message = job.status();
        let color = match message.as_str() {
            "Done" | "Already done" => Color::Green,
            "Failed" => Color::Red,
            "Resuming" | "Pausing" | "Paused" | "Stopped" | "Stopping" => Color::Yellow,
            "Waiting" | "Queued" => Color::DarkGray,
            _ => Color::Cyan,
        };
        let name = input
            .file_name()
            .unwrap_or(input.as_os_str())
            .to_string_lossy();
        let name = name.replace(char::is_control, " ");
        let name = console::truncate_str(&name, name_width, "…").into_owned();
        let length = bar.length().unwrap_or(1).max(1);
        let fraction = (bar.position() as f64 / length as f64).clamp(0.0, 1.0);
        let filled = (fraction * 12.0) as usize;
        let progress = format!(
            "{:>3}% {}{}",
            (fraction * 100.0) as u8,
            "━".repeat(filled),
            "─".repeat(12 - filled)
        );
        let active = job.active();
        let rate = bar.per_sec();
        let speed = if active && rate > 0.0 {
            format!("{}/s", HumanBytes(rate as u64))
        } else {
            "--".to_owned()
        };
        let eta = if active && rate > 0.0 {
            format!("{:#}", HumanDuration(bar.eta()))
        } else {
            "--".to_owned()
        };
        Row::new(vec![
            Cell::from((index + 1).to_string()),
            Cell::from(name),
            Cell::from(message).fg(color),
            Cell::from(progress).fg(color),
            Cell::from(speed),
            Cell::from(eta),
        ])
    });
    let table = Table::new(
        table_rows,
        [
            Constraint::Length(3),
            Constraint::Fill(1),
            Constraint::Length(12),
            Constraint::Length(18),
            Constraint::Length(12),
            Constraint::Length(8),
        ],
    )
    .header(
        Row::new(["#", "ISO", "STATUS", "PROGRESS", "SPEED", "ETA"])
            .cyan()
            .bold(),
    )
    .row_highlight_style(Style::default().bg(Color::DarkGray))
    .block(panel(" Decryption queue "));
    frame.render_stateful_widget(table, files, selection);

    let available = usize::from(log_area.height.saturating_sub(2));
    let lines: Vec<Line> = logs
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .rev()
        .take(available)
        .rev()
        .map(|text| {
            let color = if text.contains("[ERROR]") {
                Color::Red
            } else if text.contains("[WARN]") {
                Color::Yellow
            } else {
                Color::Gray
            };
            Line::from(text.clone()).fg(color)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).block(panel(" Logs ")), log_area);

    let hint = if stopping {
        " Stopping after current chunks are synced…"
    } else if done {
        " ↑/↓ select · s start/resume · p pause · c stop · +/- move · Enter/q/Esc close"
    } else {
        " ↑/↓ select · s start/resume · p pause · c stop · +/- move · q/Esc/Ctrl+C stop"
    };
    let filename = selection
        .selected()
        .and_then(|index| rows.get(index))
        .map_or_else(String::new, |job| job.input.display().to_string());
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(hint).cyan(),
            Line::from(filename).dark_gray(),
        ]),
        footer,
    );
}

fn panel(title: &str) -> Block<'_> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Color::DarkGray)
        .title(title)
}
