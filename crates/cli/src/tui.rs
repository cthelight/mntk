//! The dialog-style terminal UI for interactive sessions.
//!
//! Recreates the `dialog`-based workflow of the original bash scripts — an
//! input box for the search term, then a menu of the results — rendered
//! in-process with ratatui, so no external dialog tool is required.
//!
//! Every screen is drawn as a centered dialog: a title bar with the tool
//! name and the current step, the screen content, and a footer of key hints.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use mntk_core::naming::plan_rename;
use mntk_core::source::MovieSearchResult;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};

/// How long to wait for a key before redrawing (picks up window resizes).
const POLL: Duration = Duration::from_millis(100);
/// The menu action that sends the session back to the input box.
const SEARCH_AGAIN: &str = "Search again…";
/// Shown in the input box while it is empty.
const PLACEHOLDER: &str = "type a title…";

/// The color scheme shared by every screen.
mod theme {
    use ratatui::style::Color;

    pub const TITLE_BG: Color = Color::Blue;
    pub const TITLE_FG: Color = Color::White;
    /// Focused borders and small accents.
    pub const ACCENT: Color = Color::Cyan;
    pub const SELECTED_BG: Color = Color::Blue;
    pub const SELECTED_FG: Color = Color::White;
    pub const MUTED: Color = Color::DarkGray;
    pub const SECONDARY: Color = Color::Gray;
    pub const DANGER: Color = Color::Red;
    pub const WARNING: Color = Color::Yellow;
    pub const SUCCESS: Color = Color::Green;
}

/// Where the initial search term came from, if it came from anywhere.
///
/// The input box is always editable; this only records the origin so the UI
/// can say where the prefill came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seed {
    /// The box starts empty — the file name is never assumed.
    Empty,
    /// Prefilled from `--search <TITLE>`.
    Search(String),
    /// Prefilled from the file name via `--guess`.
    Guess(String),
}

impl Seed {
    /// The term to use as-is, for non-interactive sessions and prefilling.
    pub fn term(&self) -> Option<&str> {
        let term = match self {
            Seed::Empty => return None,
            Seed::Search(term) | Seed::Guess(term) => term,
        };
        let trimmed = term.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    }

    /// A hint about the prefill, shown under the input box.
    pub fn note(&self) -> Option<&'static str> {
        match self {
            Seed::Empty => None,
            Seed::Search(_) => Some("prefilled from --search — edit before searching"),
            Seed::Guess(_) => Some("guessed from the file name — edit before searching"),
        }
    }
}

/// Runs the interactive session: ask for a search term, show the results as
/// a menu, and repeat until the user picks a movie or quits.
///
/// `search` is called with each accepted term; its errors are shown to the
/// user, who can then retry or quit. `file` and `embed_imdb_id` feed the
/// rename preview shown for each candidate. Returns the chosen result, or
/// `None` if the user quit (Esc or Ctrl+C).
pub fn session(
    file: &Path,
    seed: Seed,
    source_name: &str,
    embed_imdb_id: bool,
    search: impl Fn(&str) -> Result<Vec<MovieSearchResult>>,
) -> Result<Option<MovieSearchResult>> {
    let terminal = ratatui::try_init().context("failed to initialize the terminal")?;
    let result = run_session(terminal, file, seed, source_name, embed_imdb_id, &search);
    ratatui::try_restore().context("failed to restore the terminal")?;
    result
}

fn run_session(
    mut terminal: DefaultTerminal,
    file: &Path,
    seed: Seed,
    source_name: &str,
    embed_imdb_id: bool,
    search: &dyn Fn(&str) -> Result<Vec<MovieSearchResult>>,
) -> Result<Option<MovieSearchResult>> {
    let mut screen = Screen::Input(Input::from_seed(&seed));

    loop {
        match &screen {
            Screen::Input(input) => terminal.draw(|frame| render_input(frame, file, input))?,
            Screen::Searching { term } => terminal.draw(|frame| render_searching(frame, term))?,
            Screen::Menu(menu) => {
                terminal.draw(|frame| render_menu(frame, menu, source_name, file, embed_imdb_id))?
            }
            Screen::Message(message) => terminal.draw(|frame| render_message(frame, message))?,
        };

        if let Screen::Searching { term } = &screen {
            let term = term.clone();
            screen = match search(&term) {
                Ok(results) if !results.is_empty() => Screen::Menu(Menu {
                    term: term.clone(),
                    results,
                    selection: 0,
                }),
                Ok(_) => Screen::Message(Message {
                    kind: MessageKind::NoResults,
                    term,
                    detail: None,
                }),
                Err(error) => Screen::Message(Message {
                    kind: MessageKind::Error,
                    term,
                    detail: Some(error.to_string()),
                }),
            };
            continue;
        }

        let page = menu_page(&terminal);
        match next_event()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                match handle_key(&mut screen, key.code, key.modifiers, page) {
                    KeyOutcome::Stay => {}
                    KeyOutcome::Quit => return Ok(None),
                    KeyOutcome::Chosen(result) => return Ok(Some(result)),
                    KeyOutcome::Search(term) => screen = Screen::Searching { term },
                    KeyOutcome::Retry(term) => screen = Screen::Input(Input::retry(term)),
                }
            }
            // Resizes and other events are picked up by the next redraw.
            _ => {}
        }
    }
}

