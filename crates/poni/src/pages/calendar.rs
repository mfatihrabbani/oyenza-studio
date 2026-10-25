//! Calendar tab, while the Calendar add-on is on. Connect pastes the
//! calendar's secret iCal link (Google Calendar's "Secret address in iCal
//! format"); after that, the rest of today's meetings, one row each: its time,
//! its title and Join for its call (bright for the one under way or about to
//! start). Under them, the first meeting after today when none are left.
//! The footer picks how long before a meeting its notice shows, and checks
//! for new meetings or removes the link.
//!
//! The meetings come from `services::calendar` through the app; nothing here
//! runs for them, and the page redraws with the app's minute tick.

use crate::commands::{CalendarCmd, Command};
use crate::gfx::{TextStyle, Weight};
use crate::icons::Icon;
use crate::state::AppState;
use crate::ui::{wid, wid_n, PillStyle, Ui, PILL_GAP, PILL_SMALL};
use poni_core::calendar::{self as model, Meeting};
use poni_core::geom::{palette, Rect};

const HEIGHT: f32 = 146.0;
/// The meetings' rows, then the footer under them.
const ROW_H: f32 = 28.0;
const ROWS: usize = 4;
const FOOT_Y: f32 = 118.0;
const TIME_W: f32 = 118.0;
/// Join is bright from this long before a meeting until it ends.
const SOON_S: i64 = 10 * 60;
const REMIND: [(u32, &str); 3] = [(1, "1 min"), (5, "5 min"), (10, "10 min")];

pub fn height(_st: &AppState) -> f32 {
    HEIGHT
}

pub fn draw(ui: &mut Ui, st: &mut AppState, area: Rect) {
    let area = Rect::new(area.x, area.y, area.w, HEIGHT);
    if !st.calendar.linked {
        let hint = note(st).unwrap_or("Copy its secret address in iCal format, then Connect");
        connect(ui, st, area, "Connect your Google Calendar", hint);
        return;
    }
    if st.calendar.fetched_at.is_none() {
        let problem = st.calendar.problem;
        match problem {
            Some(problem) => connect(ui, st, area, problem, "Check the link, or copy it again and Connect"),
            None => super::empty_state(ui, Rect::new(area.x, area.y, area.w, FOOT_Y), Icon::Video, "Reading your calendar\u{2026}", "One moment"),
        }
        footer(ui, st, area);
        return;
    }

    let now = st.unix;
    let offset = poni_core::oyenza::offset_minutes(&st.local, now);
    let use_24h = st.settings.use_24h;
    let (rest, next) = model::today(&st.calendar.meetings, now, offset);
    let rows = Rect::new(area.x, area.y, area.w, FOOT_Y);
    if rest.is_empty() {
        let hint = match next {
            Some(m) => format!("Next: {} \u{b7} {}", model::later_label(m, now, offset, use_24h), m.title),
            None => "Nothing in the next day and a half".to_string(),
        };
        super::empty_state(ui, rows, Icon::Video, "No more meetings today", &hint);
    } else {
        // A fourth row only when it's the last one; else "and N more".
        let shown = if rest.len() > ROWS { ROWS - 1 } else { rest.len() };
        let mut joined = None;
        for (i, m) in rest.iter().take(shown).enumerate() {
            let r = Rect::new(area.x, area.y + i as f32 * ROW_H, area.w, ROW_H);
            if row(ui, st, r, i, m, now, offset, use_24h) {
                joined = m.link.clone();
            }
        }
        if rest.len() > shown {
            let more = format!("and {} more today", rest.len() - shown);
            let r = Rect::new(area.x, area.y + shown as f32 * ROW_H, area.w, ROW_H);
            ui.c.text(&more, r, &TextStyle::footnote().color(palette::TEXT_DIM));
        }
        if let Some(link) = joined {
            st.cmd(Command::OpenUrl(link));
        }
    }
    footer(ui, st, area);
}

