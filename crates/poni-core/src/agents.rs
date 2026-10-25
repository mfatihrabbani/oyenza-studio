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

/// The hook events Oyenza Studio listens to in Claude Code.
const CLAUDE_EVENTS: [&str; 4] = ["UserPromptSubmit", "Notification", "Stop", "SessionEnd"];
/// What marks a hook or a notify line as Oyenza Studio's.
const OURS: &str = "--agent";

fn claude_command(exe: &str) -> String {
    format!("\"{}\" --agent claude", exe.replace('\\', "/"))
}

fn is_ours(hook: &serde_json::Value) -> bool {
    hook.get("command").and_then(|c| c.as_str()).is_some_and(|c| c.contains(OURS) && c.contains("claude"))
}

/// Whether Claude Code's settings (`~/.claude/settings.json`) carry the hooks.
pub fn claude_connected(settings: &str) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(settings) else { return false };
    CLAUDE_EVENTS.iter().all(|e| {
        v["hooks"][e].as_array().is_some_and(|groups| groups.iter().any(|g| g["hooks"].as_array().is_some_and(|hs| hs.iter().any(is_ours))))
    })
}

/// Claude Code's settings with the island's hooks in, for this poni.exe:
/// every other setting and hook stays; one of ours already there gets this
/// exe's path. `settings` may be empty (no file yet). Refuses a file that
/// isn't a JSON object, rather than lose what's in it.
pub fn claude_settings_with_hooks(settings: &str, exe: &str) -> Result<String, String> {
    let mut v = parse_settings(settings)?;
    let command = claude_command(exe);
    let root = v.as_object_mut().ok_or("not a JSON object")?;
    let hooks = root.entry("hooks").or_insert_with(|| serde_json::json!({}));
    let hooks = hooks.as_object_mut().ok_or("\"hooks\" is not an object")?;
    for event in CLAUDE_EVENTS {
        let groups = hooks.entry(event).or_insert_with(|| serde_json::json!([]));
        let groups = groups.as_array_mut().ok_or_else(|| format!("\"{event}\" is not a list"))?;
        let mut found = false;
        for g in groups.iter_mut() {
            if let Some(hs) = g.get_mut("hooks").and_then(|h| h.as_array_mut()) {
                for h in hs.iter_mut().filter(|h| is_ours(h)) {
                    h["command"] = serde_json::Value::String(command.clone());
                    found = true;
                }
            }
        }
        if !found {
            groups.push(serde_json::json!({ "hooks": [{ "type": "command", "command": command }] }));
        }
    }
    serde_json::to_string_pretty(&v).map_err(|e| e.to_string())
}

/// Claude Code's settings without the island's hooks (groups and lists
/// left empty by that go too).
pub fn claude_settings_without_hooks(settings: &str) -> Result<String, String> {
    let mut v = parse_settings(settings)?;
    if let Some(hooks) = v.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        for event in CLAUDE_EVENTS {
            let Some(groups) = hooks.get_mut(event).and_then(|g| g.as_array_mut()) else { continue };
            for g in groups.iter_mut() {
                if let Some(hs) = g.get_mut("hooks").and_then(|h| h.as_array_mut()) {
                    hs.retain(|h| !is_ours(h));
                }
            }
            groups.retain(|g| g.get("hooks").and_then(|h| h.as_array()).is_none_or(|hs| !hs.is_empty()));
            if groups.is_empty() {
                hooks.remove(event);
            }
        }
        if hooks.is_empty() {
            if let Some(root) = v.as_object_mut() {
                root.remove("hooks");
            }
        }
    }
    serde_json::to_string_pretty(&v).map_err(|e| e.to_string())
}

fn parse_settings(settings: &str) -> Result<serde_json::Value, String> {
    if settings.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    let v: serde_json::Value = serde_json::from_str(settings).map_err(|e| format!("settings.json has an error ({e})"))?;
    if v.is_object() {
        Ok(v)
    } else {
        Err("settings.json is not a JSON object".into())
    }
}

/// The top-level `notify` line of a Codex config, if there is one: its
/// index and text. Tables start at the first `[...]` header.
fn codex_notify_at(config: &str) -> Option<(usize, &str)> {
    for (i, line) in config.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with('[') {
            return None;
        }
        if let Some(rest) = t.strip_prefix("notify") {
            if rest.trim_start().starts_with('=') {
                return Some((i, line));
            }
        }
    }
    None
}

/// Whether Codex's config (`~/.codex/config.toml`) runs the island on `notify`.
pub fn codex_connected(config: &str) -> bool {
    codex_notify_at(config).is_some_and(|(_, l)| l.contains(OURS))
}