enum Screen {
    Input(Input),
    /// A term was accepted; the search is in flight.
    Searching {
        term: String,
    },
    Menu(Menu),
    Message(Message),
}

struct Input {
    buffer: Vec<char>,
    cursor: usize,
    /// Transient feedback (e.g. "enter a search term").
    status: Option<String>,
    /// Where the prefill came from, shown under the box.
    note: Option<&'static str>,
}

impl Input {
    /// First entry: prefilled from the seed, if it has a term.
    fn from_seed(seed: &Seed) -> Self {
        let term = seed.term().unwrap_or_default().to_string();
        Self {
            cursor: term.chars().count(),
            buffer: term.chars().collect(),
            status: None,
            note: seed.note(),
        }
    }

    /// Back from a search, prefilled with the last term.
    fn retry(term: String) -> Self {
        Self {
            cursor: term.chars().count(),
            buffer: term.chars().collect(),
            status: None,
            note: Some("last search term — edit before searching"),
        }
    }
}

struct Menu {
    /// The term that produced these results; prefills the box on retry.
    term: String,
    results: Vec<MovieSearchResult>,
    /// Index into `results`, or `results.len()` for "Search again…".
    selection: usize,
}

enum MessageKind {
    /// The search completed but found nothing.
    NoResults,
    /// The search failed (network, HTTP, parse...).
    Error,
}

struct Message {
    kind: MessageKind,
    /// The term that was searched for; prefills the box on retry.
    term: String,
    /// The error text, for the `Error` kind.
    detail: Option<String>,
}

enum KeyOutcome {
    /// Nothing to do; keep the current screen.
    Stay,
    /// The user wants to leave the session.
    Quit,
    /// The user accepted a term; run the search.
    Search(String),
    /// Back to the input box, prefilled with this term.
    Retry(String),
    /// The user picked a movie.
    Chosen(MovieSearchResult),
}

fn is_quit(code: KeyCode, modifiers: KeyModifiers) -> bool {
    matches!(code, KeyCode::Esc)
        || (matches!(code, KeyCode::Char('c')) && modifiers.contains(KeyModifiers::CONTROL))
}

