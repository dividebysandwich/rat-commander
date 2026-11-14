//! Repair escape sequences that reached crossterm in two pieces.
//!
//! crossterm's Unix parser decides what a lone `ESC` byte means by whether more
//! bytes came with it in the same read. When a terminal's report is split
//! right after its `ESC` — common over SSH, and constant with some terminals
//! (MobaXterm to an illumos host, issue #26) — the `ESC` becomes an Esc key
//! press and the rest of the sequence arrives as typed characters: a mouse
//! move turns into Esc (closing whatever dialog is open) followed by
//! `[<35;60;26M` landing in the command line or the quick search.
//!
//! [`SeqMender`] sits between the event stream and the handlers. A bare Esc is
//! held for a moment; if `[` or `O` follows, the characters after it are
//! collected until the sequence ends and decoded into the event the terminal
//! meant. Anything that does not continue as an escape sequence releases the
//! held events unchanged, so a real Esc only arrives [`HOLD`] later.

use std::time::{Duration, Instant};

use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

/// How long a bare Esc waits for the rest of a split sequence. Both pieces
/// were written by the terminal together, so they arrive close behind each
/// other; a person typing Esc and then `[` is far slower than this.
pub const HOLD: Duration = Duration::from_millis(60);

/// The longest sequence body collected before giving up (an SGR mouse report
/// on a huge terminal is about 16 bytes).
const MAX_BODY: usize = 32;

#[derive(Default)]
pub struct SeqMender {
    held: Option<Held>,
}

struct Held {
    since: Instant,
    /// The events withheld so far: the Esc, then the characters after it.
    events: Vec<Event>,
    /// The characters after `ESC`, starting with the introducer `[` or `O`.
    body: String,
}

impl SeqMender {
    /// Take one event from the stream; returns what should be handled now.
    pub fn push(&mut self, ev: Event, now: Instant) -> Vec<Event> {
        let Some(mut held) = self.held.take() else {
            if is_bare_esc(&ev) {
                self.held = Some(Held { since: now, events: vec![ev], body: String::new() });
                return Vec::new();
            }
            return vec![ev];
        };
        let Some(c) = typed_char(&ev) else {
            // Not part of a sequence: let go of what was held and treat this
            // event afresh (it may itself be the Esc of the next sequence).
            let mut out = held.events;
            out.extend(self.push(ev, now));
            return out;
        };
        held.body.push(c);
        held.events.push(ev);
        match step(&held.body) {
            Step::More if held.body.len() < MAX_BODY => {
                self.held = Some(held);
                Vec::new()
            }
            Step::Done(Some(ev)) => vec![ev],
            // A complete sequence this does not decode: it is still terminal
            // chatter, not typing, so it is dropped rather than inserted.
            Step::Done(None) => Vec::new(),
            Step::More | Step::NotASequence => held.events,
        }
    }

    /// Release a held Esc (and anything after it) whose window has run out.
    pub fn expire(&mut self, now: Instant) -> Vec<Event> {
        match &self.held {
            Some(h) if now.duration_since(h.since) >= HOLD => self.held.take().unwrap().events,
            _ => Vec::new(),
        }
    }

    /// When [`expire`](Self::expire) next has something to release.
    pub fn deadline(&self) -> Option<Instant> {
        self.held.as_ref().map(|h| h.since + HOLD)
    }
}

fn is_bare_esc(ev: &Event) -> bool {
    matches!(ev, Event::Key(k) if k.code == KeyCode::Esc
        && k.modifiers.is_empty()
        && k.kind == KeyEventKind::Press)
}

/// The character a plain key press typed (Shift allowed: crossterm reports an
/// upper-case letter with it).
fn typed_char(ev: &Event) -> Option<char> {
    match ev {
        Event::Key(KeyEvent {
            code: KeyCode::Char(c),
            modifiers,
            kind: KeyEventKind::Press,
            ..
        }) if (*modifiers - KeyModifiers::SHIFT).is_empty() && c.is_ascii() => Some(*c),
        _ => None,
    }
}

enum Step {
    /// Still a valid prefix of an escape sequence.
    More,
    /// The sequence is complete; the event it stands for, if understood.
    Done(Option<Event>),
    /// What followed the Esc is not an escape sequence.
    NotASequence,
}

