//! AI coding sessions (Claude Code, Codex) on the island: which are working,
//! which need you, which are done. The tools tell Oyenza Studio themselves,
//! through their own hooks: Claude Code's hooks and Codex's `notify` run
//! `poni.exe --agent claude|codex`, which reads what the tool says, hands it
//! to the running island and exits. Nothing polls or listens in between.
//!
//! This parses what each tool says and keeps the list of sessions.

use serde::{Deserialize, Serialize};

/// A finished session leaves the list after this long (seconds).
pub const DONE_KEPT_S: i64 = 30 * 60;
/// A session that hasn't said anything for this long is dropped (its
/// terminal was closed without a word).
pub const STALE_S: i64 = 6 * 3600;
/// Sessions kept at most (the oldest go first).
const MAX: usize = 12;
/// What a message keeps, in characters.
const MESSAGE_CHARS: usize = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    Claude,
    Codex,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::Claude => "Claude",
            Tool::Codex => "Codex",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Working,
    /// Asks for a permission or an answer.
    Waiting,
    Done,
    /// The session closed: it leaves the list.
    Ended,
}

/// One thing a tool said.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub tool: Tool,
    pub session: String,
    pub state: State,
    /// The folder the session works in.
    pub cwd: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub key: String,
    pub tool: Tool,
    /// The folder's name ("oyenza-studio").
    pub title: String,
    pub state: State,
    pub message: String,
    /// When its current turn started, and when it last said something (UTC seconds).
    pub since: i64,
    pub updated: i64,
    /// The window it was started from (0: not known), for Open.
    pub window: isize,
}

#[derive(Clone, Debug, Default)]
pub struct Sessions {
    pub list: Vec<Session>,
}

impl Sessions {
    /// Take in `ev`; `window` is the window in front as a turn starts.
    /// Returns the session when the island should say something about it
    /// (it needs you, or it just finished).
    pub fn apply(&mut self, ev: Event, now: i64, window: Option<isize>) -> Option<&Session> {
        let key = format!("{}:{}", ev.tool.name(), ev.session);
        let at = self.list.iter().position(|s| s.key == key);
        if ev.state == State::Ended {
            if let Some(i) = at {
                self.list.remove(i);
            }
            return None;
        }
        let i = match at {
            Some(i) => i,
            None => {
                if self.list.len() >= MAX {
                    if let Some(oldest) = self.list.iter().enumerate().min_by_key(|(_, s)| s.updated).map(|(i, _)| i) {
                        self.list.remove(oldest);
                    }
                }
                self.list.push(Session {
                    key,
                    tool: ev.tool,
                    title: folder_name(&ev.cwd),
                    state: ev.state,
                    message: String::new(),
                    since: now,
                    updated: now,
                    window: 0,
                });
                self.list.len() - 1
            }
        };
        let s = &mut self.list[i];
        let before = s.state;
        // Claude reminds an idle session that it waits for input a minute
        // after it finished: that's not news.
        if ev.state == State::Waiting && before == State::Done {
            return None;
        }
        if ev.state == State::Working && before != State::Working {
            s.since = now;
        }
        if let Some(w) = window.filter(|w| *w != 0) {
            s.window = w;
        }
        if !ev.cwd.is_empty() {
            s.title = folder_name(&ev.cwd);
        }
        s.state = ev.state;
        s.message = ev.message.chars().take(MESSAGE_CHARS).collect();
        s.updated = now;
        matches!(ev.state, State::Waiting | State::Done).then_some(&*s)
    }

    /// Drop finished sessions after a while, and silent ones after hours.
    pub fn prune(&mut self, now: i64) {
        self.list.retain(|s| match s.state {
            State::Done => now - s.updated < DONE_KEPT_S,
            _ => now - s.updated < STALE_S,
        });
    }

    pub fn get(&self, key: &str) -> Option<&Session> {
        self.list.iter().find(|s| s.key == key)
    }

    pub fn remove(&mut self, key: &str) {
        self.list.retain(|s| s.key != key);
    }