fn handle_key(
    screen: &mut Screen,
    code: KeyCode,
    modifiers: KeyModifiers,
    page: usize,
) -> KeyOutcome {
    if is_quit(code, modifiers) {
        return KeyOutcome::Quit;
    }
    match screen {
        Screen::Input(input) => match code {
            KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => {
                input.buffer.insert(input.cursor, c);
                input.cursor += 1;
                input.status = None;
            }
            KeyCode::Char('a') if modifiers.contains(KeyModifiers::CONTROL) => input.cursor = 0,
            KeyCode::Char('e') if modifiers.contains(KeyModifiers::CONTROL) => {
                input.cursor = input.buffer.len()
            }
            KeyCode::Backspace if input.cursor > 0 => {
                input.cursor -= 1;
                input.buffer.remove(input.cursor);
            }
            KeyCode::Delete if input.cursor < input.buffer.len() => {
                input.buffer.remove(input.cursor);
            }
            KeyCode::Left => input.cursor = input.cursor.saturating_sub(1),
            KeyCode::Right => input.cursor = (input.cursor + 1).min(input.buffer.len()),
            KeyCode::Home => input.cursor = 0,
            KeyCode::End => input.cursor = input.buffer.len(),
            KeyCode::Enter => {
                let term: String = input.buffer.iter().collect::<String>().trim().to_string();
                if term.is_empty() {
                    input.status = Some("enter a search term".to_string());
                } else {
                    return KeyOutcome::Search(term);
                }
            }
            _ => {}
        },
        Screen::Menu(menu) => match code {
            KeyCode::Up => {
                let total = menu.results.len() + 1;
                menu.selection = (menu.selection + total - 1) % total;
            }
            KeyCode::Down => {
                let total = menu.results.len() + 1;
                menu.selection = (menu.selection + 1) % total;
            }
            KeyCode::PageUp => {
                menu.selection = menu.selection.saturating_sub(page.max(1));
            }
            KeyCode::PageDown => {
                menu.selection = (menu.selection + page.max(1)).min(menu.results.len());
            }
            KeyCode::Char(c)
                if c.is_ascii_digit() && !modifiers.contains(KeyModifiers::CONTROL) =>
            {
                if let Some(number) = c.to_digit(10)
                    && number >= 1
                    && let Some(result) = menu.results.get(number as usize - 1)
                {
                    return KeyOutcome::Chosen(result.clone());
                }
            }
            KeyCode::Enter => {
                if menu.selection < menu.results.len() {
                    return KeyOutcome::Chosen(menu.results[menu.selection].clone());
                }
                return KeyOutcome::Retry(std::mem::take(&mut menu.term));
            }
            _ => {}
        },
        Screen::Message(message) if code == KeyCode::Enter => {
            return KeyOutcome::Retry(std::mem::take(&mut message.term));
        }
        _ => {}
    }
    KeyOutcome::Stay
}

/// A human-readable label for a result: `Title (Year) - id`.
pub fn label(result: &MovieSearchResult) -> String {
    let year = result
        .year
        .map(|year| year.to_string())
        .unwrap_or_else(|| "N/A".to_string());
    format!("{} ({year}) - {}", result.title, result.id)
}

/// The centered dialog frame for the given terminal area.
///
/// The dialog is capped so it floats in the middle of large terminals, and
/// clamped to whatever fits in small ones.
fn dialog_area(outer: Rect) -> Rect {
    let width = outer.width.saturating_sub(4).clamp(30, 84).min(outer.width);
    let height = outer
        .height
        .saturating_sub(2)
        .clamp(10, 26)
        .min(outer.height);
    Rect {
        x: outer.x + (outer.width.saturating_sub(width)) / 2,
        y: outer.y + (outer.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

/// The dialog border with a title bar: the tool name on the left, the
/// current step on the right.
fn title_block(step: &str) -> Block<'static> {
    let bar = Style::default()
        .bg(theme::TITLE_BG)
        .fg(theme::TITLE_FG)
        .bold();
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::SECONDARY))
        .title_style(theme::TITLE_BG)
        .title_top(Line::from(Span::styled(" mntk · movie ", bar)).left_aligned())
        .title_top(Line::from(Span::styled(format!(" {step} "), bar)).right_aligned())
}

/// A footer of key hints: bold key, dim action, separated by "·".
fn hint_bar<'a>(pairs: &[(&'a str, &'a str)]) -> Line<'a> {
    let mut spans = Vec::new();
    for (index, (key, action)) in pairs.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled("  ·  ", Style::default().fg(theme::MUTED)));
        }
        spans.push(Span::styled(*key, Style::default().bold()));
        spans.push(Span::styled(*action, Style::default().fg(theme::SECONDARY)));
    }
    Line::from(spans)
}

fn cursor_style() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

fn selected_style() -> Style {
    Style::default()
        .bg(theme::SELECTED_BG)
        .fg(theme::SELECTED_FG)
        .bold()
}

