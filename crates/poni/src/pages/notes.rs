//! Notes tab, while the Notes add-on is on (`poni_core::notes`): sticky
//! notes, as Stickies or Notes keep them on a Mac. Left: a search field, a
//! plus for a new note, and the notes (pinned first, then the latest
//! changed), each with its colour, title and the line after. Right: the
//! note picked, on its colour, written right there (Enter starts a new
//! line, it saves as you type), with pin and delete, its five colours, and
//! Undo for a moment after a delete. `/note …` in quick add makes one too.

use crate::gfx::{TextStyle, Weight};
use crate::icons::Icon;
use crate::state::AppState;
use crate::ui::{wid, wid_n, PillStyle, Ui};
use poni_core::geom::{palette, Color, Point, Rect};
use poni_core::notes::{self as model, NoteColor, MAX_CHARS};

const HEIGHT: f32 = 186.0;
const LIST_W: f32 = 200.0;
const COL_GAP: f32 = 16.0;
const SEARCH_H: f32 = 28.0;
const ROW_H: f32 = 36.0;
const RADIUS: f32 = 12.0;
const PAD: f32 = 12.0;
const FOOT_H: f32 = 26.0;
const DOT_R: f32 = 6.0;
const WHEEL_STEP: f32 = ROW_H * 1.5;
/// How long Undo is offered after a delete.
const UNDO_S: f64 = 8.0;

/// The id of the note's text field (a new note focuses it).
pub const TEXT_ID: &str = "notes.text";

pub fn height(_st: &AppState) -> f32 {
    HEIGHT
}

pub fn color(c: NoteColor) -> Color {
    Color::hex(c.rgb())
}

pub fn draw(ui: &mut Ui, st: &mut AppState, area: Rect) {
    let area = Rect::new(area.x, area.y, area.w, HEIGHT);
    // Notes left empty go, but not the one being written.
    if st.store.notes.drop_empty(st.ui.note_open) {
        st.save();
    }
    if st.ui.note_open.is_some_and(|id| st.store.notes.get(id).is_none()) {
        st.ui.note_open = None;
    }
    if st.ui.note_open.is_none() {
        let first = st.store.notes.ordered(&st.ui.note_search).first().map(|n| n.id);
        if let Some(first) = first {
            open(st, first);
        }
    }
    draw_list(ui, st, Rect::new(area.x, area.y, LIST_W, HEIGHT));
    let right = Rect::new(area.x + LIST_W + COL_GAP, area.y, (area.w - LIST_W - COL_GAP).max(0.0), HEIGHT);
    match st.ui.note_open {
        Some(id) => draw_note(ui, st, right, id),
        None => super::empty_state(ui, right, Icon::File, "No notes yet", "Click + to write one, or type /note in quick add"),
    }
}

/// Show note `id` on the right, ready to write in.
fn open(st: &mut AppState, id: u64) {
    st.ui.note_open = Some(id);
    st.ui.note_text = st.store.notes.get(id).map(|n| n.text.clone()).unwrap_or_default();
    st.ui.note_text_scroll = 0.0;
}

fn draw_list(ui: &mut Ui, st: &mut AppState, r: Rect) {
    let accent = st.accent;
    // ---- Search, and a new note.
    let plus = Rect::new(r.right() - SEARCH_H, r.y, SEARCH_H, SEARCH_H);
    let search = Rect::new(r.x, r.y, plus.x - 6.0 - r.x, SEARCH_H);
    let _ = ui.text_input(wid("notes.search"), search, &mut st.ui.note_search, "Search", 100);
    if ui.icon_button(wid("notes.new"), plus, Icon::Plus, accent, 16.0).clicked {
        let color = st.ui.note_open.and_then(|id| st.store.notes.get(id)).map_or(NoteColor::Yellow, |n| n.color);
        if let Some(id) = st.store.notes.add("", color, st.unix) {
            st.ui.note_search.clear();
            open(st, id);
            ui.focus(wid(TEXT_ID), 0);
            st.save();
        }
    }

    // ---- The notes.
    let view = Rect::new(r.x, r.y + SEARCH_H + 8.0, r.w, r.h - SEARCH_H - 8.0);
    let ids: Vec<(u64, NoteColor, String, String)> =
        st.store.notes.ordered(&st.ui.note_search).iter().map(|n| (n.id, n.color, model::title(&n.text), model::preview(&n.text))).collect();
    let max_scroll = (ids.len() as f32 * ROW_H - view.h).max(0.0);
    st.ui.note_scroll = (st.ui.note_scroll - ui.wheel_in(view) * WHEEL_STEP).clamp(0.0, max_scroll);
    if ids.is_empty() && !st.ui.note_search.trim().is_empty() {
        ui.c.text("No note has that", Rect::new(view.x, view.y, view.w, 20.0), &TextStyle::footnote().color(palette::TEXT_DIM));
    }
    let mut picked = None;
    ui.c.clip(view);
    for (i, (id, c, title, preview)) in ids.iter().enumerate() {
        let row = Rect::new(view.x, view.y + i as f32 * ROW_H - st.ui.note_scroll, view.w, ROW_H - 2.0);
        if row.bottom() < view.y || row.y > view.bottom() {
            continue;
        }
        let resp = ui.button(wid_n("notes.row", *id), row);
        let on = st.ui.note_open == Some(*id);
        if on || resp.hovered {
            ui.c.fill_rrect(row, 8.0, if on { Color::white(0.10) } else { palette::SURFACE });
        }
        ui.c.fill_rrect(Rect::new(row.x + 6.0, row.y + 7.0, 3.0, row.h - 14.0), 1.5, color(*c));
        let tx = row.x + 16.0;
        let tw = row.w - 22.0;
        ui.c.text(title, Rect::new(tx, row.y + 2.0, tw, 17.0), &TextStyle::footnote().weight(Weight::Semibold));
        let line = if preview.is_empty() { "No more text" } else { preview.as_str() };
        ui.c.text(line, Rect::new(tx, row.y + 18.0, tw, 14.0), &TextStyle::caption().color(palette::TEXT_DIM));
        if resp.clicked {
            picked = Some(*id);
        }
    }
    ui.c.unclip();
    if let Some(id) = picked {
        open(st, id);
    }
}

