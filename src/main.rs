use std::io::{self, IsTerminal, Stdout, Write};
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use crossterm::terminal::{
    self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
    enable_raw_mode,
};
use crossterm::{execute, queue};

const GREEN: Color = Color::Rgb {
    r: 105,
    g: 231,
    b: 138,
};
const BLUE: Color = Color::Rgb {
    r: 114,
    g: 165,
    b: 255,
};
const YELLOW: Color = Color::Rgb {
    r: 255,
    g: 209,
    b: 113,
};
const WHITE: Color = Color::Rgb {
    r: 232,
    g: 238,
    b: 247,
};
const MUTED: Color = Color::Rgb {
    r: 151,
    g: 164,
    b: 184,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Focus,
    Break,
}

impl Mode {
    fn default_minutes(self) -> u64 {
        match self {
            Self::Focus => 25,
            Self::Break => 5,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Config {
    mode: Mode,
    minutes: u64,
}

fn usage() -> &'static str {
    "Usage: focus-timer [--mode focus|break] [--minutes 1..1440]\n\
     \n\
     Defaults: focus, hourglass, 25 minutes (focus) or 5 minutes (break).\n\
     Cycle: focus, 5-minute break; a 15-minute break after every 4 focus sessions.\n\
     --minutes sets the duration of the starting mode (and later focus sessions when starting in focus mode).\n\
     Controls: space pause/resume, r restart current session, q or Ctrl+C quit."
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Option<Config>, String> {
    let mut mode = Mode::Focus;
    let mut minutes = None;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            return Ok(None);
        }
        let (flag, inline_value) = arg
            .split_once('=')
            .map_or((arg.as_str(), None), |(a, b)| (a, Some(b)));
        let value = match flag {
            "--mode" | "--minutes" => inline_value
                .map(str::to_owned)
                .or_else(|| args.next())
                .ok_or_else(|| format!("Missing value for {flag}"))?,
            _ => return Err(format!("Unknown option: {arg}")),
        };
        match flag {
            "--mode" => {
                mode = match value.as_str() {
                    "focus" => Mode::Focus,
                    "break" => Mode::Break,
                    _ => return Err(format!("Invalid mode: {value} (use focus or break)")),
                }
            }
            "--minutes" => {
                let number = value
                    .parse::<u64>()
                    .map_err(|_| format!("Invalid minutes: {value}"))?;
                if !(1..=1440).contains(&number) {
                    return Err("Minutes must be between 1 and 1440".into());
                }
                minutes = Some(number);
            }
            _ => unreachable!(),
        }
    }

    Ok(Some(Config {
        mode,
        minutes: minutes.unwrap_or_else(|| mode.default_minutes()),
    }))
}

struct Timer {
    duration: Duration,
    elapsed: Duration,
    last_tick: Instant,
    paused: bool,
}

impl Timer {
    fn new(minutes: u64, now: Instant) -> Self {
        Self {
            duration: Duration::from_secs(minutes * 60),
            elapsed: Duration::ZERO,
            last_tick: now,
            paused: false,
        }
    }

    fn tick(&mut self, now: Instant) -> Duration {
        let overflow = if self.paused {
            Duration::ZERO
        } else {
            let total = self.elapsed + now.saturating_duration_since(self.last_tick);
            self.elapsed = total.min(self.duration);
            total.saturating_sub(self.duration)
        };
        self.last_tick = now;
        overflow
    }

    fn remaining(&self) -> Duration {
        self.duration.saturating_sub(self.elapsed)
    }

    fn complete(&self) -> bool {
        self.elapsed >= self.duration
    }

    fn toggle_pause(&mut self) {
        if !self.complete() {
            self.paused = !self.paused;
        }
    }

    fn reset(&mut self, now: Instant) {
        self.elapsed = Duration::ZERO;
        self.last_tick = now;
        self.paused = false;
    }

    fn progress(&self) -> f64 {
        self.elapsed.as_secs_f64() / self.duration.as_secs_f64()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Focus,
    ShortBreak,
    LongBreak,
}

impl Phase {
    fn label(self) -> &'static str {
        match self {
            Self::Focus => "Focus session",
            Self::ShortBreak => "Short break",
            Self::LongBreak => "Long break",
        }
    }

    fn short_label(self) -> &'static str {
        match self {
            Self::Focus => "focus",
            Self::ShortBreak => "break",
            Self::LongBreak => "long break",
        }
    }
}

struct Cycle {
    phase: Phase,
    completed_focus: u64,
    focus_minutes: u64,
    timer: Timer,
}

