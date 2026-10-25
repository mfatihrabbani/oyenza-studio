//! The Today page's meetings, while the Calendar add-on is on: they take the
//! stat tiles' place, beside the date and the month. Connect pastes the
//! calendar's secret iCal link (Google Calendar's "Secret address in iCal
//! format"); after that, the rest of today's meetings, one row each: its
//! start, its title and Join for its call (bright for the one under way or
//! about to start), or the first meeting after today when none are left.
//! The refresh button checks for new meetings now; under the list, Remind
//! picks how long before a meeting its notice shows, and Remove deletes the
//! link.
//!
//! The meetings come from `services::calendar` through the app; nothing here
//! runs for them, and the page redraws with the app's minute tick.

use crate::commands::{CalendarCmd, Command};
use crate::gfx::{TextStyle, Weight};
use crate::icons::Icon;
use crate::state::AppState;
use crate::ui::{wid, wid_n, PillStyle, Ui, PILL_GAP};
use poni_core::calendar::{self as model, Meeting};
use poni_core::geom::{palette, Point, Rect};

const RADIUS: f32 = 16.0;
const PAD_X: f32 = 12.0;
const PAD_Y: f32 = 8.0;
/// The title line, the meetings' rows, then the footer.
const HEAD_H: f32 = 20.0;
const ROW_H: f32 = 26.0;
const ROWS: usize = 4;
const FOOT_H: f32 = 26.0;
const PILL_H: f32 = 24.0;
const TIME_W: f32 = 48.0;
/// Join is bright from this long before a meeting until it ends.
const SOON_S: i64 = 10 * 60;
const REMIND: [(u32, &str); 3] = [(1, "1 min"), (5, "5 min"), (10, "10 min")];

/// The meetings in `r` (the tiles' place). `card`: the Pill theme's card,
/// whose corner the box shares.
pub fn draw_panel(ui: &mut Ui, st: &mut AppState, r: Rect, card: Option<(Rect, f32)>) {
    if r.w < 120.0 {
        return;
    }
    super::clipboard::fill_box(ui, card, r, RADIUS, palette::SURFACE);
    let inner = r.inset(PAD_X, PAD_Y);
    if !st.calendar.linked {
        let hint = note(st).unwrap_or("Copy its secret address in iCal format");
        message(ui, inner, "Connect Google Calendar", hint);
        connect(ui, st, inner);
        return;
    }
    let foot = Rect::new(inner.x, inner.bottom() - FOOT_H, inner.w, FOOT_H);
    let body = Rect::new(inner.x, inner.y, inner.w, foot.y - inner.y);
    if st.calendar.fetched_at.is_none() {
        let problem = st.calendar.problem;
        match problem {
            Some(problem) => message(ui, body, problem, "Check the link, or Remove and connect again"),
            None => message(ui, body, "Reading your calendar\u{2026}", "One moment"),
        }
        footer(ui, st, foot);
        return;
    }

    let now = st.unix;
    let offset = poni_core::oyenza::offset_minutes(&st.local, now);
    let use_24h = st.settings.use_24h;
    let (rest, next) = model::today(&st.calendar.meetings, now, offset);
    head(ui, st, Rect::new(inner.x, inner.y, inner.w, HEAD_H));
    let rows = Rect::new(inner.x, inner.y + HEAD_H, inner.w, foot.y - inner.y - HEAD_H);
    if rest.is_empty() {
        let hint = match next {
            Some(m) => format!("Next: {} \u{b7} {}", model::later_label(m, now, offset, use_24h), m.title),
            None => "Nothing in the next day and a half".to_string(),
        };
        message(ui, rows, "No more meetings today", &hint);
    } else {
        // A fourth row only when it's the last one; else "and N more".
        let shown = if rest.len() > ROWS { ROWS - 1 } else { rest.len() };
        let mut joined = None;
        for (i, m) in rest.iter().take(shown).enumerate() {
            let row_r = Rect::new(rows.x, rows.y + i as f32 * ROW_H, rows.w, ROW_H);
            if row(ui, st, row_r, i, m, now, offset, use_24h) {
                joined = m.link.clone();
            }
        }
        if rest.len() > shown {
            let more = format!("and {} more today", rest.len() - shown);
            let r = Rect::new(rows.x + TIME_W, rows.y + shown as f32 * ROW_H, rows.w - TIME_W, ROW_H);
            ui.c.text(&more, r, &TextStyle::footnote().color(palette::TEXT_DIM));
        }
        if let Some(link) = joined {
            st.cmd(Command::OpenUrl(link));
        }
    }
    footer(ui, st, foot);
}