/// One meeting: its time, its title, and Join when it has a call. Returns
/// true when Join was clicked.
#[allow(clippy::too_many_arguments)]
fn row(ui: &mut Ui, st: &AppState, r: Rect, i: usize, m: &Meeting, now: i64, offset: i32, use_24h: bool) -> bool {
    let live = model::live(m, now);
    let soon = live || (m.start > now && m.start - now <= SOON_S);
    let accent = st.accent;
    let time_style = TextStyle::footnote().weight(Weight::Semibold).tabular();
    let (time, time_style) = if live { ("Now".to_string(), time_style.color(accent)) } else { (model::span(m, offset, use_24h), time_style.color(palette::TEXT_DIM)) };
    ui.c.text(&time, Rect::new(r.x, r.y, TIME_W, r.h), &time_style);

    let mut title_right = r.right();
    let mut clicked = false;
    if m.link.is_some() {
        let w = ui.pill_width("Join", true, true);
        let pill = Rect::new(r.right() - w, r.center().y - PILL_SMALL / 2.0, w, PILL_SMALL - 2.0);
        let style = if soon { PillStyle::Primary(accent) } else { PillStyle::Utility };
        clicked = ui.pill(wid_n("calendar.join", i as u64), pill, "Join", Some(Icon::Open), style).clicked;
        title_right = pill.x - 12.0;
    }
    let tx = r.x + TIME_W;
    ui.c.text(&m.title, Rect::new(tx, r.y, (title_right - tx).max(0.0), r.h), &TextStyle::body().weight(Weight::Semibold));
    clicked
}

/// Nothing to list yet: a line about it, then Connect and where to find the link.
fn connect(ui: &mut Ui, st: &mut AppState, area: Rect, title: &str, hint: &str) {
    super::empty_state(ui, Rect::new(area.x, area.y, area.w, FOOT_Y), Icon::Video, title, hint);
    if st.calendar.linked {
        return;
    }
    let y = area.y + FOOT_Y + (HEIGHT - FOOT_Y - PILL_SMALL) / 2.0;
    let (cw, hw) = (ui.pill_width("Connect", true, true), ui.pill_width("How to get the link", true, true));
    let x = area.center().x - (cw + PILL_GAP + hw) / 2.0;
    if ui.pill(wid("calendar.connect"), Rect::new(x, y, cw, PILL_SMALL), "Connect", Some(Icon::Plus), PillStyle::Primary(st.accent)).clicked {
        st.cmd(Command::Calendar(CalendarCmd::UseLink));
    }
    if ui.pill(wid("calendar.help"), Rect::new(x + cw + PILL_GAP, y, hw, PILL_SMALL), "How to get the link", Some(Icon::Open), PillStyle::Utility).clicked {
        st.cmd(Command::OpenUrl(model::HELP_URL.into()));
    }
}

/// Remind me (1, 5 or 10 minutes before) on the left; Check now and Remove
/// on the right.
fn footer(ui: &mut Ui, st: &mut AppState, area: Rect) {
    let fy = area.y + FOOT_Y;
    let label = "Remind me";
    let label_style = TextStyle::footnote().color(palette::TEXT_DIM);
    let lw = ui.c.measure(label, &label_style, 200.0).0.ceil();
    ui.c.text(label, Rect::new(area.x, fy, lw + 2.0, HEIGHT - FOOT_Y), &label_style);
    let names = REMIND.map(|r| r.1);
    let sel = REMIND.iter().position(|r| r.0 == st.settings.meeting_remind_min).unwrap_or(1);
    if let Some(i) = super::settings::picker(ui, "calendar.remind", area.x + lw + 10.0, fy + 2.0, 24.0, &names, sel) {
        st.settings.meeting_remind_min = REMIND[i].0;
        st.cmd(Command::SaveSettings);
    }

    let y = fy + (HEIGHT - FOOT_Y - PILL_SMALL) / 2.0;
    let rw = ui.pill_width("Remove", false, true);
    let remove = Rect::new(area.right() - rw, y, rw, PILL_SMALL);
    if ui.pill(wid("calendar.remove"), remove, "Remove", None, PillStyle::Utility).clicked {
        st.cmd(Command::Calendar(CalendarCmd::Forget));
    }
    let check = if st.calendar.fetching { "Checking\u{2026}" } else { "Check now" };
    let kw = ui.pill_width(check, true, true);
    if ui.pill(wid("calendar.check"), Rect::new(remove.x - PILL_GAP - kw, y, kw, PILL_SMALL), check, Some(Icon::Reset), PillStyle::Utility).clicked && !st.calendar.fetching {
        st.cmd(Command::Calendar(CalendarCmd::Refresh));
    }
}

/// What Connect last said, while it shows.
fn note(st: &AppState) -> Option<&'static str> {
    st.calendar.note.filter(|(_, until)| st.now < *until).map(|(text, _)| text)
}