/// Where `body` (everything after `ESC`) stands as an escape sequence.
fn step(body: &str) -> Step {
    let mut chars = body.chars();
    match chars.next() {
        Some('[') => {
            let rest = chars.as_str();
            let Some(last) = rest.chars().last() else { return Step::More };
            match last {
                // Parameter and intermediate bytes.
                '\x20'..='\x3f' => Step::More,
                // The final byte.
                '\x40'..='\x7e' => Step::Done(decode_csi(&rest[..rest.len() - 1], last)),
                _ => Step::NotASequence,
            }
        }
        Some('O') => match chars.next() {
            None => Step::More,
            Some(c) => match ss3_key(c) {
                Some(code) => Step::Done(Some(key(code, KeyModifiers::NONE))),
                None => Step::NotASequence,
            },
        },
        _ => Step::NotASequence,
    }
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

fn ss3_key(c: char) -> Option<KeyCode> {
    Some(match c {
        'A' => KeyCode::Up,
        'B' => KeyCode::Down,
        'C' => KeyCode::Right,
        'D' => KeyCode::Left,
        'H' => KeyCode::Home,
        'F' => KeyCode::End,
        'P'..='S' => KeyCode::F(1 + c as u8 - b'P'),
        _ => return None,
    })
}

/// Decode a CSI sequence: its parameters and final character.
fn decode_csi(params: &str, last: char) -> Option<Event> {
    if let Some(sgr) = params.strip_prefix('<') {
        return sgr_mouse(sgr, last == 'm');
    }
    let nums: Vec<u16> = if params.is_empty() {
        Vec::new()
    } else {
        params.split(';').map(|n| n.parse().ok()).collect::<Option<_>>()?
    };
    // `CSI 1;5C`: the second parameter carries the modifiers.
    let mods = modifiers(nums.get(1).copied().unwrap_or(1));
    let code = match last {
        'A' | 'B' | 'C' | 'D' | 'H' | 'F' | 'P' | 'Q' | 'S' => ss3_key(last)?,
        'Z' => return Some(key(KeyCode::BackTab, KeyModifiers::SHIFT)),
        'I' => return Some(Event::FocusGained),
        'O' => return Some(Event::FocusLost),
        '~' => match nums.first()? {
            1 | 7 => KeyCode::Home,
            2 => KeyCode::Insert,
            3 => KeyCode::Delete,
            4 | 8 => KeyCode::End,
            5 => KeyCode::PageUp,
            6 => KeyCode::PageDown,
            n @ 11..=15 => KeyCode::F((n - 10) as u8),
            n @ 17..=21 => KeyCode::F((n - 11) as u8),
            n @ 23..=24 => KeyCode::F((n - 12) as u8),
            _ => return None,
        },
        _ => return None,
    };
    Some(key(code, mods))
}

/// xterm's modifier parameter: 1 + a bit mask of Shift, Alt, Ctrl.
fn modifiers(param: u16) -> KeyModifiers {
    let bits = param.saturating_sub(1);
    let mut m = KeyModifiers::NONE;
    if bits & 1 != 0 {
        m |= KeyModifiers::SHIFT;
    }
    if bits & 2 != 0 {
        m |= KeyModifiers::ALT;
    }
    if bits & 4 != 0 {
        m |= KeyModifiers::CONTROL;
    }
    m
}

/// An SGR mouse report, `CSI < Cb ; Cx ; Cy M` (or `m` for a release).
fn sgr_mouse(params: &str, release: bool) -> Option<Event> {
    let mut it = params.split(';').map(|n| n.parse::<u16>().ok());
    let (cb, cx, cy) = (it.next()??, it.next()??, it.next()??);
    let button = (cb & 0b11) | ((cb & 0b1100_0000) >> 4);
    let dragging = cb & 0b10_0000 != 0;
    let kind = match (button, dragging) {
        (0, false) => MouseEventKind::Down(MouseButton::Left),
        (1, false) => MouseEventKind::Down(MouseButton::Middle),
        (2, false) => MouseEventKind::Down(MouseButton::Right),
        (0, true) => MouseEventKind::Drag(MouseButton::Left),
        (1, true) => MouseEventKind::Drag(MouseButton::Middle),
        (2, true) => MouseEventKind::Drag(MouseButton::Right),
        (3, false) => MouseEventKind::Up(MouseButton::Left),
        (3..=5, true) => MouseEventKind::Moved,
        (4, false) => MouseEventKind::ScrollUp,
        (5, false) => MouseEventKind::ScrollDown,
        (6, false) => MouseEventKind::ScrollLeft,
        (7, false) => MouseEventKind::ScrollRight,
        _ => return None,
    };
    let kind = match kind {
        MouseEventKind::Down(b) if release => MouseEventKind::Up(b),
        k => k,
    };
    let mut modifiers = KeyModifiers::NONE;
    if cb & 0b100 != 0 {
        modifiers |= KeyModifiers::SHIFT;
    }
    if cb & 0b1000 != 0 {
        modifiers |= KeyModifiers::ALT;
    }
    if cb & 0b1_0000 != 0 {
        modifiers |= KeyModifiers::CONTROL;
    }
    Some(Event::Mouse(MouseEvent {
        kind,
        column: cx.checked_sub(1)?,
        row: cy.checked_sub(1)?,
        modifiers,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(c: char) -> Event {
        let mods = if c.is_ascii_uppercase() { KeyModifiers::SHIFT } else { KeyModifiers::NONE };
        key(KeyCode::Char(c), mods)
    }

    fn esc() -> Event {
        key(KeyCode::Esc, KeyModifiers::NONE)
    }

    /// Feed a lone Esc, then `rest` as typed characters; collect the output.
    fn split(rest: &str) -> Vec<Event> {
        let mut m = SeqMender::default();
        let now = Instant::now();
        let mut out = m.push(esc(), now);
        for c in rest.chars() {
            out.extend(m.push(ch(c), now));
        }
        out
    }

    #[test]
    fn a_split_mouse_move_becomes_a_mouse_event() {
        let out = split("[<35;60;26M");
        assert_eq!(
            out,
            vec![Event::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: 59,
                row: 25,
                modifiers: KeyModifiers::NONE,
            })]
        );
    }

    #[test]
    fn split_wheel_and_release_decode() {
        let Event::Mouse(m) = split("[<65;3;4M")[0] else { panic!() };
        assert_eq!(m.kind, MouseEventKind::ScrollDown);
        let Event::Mouse(m) = split("[<0;3;4m")[0] else { panic!() };
        assert_eq!(m.kind, MouseEventKind::Up(MouseButton::Left));
    }

    #[test]
    fn split_keys_decode() {
        assert_eq!(split("[A"), vec![key(KeyCode::Up, KeyModifiers::NONE)]);
        assert_eq!(split("[1;5C"), vec![key(KeyCode::Right, KeyModifiers::CONTROL)]);
        assert_eq!(split("[3~"), vec![key(KeyCode::Delete, KeyModifiers::NONE)]);
        assert_eq!(split("[21~"), vec![key(KeyCode::F(10), KeyModifiers::NONE)]);
        assert_eq!(split("OQ"), vec![key(KeyCode::F(2), KeyModifiers::NONE)]);
    }

    #[test]
    fn an_unknown_complete_sequence_is_dropped_not_typed() {
        assert!(split("[?1;2c").is_empty());
    }

    #[test]
    fn esc_then_a_normal_key_passes_both_through() {
        assert_eq!(split("1"), vec![esc(), ch('1')]);
        assert_eq!(split("[\u{e9}").len(), 3, "a non-ASCII char ends the sequence");
    }

    #[test]
    fn a_held_esc_is_released_when_its_window_runs_out() {
        let mut m = SeqMender::default();
        let t = Instant::now();
        assert!(m.push(esc(), t).is_empty());
        assert!(m.expire(t).is_empty());
        assert_eq!(m.deadline(), Some(t + HOLD));
        assert_eq!(m.expire(t + HOLD), vec![esc()]);
        assert_eq!(m.deadline(), None);
    }

    #[test]
    fn a_second_esc_releases_the_first_and_is_held_itself() {
        let mut m = SeqMender::default();
        let t = Instant::now();
        m.push(esc(), t);
        assert_eq!(m.push(esc(), t), vec![esc()]);
        assert!(m.deadline().is_some());
    }

    #[test]
    fn other_events_pass_straight_through() {
        let mut m = SeqMender::default();
        assert_eq!(m.push(ch('a'), Instant::now()), vec![ch('a')]);
    }
}