fn render_input(frame: &mut Frame, file: &Path, input: &Input) {
    let area = dialog_area(frame.area());
    let block = title_block("1/2 search");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let regions = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(inner);

    let label = "  File  ";
    let name = truncate(
        &file.display().to_string(),
        regions[0].width as usize - label.len(),
    );
    let file_line = Line::from(vec![
        Span::styled(label, Style::default().bold()),
        Span::raw(name),
    ]);
    frame.render_widget(Paragraph::new(vec![file_line, Line::default()]), regions[0]);

    let field = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::ACCENT))
        .title(Span::styled(
            " search term ",
            Style::default().fg(theme::SECONDARY),
        ));
    let field_inner = field.inner(regions[1]);
    frame.render_widget(field, regions[1]);

    let text: String = input.buffer.iter().collect();
    if text.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" ", cursor_style()),
                Span::styled(PLACEHOLDER, Style::default().fg(theme::MUTED)),
            ])),
            field_inner,
        );
    } else {
        let (start, window) = input_viewport(&text, input.cursor, field_inner.width as usize);
        frame.render_widget(
            Paragraph::new(input_line(&window, input.cursor - start)),
            field_inner,
        );
    }

    let hint: Option<Line<'_>> = input
        .status
        .as_ref()
        .map(|status| {
            Line::from(Span::styled(
                format!("  {status}"),
                Style::default().fg(theme::DANGER).bold(),
            ))
        })
        .or_else(|| {
            input.note.map(|note| {
                Line::from(Span::styled(
                    format!("  {note}"),
                    Style::default().fg(theme::MUTED),
                ))
            })
        });
    if let Some(hint) = hint {
        frame.render_widget(Paragraph::new(hint), regions[2]);
    }

    frame.render_widget(
        Paragraph::new(hint_bar(&[("Enter", "  search"), ("Esc", "  quit")])),
        regions[3],
    );
}

fn render_menu(
    frame: &mut Frame,
    menu: &Menu,
    source_name: &str,
    file: &Path,
    embed_imdb_id: bool,
) {
    let area = dialog_area(frame.area());
    let block = title_block("2/2 pick");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let regions = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .split(inner);

    let count = menu.results.len();
    let fixed = format!("  {count} matches for \"\"  ·  {source_name}");
    let term = truncate(
        &menu.term,
        (inner.width as usize).saturating_sub(fixed.chars().count()),
    );
    let header = Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(format!("{count} matches"), Style::default().bold()),
        Span::raw(" for "),
        Span::styled(format!("\"{term}\""), Style::default().fg(theme::ACCENT)),
        Span::styled(
            format!("  ·  {source_name}"),
            Style::default().fg(theme::MUTED),
        ),
    ]);
    frame.render_widget(Paragraph::new(vec![header, Line::default()]), regions[0]);

    let width = regions[1].width as usize;
    let visible = regions[1].height as usize;
    let mut lines: Vec<Line<'static>> = menu
        .results
        .iter()
        .enumerate()
        .map(|(index, result)| result_row(width, index + 1, count, result, index == menu.selection))
        .collect();
    lines.push(separator_line(width));
    lines.push(search_again_line(width, menu.selection == count));

    let active = if menu.selection < count {
        menu.selection
    } else {
        count + 1
    };
    let first = visible_window(lines.len(), active, visible);
    let shown: Vec<Line<'static>> = lines.iter().skip(first).take(visible).cloned().collect();
    frame.render_widget(Paragraph::new(shown), regions[1]);

    render_preview(frame, regions[2], menu, file, embed_imdb_id);

    frame.render_widget(
        Paragraph::new(hint_bar(&[
            ("↑/↓", "  move"),
            ("1-9", "  pick"),
            ("PgUp/PgDn", "  page"),
            ("Enter", "  select"),
            ("Esc", "  quit"),
        ])),
        regions[3],
    );
}

fn render_message(frame: &mut Frame, message: &Message) {
    let area = dialog_area(frame.area());
    let block = title_block("2/2 pick");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let regions = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(inner);

    let (icon, heading, color) = match message.kind {
        MessageKind::NoResults => (
            "!",
            format!("no results for {:?}", message.term),
            theme::WARNING,
        ),
        MessageKind::Error => ("✗", "search failed".to_string(), theme::DANGER),
    };
    let heading_line = Line::from(vec![
        Span::styled(format!("  {icon}  "), Style::default().fg(color).bold()),
        Span::styled(heading, Style::default().fg(color).bold()),
    ]);
    frame.render_widget(Paragraph::new(heading_line), regions[0]);

    let body = match &message.detail {
        Some(detail) => Line::from(Span::styled(
            format!("  {detail}"),
            Style::default().fg(theme::SECONDARY),
        )),
        None => Line::from(Span::styled(
            "  try a different search term",
            Style::default().fg(theme::SECONDARY),
        )),
    };
    frame.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }), regions[1]);

    frame.render_widget(
        Paragraph::new(hint_bar(&[("Enter", "  continue"), ("Esc", "  quit")])),
        regions[2],
    );
}

