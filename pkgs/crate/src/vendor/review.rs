use crate::prelude::*;

use super::update::ReviewFile;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use std::io::{self, IsTerminal, Write};
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Style as SyntectStyle, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

pub(super) enum ReviewOutcome {
    Accepted,
    Rejected,
    Cancelled,
}

enum Screen {
    File(usize),
    Recap,
}

struct ReviewApp<'a> {
    files: &'a mut [ReviewFile],
    diffs: Vec<Text<'static>>,
    recap: Text<'static>,
    screen: Screen,
    scroll: u16,
}

pub(super) fn run(files: &mut [ReviewFile]) -> Result<ReviewOutcome> {
    if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
        return run_plain(files);
    }

    let syntaxes = two_face::syntax::extra_newlines();
    let themes = ThemeSet::load_defaults();
    let theme = themes
        .themes
        .get("base16-ocean.dark")
        .or_else(|| themes.themes.values().next())
        .context("Syntect did not provide a highlighting theme")?;
    let diffs = files
        .iter()
        .map(|file| {
            highlight_diff(
                &file.path,
                &file.diff,
                &file.old_source,
                &file.new_source,
                &syntaxes,
                theme,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let mut app = ReviewApp {
        files,
        diffs,
        recap: Text::default(),
        screen: Screen::File(0),
        scroll: 0,
    };

    enable_raw_mode().context("Failed to enable terminal raw mode")?;
    let mut stdout = io::stdout();
    if let Err(error) = execute!(stdout, EnterAlternateScreen) {
        let _ = disable_raw_mode();
        return Err(error).context("Failed to enter terminal review screen");
    }
    let mut terminal = match Terminal::new(CrosstermBackend::new(stdout)) {
        Ok(terminal) => terminal,
        Err(error) => {
            let _ = disable_raw_mode();
            let mut stdout = io::stdout();
            let _ = execute!(stdout, LeaveAlternateScreen);
            return Err(error).context("Failed to initialize terminal review screen");
        }
    };

    let result = run_terminal(&mut terminal, &mut app);
    let raw_result = disable_raw_mode().context("Failed to restore terminal mode");
    let screen_result = execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("Failed to leave terminal review screen");
    let cursor_result = terminal
        .show_cursor()
        .context("Failed to restore terminal cursor");
    let cleanup = raw_result.and(screen_result).and(cursor_result);
    match result {
        Ok(outcome) => {
            cleanup?;
            Ok(outcome)
        }
        Err(error) => Err(error),
    }
}

fn run_terminal(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut ReviewApp<'_>,
) -> Result<ReviewOutcome> {
    loop {
        terminal.draw(|frame| draw(frame, app))?;
        let Event::Key(key) = event::read().context("Failed to read terminal input")? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let viewport_height = terminal.size()?.height.saturating_sub(3);
        if let Some(outcome) = handle_key(app, key, viewport_height) {
            return Ok(outcome);
        }
    }
}

fn draw(frame: &mut ratatui::Frame<'_>, app: &ReviewApp<'_>) {
    let [content, footer] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(frame.area());
    let (title, text, help) = match app.screen {
        Screen::File(index) => (
            format!(
                " Review {}/{}: {} ",
                index + 1,
                app.files.len(),
                app.files[index].path
            ),
            &app.diffs[index],
            " [a] accept  [r] reject  [c/esc] cancel  [j/k/up/down] scroll ",
        ),
        Screen::Recap => (
            " Review recap ".to_owned(),
            &app.recap,
            " [a] apply accepted changes  [r] reject update  [j/k/up/down] scroll ",
        ),
    };
    let visible_height = usize::from(content.height.saturating_sub(2));
    let visible = Text::from(
        text.lines
            .iter()
            .skip(usize::from(app.scroll))
            .take(visible_height)
            .cloned()
            .collect::<Vec<_>>(),
    );
    frame.render_widget(
        Paragraph::new(visible).block(Block::default().title(title).borders(Borders::ALL)),
        content,
    );
    frame.render_widget(
        Paragraph::new(help).style(Style::default().fg(Color::DarkGray)),
        footer,
    );
}

fn handle_key(
    app: &mut ReviewApp<'_>,
    key: KeyEvent,
    viewport_height: u16,
) -> Option<ReviewOutcome> {
    let line_count = match app.screen {
        Screen::File(index) => app.diffs[index].lines.len(),
        Screen::Recap => app.recap.lines.len(),
    };
    let max_scroll = line_count
        .saturating_sub(usize::from(viewport_height))
        .min(usize::from(u16::MAX)) as u16;
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            app.scroll = app.scroll.saturating_add(1).min(max_scroll)
        }
        KeyCode::Char('k') | KeyCode::Up => app.scroll = app.scroll.saturating_sub(1),
        KeyCode::PageDown => app.scroll = app.scroll.saturating_add(10).min(max_scroll),
        KeyCode::PageUp => app.scroll = app.scroll.saturating_sub(10),
        KeyCode::Char('c') | KeyCode::Esc if matches!(app.screen, Screen::File(_)) => {
            return Some(ReviewOutcome::Cancelled);
        }
        KeyCode::Char(choice @ ('a' | 'r')) => match app.screen {
            Screen::File(index) => {
                app.files[index].accepted = choice == 'a';
                app.scroll = 0;
                if index + 1 == app.files.len() {
                    app.recap = recap(app.files);
                    app.screen = Screen::Recap;
                } else {
                    app.screen = Screen::File(index + 1);
                }
            }
            Screen::Recap => {
                return Some(if choice == 'a' {
                    ReviewOutcome::Accepted
                } else {
                    ReviewOutcome::Rejected
                });
            }
        },
        _ => {}
    }
    None
}

