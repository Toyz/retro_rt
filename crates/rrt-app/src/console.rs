//! A drop-down debug console: typed commands the game answers, drawn as an
//! overlay at the window's resolution.
//!
//! The console only collects lines: the game takes them each tick
//! ([`Console::take_commands`]), does what they ask with its own state, and
//! answers with [`Console::say`]. That way a command can touch anything the
//! game owns, with no callbacks borrowing it. `help` and `clear` are
//! answered here; [`Console::describe`] adds a command to `help`.
//!
//! Wiring, in the game:
//!
//! ```ignore
//! fn event(&mut self, e: &WindowEvent) -> bool { self.console.event(e) }
//! fn tick(&mut self, t: &mut Tick) {
//!     for line in self.console.take_commands() { /* match line, self.console.say(..) */ }
//!     if self.console.open { return; }           // the pad is the console's while open
//!     /* ... */
//! }
//! fn draw(&mut self, d: &mut Draw<'_>) {
//!     let overlay = self.console.overlay(d.width, d.height);
//!     d.present_picture(&self.picture, overlay.as_ref());
//! }
//! ```

use std::collections::{BTreeMap, VecDeque};

use rrt_image::{Picture, font};
use winit::event::{ElementState, WindowEvent};
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};

/// A key as the console handles it: what [`Console::event`] makes of a
/// window event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsoleKey {
    /// A typed character.
    Char(char),
    /// Run the line.
    Enter,
    /// Delete the last character.
    Backspace,
    /// The previous line run.
    Up,
    /// The next line run.
    Down,
    /// Close.
    Escape,
    /// Open or close.
    Toggle,
}

/// The console's state: open or closed, its scrollback, the line being
/// typed, the history of lines run.
#[derive(Clone, Debug)]
pub struct Console {
    /// Shown and taking keys.
    pub open: bool,
    /// The physical keys that open and close it: F1 and the key left of 1
    /// (backquote) by default.
    pub toggle: Vec<KeyCode>,
    /// Font pixels per glyph pixel when drawn.
    pub scale: u32,
    /// Lines of scrollback kept.
    pub keep: usize,
    lines: VecDeque<String>,
    input: String,
    history: Vec<String>,
    browsing: Option<usize>,
    pending: Vec<String>,
    help: BTreeMap<String, String>,
}

impl Default for Console {
    fn default() -> Self {
        Console::new()
    }
}

impl Console {
    /// A closed console: F1 or backquote toggles it, drawn at scale 2,
    /// keeping 200 lines.
    pub fn new() -> Console {
        Console {
            open: false,
            toggle: vec![KeyCode::F1, KeyCode::Backquote],
            scale: 2,
            keep: 200,
            lines: VecDeque::new(),
            input: String::new(),
            history: Vec::new(),
            browsing: None,
            pending: Vec::new(),
            help: BTreeMap::new(),
        }
    }

    /// Lists `name` under `help` with `about`.
    pub fn describe(&mut self, name: impl Into<String>, about: impl Into<String>) {
        self.help.insert(name.into(), about.into());
    }

    /// Prints `line` (or several, split at newlines) to the scrollback.
    pub fn say(&mut self, text: impl AsRef<str>) {
        for line in text.as_ref().lines() {
            self.lines.push_back(line.to_string());
        }
        while self.lines.len() > self.keep {
            self.lines.pop_front();
        }
    }

