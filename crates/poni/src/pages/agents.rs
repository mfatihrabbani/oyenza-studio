//! AI tab, while a Claude Code or Codex session has told the island about
//! itself (`poni_core::agents`): one row per session, the ones that need you
//! first, each with its folder, what it's doing and Open, which brings its
//! window to the front. Finished sessions leave after half an hour, and the
//! tab goes with the last one.

use crate::commands::Command;
use crate::gfx::{TextStyle, Weight};
use crate::state::AppState;
use crate::ui::{wid_n, PillStyle, Ui};
use poni_core::agents::{status_line, State};
use poni_core::geom::{palette, Point, Rect};

const HEIGHT: f32 = 146.0;
const ROW_H: f32 = 29.0;
const ROWS: usize = 5;
const DOT_R: f32 = 4.0;
const NAME_X: f32 = 18.0;
const PILL_H: f32 = 24.0;

pub fn height(_st: &AppState) -> f32 {
    HEIGHT
}

pub fn draw(ui: &mut Ui, st: &mut AppState, area: Rect) {
    let now = st.unix;
    let accent = st.accent;
    let mut open = None;
    let rows: Vec<_> = st.agents.sessions.ordered().into_iter().take(ROWS).cloned().collect();
    for (i, s) in rows.iter().enumerate() {
        let r = Rect::new(area.x, area.y + i as f32 * ROW_H, area.w, ROW_H);
        let cy = r.center().y;
        let dot = match s.state {
            State::Waiting => palette::ORANGE,
            State::Working => accent,
            _ => palette::GREEN,
        };
        ui.c.fill_circle(Point::new(r.x + DOT_R + 2.0, cy), DOT_R, dot);

        let w = ui.pill_width("Open", false, true);
        let pill = Rect::new(r.right() - w, cy - PILL_H / 2.0, w, PILL_H);
        let style = if s.state == State::Waiting { PillStyle::Primary(palette::ORANGE) } else { PillStyle::Utility };
        if ui.pill(wid_n("agents.open", i as u64), pill, "Open", None, style).clicked {
            open = Some(s.key.clone());
        }

        let name = format!("{} \u{b7} {}", s.tool.name(), s.title);
        let name_style = TextStyle::body().weight(Weight::Semibold);
        let x = r.x + NAME_X;
        let nw = ui.c.measure(&name, &name_style, 400.0).0.ceil().min((pill.x - 12.0 - x) * 0.6);
        ui.c.text(&name, Rect::new(x, r.y, nw + 2.0, r.h), &name_style);
        let status = match (s.state, s.message.is_empty()) {
            (State::Waiting, false) => s.message.clone(),
            _ => status_line(s, now),
        };
        let sx = x + nw + 10.0;
        let color = if s.state == State::Waiting { palette::ORANGE } else { palette::TEXT_DIM };
        ui.c.text(&status, Rect::new(sx, r.y, (pill.x - 10.0 - sx).max(0.0), r.h), &TextStyle::footnote().color(color));
    }
    if let Some(key) = open {
        st.cmd(Command::OpenAgent(key));
    }
}