impl Cycle {
    fn new(config: Config, now: Instant) -> Self {
        Self {
            phase: if config.mode == Mode::Focus {
                Phase::Focus
            } else {
                Phase::ShortBreak
            },
            completed_focus: 0,
            focus_minutes: if config.mode == Mode::Focus {
                config.minutes
            } else {
                Mode::Focus.default_minutes()
            },
            timer: Timer::new(config.minutes, now),
        }
    }

    fn tick(&mut self, now: Instant) {
        let mut overflow = self.timer.tick(now);
        while self.timer.complete() {
            if self.phase == Phase::Focus {
                self.completed_focus += 1;
            }
            self.phase = match self.phase {
                Phase::Focus if self.completed_focus.is_multiple_of(4) => Phase::LongBreak,
                Phase::Focus => Phase::ShortBreak,
                Phase::ShortBreak | Phase::LongBreak => Phase::Focus,
            };
            let minutes = match self.phase {
                Phase::Focus => self.focus_minutes,
                Phase::ShortBreak => 5,
                Phase::LongBreak => 15,
            };
            self.timer = Timer::new(minutes, now);
            if overflow.is_zero() {
                break;
            }
            let consumed = overflow.min(self.timer.duration);
            self.timer.elapsed = consumed;
            overflow -= consumed;
        }
    }
}

#[derive(Clone, Copy)]
struct Cell {
    ch: char,
    color: Color,
}

struct Canvas {
    width: usize,
    height: usize,
    cells: Vec<Cell>,
}

impl Canvas {
    fn new(width: u16, height: u16) -> Self {
        let width = width as usize;
        let height = height as usize;
        Self {
            width,
            height,
            cells: vec![
                Cell {
                    ch: ' ',
                    color: WHITE
                };
                width * height
            ],
        }
    }

    fn put(&mut self, x: i32, y: i32, ch: char, color: Color) {
        if x >= 0 && y >= 0 && (x as usize) < self.width && (y as usize) < self.height {
            self.cells[y as usize * self.width + x as usize] = Cell { ch, color };
        }
    }

    fn text(&mut self, x: i32, y: i32, value: &str, color: Color) {
        for (offset, ch) in value.chars().enumerate() {
            self.put(x + offset as i32, y, ch, color);
        }
    }

    fn centered(&mut self, y: i32, parts: &[(&str, Color)]) {
        let len: usize = parts.iter().map(|(part, _)| part.chars().count()).sum();
        let mut x = (self.width as i32 - len as i32) / 2;
        for (part, color) in parts {
            self.text(x, y, part, *color);
            x += part.chars().count() as i32;
        }
    }

    fn draw(&self, out: &mut Stdout) -> io::Result<()> {
        if self.width == 0 || self.height == 0 {
            return Ok(());
        }
        queue!(out, Clear(ClearType::All))?;
        let mut current_color = None;
        for y in 0..self.height {
            queue!(out, MoveTo(0, y as u16))?;
            // Writing the bottom-right cell can make some terminals scroll.
            let row_end = (y + 1) * self.width - usize::from(y + 1 == self.height);
            for cell in &self.cells[y * self.width..row_end] {
                if current_color != Some(cell.color) {
                    queue!(out, SetForegroundColor(cell.color))?;
                    current_color = Some(cell.color);
                }
                queue!(out, Print(cell.ch))?;
            }
        }
        out.flush()
    }
}

