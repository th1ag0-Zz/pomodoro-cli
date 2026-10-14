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
    fn label(self) -> &'static str {
        match self {
            Self::Focus => "Focus session",
            Self::Break => "Break session",
        }
    }

    fn default_minutes(self) -> u64 {
        match self {
            Self::Focus => 25,
            Self::Break => 5,
        }
    }

    fn flag(self) -> &'static str {
        match self {
            Self::Focus => "focus",
            Self::Break => "break",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Clock,
    Hourglass,
}

impl View {
    fn flag(self) -> &'static str {
        match self {
            Self::Clock => "clock",
            Self::Hourglass => "hourglass",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Config {
    mode: Mode,
    view: View,
    minutes: u64,
}

fn usage() -> &'static str {
    "Usage: pomodoro [--mode focus|break] [--view clock|hourglass] [--minutes 1..1440]\n\
     \n\
     Defaults: focus, clock, 25 minutes (focus) or 5 minutes (break).\n\
     Controls: space pause/resume, r restart, q or Ctrl+C quit."
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Option<Config>, String> {
    let mut mode = Mode::Focus;
    let mut view = View::Clock;
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
            "--mode" | "--view" | "--minutes" => inline_value
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
            "--view" => {
                view = match value.as_str() {
                    "clock" => View::Clock,
                    "hourglass" => View::Hourglass,
                    _ => return Err(format!("Invalid view: {value} (use clock or hourglass)")),
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
        view,
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

    fn tick(&mut self, now: Instant) {
        if !self.paused {
            self.elapsed =
                (self.elapsed + now.saturating_duration_since(self.last_tick)).min(self.duration);
        }
        self.last_tick = now;
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

fn point(cx: i32, cy: i32, rx: i32, ry: i32, angle: f64) -> (i32, i32) {
    (
        cx + (angle.sin() * rx as f64).round() as i32,
        cy - (angle.cos() * ry as f64).round() as i32,
    )
}

fn hand(canvas: &mut Canvas, cx: i32, cy: i32, x: i32, y: i32, color: Color) {
    let dx = x - cx;
    let dy = y - cy;
    let steps = dx.abs().max(dy.abs()) * 2;
    if steps == 0 {
        return;
    }
    let glyph = if dx.abs() * 2 < dy.abs() {
        '│'
    } else if dy.abs() * 2 < dx.abs() {
        '─'
    } else if dx.signum() == dy.signum() {
        '╲'
    } else {
        '╱'
    };
    for step in 1..=steps {
        let x = cx + (dx as f64 * step as f64 / steps as f64).round() as i32;
        let y = cy + (dy as f64 * step as f64 / steps as f64).round() as i32;
        canvas.put(x, y, glyph, color);
    }
}

fn draw_clock(canvas: &mut Canvas, cx: i32, cy: i32, radius: i32, timer: &Timer) {
    let rx = radius * 2 + 2;
    let tick_step = if radius < 5 { 5 } else { 1 };
    for tick in (0..60).step_by(tick_step) {
        let angle = std::f64::consts::TAU * tick as f64 / 60.0;
        let (x, y) = point(cx, cy, rx, radius, angle);
        canvas.put(
            x,
            y,
            if tick % 5 == 0 { '•' } else { '·' },
            if tick % 5 == 0 { WHITE } else { MUTED },
        );
        if tick % 5 == 0 {
            let (inner_x, inner_y) = point(cx, cy, rx - 2, radius - 1, angle);
            canvas.put(inner_x, inner_y, '•', WHITE);
        }
    }
    let minute_angle = std::f64::consts::TAU * timer.progress();
    let second_angle = std::f64::consts::TAU * (timer.elapsed.as_secs_f64() % 60.0) / 60.0;
    let (minute_x, minute_y) = point(cx, cy, (rx - 8).max(2), (radius - 3).max(1), minute_angle);
    let (second_x, second_y) = point(cx, cy, (rx - 5).max(2), (radius - 2).max(1), second_angle);
    hand(canvas, cx, cy, minute_x, minute_y, WHITE);
    hand(canvas, cx, cy, second_x, second_y, BLUE);
    canvas.put(cx, cy, '●', WHITE);
    if radius >= 5 {
        let number_radius = radius - 2;
        for (number, angle) in [
            ("12", 0.0),
            ("3", std::f64::consts::FRAC_PI_2),
            ("6", std::f64::consts::PI),
            ("9", 3.0 * std::f64::consts::FRAC_PI_2),
        ] {
            let (x, y) = point(cx, cy, rx - 4, number_radius, angle);
            canvas.text(x - (number.len() as i32 / 2), y, number, WHITE);
        }
    }
}

fn draw_hourglass(canvas: &mut Canvas, cx: i32, top: i32, half_height: i32, timer: &Timer) {
    let widest = half_height * 3 / 2;
    let bottom = top + 2 * half_height + 2;
    for x in -widest..=widest {
        canvas.put(cx + x, top, '─', WHITE);
        canvas.put(cx + x, bottom, '─', WHITE);
    }

    let mut upper = Vec::new();
    let mut lower = Vec::new();
    for row in 0..half_height {
        let half_width = (widest - row * (widest - 1) / half_height).max(1);
        let y = top + row + 1;
        canvas.put(cx - half_width - 1, y, '╲', WHITE);
        canvas.put(cx + half_width + 1, y, '╱', WHITE);
        for x in ((-half_width + 1)..half_width).step_by(2) {
            upper.push((cx + x, y));
        }
    }
    for row in 0..half_height {
        let half_width = (1 + (row + 1) * (widest - 1) / half_height).min(widest);
        let y = top + half_height + 2 + row;
        canvas.put(cx - half_width - 1, y, '╱', WHITE);
        canvas.put(cx + half_width + 1, y, '╲', WHITE);
        for x in ((-half_width + 1)..half_width).step_by(2) {
            lower.push((cx + x, y));
        }
    }
    canvas.put(cx, top + half_height + 1, '·', YELLOW);

    upper.sort_by_key(|(_, y)| -y);
    lower.sort_by_key(|(_, y)| -y);
    let upper_count = ((1.0 - timer.progress()) * upper.len() as f64).ceil() as usize;
    let lower_count = (timer.progress() * lower.len() as f64).floor() as usize;
    for &(x, y) in upper.iter().take(upper_count) {
        canvas.put(x, y, '•', YELLOW);
    }
    for &(x, y) in lower.iter().take(lower_count) {
        canvas.put(x, y, '•', YELLOW);
    }
    if !timer.paused && !timer.complete() && timer.elapsed >= Duration::from_millis(200) {
        let phase = timer.elapsed.as_millis() / 180 % 3;
        canvas.put(cx, top + half_height + 1 + phase as i32, '•', YELLOW);
    }
}

fn render(config: Config, timer: &Timer, width: u16, height: u16) -> Canvas {
    let mut canvas = Canvas::new(width, height);
    let remaining_duration = timer.remaining();
    let remaining = remaining_duration.as_secs() + u64::from(remaining_duration.subsec_nanos() > 0);
    let time = format!("{:02}:{:02}", remaining / 60, remaining % 60);

    if width < 18 || height < 9 {
        let label = if timer.complete() {
            "Done".to_owned()
        } else if timer.paused {
            format!("{time} P")
        } else if width < 8 {
            time.clone()
        } else {
            format!(
                "{} {time}",
                if config.view == View::Clock { 'C' } else { 'H' }
            )
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
        ((height as i32 - art_height) / 2 - 1).max(5)
    };
    let status_y = art_top - if compact { 1 } else { 2 };
    if compact {
        let status = if timer.complete() {
            format!("{} done", config.mode.flag())
        } else if timer.paused {
            format!("Paused {time}")
        } else {
            format!("{} {time}", config.mode.flag())
        };
        canvas.centered(status_y, &[(&status, GREEN)]);
    } else {
        let command = format!(
            "pomodoro --mode {} --view {} --minutes {}",
            config.mode.flag(),
            config.view.flag(),
            config.minutes
        );
        canvas.text(2, 1, "❯", GREEN);
        canvas.text(4, 1, &command, MUTED);
        if timer.complete() {
            canvas.centered(
                status_y,
                &[(config.mode.label(), GREEN), ("  •  Complete!", WHITE)],
            );
        } else {
            let state = if timer.paused { "paused" } else { "remaining" };
            canvas.centered(
                status_y,
                &[
                    (config.mode.label(), GREEN),
                    ("  •  ", MUTED),
                    (&time, WHITE),
                    (" ", WHITE),
                    (state, MUTED),
                ],
            );
        }
    }

    let cx = width as i32 / 2;
    match config.view {
        View::Clock => draw_clock(
            &mut canvas,
            cx,
            art_top + half_height + 1,
            half_height,
            timer,
        ),
        View::Hourglass => draw_hourglass(&mut canvas, cx, art_top, half_height, timer),
    }

    let controls_y = art_top + art_height + i32::from(!compact);
    if compact {
        let controls = if timer.complete() {
            "r:restart q:quit".to_owned()
        } else {
            let action = if timer.paused { "resume" } else { "pause" };
            let full = format!("spc:{action} r:rst q:quit");
            if full.len() < width as usize {
                full
            } else {
                format!("spc:{action} r q")
            }
        };
        canvas.centered(controls_y, &[(&controls, BLUE)]);
    } else if timer.complete() {
        canvas.centered(
            controls_y,
            &[
                ("[r]", BLUE),
                (" restart     ", MUTED),
                ("[q]", BLUE),
                (" quit", MUTED),
            ],
        );
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
    let mut timer = Timer::new(config.minutes, Instant::now());
    loop {
        timer.tick(Instant::now());
        let (width, height) = terminal::size()?;
        render(config, &timer, width, height).draw(&mut terminal.stdout)?;
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(KeyEvent {
                code,
                modifiers,
                kind: KeyEventKind::Press,
                ..
            }) = event::read()?
        {
            timer.tick(Instant::now());
            match code {
                KeyCode::Char('q') => break,
                KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => break,
                KeyCode::Char(' ') => timer.toggle_pause(),
                KeyCode::Char('r') => timer.reset(Instant::now()),
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
                eprintln!("pomodoro: {error}");
                std::process::exit(1);
            }
        }
        Ok(None) => println!("{}", usage()),
        Err(error) => {
            eprintln!("pomodoro: {error}\n\n{}", usage());
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_use_mode_default_and_explicit_minutes() {
        let break_config = parse_args(["--mode", "break", "--view=hourglass"].map(str::to_owned))
            .unwrap()
            .unwrap();
        assert_eq!(
            break_config,
            Config {
                mode: Mode::Break,
                view: View::Hourglass,
                minutes: 5
            }
        );
        let explicit = parse_args(["--minutes", "15", "--mode", "break"].map(str::to_owned))
            .unwrap()
            .unwrap();
        assert_eq!(explicit.minutes, 15);
        assert!(parse_args(["--minutes", "0"].map(str::to_owned)).is_err());
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
    fn both_views_change_as_time_passes() {
        let start = Instant::now();
        let mut timer = Timer::new(1, start);
        let mut initial_clock = Canvas::new(90, 30);
        let mut initial_hourglass = Canvas::new(90, 30);
        draw_clock(&mut initial_clock, 45, 15, 9, &timer);
        draw_hourglass(&mut initial_hourglass, 45, 5, 9, &timer);

        timer.tick(start + Duration::from_secs(30));
        let mut later_clock = Canvas::new(90, 30);
        let mut later_hourglass = Canvas::new(90, 30);
        draw_clock(&mut later_clock, 45, 15, 9, &timer);
        draw_hourglass(&mut later_hourglass, 45, 5, 9, &timer);

        assert_ne!(
            initial_clock
                .cells
                .iter()
                .map(|cell| cell.ch)
                .collect::<String>(),
            later_clock
                .cells
                .iter()
                .map(|cell| cell.ch)
                .collect::<String>()
        );
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
        let timer = Timer::new(25, Instant::now());
        for view in [View::Clock, View::Hourglass] {
            let config = Config {
                mode: Mode::Focus,
                view,
                minutes: 25,
            };
            let canvas = render(config, &timer, 24, 12);
            let rows: Vec<String> = canvas
                .cells
                .chunks(canvas.width)
                .map(|row| row.iter().map(|cell| cell.ch).collect())
                .collect();
            assert!(rows.iter().any(|row| row.contains("25:00")));
            assert!(rows.iter().any(|row| row.contains("spc")));
            let art = match view {
                View::Clock => '●',
                View::Hourglass => '╲',
            };
            assert!(rows.iter().any(|row| row.contains(art)));
        }
    }

    #[test]
    fn very_small_terminal_keeps_countdown_and_keys_separate() {
        let timer = Timer::new(25, Instant::now());
        let config = Config {
            mode: Mode::Focus,
            view: View::Clock,
            minutes: 25,
        };
        let canvas = render(config, &timer, 12, 2);
        let first: String = canvas.cells[..12].iter().map(|cell| cell.ch).collect();
        let second: String = canvas.cells[12..].iter().map(|cell| cell.ch).collect();
        assert!(first.contains("25:00"));
        assert!(second.contains("spc r q"));
    }
}
