//! Sections tab, while the Sections add-on is on (`poni_core::sections`).
//! Left: the sections, numbered as Alt+Shift+1–9 reaches them (the one
//! that's on has a dot), and a field to make a new one. Right: the section
//! picked on the left, its name (Enter renames), Use or Turn off, Delete,
//! and its tabs as chips to show or hide.

use crate::commands::Command;
use crate::gfx::{TextStyle, Weight};
use crate::icons::Icon;
use crate::pages::PageKind;
use crate::state::AppState;
use crate::ui::{wid, wid_n, PillStyle, Ui, PILL_GAP};
use poni_core::geom::{palette, Color, Point, Rect};
use poni_core::sections::MAX_NAME;

const HEIGHT: f32 = 146.0;
const LIST_W: f32 = 200.0;
const COL_GAP: f32 = 18.0;
const ROW_H: f32 = 22.0;
const INPUT_H: f32 = 28.0;
const CHIP_H: f32 = 26.0;
const CHIP_GAP: f32 = 6.0;
const DOT_R: f32 = 3.0;
const WHEEL_STEP: f32 = ROW_H * 1.5;

pub fn height(_st: &AppState) -> f32 {
    HEIGHT
}

/// The tabs a section can hold: every one the island has now but these
/// (Add-ons and Sections always show; Settings sits behind the gear).
pub fn choosable(st: &AppState) -> Vec<PageKind> {
    PageKind::ALL.into_iter().filter(|p| !matches!(p, PageKind::Addons | PageKind::Sections | PageKind::Settings) && super::available(st, *p)).collect()
}

pub fn draw(ui: &mut Ui, st: &mut AppState, area: Rect) {
    let area = Rect::new(area.x, area.y, area.w, HEIGHT);
    let list_r = Rect::new(area.x, area.y, LIST_W, HEIGHT);
    draw_list(ui, st, list_r);
    let right = Rect::new(area.x + LIST_W + COL_GAP, area.y, (area.w - LIST_W - COL_GAP).max(0.0), HEIGHT);
    match st.ui.section_edit.filter(|i| *i < st.settings.sections.list.len()) {
        Some(i) => draw_section(ui, st, right, i),
        None => {
            let hint = if st.settings.sections.list.is_empty() { "Name one on the left, then pick its tabs" } else { "Pick one on the left to change it" };
            super::empty_state(ui, right, Icon::Layers, "Sections", hint);
        }
    }
}

fn draw_list(ui: &mut Ui, st: &mut AppState, r: Rect) {
    let n = st.settings.sections.list.len();
    let can_add = n < poni_core::sections::MAX;
    let view = Rect::new(r.x, r.y, r.w, r.h - if can_add { INPUT_H + 6.0 } else { 0.0 });
    let max_scroll = (n as f32 * ROW_H - view.h).max(0.0);
    st.ui.section_scroll = (st.ui.section_scroll - ui.wheel_in(view) * WHEEL_STEP).clamp(0.0, max_scroll);
    let accent = st.accent;
    let mut picked = None;
    ui.c.clip(view);
    for i in 0..n {
        let row = Rect::new(r.x, view.y + i as f32 * ROW_H - st.ui.section_scroll, r.w, ROW_H);
        if row.bottom() < view.y || row.y > view.bottom() {
            continue;
        }
        let resp = ui.button(wid_n("sections.row", i as u64), row);
        let editing = st.ui.section_edit == Some(i);
        if editing || resp.hovered {
            ui.c.fill_rrect(row, 7.0, if editing { Color::white(0.10) } else { palette::SURFACE });
        }
        let s = &st.settings.sections.list[i];
        ui.c.text(&format!("{}", i + 1), Rect::new(row.x + 8.0, row.y, 14.0, row.h), &TextStyle::footnote().tabular().color(palette::TEXT_DIM));
        ui.c.text(&s.name, Rect::new(row.x + 26.0, row.y, row.w - 44.0, row.h), &TextStyle::footnote().weight(Weight::Semibold));
        if st.settings.sections.active == Some(i) {
            ui.c.fill_circle(Point::new(row.right() - 10.0, row.center().y), DOT_R, accent);
        }
        if resp.clicked {
            picked = Some(i);
        }
    }
    ui.c.unclip();
    if let Some(i) = picked {
        st.ui.section_edit = Some(i);
        st.ui.section_rename = st.settings.sections.list[i].name.clone();
    }

    if can_add {
        let input = Rect::new(r.x, r.bottom() - INPUT_H, r.w, INPUT_H);
        let placeholder = note(st).unwrap_or("New section, Enter");
        let resp = ui.text_input_row(wid("sections.new"), input, &mut st.ui.section_input, placeholder, MAX_NAME, accent, Some(Icon::Plus));
        if resp.submitted {
            // A new section starts with the tabs there are now.
            let tabs: Vec<String> = choosable(st).iter().map(|p| p.key().to_string()).collect();
            let name = st.ui.section_input.clone();
            match st.settings.sections.add(&name, tabs) {
                Ok(i) => {
                    st.ui.section_input.clear();
                    ui.input.caret = 0;
                    st.ui.section_edit = Some(i);
                    st.ui.section_rename = st.settings.sections.list[i].name.clone();
                    st.cmd(Command::SaveSettings);
                }
                Err(why) => st.ui.section_note = Some((why, st.now + 4.0)),
            }
        }
    }
}

