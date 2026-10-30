//! Commands in the Tasks page's field (`poni_core::quick`): what Enter will
//! do, shown at the field's right while one is typed, and doing it. Plain
//! words stay a task. Alt+Shift+Space opens the field from any app.

use crate::commands::Command;
use crate::icons::Icon;
use crate::pages::PageKind;
use crate::state::AppState;
use poni_core::calendar::{self, Meeting};
use poni_core::cliphist::preview;
use poni_core::pomodoro::Phase;
use poni_core::quick::{self, Quick};

/// What Enter does with `q`, for the line beside the field.
pub fn line(st: &AppState, q: &Quick) -> String {
    match q {
        Quick::Join => match join_target(st) {
            Some(m) => format!("\u{21b5} Join {}", m.title),
            None => "No meeting to join".into(),
        },
        Quick::Focus(_) if st.pomodoro.is_active() => "\u{21b5} Open Focus".into(),
        Quick::Focus(min) => format!("\u{21b5} Focus {} min", min.unwrap_or(st.settings.pomodoro.focus_min)),
        Quick::Clip(words) => match clip(st, words) {
            Some(text) => format!("\u{21b5} Copy \u{201c}{}\u{201d}", preview(text, 28)),
            None => "Not in the clipboard history".into(),
        },
        Quick::Calc(Some(v)) => format!("= {}  \u{21b5} Copy", quick::fmt_number(*v)),
        Quick::Calc(None) => "= \u{2026}".into(),
        Quick::Section(_) if !st.sections_on() => "Sections are off (switch them on in +)".into(),
        Quick::Section(name) if name.trim().is_empty() => "\u{21b5} Every tab".into(),
        Quick::Section(name) => match st.settings.sections.find(name) {
            Some(i) => format!("\u{21b5} Section {}", st.settings.sections.list[i].name),
            None => "No section with that name".into(),
        },
        Quick::Record if st.recording.is_some() => "\u{21b5} Stop recording".into(),
        Quick::Record => "\u{21b5} Record the meeting's sound".into(),
        Quick::Note(_) if !st.notes_on() => "Notes are off (switch them on in +)".into(),
        Quick::Note(text) if text.trim().is_empty() => "Type the note after /note".into(),
        Quick::Note(_) => "\u{21b5} Save as a note".into(),
        Quick::Unknown => quick::HELP.into(),
    }
}

/// Do `q`. Returns true when it did something (the field then clears and
/// the island lets go of the keyboard).
pub fn run(st: &mut AppState, q: &Quick) -> bool {
    match q {
        Quick::Join => {
            let Some(link) = join_target(st).and_then(|m| m.link.clone()) else { return false };
            st.cmd(Command::OpenUrl(link));
        }
        Quick::Focus(_) if st.pomodoro.is_active() => st.cmd(Command::SetPage(PageKind::Focus)),
        Quick::Focus(min) => {
            // A number sets the focus length from now on, as on the Focus page.
            if let Some(m) = *min {
                st.settings.pomodoro.focus_min = m;
                st.pomodoro.cfg.focus_min = m;
                st.cmd(Command::SaveSettings);
            }
            if st.pomodoro.phase != Phase::Focus {
                st.pomodoro.set_phase(Phase::Focus);
            }
            st.pomodoro.start(st.now);
        }
        Quick::Clip(words) => {
            let Some(text) = clip(st, words).map(str::to_string) else { return false };
            st.notify(Icon::Copy, st.accent, "Copied", preview(&text, 40), 2.5, None);
            st.cmd(Command::CopyText(text));
        }
        Quick::Calc(Some(v)) => {
            let text = quick::fmt_number(*v);
            st.notify(Icon::Copy, st.accent, "Copied", text.clone(), 2.5, None);
            st.cmd(Command::CopyText(text));
        }
        Quick::Section(name) => {
            if !st.sections_on() {
                return false;
            }
            let target = if name.trim().is_empty() {
                None
            } else {
                let Some(i) = st.settings.sections.find(name) else { return false };
                Some(i)
            };
            st.cmd(Command::SetSection(target));
        }
        Quick::Record => st.cmd(Command::ToggleRecording),
        Quick::Note(text) => {
            if !st.notes_on() || text.trim().is_empty() {
                return false;
            }
            if st.store.notes.add(text.trim(), poni_core::notes::NoteColor::Yellow, st.unix).is_none() {
                return false;
            }
            st.save();
            st.notify(Icon::File, st.accent, "Note saved", poni_core::notes::title(text), 3.0, Some(crate::state::NoticeAction::OpenPage(PageKind::Notes)));
        }
        Quick::Calc(None) | Quick::Unknown => return false,
    }
    true
}

/// The meeting `/join` opens: the first of today's left (one under way
/// first) that has a call, while the Calendar add-on is on.
fn join_target(st: &AppState) -> Option<&Meeting> {
    if !st.calendar_on() {
        return None;
    }
    let offset = poni_core::oyenza::offset_minutes(&st.local, st.unix);
    calendar::today(&st.calendar.meetings, st.unix, offset).0.into_iter().find(|m| m.link.is_some())
}

fn clip<'a>(st: &'a AppState, words: &str) -> Option<&'a str> {
    quick::find_clip(st.clip.iter().map(|c| c.text.as_str()), words)
}