    /// The sessions as the tab lists them: the ones that need you first,
    /// then working, then done, newest first in each.
    pub fn ordered(&self) -> Vec<&Session> {
        let rank = |s: &State| match s {
            State::Waiting => 0,
            State::Working => 1,
            _ => 2,
        };
        let mut v: Vec<&Session> = self.list.iter().collect();
        v.sort_by(|a, b| rank(&a.state).cmp(&rank(&b.state)).then(b.updated.cmp(&a.updated)));
        v
    }

    /// Sessions working or waiting.
    pub fn active(&self) -> usize {
        self.list.iter().filter(|s| matches!(s.state, State::Working | State::Waiting)).count()
    }
}

/// "oyenza-studio" from "C:\\Users\\me\\code\\oyenza-studio\\".
pub fn folder_name(cwd: &str) -> String {
    let name = cwd.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or("");
    if name.is_empty() {
        "session".into()
    } else {
        name.into()
    }
}

/// What Claude Code passes a hook on stdin: `hook_event_name`, `session_id`,
/// `cwd`, and `message` for a notification.
pub fn parse_claude(json: &str) -> Option<Event> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let state = match v.get("hook_event_name")?.as_str()? {
        "UserPromptSubmit" | "SessionStart" => State::Working,
        "Notification" => State::Waiting,
        "Stop" => State::Done,
        "SessionEnd" => State::Ended,
        _ => return None,
    };
    let text = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    Some(Event { tool: Tool::Claude, session: text("session_id"), state, cwd: text("cwd"), message: one_line(&text("message")) })
}

/// What Codex passes its `notify` program (the last argument): a turn
/// finished, with `thread-id`, `cwd` and `last-assistant-message`.
pub fn parse_codex(json: &str) -> Option<Event> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    if v.get("type")?.as_str()? != "agent-turn-complete" {
        return None;
    }
    let text = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let cwd = text("cwd");
    let session = match text("thread-id") {
        id if !id.is_empty() => id,
        _ => cwd.clone(),
    };
    Some(Event { tool: Tool::Codex, session, state: State::Done, cwd, message: one_line(&text("last-assistant-message")) })
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// "working 3m", "needs you", "done 12m ago".
pub fn status_line(s: &Session, now: i64) -> String {
    let mins = |secs: i64| match secs.max(0) / 60 {
        0 => "<1m".to_string(),
        m if m < 60 => format!("{m}m"),
        m => format!("{}h {}m", m / 60, m % 60),
    };
    match s.state {
        State::Working => format!("working {}", mins(now - s.since)),
        State::Waiting => "needs you".into(),
        State::Done => format!("done {} ago", mins(now - s.updated)),
        State::Ended => "ended".into(),
    }
}