fn render_searching(frame: &mut Frame, term: &str) {
    let area = dialog_area(frame.area());
    let block = title_block("2/2 pick");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let line = Line::from(vec![
        Span::styled("  ▸  ", Style::default().fg(theme::ACCENT)),
        Span::styled(
            format!("searching for {term:?} …"),
            Style::default().fg(theme::SECONDARY).italic(),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), inner);
}

/// The rename plan for the highlighted result, in a stable box so the list
/// above does not shift as the selection moves between a result and
/// "Search again…".
fn render_preview(frame: &mut Frame, area: Rect, menu: &Menu, file: &Path, embed_imdb_id: bool) {
    let panel = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::SECONDARY))
        .title(Span::styled(
            " rename ",
            Style::default().fg(theme::SECONDARY),
        ));
    let inner = panel.inner(area);
    frame.render_widget(panel, area);

    if let Some(result) = menu.results.get(menu.selection) {
        let plan = plan_rename(file, result, embed_imdb_id);
        let target = format!("{}/{}", plan.dir_name, plan.file_name);
        let budget = inner.width.saturating_sub(4) as usize;
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  → ", Style::default().fg(theme::SUCCESS).bold()),
                Span::raw(middle_truncate(&target, budget)),
            ])),
            inner,
        );
    } else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  edit the search term and search again",
                Style::default().fg(theme::MUTED).italic(),
            ))),
            inner,
        );
    }
}

/// One result row: a dim index, the title with year, and a right-aligned
/// dim id. The row is always padded or truncated to exactly `width`.
fn result_row(
    width: usize,
    index: usize,
    count: usize,
    result: &MovieSearchResult,
    selected: bool,
) -> Line<'static> {
    let idx = format!("  {:>w$}", index, w = count.to_string().len());
    let year = result
        .year
        .map(|year| year.to_string())
        .unwrap_or_else(|| "N/A".to_string());
    let title = format!("  {} ({year})", result.title);
    let right = truncate(&format!("  {}", result.id), width);
    let main_budget = width.saturating_sub(right.chars().count());
    let full = format!("{idx}{title}");
    let truncated = full.chars().count() > main_budget;
    let main = if truncated {
        let mut cut = truncate(&full, main_budget);
        cut.push_str(&" ".repeat(main_budget - cut.chars().count()));
        cut
    } else {
        let mut padded = full;
        padded.push_str(&" ".repeat(main_budget - padded.chars().count()));
        padded
    };
    let title_text: String = main.get(idx.len()..).unwrap_or("").chars().collect();

    if selected {
        return Line::from(Span::styled(main + right.as_str(), selected_style()));
    }
    if truncated {
        return Line::from(vec![
            Span::raw(main),
            Span::styled(right, Style::default().fg(theme::MUTED)),
        ]);
    }
    Line::from(vec![
        Span::styled(idx, Style::default().fg(theme::MUTED)),
        Span::raw(title_text),
        Span::styled(right, Style::default().fg(theme::MUTED)),
    ])
}

fn separator_line(width: usize) -> Line<'static> {
    Line::from(Span::styled(
        "─".repeat(width),
        Style::default().fg(theme::MUTED),
    ))
}

fn search_again_line(width: usize, selected: bool) -> Line<'static> {
    if selected {
        return Line::from(Span::styled(
            format!("{:width$}", format!("  ↺  {SEARCH_AGAIN}"), width = width),
            selected_style(),
        ));
    }
    Line::from(vec![
        Span::styled("  ↺  ", Style::default().fg(theme::ACCENT)),
        Span::styled(SEARCH_AGAIN, Style::default().fg(theme::SECONDARY)),
    ])
}

/// The index of the first item to show in a `visible`-tall window so that
/// the `active` item stays on screen, with context kept above it.
fn visible_window(total: usize, active: usize, visible: usize) -> usize {
    if total <= visible {
        return 0;
    }
    active.saturating_sub(visible / 2).min(total - visible)
}

/// The slice of `text` shown in a field `width` columns wide, keeping
/// `cursor` visible. Returns the window plus the index of its first char.
fn input_viewport(text: &str, cursor: usize, width: usize) -> (usize, String) {
    let chars: Vec<char> = text.chars().collect();
    let start = visible_window(chars.len(), cursor, width);
    (start, chars[start..].iter().take(width).collect())
}