/// Codex's config with the island's `notify` line, for this poni.exe. Its
/// own line is replaced; another program's `notify` is left alone (Codex
/// runs only one), and that's an error to show.
pub fn codex_config_with_notify(config: &str, exe: &str) -> Result<String, String> {
    let line = codex_notify_line(exe);
    let mut lines: Vec<&str> = config.lines().collect();
    match codex_notify_at(config) {
        Some((i, l)) if l.contains(OURS) => lines[i] = line.as_str(),
        Some(_) => return Err("Codex already runs another notify program".into()),
        // Top-level keys go before the first table.
        None => lines.insert(0, line.as_str()),
    }
    Ok(lines.join("\n") + "\n")
}

/// Codex's config without the island's `notify` line.
pub fn codex_config_without_notify(config: &str) -> String {
    match codex_notify_at(config) {
        Some((i, l)) if l.contains(OURS) => {
            let mut lines: Vec<&str> = config.lines().collect();
            lines.remove(i);
            if lines.is_empty() {
                String::new()
            } else {
                lines.join("\n") + "\n"
            }
        }
        _ => config.to_string(),
    }
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
    fn claude_settings_get_the_hooks_and_keep_everything_else() {
        let exe = r"C:\P\poni.exe";
        let before = r#"{"model":"opus","hooks":{"Stop":[{"hooks":[{"type":"command","command":"say done"}]}],"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"lint"}]}]}}"#;
        assert!(!claude_connected(before));
        let after = claude_settings_with_hooks(before, exe).unwrap();
        assert!(claude_connected(&after));
        let v: serde_json::Value = serde_json::from_str(&after).unwrap();
        assert_eq!(v["model"], "opus");
        assert_eq!(v["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "lint");
        // Their own Stop hook stays, ours joins it.
        let stop = v["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
        assert_eq!(stop[0]["hooks"][0]["command"], "say done");
        assert_eq!(stop[1]["hooks"][0]["command"], "\"C:/P/poni.exe\" --agent claude");
        // Connecting again (a moved exe) updates ours instead of adding.
        let again = claude_settings_with_hooks(&after, r"D:\New\poni.exe").unwrap();
        let v2: serde_json::Value = serde_json::from_str(&again).unwrap();
        assert_eq!(v2["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert_eq!(v2["hooks"]["Stop"][1]["hooks"][0]["command"], "\"D:/New/poni.exe\" --agent claude");
        // Removing leaves theirs and drops what's left empty.
        let off = claude_settings_without_hooks(&again).unwrap();
        assert!(!claude_connected(&off));
        let v3: serde_json::Value = serde_json::from_str(&off).unwrap();
        assert_eq!(v3["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert!(v3["hooks"].get("UserPromptSubmit").is_none());
        assert_eq!(v3["hooks"]["PreToolUse"][0]["matcher"], "Bash");
        // No file yet, and a broken one.
        assert!(claude_connected(&claude_settings_with_hooks("", exe).unwrap()));
        assert_eq!(claude_settings_without_hooks(&claude_settings_with_hooks("", exe).unwrap()).unwrap(), "{}");
        assert!(claude_settings_with_hooks("{ oops", exe).is_err());
        assert!(claude_settings_with_hooks("[1]", exe).is_err());
    }

    #[test]
    fn codex_config_gets_one_notify_line_at_the_top_level() {
        let exe = r"C:\P\poni.exe";
        let before = "model = \"o4\"\n\n[mcp_servers.x]\ncommand = \"y\"\nnotify = \"inside a table\"\n";
        assert!(!codex_connected(before));
        let after = codex_config_with_notify(before, exe).unwrap();
        assert!(codex_connected(&after));
        assert!(after.starts_with("notify = [\"C:/P/poni.exe\", \"--agent\", \"codex\"]\nmodel"));
        assert!(after.contains("[mcp_servers.x]\ncommand = \"y\"\nnotify = \"inside a table\""));
        // Again: replaced, not doubled.
        let again = codex_config_with_notify(&after, r"D:\poni.exe").unwrap();
        assert_eq!(again.matches("--agent").count(), 1);
        assert!(again.contains("D:/poni.exe"));
        // Someone else's notify stays theirs.
        assert!(codex_config_with_notify("notify = [\"other\"]\n", exe).is_err());
        // Off: our line goes, the rest stays.
        assert_eq!(codex_config_without_notify(&again), before);
        assert_eq!(codex_config_without_notify(&codex_config_with_notify("", exe).unwrap()), "");
        assert_eq!(codex_config_without_notify("notify = [\"other\"]\n"), "notify = [\"other\"]\n");
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