fn draw_section(ui: &mut Ui, st: &mut AppState, r: Rect, i: usize) {
    let accent = st.accent;
    let on = st.settings.sections.active == Some(i);

    // ---- Name, Use / Turn off, Delete.
    let del_w = ui.pill_width("Delete", false, true);
    let use_label = if on { "Turn off" } else { "Use" };
    let use_w = ui.pill_width(use_label, false, true);
    let del = Rect::new(r.right() - del_w, r.y, del_w, INPUT_H);
    let use_r = Rect::new(del.x - PILL_GAP - use_w, r.y, use_w, INPUT_H);
    let name_r = Rect::new(r.x, r.y, (use_r.x - 10.0 - r.x).max(0.0), INPUT_H);
    let resp = ui.text_input(wid_n("sections.rename", i as u64), name_r, &mut st.ui.section_rename, "Name", MAX_NAME);
    if resp.submitted {
        let name = st.ui.section_rename.clone();
        match st.settings.sections.rename(i, &name) {
            Ok(()) => st.cmd(Command::SaveSettings),
            Err(why) => st.ui.section_note = Some((why, st.now + 4.0)),
        }
    }
    let use_style = if on { PillStyle::Utility } else { PillStyle::Primary(accent) };
    if ui.pill(wid("sections.use"), use_r, use_label, None, use_style).clicked {
        st.settings.sections.activate(if on { None } else { Some(i) });
        st.cmd(Command::SaveSettings);
    }
    if ui.pill(wid("sections.delete"), del, "Delete", None, PillStyle::Utility).clicked {
        st.settings.sections.remove(i);
        st.ui.section_edit = None;
        st.cmd(Command::SaveSettings);
        return;
    }

    // ---- Its tabs, as chips.
    let mut x = r.x;
    let mut y = r.y + INPUT_H + 12.0;
    let mut toggle = None;
    for (j, p) in choosable(st).into_iter().enumerate() {
        let label = p.title();
        let w = ui.pill_width(label, true, true);
        if x + w > r.right() && x > r.x {
            x = r.x;
            y += CHIP_H + CHIP_GAP;
        }
        let shown = st.settings.sections.list[i].tabs.iter().any(|t| t == p.key());
        let style = if shown { PillStyle::Primary(accent) } else { PillStyle::Utility };
        if ui.pill(wid_n("sections.tab", j as u64), Rect::new(x, y, w, CHIP_H), label, Some(p.icon()), style).clicked {
            toggle = Some(p.key());
        }
        x += w + CHIP_GAP;
    }
    if let Some(key) = toggle {
        st.settings.sections.toggle_tab(i, key);
        st.cmd(Command::SaveSettings);
    }

    let line = note(st).map(str::to_string).unwrap_or_else(|| format!("Alt+Shift+{} switches to it \u{b7} Alt+Shift+0: every tab", i + 1));
    ui.c.text(&line, Rect::new(r.x, r.bottom() - 16.0, r.w, 16.0), &TextStyle::caption().color(palette::TEXT_DIM));
}

/// What the last change said went wrong, while it shows.
fn note(st: &AppState) -> Option<&'static str> {
    st.ui.section_note.filter(|(_, until)| st.now < *until).map(|(text, _)| text)
}