fn recap(files: &[ReviewFile]) -> Text<'static> {
    Text::from(
        files
            .iter()
            .map(|file| {
                let (decision, color) = if file.accepted {
                    ("accepted", Color::Green)
                } else {
                    ("rejected", Color::Red)
                };
                Line::from(vec![
                    Span::raw(format!("{}  ", file.path)),
                    Span::styled(
                        format!("+{}", file.additions),
                        Style::default().fg(Color::Green),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        format!("-{}", file.deletions),
                        Style::default().fg(Color::Red),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        decision,
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    ),
                ])
            })
            .collect::<Vec<_>>(),
    )
}

fn highlight_diff(
    path: &str,
    diff: &str,
    old_source: &str,
    new_source: &str,
    syntaxes: &SyntaxSet,
    theme: &syntect::highlighting::Theme,
) -> Result<Text<'static>> {
    let path = Path::new(path);
    let syntax = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| syntaxes.find_syntax_by_extension(name))
        .or_else(|| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .and_then(|extension| syntaxes.find_syntax_by_extension(extension))
        })
        .unwrap_or_else(|| syntaxes.find_syntax_plain_text());
    let old_lines = highlight_source(old_source, syntax, syntaxes, theme)?;
    let new_lines = highlight_source(new_source, syntax, syntaxes, theme)?;
    let mut old_index = 0;
    let mut new_index = 0;
    let mut lines = Vec::new();

    for line in diff.split_terminator('\n') {
        if line.starts_with("--- ") || line.starts_with("+++ ") {
            lines.push(Line::styled(
                line.to_owned(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
            continue;
        }
        if line.starts_with("@@") {
            let mut ranges = line.split_whitespace().skip(1);
            old_index = ranges.next().and_then(hunk_start).unwrap_or(old_index);
            new_index = ranges.next().and_then(hunk_start).unwrap_or(new_index);
            lines.push(Line::styled(
                line.to_owned(),
                Style::default().fg(Color::Magenta),
            ));
            continue;
        }

        let (prefix, code_spans, background, marker) = match line.as_bytes().first() {
            Some(b'+') => (
                '+',
                source_spans(&new_lines, new_index, &line[1..]),
                Color::Rgb(16, 48, 32),
                Color::Green,
            ),
            Some(b'-') => (
                '-',
                source_spans(&old_lines, old_index, &line[1..]),
                Color::Rgb(55, 25, 30),
                Color::Red,
            ),
            Some(b' ') => (
                ' ',
                source_spans(&new_lines, new_index, &line[1..]),
                Color::Reset,
                Color::DarkGray,
            ),
            _ => (
                ' ',
                vec![Span::raw(line.to_owned())],
                Color::Reset,
                Color::DarkGray,
            ),
        };
        match line.as_bytes().first() {
            Some(b'+') => new_index += 1,
            Some(b'-') => old_index += 1,
            Some(b' ') => {
                old_index += 1;
                new_index += 1;
            }
            _ => {}
        }
        let mut spans = vec![Span::styled(
            prefix.to_string(),
            Style::default().fg(marker),
        )];
        spans.extend(code_spans);
        let style = if background == Color::Reset {
            Style::default()
        } else {
            Style::default().bg(background)
        };
        lines.push(Line::from(spans).style(style));
    }
    Ok(Text::from(lines))
}

fn highlight_source(
    source: &str,
    syntax: &syntect::parsing::SyntaxReference,
    syntaxes: &SyntaxSet,
    theme: &syntect::highlighting::Theme,
) -> Result<Vec<Vec<Span<'static>>>> {
    let mut highlighter = HighlightLines::new(syntax, theme);
    LinesWithEndings::from(source)
        .map(|line| {
            highlighter
                .highlight_line(line, syntaxes)
                .context("Failed to syntax highlight source")
                .map(|ranges| {
                    ranges
                        .into_iter()
                        .map(|(style, value)| {
                            Span::styled(
                                value.trim_end_matches(['\r', '\n']).to_owned(),
                                convert_style(style),
                            )
                        })
                        .collect()
                })
        })
        .collect()
}

fn source_spans(lines: &[Vec<Span<'static>>], index: usize, fallback: &str) -> Vec<Span<'static>> {
    lines
        .get(index)
        .cloned()
        .unwrap_or_else(|| vec![Span::raw(fallback.to_owned())])
}

fn hunk_start(range: &str) -> Option<usize> {
    range
        .get(1..)?
        .split(',')
        .next()?
        .parse::<usize>()
        .ok()
        .map(|line| line.saturating_sub(1))
}

fn convert_style(style: SyntectStyle) -> Style {
    let mut result = Style::default().fg(Color::Rgb(
        style.foreground.r,
        style.foreground.g,
        style.foreground.b,
    ));
    if style.font_style.contains(FontStyle::BOLD) {
        result = result.add_modifier(Modifier::BOLD);
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        result = result.add_modifier(Modifier::ITALIC);
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        result = result.add_modifier(Modifier::UNDERLINED);
    }
    result
}

fn run_plain(files: &mut [ReviewFile]) -> Result<ReviewOutcome> {
    let file_count = files.len();
    for (index, file) in files.iter_mut().enumerate() {
        println!("\n[{}/{}] {}", index + 1, file_count, file.path);
        print!("{}", file.diff);
        loop {
            print!("Accept, reject, or cancel? [a/r/c] ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            match input.trim().to_ascii_lowercase().as_str() {
                "a" => {
                    file.accepted = true;
                    break;
                }
                "r" => break,
                "c" => return Ok(ReviewOutcome::Cancelled),
                _ => {}
            }
        }
    }
    println!("\nReview recap:");
    for file in files.iter() {
        println!(
            "  {}  +{} -{}  {}",
            file.path,
            file.additions,
            file.deletions,
            if file.accepted {
                "accepted"
            } else {
                "rejected"
            }
        );
    }
    loop {
        print!("Apply accepted changes? [a/r] ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        match input.trim().to_ascii_lowercase().as_str() {
            "a" => return Ok(ReviewOutcome::Accepted),
            "r" => return Ok(ReviewOutcome::Rejected),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_review_scrolling_to_content() {
        let mut files = [];
        let mut app = ReviewApp {
            files: &mut files,
            diffs: Vec::new(),
            recap: Text::from(
                (0..20)
                    .map(|index| Line::raw(index.to_string()))
                    .collect::<Vec<_>>(),
            ),
            screen: Screen::Recap,
            scroll: 0,
        };
        let down = KeyEvent::new(KeyCode::Char('j'), crossterm::event::KeyModifiers::NONE);
        for _ in 0..30 {
            handle_key(&mut app, down, 5);
        }
        assert_eq!(app.scroll, 15);

        let up = KeyEvent::new(KeyCode::Up, crossterm::event::KeyModifiers::NONE);
        handle_key(&mut app, up, 5);
        assert_eq!(app.scroll, 14);
    }

    #[test]
    fn highlights_diff_markers_and_source_code() {
        let syntaxes = two_face::syntax::extra_newlines();
        let themes = ThemeSet::load_defaults();
        let theme = &themes.themes["base16-ocean.dark"];
        assert_eq!(
            syntaxes.find_syntax_by_extension("ts").unwrap().name,
            "TypeScript"
        );
        let text = highlight_diff(
            "file.rs",
            "--- a/file.rs\n+++ b/file.rs\n@@ -1 +1 @@\n-let old = true;\n+let new = false;\n",
            "let old = true;\n",
            "let new = false;\n",
            &syntaxes,
            theme,
        )
        .unwrap();

        assert_eq!(text.lines.len(), 5);
        assert_eq!(text.lines[3].style.bg, Some(Color::Rgb(55, 25, 30)));
        assert_eq!(text.lines[4].style.bg, Some(Color::Rgb(16, 48, 32)));
        assert!(text.lines[4].spans.len() > 2);
        let colors = text.lines[4]
            .spans
            .iter()
            .filter_map(|span| span.style.fg)
            .collect::<Vec<_>>();
        assert!(colors.iter().skip(2).any(|color| *color != colors[1]));
    }
}