fn draw_hourglass(canvas: &mut Canvas, cx: i32, top: i32, half_height: i32, timer: &Timer) {
    let widest = (half_height * 3 / 2).max(3);
    let bottom = top + 2 * half_height + 2;
    for x in -widest..=widest {
        canvas.put(cx + x, top, '─', WHITE);
        canvas.put(cx + x, bottom, '─', WHITE);
    }
    for x in (-widest + 1)..widest {
        canvas.put(cx + x, top + 1, '═', WHITE);
        canvas.put(cx + x, bottom - 1, '═', WHITE);
    }
    for (x, upper_ch, lower_ch) in [(-widest, '╱', '╲'), (widest, '╲', '╱')] {
        canvas.put(cx + x, top + 1, upper_ch, WHITE);
        canvas.put(cx + x, bottom - 1, lower_ch, WHITE);
    }
    for x in [-widest - 1, widest + 1] {
        canvas.put(cx + x, top + 1, '│', WHITE);
        canvas.put(cx + x, bottom - 1, '│', WHITE);
    }

    let mut upper = Vec::new();
    let mut lower = Vec::new();
    for row in 1..half_height {
        let half_width = widest - row * (widest - 1) / (half_height - 1);
        let y = top + row + 1;
        canvas.put(cx - half_width - 1, y, '╲', WHITE);
        canvas.put(cx + half_width + 1, y, '╱', WHITE);
        for x in ((-half_width + 1)..half_width).step_by(2) {
            upper.push((cx + x, y));
        }
    }
    for row in 0..(half_height - 1) {
        let half_width = 1 + row * (widest - 1) / (half_height - 1);
        let y = top + half_height + 2 + row;
        canvas.put(cx - half_width - 1, y, '╱', WHITE);
        canvas.put(cx + half_width + 1, y, '╲', WHITE);
        for x in ((-half_width + 1)..half_width).step_by(2) {
            lower.push((cx + x, y));
        }
    }
    let neck = top + half_height + 1;
    canvas.put(cx, neck, '•', YELLOW);

    upper.sort_by_key(|(_, y)| -y);
    lower.sort_by_key(|(_, y)| -y);
    // A little sand in the lower bulb makes the hourglass readable from the first frame.
    let lower_share = 0.3 + 0.7 * timer.progress();
    let upper_count = ((1.0 - lower_share) * upper.len() as f64).ceil() as usize;
    let lower_count = (lower_share * lower.len() as f64).floor() as usize;
    for &(x, y) in upper.iter().take(upper_count) {
        canvas.put(x, y, '•', YELLOW);
    }
    for &(x, y) in lower.iter().take(lower_count) {
        canvas.put(x, y, '•', YELLOW);
    }
    if !timer.paused && !timer.complete() && timer.elapsed >= Duration::from_millis(200) {
        let phase = (timer.elapsed.as_millis() / 180 % 2) as i32;
        let pile_top = lower
            .iter()
            .take(lower_count)
            .map(|(_, y)| *y)
            .min()
            .unwrap_or(bottom - 1);
        for y in (neck + 1)..pile_top {
            if (y + phase) % 2 == 0 {
                canvas.put(cx, y, '•', YELLOW);
            }
        }
    }
}

fn render(cycle: &Cycle, width: u16, height: u16) -> Canvas {
    let mut canvas = Canvas::new(width, height);
    let timer = &cycle.timer;
    let remaining_duration = timer.remaining();
    let remaining = remaining_duration.as_secs() + u64::from(remaining_duration.subsec_nanos() > 0);
    let time = format!("{:02}:{:02}", remaining / 60, remaining % 60);

    if width < 18 || height < 9 {
        let label = if timer.paused {
            format!("{time} P")
        } else {
            time.clone()
        };
        let status_y = if height <= 2 { 0 } else { height as i32 / 2 };
        canvas.centered(status_y, &[(&label, WHITE)]);
        if height > 1 {
            let controls = if width > 8 { "spc r q" } else { "r q" };
            canvas.centered(height as i32 - 1, &[(controls, BLUE)]);
        }
        return canvas;
    }

    let compact = width < 42 || height < 20;
    let half_height = if compact {
        ((height as i32 - 5) / 2)
            .min((width as i32 - 7) / 4)
            .clamp(2, 4)
    } else {
        (((height as i32 - 11) / 2).clamp(3, 9)).min((width as i32 - 12) / 5)
    };
    let art_height = 2 * half_height + 3;
    let art_top = if compact {
        ((height as i32 - art_height - 2) / 2).max(0) + 1
    } else {
        ((height as i32 - art_height - 4) / 2).max(0) + 2
    };
    let status_y = art_top - if compact { 1 } else { 2 };
    if compact {
        let state = if timer.paused { " paused" } else { "" };
        let status = format!("{} {time}{state}", cycle.phase.short_label());
        canvas.centered(status_y, &[(&status, GREEN)]);
    } else {
        let label = if cycle.phase == Phase::Focus {
            format!("{} {}", cycle.phase.label(), cycle.completed_focus + 1)
        } else {
            cycle.phase.label().to_owned()
        };
        let state = if timer.paused { "paused" } else { "remaining" };
        canvas.centered(
            status_y,
            &[
                (&label, GREEN),
                ("  •  ", MUTED),
                (&time, WHITE),
                (" ", WHITE),
                (state, MUTED),
            ],
        );
    }

    draw_hourglass(&mut canvas, width as i32 / 2, art_top, half_height, timer);

    let controls_y = art_top + art_height + i32::from(!compact);
    if compact {
        let action = if timer.paused { "resume" } else { "pause" };
        let full = format!("spc:{action} r:rst q:quit");
        let controls = if full.len() < width as usize {
            full
        } else {
            format!("spc:{action} r q")
        };
        canvas.centered(controls_y, &[(&controls, BLUE)]);
    } else {
        let pause_label = if timer.paused {
            " resume     "
        } else {
            " pause     "
        };
        canvas.centered(
            controls_y,
            &[
                ("[space]", BLUE),
                (pause_label, MUTED),
                ("[r]", BLUE),
                (" reset     ", MUTED),
                ("[q]", BLUE),
                (" quit", MUTED),
            ],
        );
    }
    canvas
}

