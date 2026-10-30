//! Notes: quick sticky notes, like Stickies or Notes on a Mac. Each one is
//! a bit of text in a colour; its first line is its title. Pinned ones come
//! first, then the latest changed. Kept in data.json with the tasks.

use serde::{Deserialize, Serialize};

pub const MAX_NOTES: usize = 200;
pub const MAX_CHARS: usize = 5000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteColor {
    #[default]
    Yellow,
    Green,
    Blue,
    Pink,
    Purple,
}

impl NoteColor {
    pub const ALL: [NoteColor; 5] = [NoteColor::Yellow, NoteColor::Green, NoteColor::Blue, NoteColor::Pink, NoteColor::Purple];

    /// The colour as 0xRRGGBB, bright enough to read on the black island.
    pub fn rgb(self) -> u32 {
        match self {
            NoteColor::Yellow => 0xF5C842,
            NoteColor::Green => 0x7BD88F,
            NoteColor::Blue => 0x6CB6FF,
            NoteColor::Pink => 0xFF8FB1,
            NoteColor::Purple => 0xB9AEF7,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Note {
    pub id: u64,
    pub text: String,
    pub color: NoteColor,
    pub pinned: bool,
    pub created_unix: i64,
    pub updated_unix: i64,
}

impl Default for Note {
    fn default() -> Self {
        Self { id: 0, text: String::new(), color: NoteColor::Yellow, pinned: false, created_unix: 0, updated_unix: 0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NoteList {
    pub notes: Vec<Note>,
    pub next_id: u64,
}

impl Default for NoteList {
    fn default() -> Self {
        Self { notes: Vec::new(), next_id: 1 }
    }
}

impl NoteList {
    /// A new note (it may start empty, to be typed in); its id, or `None`
    /// when the list is full.
    pub fn add(&mut self, text: &str, color: NoteColor, now_unix: i64) -> Option<u64> {
        if self.notes.len() >= MAX_NOTES {
            return None;
        }
        let id = self.next_id.max(1);
        self.next_id = id + 1;
        let text: String = text.chars().take(MAX_CHARS).collect();
        self.notes.push(Note { id, text, color, pinned: false, created_unix: now_unix, updated_unix: now_unix });
        Some(id)
    }

    pub fn get(&self, id: u64) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut Note> {
        self.notes.iter_mut().find(|n| n.id == id)
    }

    /// New text for note `id` (cut at [`MAX_CHARS`]); false when it's gone
    /// or nothing changed.
    pub fn set_text(&mut self, id: u64, text: &str, now_unix: i64) -> bool {
        let Some(n) = self.get_mut(id) else { return false };
        let text: String = text.chars().take(MAX_CHARS).collect();
        if n.text == text {
            return false;
        }
        n.text = text;
        n.updated_unix = now_unix;
        true
    }

    pub fn set_color(&mut self, id: u64, color: NoteColor) {
        if let Some(n) = self.get_mut(id) {
            n.color = color;
        }
    }

    pub fn toggle_pin(&mut self, id: u64) {
        if let Some(n) = self.get_mut(id) {
            n.pinned = !n.pinned;
        }
    }

    /// Remove note `id`, handing it back (for Undo).
    pub fn remove(&mut self, id: u64) -> Option<Note> {
        let i = self.notes.iter().position(|n| n.id == id)?;
        Some(self.notes.remove(i))
    }

    /// Put a removed note back as it was.
    pub fn restore(&mut self, note: Note) {
        if self.get(note.id).is_none() && self.notes.len() < MAX_NOTES {
            self.next_id = self.next_id.max(note.id + 1);
            self.notes.push(note);
        }
    }

    /// Notes left with nothing in them go, except `keep` (the one being written).
    pub fn drop_empty(&mut self, keep: Option<u64>) -> bool {
        let before = self.notes.len();
        self.notes.retain(|n| Some(n.id) == keep || !n.text.trim().is_empty());
        self.notes.len() != before
    }

    /// Pinned first, then the latest changed; with `query`, only the notes
    /// holding every one of its words (ignoring case).
    pub fn ordered(&self, query: &str) -> Vec<&Note> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let mut v: Vec<&Note> = self
            .notes
            .iter()
            .filter(|n| {
                let lower = n.text.to_lowercase();
                words.iter().all(|w| lower.contains(w.as_str()))
            })
            .collect();
        v.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.updated_unix.cmp(&a.updated_unix)).then(b.id.cmp(&a.id)));
        v
    }
}

/// A note's title: its first line with anything in it.
pub fn title(text: &str) -> String {
    text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("New note").chars().take(80).collect()
}

/// What follows the title, on one line, for the list.
pub fn preview(text: &str) -> String {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    lines.next();
    lines.collect::<Vec<_>>().join(" ").chars().take(120).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_are_written_kept_and_ordered() {
        let mut l = NoteList::default();
        let a = l.add("Groceries\nmilk\neggs", NoteColor::Yellow, 10).unwrap();
        let b = l.add("", NoteColor::Blue, 20).unwrap();
        assert!(l.set_text(b, "Ideas\n- a notch for Windows", 30));
        assert!(!l.set_text(b, "Ideas\n- a notch for Windows", 40), "nothing changed");
        let order: Vec<u64> = l.ordered("").iter().map(|n| n.id).collect();
        assert_eq!(order, [b, a], "the latest changed first");
        l.toggle_pin(a);
        assert_eq!(l.ordered("")[0].id, a, "pinned first");
        assert_eq!(l.ordered("EGGS").len(), 1);
        assert_eq!(l.ordered("notch windows")[0].id, b);
        assert!(l.ordered("bread").is_empty());
        l.set_color(a, NoteColor::Pink);
        assert_eq!(l.get(a).unwrap().color, NoteColor::Pink);
    }

    #[test]
    fn removed_notes_come_back_and_empty_ones_go() {
        let mut l = NoteList::default();
        let a = l.add("keep me", NoteColor::Green, 1).unwrap();
        let blank = l.add("  \n ", NoteColor::Yellow, 2).unwrap();
        let typing = l.add("", NoteColor::Yellow, 3).unwrap();
        assert!(l.drop_empty(Some(typing)));
        assert!(l.get(blank).is_none() && l.get(typing).is_some());
        let gone = l.remove(a).unwrap();
        assert!(l.get(a).is_none());
        l.restore(gone);
        assert_eq!(l.get(a).unwrap().text, "keep me");
        assert!(l.add("x", NoteColor::Yellow, 4).unwrap() > a, "ids never repeat");
    }

    #[test]
    fn titles_and_previews() {
        assert_eq!(title("\n  Groceries \nmilk"), "Groceries");
        assert_eq!(title("   "), "New note");
        assert_eq!(preview("Groceries\n\nmilk\n eggs "), "milk eggs");
        assert_eq!(preview("Only a title"), "");
        let long = "x".repeat(MAX_CHARS + 10);
        let mut l = NoteList::default();
        let id = l.add(&long, NoteColor::Yellow, 0).unwrap();
        assert_eq!(l.get(id).unwrap().text.chars().count(), MAX_CHARS);
    }

    #[test]
    fn old_files_without_notes_load() {
        let l: NoteList = serde_json::from_str("{}").unwrap();
        assert_eq!((l.notes.len(), l.next_id), (0, 1));
        let n: Note = serde_json::from_str(r#"{"id":3,"text":"hi","color":"purple"}"#).unwrap();
        assert_eq!((n.color, n.pinned), (NoteColor::Purple, false));
    }
}
