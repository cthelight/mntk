//! The dialog-style terminal UI for interactive sessions.
//!
//! Recreates the `dialog`-based workflow of the original bash scripts — an
//! input box for the search term, then a menu of the results — rendered
//! in-process with ratatui, so no external dialog tool is required.

use std::time::Duration;

use anyhow::{Context, Result};
use mntk_core::source::MovieSearchResult;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};

/// How long to wait for a key before redrawing (picks up window resizes).
const POLL: Duration = Duration::from_millis(100);
/// The last menu item, which sends the session back to the input box.
const SEARCH_AGAIN: &str = "Search again…";

/// Runs the interactive session: ask for a search term, show the results as
/// a menu, and repeat until the user picks a movie or quits.
///
/// `search` is called with each accepted term; its errors are shown to the
/// user, who can then retry or quit. Returns the chosen result, or `None`
/// if the user quit (Esc or Ctrl+C).
pub fn session(
    file_name: &str,
    initial_term: Option<String>,
    search: impl Fn(&str) -> Result<Vec<MovieSearchResult>>,
) -> Result<Option<MovieSearchResult>> {
    let terminal = ratatui::try_init().context("failed to initialize the terminal")?;
    let result = run_session(terminal, file_name, initial_term, &search);
    ratatui::try_restore().context("failed to restore the terminal")?;
    result
}

fn run_session(
    mut terminal: DefaultTerminal,
    file_name: &str,
    initial_term: Option<String>,
    search: &dyn Fn(&str) -> Result<Vec<MovieSearchResult>>,
) -> Result<Option<MovieSearchResult>> {
    let mut screen = Screen::Input(Input::prefilled(initial_term.unwrap_or_default()));

    loop {
        match &mut screen {
            Screen::Input(input) => terminal.draw(|frame| render_input(frame, file_name, input))?,
            Screen::Menu(menu) => terminal.draw(|frame| render_menu(frame, menu))?,
            Screen::Message(message) => terminal.draw(|frame| render_message(frame, message))?,
            Screen::Searching { term } => terminal.draw(|frame| render_searching(frame, term))?,
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
                    lines: vec![format!("No results for {term:?} — try a different term.")],
                    retry_term: term,
                }),
                Err(error) => Screen::Message(Message {
                    lines: vec![error.to_string()],
                    retry_term: term,
                }),
            };
            continue;
        }

        while !event::poll(POLL)? {}
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                match handle_key(&mut screen, key.code, key.modifiers) {
                    KeyOutcome::Stay => {}
                    KeyOutcome::Quit => return Ok(None),
                    KeyOutcome::Chosen(result) => return Ok(Some(result)),
                    KeyOutcome::Search(term) => screen = Screen::Searching { term },
                    KeyOutcome::Retry(term) => screen = Screen::Input(Input::prefilled(term)),
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
}