struct TerminalGuard {
    stdout: Stdout,
}

impl TerminalGuard {
    fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen, Hide) {
            let _ = disable_raw_mode();
            return Err(error);
        }
        Ok(Self { stdout })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(self.stdout, ResetColor, Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

fn run(config: Config) -> io::Result<()> {
    if !io::stdout().is_terminal() || !io::stdin().is_terminal() {
        return Err(io::Error::other("An interactive terminal is required"));
    }
    let mut terminal = TerminalGuard::new()?;
    let mut cycle = Cycle::new(config, Instant::now());
    loop {
        cycle.tick(Instant::now());
        let (width, height) = terminal::size()?;
        render(&cycle, width, height).draw(&mut terminal.stdout)?;
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(KeyEvent {
                code,
                modifiers,
                kind: KeyEventKind::Press,
                ..
            }) = event::read()?
        {
            cycle.tick(Instant::now());
            match code {
                KeyCode::Char('q') => break,
                KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => break,
                KeyCode::Char(' ') => cycle.timer.toggle_pause(),
                KeyCode::Char('r') => cycle.timer.reset(Instant::now()),
                _ => {}
            }
        }
    }
    Ok(())
}

fn main() {
    match parse_args(std::env::args().skip(1)) {
        Ok(Some(config)) => {
            if let Err(error) = run(config) {
                eprintln!("focus-timer: {error}");
                std::process::exit(1);
            }
        }
        Ok(None) => println!("{}", usage()),
        Err(error) => {
            eprintln!("focus-timer: {error}\n\n{}", usage());
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_use_mode_default_and_explicit_minutes() {
        let break_config = parse_args(["--mode", "break"].map(str::to_owned))
            .unwrap()
            .unwrap();
        assert_eq!(
            break_config,
            Config {
                mode: Mode::Break,
                minutes: 5
            }
        );
        let explicit = parse_args(["--minutes", "15", "--mode", "break"].map(str::to_owned))
            .unwrap()
            .unwrap();
        assert_eq!(explicit.minutes, 15);
        assert!(parse_args(["--minutes", "0"].map(str::to_owned)).is_err());
        assert!(parse_args(["--view", "clock"].map(str::to_owned)).is_err());
    }

    #[test]
    fn pause_and_reset_preserve_elapsed_time() {
        let start = Instant::now();
        let mut timer = Timer::new(1, start);
        timer.tick(start + Duration::from_secs(12));
        timer.toggle_pause();
        timer.tick(start + Duration::from_secs(42));
        assert_eq!(timer.remaining(), Duration::from_secs(48));
        timer.toggle_pause();
        timer.tick(start + Duration::from_secs(50));
        assert_eq!(timer.remaining(), Duration::from_secs(40));
        timer.reset(start + Duration::from_secs(50));
        assert_eq!(timer.remaining(), Duration::from_secs(60));
        assert!(!timer.paused);
    }

    #[test]
    fn completed_timer_stays_at_zero() {
        let start = Instant::now();
        let mut timer = Timer::new(1, start);
        timer.tick(start + Duration::from_secs(65));
        assert!(timer.complete());
        assert_eq!(timer.remaining(), Duration::ZERO);
        timer.tick(start + Duration::from_secs(120));
        assert_eq!(timer.progress(), 1.0);
    }

    #[test]
    fn cycle_uses_short_breaks_then_a_long_break() {
        let start = Instant::now();
        let config = parse_args([]).unwrap().unwrap();
        let mut cycle = Cycle::new(config, start);
        assert_eq!(cycle.phase, Phase::Focus);
        assert_eq!(cycle.timer.remaining(), Duration::from_secs(25 * 60));

        for session in 1..=4 {
            let focus_end = start + Duration::from_secs(session * 25 * 60 + (session - 1) * 5 * 60);
            cycle.tick(focus_end);
            assert_eq!(cycle.completed_focus, session);
            assert_eq!(
                cycle.phase,
                if session == 4 {
                    Phase::LongBreak
                } else {
                    Phase::ShortBreak
                }
            );
            let break_minutes = if session == 4 { 15 } else { 5 };
            assert_eq!(
                cycle.timer.remaining(),
                Duration::from_secs(break_minutes * 60)
            );
            cycle.tick(focus_end + Duration::from_secs(break_minutes * 60));
            assert_eq!(cycle.phase, Phase::Focus);
        }
        assert_eq!(cycle.completed_focus, 4);
        assert_eq!(cycle.timer.remaining(), Duration::from_secs(25 * 60));
    }

    #[test]
    fn cycle_carries_late_ticks_and_preserves_pause_and_restart() {
        let start = Instant::now();
        let config = parse_args(["--minutes", "10"].map(str::to_owned))
            .unwrap()
            .unwrap();
        let mut cycle = Cycle::new(config, start);
        cycle.tick(start + Duration::from_secs(16 * 60));
        assert_eq!(cycle.phase, Phase::Focus);
        assert_eq!(cycle.completed_focus, 1);
        assert_eq!(cycle.timer.remaining(), Duration::from_secs(9 * 60));

        cycle.timer.toggle_pause();
        cycle.tick(start + Duration::from_secs(30 * 60));
        assert_eq!(cycle.timer.remaining(), Duration::from_secs(9 * 60));
        cycle.timer.toggle_pause();
        cycle.timer.reset(start + Duration::from_secs(30 * 60));
        assert_eq!(cycle.completed_focus, 1);
        assert_eq!(cycle.timer.remaining(), Duration::from_secs(10 * 60));
    }

    #[test]
    fn starting_in_break_then_moves_to_default_focus_duration() {
        let start = Instant::now();
        let config = parse_args(["--mode", "break", "--minutes", "3"].map(str::to_owned))
            .unwrap()
            .unwrap();
        let mut cycle = Cycle::new(config, start);
        cycle.tick(start + Duration::from_secs(3 * 60));
        assert_eq!(cycle.phase, Phase::Focus);
        assert_eq!(cycle.timer.remaining(), Duration::from_secs(25 * 60));
    }

    #[test]
    fn hourglass_changes_as_time_passes() {
        let start = Instant::now();
        let mut timer = Timer::new(1, start);
        let mut initial_hourglass = Canvas::new(90, 30);
        draw_hourglass(&mut initial_hourglass, 45, 5, 9, &timer);
        assert!(
            initial_hourglass
                .cells
                .chunks(initial_hourglass.width)
                .skip(16)
                .any(|row| row
                    .iter()
                    .any(|cell| cell.ch == '•' && cell.color == YELLOW))
        );

        timer.tick(start + Duration::from_secs(30));
        let mut later_hourglass = Canvas::new(90, 30);
        draw_hourglass(&mut later_hourglass, 45, 5, 9, &timer);

        assert_ne!(
            initial_hourglass
                .cells
                .iter()
                .map(|cell| cell.ch)
                .collect::<String>(),
            later_hourglass
                .cells
                .iter()
                .map(|cell| cell.ch)
                .collect::<String>()
        );
    }

    #[test]
    fn compact_terminal_keeps_timer_art_and_controls_visible() {
        let config = Config {
            mode: Mode::Focus,
            minutes: 25,
        };
        let cycle = Cycle::new(config, Instant::now());
        let canvas = render(&cycle, 24, 12);
        let rows: Vec<String> = canvas
            .cells
            .chunks(canvas.width)
            .map(|row| row.iter().map(|cell| cell.ch).collect())
            .collect();
        assert!(rows.iter().any(|row| row.contains("25:00")));
        assert!(rows.iter().any(|row| row.contains("spc")));
        assert!(rows.iter().any(|row| row.contains('═')));
    }

    #[test]
    fn very_small_terminal_keeps_countdown_and_keys_separate() {
        let config = Config {
            mode: Mode::Focus,
            minutes: 25,
        };
        let cycle = Cycle::new(config, Instant::now());
        let canvas = render(&cycle, 12, 2);
        let first: String = canvas.cells[..12].iter().map(|cell| cell.ch).collect();
        let second: String = canvas.cells[12..].iter().map(|cell| cell.ch).collect();
        assert!(first.contains("25:00"));
        assert!(second.contains("spc r q"));
    }
}