/// "MEETINGS" on the left, the refresh button on the right.
fn head(ui: &mut Ui, st: &mut AppState, r: Rect) {
    ui.c.text("MEETINGS", Rect::new(r.x, r.y, r.w - 24.0, r.h), &TextStyle::caption().weight(Weight::Bold).color(st.accent));
    let b = Rect::new(r.right() - 20.0, r.y, 20.0, r.h);
    let color = if st.calendar.fetching { palette::TEXT_FAINT } else { palette::TEXT_DIM };
    if ui.icon_button(wid("calendar.check"), b, Icon::Reset, color, 13.0).clicked && !st.calendar.fetching {
        st.cmd(Command::Calendar(CalendarCmd::Refresh));
    }
}

/// One meeting: when it starts, its title, and Join when it has a call.
/// Returns true when Join was clicked.
#[allow(clippy::too_many_arguments)]
fn row(ui: &mut Ui, st: &AppState, r: Rect, i: usize, m: &Meeting, now: i64, offset: i32, use_24h: bool) -> bool {
    let live = model::live(m, now);
    let soon = live || (m.start > now && m.start - now <= SOON_S);
    let accent = st.accent;
    let time_style = TextStyle::footnote().weight(Weight::Semibold).tabular();
    let (time, time_style) = if live { ("Now".to_string(), time_style.color(accent)) } else { (model::local_clock(m.start, offset, use_24h), time_style.color(palette::TEXT_DIM)) };
    ui.c.text(&time, Rect::new(r.x, r.y, TIME_W, r.h), &time_style);

    let mut title_right = r.right();
    let mut clicked = false;
    if m.link.is_some() {
        let w = ui.pill_width("Join", false, true);
        let pill = Rect::new(r.right() - w, r.center().y - PILL_H / 2.0, w, PILL_H);
        let style = if soon { PillStyle::Primary(accent) } else { PillStyle::Utility };
        clicked = ui.pill(wid_n("calendar.join", i as u64), pill, "Join", None, style).clicked;
        title_right = pill.x - 8.0;
    }
    let tx = r.x + TIME_W;
    ui.c.text(&m.title, Rect::new(tx, r.y, (title_right - tx).max(0.0), r.h), &TextStyle::footnote().weight(Weight::Semibold));
    clicked
}

/// A line about what's there (or isn't), centred in `r`.
fn message(ui: &mut Ui, r: Rect, title: &str, hint: &str) {
    let c = r.center();
    ui.c.icon(Icon::Video, Rect::from_center(Point::new(c.x, c.y - 22.0), 20.0, 20.0), palette::TEXT_FAINT);
    ui.c.text(title, Rect::new(r.x, c.y - 8.0, r.w, 18.0), &TextStyle::footnote().weight(Weight::Semibold).center());
    ui.c.text(hint, Rect::new(r.x, c.y + 10.0, r.w, 16.0), &TextStyle::caption().color(palette::TEXT_DIM).center());
}

/// Connect, and where to find the link, at the bottom of `r`.
fn connect(ui: &mut Ui, st: &mut AppState, r: Rect) {
    let y = r.bottom() - PILL_H;
    let (cw, hw) = (ui.pill_width("Connect", false, true), ui.pill_width("How?", true, true));
    let x = r.center().x - (cw + PILL_GAP + hw) / 2.0;
    if ui.pill(wid("calendar.connect"), Rect::new(x, y, cw, PILL_H), "Connect", None, PillStyle::Primary(st.accent)).clicked {
        st.cmd(Command::Calendar(CalendarCmd::UseLink));
    }
    if ui.pill(wid("calendar.help"), Rect::new(x + cw + PILL_GAP, y, hw, PILL_H), "How?", Some(Icon::Open), PillStyle::Utility).clicked {
        st.cmd(Command::OpenUrl(model::HELP_URL.into()));
    }
}

/// Remind (1, 5 or 10 minutes before) on the left, Remove on the right.
fn footer(ui: &mut Ui, st: &mut AppState, r: Rect) {
    let label = "Remind";
    let label_style = TextStyle::caption().color(palette::TEXT_DIM);
    let lw = ui.c.measure(label, &label_style, 200.0).0.ceil();
    ui.c.text(label, Rect::new(r.x, r.y, lw + 2.0, r.h), &label_style);
    let names = REMIND.map(|m| m.1);
    let sel = REMIND.iter().position(|m| m.0 == st.settings.meeting_remind_min).unwrap_or(1);
    if let Some(i) = super::settings::picker(ui, "calendar.remind", r.x + lw + 8.0, r.y + 2.0, 22.0, &names, sel) {
        st.settings.meeting_remind_min = REMIND[i].0;
        st.cmd(Command::SaveSettings);
    }
    let rw = ui.pill_width("Remove", false, true);
    let remove = Rect::new(r.right() - rw, r.y + (r.h - PILL_H) / 2.0, rw, PILL_H);
    if ui.pill(wid("calendar.remove"), remove, "Remove", None, PillStyle::Utility).clicked {
        st.cmd(Command::Calendar(CalendarCmd::Forget));
    }
}

/// What Connect last said, while it shows.
fn note(st: &AppState) -> Option<&'static str> {
    st.calendar.note.filter(|(_, until)| st.now < *until).map(|(text, _)| text)
}