impl Input {
    fn prefilled(term: String) -> Self {
        Self {
            cursor: term.chars().count(),
            buffer: term.chars().collect(),
            status: None,
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

struct Message {
    lines: Vec<String>,
    /// The term the input box is prefilled with when the user continues.
    retry_term: String,
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

fn handle_key(screen: &mut Screen, code: KeyCode, modifiers: KeyModifiers) -> KeyOutcome {
    if is_quit(code, modifiers) {
        return KeyOutcome::Quit;
    }
    match screen {
        Screen::Input(input) => match code {
            KeyCode::Char(c) => {
                input.buffer.insert(input.cursor, c);
                input.cursor += 1;
                input.status = None;
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
            KeyCode::Enter => {
                if menu.selection < menu.results.len() {
                    return KeyOutcome::Chosen(menu.results[menu.selection].clone());
                }
                return KeyOutcome::Retry(std::mem::take(&mut menu.term));
            }
            _ => {}
        },
        Screen::Message(message) if code == KeyCode::Enter => {
            return KeyOutcome::Retry(std::mem::take(&mut message.retry_term));
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

/// Draws the outer dialog frame and returns the area inside it.
fn content_area(frame: &mut Frame) -> Rect {
    let area = frame.area();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" mntk movie ", Style::default().bold()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

fn render_input(frame: &mut Frame, file_name: &str, input: &Input) {
    let inner = content_area(frame);
    let areas = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(inner);

    let context = Line::from(vec![
        Span::styled("Title search for: ", Style::default().bold()),
        Span::raw(file_name),
    ]);
    frame.render_widget(Paragraph::new(context), areas[0]);

    let field = Block::default().borders(Borders::ALL).title("Search term");
    let field_inner = field.inner(areas[2]);
    frame.render_widget(field, areas[2]);
    let text: String = input.buffer.iter().collect();
    let (start, window) = input_viewport(&text, input.cursor, field_inner.width as usize);
    frame.render_widget(
        Paragraph::new(input_line(&window, input.cursor - start)),
        field_inner,
    );

    if let Some(status) = &input.status {
        frame.render_widget(Paragraph::new(status.clone()).red(), areas[3]);
    }
    frame.render_widget(
        Paragraph::new("Enter: search    Esc/Ctrl+C: quit").dim(),
        areas[4],
    );
}

fn render_menu(frame: &mut Frame, menu: &Menu) {
    let inner = content_area(frame);
    let areas = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(inner);

    let context = Line::from(vec![
        Span::styled("Results for ", Style::default().bold()),
        Span::raw(format!("{:?}", menu.term)),
        Span::styled(
            format!("  ({} matches)", menu.results.len()),
            Style::default().dim(),
        ),
    ]);
    frame.render_widget(Paragraph::new(context), areas[0]);

    let mut lines = menu
        .results
        .iter()
        .enumerate()
        .map(|(index, result)| {
            menu_line(
                format!("{}. {}", index + 1, label(result)),
                index == menu.selection,
            )
        })
        .collect::<Vec<_>>();
    lines.push(menu_line(
        SEARCH_AGAIN.to_string(),
        menu.selection == menu.results.len(),
    ));
    frame.render_widget(Paragraph::new(lines), areas[2]);

    frame.render_widget(
        Paragraph::new("↑/↓: move    Enter: select    Esc/Ctrl+C: quit").dim(),
        areas[3],
    );
}

fn render_message(frame: &mut Frame, message: &Message) {
    let inner = content_area(frame);
    let areas = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(inner);

    let text = Paragraph::new(message.lines.join("\n")).wrap(Wrap { trim: true });
    frame.render_widget(text, areas[2]);
    frame.render_widget(
        Paragraph::new("Enter: continue    Esc/Ctrl+C: quit").dim(),
        areas[3],
    );
}

fn render_searching(frame: &mut Frame, term: &str) {
    let inner = content_area(frame);
    let areas = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);
    frame.render_widget(
        Paragraph::new(format!("Searching for {term:?} …")).italic(),
        areas[0],
    );
}

fn menu_line(text: String, selected: bool) -> Line<'static> {
    if selected {
        Line::from(Span::styled(
            text,
            Style::default().add_modifier(Modifier::REVERSED),
        ))
    } else {
        Line::from(Span::raw(text))
    }
}

/// The slice of `text` shown in a field `width` columns wide, keeping
/// `cursor` visible. Returns the window plus the index of its first char.
fn input_viewport(text: &str, cursor: usize, width: usize) -> (usize, String) {
    let len = text.chars().count();
    if len <= width {
        return (0, text.to_string());
    }
    let start = cursor.saturating_sub(width / 2).min(len - width);
    let window = text.chars().skip(start).take(width).collect::<String>();
    (start, window)
}

/// The input field as styled spans: the char under `cursor` is reversed.
fn input_line(window: &str, cursor: usize) -> Line<'_> {
    let chars: Vec<char> = window.chars().collect();
    let mut spans = Vec::new();
    if cursor > 0 {
        spans.push(Span::raw(chars[..cursor].iter().collect::<String>()));
    }
    if cursor < chars.len() {
        spans.push(Span::styled(
            chars[cursor].to_string(),
            Style::default().add_modifier(Modifier::REVERSED),
        ));
    }
    if cursor + 1 < chars.len() {
        spans.push(Span::raw(chars[cursor + 1..].iter().collect::<String>()));
    }
    Line::from(spans)
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
    fn input_line_keeps_all_chars_and_marks_the_cursor() {
        let line = input_line("abc", 1);
        let rendered: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(rendered, "abc");

        // Cursor past the end: one span, nothing reversed.
        let line = input_line("ab", 2);
        assert_eq!(line.spans.len(), 1);

        // Empty field: no spans at all.
        let line = input_line("", 0);
        assert!(line.spans.is_empty());
    }

    #[test]
    fn prefilled_input_puts_the_cursor_at_the_end() {
        let input = Input::prefilled("the matrix".to_string());
        assert_eq!(input.cursor, 10);
        let text: String = input.buffer.iter().collect();
        assert_eq!(text, "the matrix");
        assert!(input.status.is_none());
    }
}
