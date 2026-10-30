//! Sections: sets of tabs the person makes and names ("Client A",
//! "Evening"), and switches between with Alt+Shift+1–9 (Alt+Shift+0: every
//! tab again). While one is on, the island shows only its tabs; Sections and
//! Add-ons always stay, so a section can never hide the way back.
//! Kept in settings.json, with the section that's on.

use serde::{Deserialize, Serialize};

/// As many as there are number keys.
pub const MAX: usize = 9;
pub const MAX_NAME: usize = 24;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Section {
    pub name: String,
    /// The tabs it shows, by page key ("media", "tasks", …).
    pub tabs: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sections {
    pub list: Vec<Section>,
    /// The section that's on (an index in `list`); none: every tab.
    pub active: Option<usize>,
}

impl Sections {
    /// A new section showing `tabs`; its index, or why not.
    pub fn add(&mut self, name: &str, tabs: Vec<String>) -> Result<usize, &'static str> {
        let name = self.clean_name(name, None)?;
        if self.list.len() >= MAX {
            return Err("Nine sections at most");
        }
        self.list.push(Section { name, tabs });
        Ok(self.list.len() - 1)
    }

    pub fn rename(&mut self, i: usize, name: &str) -> Result<(), &'static str> {
        let name = self.clean_name(name, Some(i))?;
        let s = self.list.get_mut(i).ok_or("No such section")?;
        s.name = name;
        Ok(())
    }

    /// Remove section `i`; the one that's on stays on (or none, if it was this one).
    pub fn remove(&mut self, i: usize) {
        if i >= self.list.len() {
            return;
        }
        self.list.remove(i);
        self.active = match self.active {
            Some(a) if a == i => None,
            Some(a) if a > i => Some(a - 1),
            a => a,
        };
    }

    /// Show or hide tab `key` in section `i`.
    pub fn toggle_tab(&mut self, i: usize, key: &str) {
        let Some(s) = self.list.get_mut(i) else { return };
        match s.tabs.iter().position(|t| t == key) {
            Some(p) => {
                s.tabs.remove(p);
            }
            None => s.tabs.push(key.to_string()),
        }
    }

    /// Turn section `i` on (`None`: every tab). False for one that doesn't exist.
    pub fn activate(&mut self, i: Option<usize>) -> bool {
        match i {
            Some(i) if i >= self.list.len() => false,
            i => {
                self.active = i;
                true
            }
        }
    }

    pub fn current(&self) -> Option<&Section> {
        self.active.and_then(|i| self.list.get(i))
    }

    /// Whether tab `key` shows: every tab while no section is on.
    pub fn shows(&self, key: &str) -> bool {
        self.current().is_none_or(|s| s.tabs.iter().any(|t| t == key))
    }

    /// The section a typed name means: the same name, else the first one it
    /// starts, else the first one holding it (ignoring case).
    pub fn find(&self, query: &str) -> Option<usize> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return None;
        }
        let names: Vec<String> = self.list.iter().map(|s| s.name.to_lowercase()).collect();
        names
            .iter()
            .position(|n| *n == q)
            .or_else(|| names.iter().position(|n| n.starts_with(&q)))
            .or_else(|| names.iter().position(|n| n.contains(&q)))
    }

    /// Hand-edited files: no more than nine, a known one on (or none).
    pub fn sanitize(&mut self) {
        self.list.retain(|s| !s.name.trim().is_empty());
        self.list.truncate(MAX);
        if self.active.is_some_and(|a| a >= self.list.len()) {
            self.active = None;
        }
    }

    /// A name to use: trimmed, not empty, not too long, not another
    /// section's (`except`: the one being renamed).
    fn clean_name(&self, name: &str, except: Option<usize>) -> Result<String, &'static str> {
        let name: String = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if name.is_empty() {
            return Err("Give it a name");
        }
        let name: String = name.chars().take(MAX_NAME).collect();
        let taken = self.list.iter().enumerate().any(|(j, s)| Some(j) != except && s.name.eq_ignore_ascii_case(&name));
        if taken {
            return Err("There's a section with that name");
        }
        Ok(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(keys: &[&str]) -> Vec<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    #[test]
    fn sections_are_made_named_and_switched() {
        let mut s = Sections::default();
        assert!(s.shows("media"), "no section: every tab");
        let a = s.add("  Client   A ", tabs(&["tasks", "today"])).unwrap();
        assert_eq!(s.list[a].name, "Client A");
        assert_eq!(s.add("client a", vec![]), Err("There's a section with that name"));
        assert_eq!(s.add("   ", vec![]), Err("Give it a name"));
        let b = s.add("Evening", tabs(&["media"])).unwrap();
        assert!(s.activate(Some(a)));
        assert!(s.shows("tasks") && !s.shows("media"));
        s.toggle_tab(a, "media");
        assert!(s.shows("media"));
        s.toggle_tab(a, "media");
        assert!(!s.shows("media"));
        assert!(!s.activate(Some(7)), "no ninth section yet");
        assert_eq!(s.current().map(|c| c.name.as_str()), Some("Client A"));
        assert!(s.rename(b, "client a").is_err());
        s.rename(b, "Night").unwrap();
        assert_eq!(s.list[b].name, "Night");
        assert!(s.activate(None));
        assert!(s.shows("media"));
    }

    #[test]
    fn removing_keeps_the_right_one_on() {
        let mut s = Sections::default();
        for n in ["A", "B", "C"] {
            s.add(n, vec![]).unwrap();
        }
        s.activate(Some(2));
        s.remove(0);
        assert_eq!(s.current().map(|c| c.name.as_str()), Some("C"));
        s.remove(1);
        assert_eq!(s.active, None, "the one that was on is gone");
        s.remove(5);
        assert_eq!(s.list.len(), 1);
    }

    #[test]
    fn nine_at_most_and_names_are_found_by_a_part() {
        let mut s = Sections::default();
        for i in 0..MAX {
            s.add(&format!("S{i}"), vec![]).unwrap();
        }
        assert_eq!(s.add("Ten", vec![]), Err("Nine sections at most"));
        let mut t = Sections::default();
        for n in ["Client A", "Clients", "Evening"] {
            t.add(n, vec![]).unwrap();
        }
        assert_eq!(t.find("clients"), Some(1));
        assert_eq!(t.find("cli"), Some(0));
        assert_eq!(t.find("ning"), Some(2));
        assert_eq!(t.find("x"), None);
        assert_eq!(t.find(" "), None);
    }

    #[test]
    fn hand_edited_files_are_tidied() {
        let mut s: Sections = serde_json::from_str(r#"{"list":[{"name":"A","tabs":["tasks"]},{"name":" "}],"active":5}"#).unwrap();
        s.sanitize();
        assert_eq!((s.list.len(), s.active), (1, None));
    }
}