/// The input field as styled spans: the char under `cursor` is reversed, and
/// a block is drawn at the end when the cursor sits past the last char.
fn input_line(window: &str, cursor: usize) -> Line<'_> {
    let chars: Vec<char> = window.chars().collect();
    let mut spans = Vec::new();
    if cursor > 0 {
        spans.push(Span::raw(chars[..cursor].iter().collect::<String>()));
    }
    if cursor < chars.len() {
        spans.push(Span::styled(chars[cursor].to_string(), cursor_style()));
    } else if !chars.is_empty() {
        spans.push(Span::styled(" ".to_string(), cursor_style()));
    }
    if cursor + 1 < chars.len() {
        spans.push(Span::raw(chars[cursor + 1..].iter().collect::<String>()));
    }
    Line::from(spans)
}

/// Truncates `text` to at most `max` chars, ending with "…" when cut.
fn truncate(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_string();
    }
    if max < 2 {
        return String::new();
    }
    let mut out: String = chars[..max - 1].iter().collect();
    out.push('…');
    out
}

/// Truncates `text` to at most `max` chars by dropping the middle, so both
/// the beginning and the end (the file name) stay visible.
fn middle_truncate(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_string();
    }
    if max < 3 {
        return truncate(text, max);
    }
    let keep = (max - 1) / 2;
    let head: String = chars[..keep].iter().collect();
    let tail: String = chars[chars.len() - keep..].iter().collect();
    format!("{head}…{tail}")
}

/// The list region height for a dialog inner height (mirrors `render_menu`).
fn menu_list_height(inner_height: u16) -> u16 {
    Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .split(Rect::new(0, 0, 80, inner_height))[1]
        .height
}

/// How many list rows fit on screen right now (for PgUp/PgDn).
fn menu_page(terminal: &DefaultTerminal) -> usize {
    let size = match terminal.size() {
        Ok(size) => size,
        Err(_) => return 1,
    };
    let dialog = dialog_area(Rect::new(0, 0, size.width, size.height));
    let inner = Block::default().borders(Borders::ALL).inner(dialog);
    menu_list_height(inner.height) as usize
}