    /// The scrollback, oldest first.
    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().map(String::as_str)
    }

    /// The line being typed.
    pub fn input(&self) -> &str {
        &self.input
    }

    /// The commands run since the last call, oldest first, for the game to
    /// answer. `help` and `clear` never appear: the console answers them.
    pub fn take_commands(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending)
    }

    /// Runs `line` as though typed: echoed, kept in history, answered here
    /// if it is `help` or `clear`, else queued for the game. For scripted
    /// and replayed commands too.
    pub fn run(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        self.say(format!("> {line}"));
        if self.history.last().map(String::as_str) != Some(line) {
            self.history.push(line.to_string());
        }
        match line {
            "help" => {
                let mut text = String::from("help - this list\nclear - empty the console");
                for (name, about) in &self.help {
                    text.push_str(&format!("\n{name} - {about}"));
                }
                self.say(text);
            }
            "clear" => self.lines.clear(),
            _ => self.pending.push(line.to_string()),
        }
    }

    /// Handles one key. Returns whether the console took it: always while
    /// open, only the toggle while closed.
    pub fn key(&mut self, key: ConsoleKey) -> bool {
        if key == ConsoleKey::Toggle {
            self.open = !self.open;
            return true;
        }
        if !self.open {
            return false;
        }
        match key {
            ConsoleKey::Char(c) if !c.is_control() => self.input.push(c),
            ConsoleKey::Char(_) => {}
            ConsoleKey::Backspace => {
                self.input.pop();
            }
            ConsoleKey::Enter => {
                let line = std::mem::take(&mut self.input);
                self.browsing = None;
                self.run(&line);
            }
            ConsoleKey::Up if !self.history.is_empty() => {
                let i = self.browsing.map_or(self.history.len() - 1, |i| i.saturating_sub(1));
                self.browsing = Some(i);
                self.input = self.history[i].clone();
            }
            ConsoleKey::Down => match self.browsing {
                Some(i) if i + 1 < self.history.len() => {
                    self.browsing = Some(i + 1);
                    self.input = self.history[i + 1].clone();
                }
                _ => {
                    self.browsing = None;
                    self.input.clear();
                }
            },
            ConsoleKey::Up => {}
            ConsoleKey::Escape => self.open = false,
            ConsoleKey::Toggle => unreachable!(),
        }
        true
    }

    /// Handles a window event, for [`crate::Game::event`]: returns whether
    /// the console took it (so the pad does not see keys typed into it).
    pub fn event(&mut self, event: &WindowEvent) -> bool {
        let WindowEvent::KeyboardInput { event, .. } = event else { return false };
        if event.state != ElementState::Pressed {
            return self.open;
        }
        if let PhysicalKey::Code(code) = event.physical_key
            && self.toggle.contains(&code)
        {
            return self.key(ConsoleKey::Toggle);
        }
        let key = match &event.logical_key {
            Key::Named(NamedKey::Enter) => ConsoleKey::Enter,
            Key::Named(NamedKey::Backspace) => ConsoleKey::Backspace,
            Key::Named(NamedKey::ArrowUp) => ConsoleKey::Up,
            Key::Named(NamedKey::ArrowDown) => ConsoleKey::Down,
            Key::Named(NamedKey::Escape) => ConsoleKey::Escape,
            _ => match event.text.as_ref().and_then(|t| t.chars().next()) {
                Some(c) => ConsoleKey::Char(c),
                None => return self.open,
            },
        };
        self.key(key)
    }

    /// The console drawn for a `width` x `height` window: a translucent
    /// panel over the top half with the scrollback's latest lines and the
    /// prompt, for [`crate::Draw::present_picture`]'s overlay. None while
    /// closed.
    pub fn overlay(&self, width: u32, height: u32) -> Option<Picture> {
        if !self.open || width == 0 || height == 0 {
            return None;
        }
        let s = self.scale.max(1);
        let line_h = font::HEIGHT * s;
        let panel_h = (height / 2).max(line_h * 2);
        let mut p = Picture::filled(width, panel_h, [16, 16, 24, 210]);
        p.fill_rect(0, panel_h - s, width, s, [120, 120, 160, 255]);
        let rows = (panel_h / line_h).saturating_sub(1) as usize;
        let shown: Vec<&String> = self.lines.iter().rev().take(rows).collect();
        for (k, line) in shown.iter().rev().enumerate() {
            font::draw(&mut p, 4 * s, k as u32 * line_h, line, [220, 220, 220, 255], s);
        }
        let prompt_y = panel_h - line_h - s;
        let end = font::draw(&mut p, 4 * s, prompt_y, &format!("] {}", self.input), [255, 255, 160, 255], s);
        p.fill_rect(end, prompt_y + 2 * s, font::WIDTH * s, line_h - 4 * s, [255, 255, 160, 255]);
        Some(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(c: &mut Console, text: &str) {
        for ch in text.chars() {
            c.key(ConsoleKey::Char(ch));
        }
        c.key(ConsoleKey::Enter);
    }

    #[test]
    fn closed_it_takes_only_the_toggle() {
        let mut c = Console::new();
        assert!(!c.key(ConsoleKey::Char('a')));
        assert!(c.key(ConsoleKey::Toggle));
        assert!(c.open && c.key(ConsoleKey::Char('a')));
        assert_eq!(c.input(), "a");
        c.key(ConsoleKey::Escape);
        assert!(!c.open);
    }

    #[test]
    fn typed_lines_go_to_the_game_and_help_and_clear_are_answered() {
        let mut c = Console::new();
        c.open = true;
        c.describe("rumble", "on or off");
        typed(&mut c, "rumble on");
        typed(&mut c, "help");
        assert_eq!(c.take_commands(), ["rumble on"]);
        assert!(c.take_commands().is_empty(), "taken once");
        let text: Vec<&str> = c.lines().collect();
        assert!(text.contains(&"> rumble on") && text.contains(&"rumble - on or off"));
        typed(&mut c, "clear");
        assert_eq!(c.lines().count(), 0);
        c.key(ConsoleKey::Char('x'));
        c.key(ConsoleKey::Backspace);
        c.key(ConsoleKey::Char('\u{8}'));
        assert_eq!(c.input(), "", "backspace deletes, control characters are not typed");
    }

    #[test]
    fn up_and_down_walk_the_history() {
        let mut c = Console::new();
        c.open = true;
        typed(&mut c, "one");
        typed(&mut c, "two");
        typed(&mut c, "two");
        c.key(ConsoleKey::Up);
        assert_eq!(c.input(), "two", "a repeated line is kept once");
        c.key(ConsoleKey::Up);
        assert_eq!(c.input(), "one");
        c.key(ConsoleKey::Up);
        assert_eq!(c.input(), "one", "stops at the oldest");
        c.key(ConsoleKey::Down);
        assert_eq!(c.input(), "two");
        c.key(ConsoleKey::Down);
        assert_eq!(c.input(), "");
    }

    #[test]
    fn the_scrollback_keeps_its_limit() {
        let mut c = Console::new();
        c.keep = 3;
        c.say("a\nb\nc\nd");
        assert_eq!(c.lines().collect::<Vec<_>>(), ["b", "c", "d"]);
    }

    #[test]
    fn the_overlay_is_drawn_only_while_open() {
        let mut c = Console::new();
        assert!(c.overlay(640, 480).is_none());
        c.open = true;
        c.say("hello");
        let p = c.overlay(640, 480).unwrap();
        assert_eq!((p.width, p.height), (640, 240));
        assert_eq!(p.get(600, 100), Some([16, 16, 24, 210]), "the translucent panel");
        assert_eq!(p.get(10, 239), Some([120, 120, 160, 255]), "its bottom edge");
        let lit =
            (0..80).flat_map(|x| (0..26).map(move |y| (x, y))).any(|(x, y)| p.get(x, y) == Some([220, 220, 220, 255]));
        assert!(lit, "the line is drawn");
    }
}