fn draw_note(ui: &mut Ui, st: &mut AppState, r: Rect, id: u64) {
    let Some(note) = st.store.notes.get(id).cloned() else { return };
    let tint = color(note.color);
    ui.c.fill_rrect(r, RADIUS, tint.with_alpha(0.10));
    ui.c.stroke_rrect(r, RADIUS, tint.with_alpha(0.35), 1.0);
    let inner = r.inset(PAD, PAD - 2.0);

    // ---- The text, written in place.
    let text_r = Rect::new(inner.x, inner.y, inner.w, inner.h - FOOT_H - 6.0);
    let mut scroll = st.ui.note_text_scroll;
    let resp = ui.text_area(wid(TEXT_ID), text_r, &mut st.ui.note_text, "Write something\u{2026}", MAX_CHARS, &mut scroll, palette::TEXT);
    st.ui.note_text_scroll = scroll;
    if resp.changed && st.store.notes.set_text(id, &st.ui.note_text, st.unix) {
        st.save();
    }

    // ---- Colours on the left; pin and delete (or Undo) on the right.
    let fy = inner.bottom() - FOOT_H;
    let cy = fy + FOOT_H / 2.0;
    let mut x = inner.x + DOT_R;
    for (i, c) in NoteColor::ALL.iter().enumerate() {
        let dot = Rect::from_center(Point::new(x, cy), 2.0 * DOT_R + 6.0, 2.0 * DOT_R + 6.0);
        let resp = ui.button(wid_n("notes.color", i as u64), dot);
        if *c == note.color {
            ui.c.stroke_circle(Point::new(x, cy), DOT_R + 2.5, color(*c), 1.5);
        }
        ui.c.fill_circle(Point::new(x, cy), if resp.hovered { DOT_R + 0.5 } else { DOT_R }, color(*c));
        if resp.clicked && *c != note.color {
            st.store.notes.set_color(id, *c);
            st.save();
        }
        x += 2.0 * DOT_R + 10.0;
    }

    let trash = Rect::new(inner.right() - 24.0, fy, 24.0, FOOT_H);
    let pin = Rect::new(trash.x - 28.0, fy, 24.0, FOOT_H);
    let pin_icon = if note.pinned { Icon::PinFill } else { Icon::Pin };
    if ui.icon_button(wid("notes.pin"), pin, pin_icon, if note.pinned { tint } else { palette::TEXT_DIM }, 15.0).clicked {
        st.store.notes.toggle_pin(id);
        st.save();
    }
    if ui.icon_button(wid("notes.delete"), trash, Icon::Trash, palette::TEXT_DIM, 15.0).clicked {
        if let Some(gone) = st.store.notes.remove(id) {
            st.ui.note_undo = Some((gone, st.now));
            st.ui.note_open = None;
            st.save();
        }
        return;
    }
    let undo = st.ui.note_undo.as_ref().filter(|(_, t)| st.now - t < UNDO_S).map(|(n, _)| n.clone());
    match undo {
        Some(gone) => {
            let w = ui.pill_width("Undo", true, true);
            let u = Rect::new(pin.x - 10.0 - w, cy - 12.0, w, 24.0);
            if ui.pill(wid("notes.undo"), u, "Undo", Some(Icon::Undo), PillStyle::UtilityAccent(tint)).clicked {
                let back = gone.id;
                st.store.notes.restore(gone);
                st.ui.note_undo = None;
                open(st, back);
                st.save();
            }
        }
        None => st.ui.note_undo = None,
    }
}

/// What's left of a delete's Undo, for the app's wake timer.
pub fn next_wake(st: &AppState) -> Option<f64> {
    st.ui.note_undo.as_ref().map(|(_, t)| t + UNDO_S).filter(|t| *t > st.now)
}