/// Reads the next event, draining anything already queued first so fast key
/// presses are not dropped between redraws.
fn next_event() -> Result<Event> {
    loop {
        if event::poll(Duration::ZERO)? || event::poll(POLL)? {
            return event::read().context("failed to read a terminal event");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mntk_core::source::SourceId;

    fn result(title: &str, id: &str, year: Option<u16>) -> MovieSearchResult {
        MovieSearchResult {
            id: SourceId::new(id),
            imdb_id: Some(id.to_string()),
            title: title.to_string(),
            year,
        }
    }

    #[test]
    fn label_shows_title_year_and_id() {
        let formatted = label(&result("The Matrix", "tt0133093", Some(1999)));
        assert_eq!(formatted, "The Matrix (1999) - tt0133093");

        let formatted = label(&result("Mystery", "tt0000001", None));
        assert_eq!(formatted, "Mystery (N/A) - tt0000001");
    }

    #[test]
    fn seed_term_trims_and_rejects_blank() {
        assert_eq!(Seed::Empty.term(), None);
        assert_eq!(Seed::Empty.note(), None);
        let search = Seed::Search("  the matrix  ".to_string());
        assert_eq!(search.term(), Some("the matrix"));
        assert!(search.note().is_some());
        assert_eq!(Seed::Guess("x".into()).term(), Some("x"));
        assert_eq!(Seed::Guess("   ".into()).term(), None);
        assert!(Seed::Guess("x".into()).note().is_some());
    }

    #[test]
    fn viewport_fits_short_text() {
        assert_eq!(input_viewport("abc", 1, 10), (0, "abc".to_string()));
        assert_eq!(input_viewport("", 0, 10), (0, String::new()));
    }

    #[test]
    fn viewport_clips_long_text_around_the_cursor() {
        let text = "0123456789";
        // Cursor at the end: the window ends at the last character.
        let (start, window) = input_viewport(text, 10, 4);
        assert_eq!((start, window.as_str()), (6, "6789"));
        // Cursor at the start: the window starts at the first character.
        let (start, window) = input_viewport(text, 0, 4);
        assert_eq!((start, window.as_str()), (0, "0123"));
        // Cursor in the middle: it stays visible.
        let (start, window) = input_viewport(text, 5, 4);
        assert_eq!(start, 3);
        assert!(window.contains('5'), "{window}");
    }

    #[test]
    fn visible_window_keeps_the_active_row_on_screen() {
        assert_eq!(visible_window(3, 2, 5), 0);
        assert_eq!(visible_window(10, 0, 4), 0);
        assert_eq!(visible_window(10, 9, 4), 6);
        assert_eq!(visible_window(10, 5, 4), 3);
    }

    #[test]
    fn truncate_keeps_short_text() {
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("abc", 3), "abc");
    }

    #[test]
    fn truncate_marks_cut_text() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("abcdef", 1), "");
        assert_eq!(truncate("abcdef", 0), "");
    }

    #[test]
    fn middle_truncate_keeps_both_ends() {
        assert_eq!(middle_truncate("abcdefghij", 10), "abcdefghij");
        let cut = middle_truncate("a/b/c/very/long/path/file.mkv", 15);
        assert!(cut.starts_with("a/b"), "{cut}");
        assert!(cut.ends_with("mkv"), "{cut}");
        assert_eq!(cut.chars().count(), 15);
    }

    #[test]
    fn result_row_never_exceeds_the_width() {
        let long = result(
            "A Very Long Movie Title That Goes On And On And On",
            "tt0133093",
            Some(1999),
        );
        for width in [60usize, 40, 24, 12, 6] {
            for selected in [false, true] {
                let line = result_row(width, 1, 3, &long, selected);
                let total: usize = line
                    .spans
                    .iter()
                    .map(|span| span.content.as_ref().chars().count())
                    .sum();
                assert_eq!(total, width, "width {width}, selected {selected}");
            }
        }
    }

    #[test]
    fn result_row_keeps_the_id_visible() {
        let long = result(
            "A Very Long Movie Title That Goes On And On And On",
            "tt0133093",
            Some(1999),
        );
        let line = result_row(24, 2, 12, &long, false);
        let text: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert!(text.contains("tt0133093"), "{text}");
        assert!(text.contains('2'), "{text}");
    }

    #[test]
    fn input_line_keeps_all_chars_and_marks_the_cursor() {
        let line = input_line("abc", 1);
        let rendered: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(rendered, "abc");

        // Cursor at the end: a trailing block stands in for the cursor.
        let line = input_line("ab", 2);
        assert_eq!(line.spans.len(), 2);
        assert_eq!(line.spans[1].content.as_ref(), " ");

        // Empty field: no spans at all (the caller draws the placeholder).
        let line = input_line("", 0);
        assert!(line.spans.is_empty());
    }

    #[test]
    fn input_from_seed_puts_the_cursor_at_the_end() {
        let input = Input::from_seed(&Seed::Search("the matrix".to_string()));
        assert_eq!(input.cursor, 10);
        let text: String = input.buffer.iter().collect();
        assert_eq!(text, "the matrix");
        assert!(input.note.is_some());
        assert!(input.status.is_none());

        let input = Input::from_seed(&Seed::Empty);
        assert!(input.buffer.is_empty());
        assert_eq!(input.cursor, 0);
        assert!(input.note.is_none());
    }

    #[test]
    fn input_retry_uses_the_last_term_note() {
        let input = Input::retry("inception".to_string());
        let text: String = input.buffer.iter().collect();
        assert_eq!(text, "inception");
        assert_eq!(input.cursor, 9);
        assert_eq!(input.note, Some("last search term — edit before searching"));
        assert!(input.status.is_none());
    }

    #[test]
    fn menu_list_height_shrinks_with_the_dialog() {
        assert!(menu_list_height(20) > menu_list_height(10));
        assert!(menu_list_height(20) > 0);
    }

    #[test]
    fn dialog_area_stays_inside_the_terminal() {
        let area = dialog_area(Rect::new(0, 0, 200, 50));
        assert!((30..=84).contains(&area.width), "width {}", area.width);
        assert!((10..=26).contains(&area.height), "height {}", area.height);
        assert!(area.x > 0 && area.y > 0, "should be centered");

        let area = dialog_area(Rect::new(0, 0, 12, 8));
        assert_eq!((area.width, area.height), (12, 8));
    }

    #[test]
    fn hint_bar_lists_keys_and_actions() {
        let line = hint_bar(&[("Enter", "  search"), ("Esc", "  quit")]);
        let text: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(text, "Enter  search  ·  Esc  quit");
    }
}