/// The hooks to paste into Claude Code's settings (`~/.claude/settings.json`),
/// for this copy of poni.exe.
pub fn claude_hooks_json(exe: &str) -> String {
    let command = format!("\"{}\" --agent claude", exe.replace('\\', "/"));
    let hook = serde_json::json!([{ "hooks": [{ "type": "command", "command": command }] }]);
    let v = serde_json::json!({ "hooks": {
        "UserPromptSubmit": hook,
        "Notification": hook,
        "Stop": hook,
        "SessionEnd": hook,
    }});
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

/// The line for Codex's `~/.codex/config.toml`.
pub fn codex_notify_line(exe: &str) -> String {
    format!("notify = [\"{}\", \"--agent\", \"codex\"]", exe.replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude(event: &str, extra: &str) -> Event {
        parse_claude(&format!(r#"{{"session_id":"abc","cwd":"C:\\code\\oyenza-studio","hook_event_name":"{event}"{extra}}}"#)).unwrap()
    }

    #[test]
    fn claude_says_what_its_hooks_see() {
        assert_eq!(claude("UserPromptSubmit", r#","prompt":"fix it""#).state, State::Working);
        let n = claude("Notification", r#","message":"Claude needs your\npermission to use Bash""#);
        assert_eq!((n.state, n.message.as_str()), (State::Waiting, "Claude needs your permission to use Bash"));
        assert_eq!(claude("Stop", "").state, State::Done);
        assert_eq!(claude("SessionEnd", "").state, State::Ended);
        assert_eq!(parse_claude(r#"{"hook_event_name":"PreToolUse"}"#), None);
        assert_eq!(parse_claude("not json"), None);
    }

    #[test]
    fn codex_says_when_a_turn_is_done() {
        let e = parse_codex(r#"{"type":"agent-turn-complete","thread-id":"t1","turn-id":"1","cwd":"/home/me/api","input-messages":["x"],"last-assistant-message":"Done: tests pass."}"#).unwrap();
        assert_eq!((e.tool, e.state, e.session.as_str(), e.message.as_str()), (Tool::Codex, State::Done, "t1", "Done: tests pass."));
        assert_eq!(folder_name(&e.cwd), "api");
        assert_eq!(parse_codex(r#"{"type":"other"}"#), None);
    }

    #[test]
    fn a_session_works_waits_finishes_and_goes() {
        let mut s = Sessions::default();
        assert!(s.apply(claude("UserPromptSubmit", ""), 100, Some(7)).is_none(), "starting is no news");
        assert_eq!(s.list[0].title, "oyenza-studio");
        assert_eq!((s.list[0].window, s.active()), (7, 1));
        assert_eq!(status_line(&s.list[0], 100 + 185), "working 3m");
        let w = s.apply(claude("Notification", r#","message":"needs permission""#), 200, None).unwrap();
        assert_eq!((w.state, w.window), (State::Waiting, 7), "the window stays");
        assert!(s.apply(claude("Stop", ""), 300, None).is_some());
        // The idle reminder a minute later is not news.
        assert!(s.apply(claude("Notification", r#","message":"Claude is waiting for your input""#), 360, None).is_none());
        assert_eq!(s.list[0].state, State::Done);
        assert_eq!(status_line(&s.list[0], 300 + 720), "done 12m ago");
        // A new prompt starts the clock again.
        s.apply(claude("UserPromptSubmit", ""), 1000, None);
        assert_eq!((s.list[0].state, s.list[0].since), (State::Working, 1000));
        s.apply(claude("SessionEnd", ""), 1100, None);
        assert!(s.list.is_empty());
    }

    #[test]
    fn finished_and_silent_sessions_leave_and_the_list_orders_by_need() {
        let mut s = Sessions::default();
        let ev = |id: &str, state| Event { tool: Tool::Claude, session: id.into(), state, cwd: format!("/p/{id}"), message: String::new() };
        s.apply(ev("a", State::Done), 0, None);
        s.apply(ev("b", State::Working), 10, None);
        s.apply(ev("c", State::Waiting), 20, None);
        let order: Vec<&str> = s.ordered().iter().map(|x| x.title.as_str()).collect();
        assert_eq!(order, ["c", "b", "a"]);
        s.prune(DONE_KEPT_S);
        assert!(s.get("Claude:a").is_none() && s.get("Claude:b").is_some());
        s.prune(10 + STALE_S);
        assert!(s.get("Claude:b").is_none());
        s.remove("Claude:c");
        assert!(s.list.is_empty());
    }

    #[test]
    fn setup_snippets_name_this_exe() {
        let j = claude_hooks_json(r"C:\Users\me\AppData\Local\Programs\Poni\poni.exe");
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        let cmd = v["hooks"]["Stop"][0]["hooks"][0]["command"].as_str().unwrap();
        assert_eq!(cmd, "\"C:/Users/me/AppData/Local/Programs/Poni/poni.exe\" --agent claude");
        assert!(v["hooks"]["UserPromptSubmit"].is_array() && v["hooks"]["Notification"].is_array() && v["hooks"]["SessionEnd"].is_array());
        assert_eq!(codex_notify_line(r"C:\P\poni.exe"), r#"notify = ["C:/P/poni.exe", "--agent", "codex"]"#);
        assert_eq!(folder_name("C:\\code\\oyenza-studio\\"), "oyenza-studio");
        assert_eq!(folder_name(""), "session");
    }
}
