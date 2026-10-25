//! Meetings from a calendar's secret iCal address (Google Calendar's
//! "Secret address in iCal format", or any other .ics link), for the notice
//! the island shows a few minutes before each one, with Join for its video
//! call. Read only: no sign-in, no Google Cloud project, nothing written back.
//!
//! The Windows crate fetches the file (`services::calendar`); this parses it:
//! unfolded lines, VTIMEZONE offsets (with yearly daylight-saving rules), and
//! the recurring events people actually use (daily, weekly, monthly and
//! yearly rules with INTERVAL, COUNT, UNTIL, BYDAY and BYMONTHDAY, EXDATE,
//! moved or cancelled single instances). A rule with parts this doesn't know
//! (BYSETPOS, hourly) keeps only its first time rather than guessing. All-day
//! events and meetings the calendar's owner declined are left out.

use crate::datetime::{civil_from_days, days_from_civil, days_in_month, fmt_clock, fmt_date_short, weekday_from_days, LocalDateTime};
use std::collections::HashMap;

/// Where Google Calendar's "Secret address in iCal format" is (Google's help).
pub const HELP_URL: &str = "https://support.google.com/calendar/answer/37648";

/// How often the file is fetched while a link is saved and someone is at
/// the PC: a meeting added or moved shows up within this long.
pub const FETCH_EVERY_S: i64 = 15 * 60;
/// The meetings kept: from a little before now to this far ahead (the next
/// fetch comes long before they run out).
pub const AHEAD_S: i64 = 36 * 3600;
/// How long to wait before fetching again after `fails` failures in a row:
/// 2, 4 and 8 minutes, then the usual [`FETCH_EVERY_S`].
pub fn retry_after_s(fails: u32) -> i64 {
    match fails {
        0 => FETCH_EVERY_S,
        n => (60i64 << n.min(4)).min(FETCH_EVERY_S),
    }
}

/// The notice stays up until this long after the meeting starts.
pub const START_GRACE_S: i64 = 120;
/// A meeting with no end in the file lasts this long.
const DEFAULT_LEN_S: i64 = 30 * 60;
/// COUNT rules are counted from their first day; give up after this many days.
const COUNT_SCAN_DAYS: i64 = 3660;

/// One meeting (a single time of a recurring event is its own meeting).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meeting {
    /// Stays the same across fetches: the event's UID and its start.
    pub key: String,
    pub title: String,
    /// UTC seconds.
    pub start: i64,
    pub end: i64,
    /// The video call: Google Meet, Zoom, Teams or Webex.
    pub link: Option<String>,
}

/// The link the person copied, as a fetchable https address: `webcal://`
/// becomes `https://`. `None` for anything that isn't a calendar file link.
pub fn clean_link(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() || t.len() > 2000 || t.chars().any(char::is_whitespace) {
        return None;
    }
    let url = if let Some(rest) = t.strip_prefix("webcal://") { format!("https://{rest}") } else { t.to_string() };
    let lower = url.to_ascii_lowercase();
    let host = lower.strip_prefix("https://")?.split(['/', '?']).next().unwrap_or("");
    if host.is_empty() || !host.contains('.') {
        return None;
    }
    let path = lower.split('?').next().unwrap_or("");
    (path.ends_with(".ics") || path.contains("/ical/")).then_some(url)
}

/// The meetings in `ics` that are still running or start before `to` (UTC
/// seconds), soonest first. `local_offset_min` is the PC's offset from UTC,
/// for times the file gives without a zone it describes.
pub fn meetings(ics: &str, from: i64, to: i64, local_offset_min: i32) -> Vec<Meeting> {
    let cal = parse(ics);
    let zones = Zones { zones: &cal.zones, fallback_min: local_offset_min };
    let owner = cal.owner.to_ascii_lowercase();
    // Single times moved or cancelled: the master skips them.
    let mut overridden: HashMap<&str, Vec<i64>> = HashMap::new();
    for e in &cal.events {
        if let Some(rid) = &e.recurrence_id {
            overridden.entry(e.uid.as_str()).or_default().push(zones.utc(rid));
        }
    }
    let mut out = Vec::new();
    for e in &cal.events {
        let Some(start) = &e.start else { continue };
        if e.all_day || e.cancelled || (!owner.is_empty() && e.declined_by.iter().any(|a| *a == owner)) {
            continue;
        }
        let len = match (&e.end, e.duration) {
            (Some(end), _) => (zones.utc(end) - zones.utc(start)).max(0),
            (None, Some(d)) => d.max(0),
            (None, None) => DEFAULT_LEN_S,
        };
        // A moved instance stands on its own; its master skips the time it replaced.
        let moved = overridden.get(e.uid.as_str()).filter(|_| e.recurrence_id.is_none());
        let skip: Vec<i64> = e.exdates.iter().map(|t| zones.utc(t)).chain(moved.into_iter().flatten().copied()).collect();
        let mut push = |s: i64| {
            if s + len.max(START_GRACE_S) > from && s < to && !skip.contains(&s) {
                out.push(Meeting { key: format!("{}@{s}", e.uid), title: e.title.clone(), start: s, end: s + len, link: e.link.clone() });
            }
        };
        match e.rule.as_ref().filter(|_| e.recurrence_id.is_none()) {
            None => push(zones.utc(start)),
            Some(rule) => {
                for s in occurrences(rule, start, &zones, from - len.max(START_GRACE_S), to) {
                    push(s);
                }
            }
        }
    }
    out.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.title.cmp(&b.title)));
    out.dedup_by(|a, b| a.key == b.key);
    out
}

/// The meeting whose notice is due at `now`: from `before_min` minutes
/// before it starts until [`START_GRACE_S`] after, not yet in `reminded`.
pub fn to_remind<'a>(meetings: &'a [Meeting], now: i64, before_min: u32, reminded: &[String]) -> Option<&'a Meeting> {
    let before = before_min as i64 * 60;
    meetings.iter().find(|m| m.start - before <= now && now < m.start + START_GRACE_S && !reminded.contains(&m.key))
}

/// Minutes until `m` starts, rounded up (0 once it has started).
pub fn minutes_left(m: &Meeting, now: i64) -> i64 {
    ((m.start - now).max(0) + 59) / 60
}

/// The notice's second line: "In 5 min · 10:00", then "Now · 10:00".
pub fn notice_subtitle(m: &Meeting, now: i64, local_offset_min: i32, use_24h: bool) -> String {
    let clock = local_clock(m.start, local_offset_min, use_24h);
    match minutes_left(m, now) {
        0 => format!("Now \u{b7} {clock}"),
        n => format!("In {n} min \u{b7} {clock}"),
    }
}

/// A UTC time on the PC's wall clock.
fn local_time(t: i64, local_offset_min: i32) -> LocalDateTime {
    let local = t + local_offset_min as i64 * 60;
    let (y, mo, d) = civil_from_days(local.div_euclid(86400));
    let sod = local.rem_euclid(86400);
    LocalDateTime { year: y, month: mo, day: d, hour: (sod / 3600) as u8, minute: (sod / 60 % 60) as u8, second: (sod % 60) as u8 }
}

/// "10:00" (or "10:00 AM") for a UTC time on the PC's clock.
pub fn local_clock(t: i64, local_offset_min: i32, use_24h: bool) -> String {
    fmt_clock(&local_time(t, local_offset_min), use_24h)
}

/// "10:00 – 10:30", the Calendar page's time for a meeting.
pub fn span(m: &Meeting, local_offset_min: i32, use_24h: bool) -> String {
    format!("{} \u{2013} {}", local_clock(m.start, local_offset_min, use_24h), local_clock(m.end, local_offset_min, use_24h))
}

/// The day of a later meeting and its time: "Tomorrow 09:00", "Tue 27 Oct 09:00".
pub fn later_label(m: &Meeting, now: i64, local_offset_min: i32, use_24h: bool) -> String {
    let off = local_offset_min as i64 * 60;
    let day = (m.start + off).div_euclid(86400) - (now + off).div_euclid(86400);
    let clock = local_clock(m.start, local_offset_min, use_24h);
    match day {
        0 => clock,
        1 => format!("Tomorrow {clock}"),
        _ => format!("{} {clock}", fmt_date_short(&local_time(m.start, local_offset_min))),
    }
}

/// What the Calendar page lists: today's meetings that aren't over yet
/// (soonest first, one under way included), and the first one after today.
pub fn today(meetings: &[Meeting], now: i64, local_offset_min: i32) -> (Vec<&Meeting>, Option<&Meeting>) {
    let off = local_offset_min as i64 * 60;
    let day = (now + off).div_euclid(86400);
    let rest = meetings.iter().filter(|m| m.end > now && (m.start + off).div_euclid(86400) <= day).collect();
    let next = meetings.iter().find(|m| (m.start + off).div_euclid(86400) > day);
    (rest, next)
}

/// Under way at `now`.
pub fn live(m: &Meeting, now: i64) -> bool {
    m.start <= now && now < m.end
}

// ------------------------------------------------------------ the file

/// A time as the file gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Time {
    Utc(i64),
    /// Wall-clock seconds (days since 1970 × 86400 + second of the day) in
    /// the named zone, or the PC's zone when it names none.
    Local { secs: i64, tz: Option<String> },
}

#[derive(Debug, Default)]
struct Event {
    uid: String,
    title: String,
    start: Option<Time>,
    end: Option<Time>,
    duration: Option<i64>,
    all_day: bool,
    rule: Option<Rule>,
    exdates: Vec<Time>,
    recurrence_id: Option<Time>,
    cancelled: bool,
    /// Attendees (lower-case addresses) who said no.
    declined_by: Vec<String>,
    link: Option<String>,
}

#[derive(Debug, Default)]
struct Calendar {
    /// Google names a primary calendar after its owner's address.
    owner: String,
    zones: HashMap<String, Vec<Observance>>,
    events: Vec<Event>,
}

/// One part of a VTIMEZONE (STANDARD or DAYLIGHT).
#[derive(Clone, Debug, Default)]
struct Observance {
    from_min: i32,
    to_min: i32,
    /// Wall-clock seconds it first applies.
    start: i64,
    /// Yearly: (month, nth weekday, weekday 0 = Sunday), e.g. (3, -1, 0).
    yearly: Option<(u8, i8, u8)>,
}

/// One content line: NAME;PARAM=VALUE:VALUE.
struct Line<'a> {
    name: String,
    params: Vec<(String, &'a str)>,
    value: &'a str,
}

impl Line<'_> {
    fn param(&self, key: &str) -> Option<&str> {
        self.params.iter().find(|(k, _)| k == key).map(|(_, v)| v.trim_matches('"'))
    }
}

/// Lines folded onto the next ones (CRLF and a space or tab) joined back.
fn unfold(ics: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in ics.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        match (raw.strip_prefix(' ').or_else(|| raw.strip_prefix('\t')), out.last_mut()) {
            (Some(rest), Some(last)) => last.push_str(rest),
            _ => out.push(raw.to_string()),
        }
    }
    out
}

fn split_line(s: &str) -> Option<Line<'_>> {
    // The value starts at the first ':' outside quotes.
    let mut quoted = false;
    let colon = s.char_indices().find(|&(_, c)| {
        if c == '"' {
            quoted = !quoted;
        }
        c == ':' && !quoted
    })?;
    let (head, value) = (&s[..colon.0], &s[colon.0 + 1..]);
    let mut parts = Vec::new();
    let (mut quoted, mut begin) = (false, 0);
    for (i, c) in head.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ';' if !quoted => {
                parts.push(&head[begin..i]);
                begin = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&head[begin..]);
    let name = parts[0].trim().to_ascii_uppercase();
    let params = parts[1..].iter().filter_map(|p| p.split_once('=')).map(|(k, v)| (k.trim().to_ascii_uppercase(), v)).collect();
    Some(Line { name, params, value })
}

/// TEXT values: \n, \, \; and \\ escapes.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

fn parse(ics: &str) -> Calendar {
    let mut cal = Calendar::default();
    let mut stack: Vec<String> = Vec::new();
    let mut ev: Option<Event> = None;
    let mut zone_id = String::new();
    let mut zone: Vec<Observance> = Vec::new();
    let mut obs: Option<Observance> = None;
    let mut texts: Vec<String> = Vec::new();
    for raw in unfold(ics) {
        let Some(l) = split_line(&raw) else { continue };
        let inner = stack.last().map(String::as_str);
        match l.name.as_str() {
            "BEGIN" => {
                let what = l.value.trim().to_ascii_uppercase();
                match what.as_str() {
                    "VEVENT" => {
                        ev = Some(Event::default());
                        texts.clear();
                    }
                    "VTIMEZONE" => {
                        zone_id.clear();
                        zone.clear();
                    }
                    "STANDARD" | "DAYLIGHT" => obs = Some(Observance::default()),
                    _ => {}
                }
                stack.push(what);
                continue;
            }
            "END" => {
                match stack.pop().as_deref() {
                    Some("VEVENT") => {
                        if let Some(mut e) = ev.take() {
                            e.link = texts.iter().find_map(|t| call_link(t));
                            if e.title.trim().is_empty() {
                                e.title = "Meeting".into();
                            }
                            cal.events.push(e);
                        }
                    }
                    Some("VTIMEZONE") if !zone_id.is_empty() => {
                        cal.zones.insert(std::mem::take(&mut zone_id), std::mem::take(&mut zone));
                    }
                    Some("STANDARD" | "DAYLIGHT") => zone.extend(obs.take()),
                    _ => {}
                }
                continue;
            }
            _ => {}
        }
        match inner {
            Some("VCALENDAR") if l.name == "X-WR-CALNAME" => cal.owner = unescape(l.value).trim().to_string(),
            Some("VTIMEZONE") if l.name == "TZID" => zone_id = l.value.trim().to_string(),
            Some("STANDARD" | "DAYLIGHT") => {
                let Some(o) = obs.as_mut() else { continue };
                match l.name.as_str() {
                    "TZOFFSETFROM" => o.from_min = parse_offset(l.value).unwrap_or(0),
                    "TZOFFSETTO" => o.to_min = parse_offset(l.value).unwrap_or(0),
                    "DTSTART" => o.start = parse_local(l.value.trim()).map_or(0, |(s, _)| s),
                    "RRULE" => o.yearly = yearly_rule(l.value),
                    _ => {}
                }
            }
            Some("VEVENT") => {
                let Some(e) = ev.as_mut() else { continue };
                match l.name.as_str() {
                    "UID" => e.uid = l.value.trim().to_string(),
                    "SUMMARY" => e.title = unescape(l.value).trim().to_string(),
                    "DTSTART" => {
                        e.all_day = l.param("VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE")) || !l.value.contains('T');
                        e.start = parse_time(l.value, l.param("TZID"));
                    }
                    "DTEND" => e.end = parse_time(l.value, l.param("TZID")),
                    "DURATION" => e.duration = parse_duration(l.value),
                    "RRULE" => e.rule = Rule::parse(l.value),
                    "EXDATE" => e.exdates.extend(l.value.split(',').filter_map(|v| parse_time(v, l.param("TZID")))),
                    "RECURRENCE-ID" => e.recurrence_id = parse_time(l.value, l.param("TZID")),
                    "STATUS" => e.cancelled = l.value.trim().eq_ignore_ascii_case("CANCELLED"),
                    "ATTENDEE" => {
                        if l.param("PARTSTAT").is_some_and(|p| p.eq_ignore_ascii_case("DECLINED")) {
                            let who = l.value.trim();
                            let who = who.strip_prefix("mailto:").or_else(|| who.strip_prefix("MAILTO:")).unwrap_or(who);
                            e.declined_by.push(who.to_ascii_lowercase());
                        }
                    }
                    // Where the call's link may be, the likeliest first.
                    "X-GOOGLE-CONFERENCE" => texts.insert(0, l.value.to_string()),
                    "LOCATION" | "DESCRIPTION" | "URL" => texts.push(unescape(l.value)),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    cal
}

/// "20261025T100000Z", "20261025T100000" (with its TZID or none) or a
/// date alone, "20261025" (midnight).
fn parse_time(v: &str, tz: Option<&str>) -> Option<Time> {
    let v = v.trim();
    let (secs, utc) = parse_local(v)?;
    Some(if utc { Time::Utc(secs) } else { Time::Local { secs, tz: tz.map(str::to_string) } })
}

/// Wall-clock seconds and whether the value ended in Z.
fn parse_local(v: &str) -> Option<(i64, bool)> {
    let num = |s: &str| -> Option<i64> { s.bytes().all(|b| b.is_ascii_digit()).then(|| s.parse().ok()).flatten() };
    let date = v.get(0..8)?;
    let (y, m, d) = (num(&date[0..4])?, num(&date[4..6])?, num(&date[6..8])?);
    if !(1..=12).contains(&m) || d < 1 || d > days_in_month(y as i32, m as u8) as i64 {
        return None;
    }
    let days = days_from_civil(y as i32, m as u8, d as u8);
    let rest = &v[8..];
    if rest.is_empty() {
        return Some((days * 86400, false));
    }
    let t = rest.strip_prefix('T')?;
    let (hms, utc) = match t.strip_suffix('Z') {
        Some(h) => (h, true),
        None => (t, false),
    };
    if hms.len() != 6 || !hms.is_ascii() {
        return None;
    }
    let (h, mi, s) = (num(&hms[0..2])?, num(&hms[2..4])?, num(&hms[4..6])?);
    Some((days * 86400 + h * 3600 + mi * 60 + s.min(59), utc))
}

/// "+0700", "-0430", "+053000" in minutes.
fn parse_offset(v: &str) -> Option<i32> {
    let v = v.trim();
    let (sign, digits) = match v.as_bytes().first()? {
        b'+' => (1, &v[1..]),
        b'-' => (-1, &v[1..]),
        _ => (1, v),
    };
    if digits.len() < 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let h: i32 = digits[0..2].parse().ok()?;
    let m: i32 = digits[2..4].parse().ok()?;
    Some(sign * (h * 60 + m))
}

/// "PT30M", "PT1H30M", "P1D" in seconds.
fn parse_duration(v: &str) -> Option<i64> {
    let v = v.trim();
    let (sign, v) = match v.strip_prefix('-') {
        Some(r) => (-1, r),
        None => (1, v.strip_prefix('+').unwrap_or(v)),
    };
    let v = v.strip_prefix('P')?;
    let (mut total, mut n) = (0i64, 0i64);
    for c in v.chars() {
        match c {
            '0'..='9' => n = n * 10 + c.to_digit(10)? as i64,
            'W' => total += n * 7 * 86400,
            'D' => total += n * 86400,
            'H' => total += n * 3600,
            'M' => total += n * 60,
            'S' => total += n,
            'T' => {}
            _ => return None,
        }
        if c.is_ascii_alphabetic() {
            n = 0;
        }
    }
    Some(sign * total)
}

/// A daylight-saving switch's RRULE: FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU.
fn yearly_rule(v: &str) -> Option<(u8, i8, u8)> {
    let r = Rule::parse(v)?;
    if r.freq != Freq::Yearly || r.by_month.len() != 1 || r.by_day.len() != 1 {
        return None;
    }
    let (nth, wd) = r.by_day[0];
    Some((r.by_month[0], if nth == 0 { 1 } else { nth }, wd))
}

/// A video call's link in `text`: the first Google Meet, Zoom, Teams or
/// Webex address. Other links (documents, maps) are never opened.
fn call_link(text: &str) -> Option<String> {
    const HOSTS: [&str; 5] = ["meet.google.com", "zoom.us", "teams.microsoft.com", "teams.live.com", "webex.com"];
    let mut rest = text;
    while let Some(i) = rest.find("https://") {
        let tail = &rest[i..];
        let end = tail.find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')' | ']' | '\\')).unwrap_or(tail.len());
        let url = tail[..end].trim_end_matches(['.', ',', ';']);
        let host = url["https://".len()..].split(['/', '?', '#']).next().unwrap_or("").to_ascii_lowercase();
        if HOSTS.iter().any(|h| host == *h || host.ends_with(&format!(".{h}"))) {
            return Some(url.to_string());
        }
        rest = &tail[end.max(1)..];
    }
    None
}

// ------------------------------------------------------------ time zones

struct Zones<'a> {
    zones: &'a HashMap<String, Vec<Observance>>,
    fallback_min: i32,
}

impl Zones<'_> {
    fn utc(&self, t: &Time) -> i64 {
        match t {
            Time::Utc(s) => *s,
            Time::Local { secs, tz } => secs - self.offset_min(tz.as_deref(), *secs) as i64 * 60,
        }
    }

    /// The zone's offset at a wall-clock time: the observance that began
    /// last. A zone the file doesn't describe is taken as the PC's.
    fn offset_min(&self, tz: Option<&str>, local: i64) -> i32 {
        let Some(obs) = tz.and_then(|id| self.zones.get(id)).filter(|o| !o.is_empty()) else { return self.fallback_min };
        let year = civil_from_days(local.div_euclid(86400)).0;
        let mut best: Option<(i64, i32)> = None;
        for o in obs {
            let onsets: Vec<i64> = match o.yearly {
                None => vec![o.start],
                Some((month, nth, wd)) => {
                    let sod = o.start.rem_euclid(86400);
                    [year - 1, year].iter().filter_map(|&y| nth_weekday(y, month, nth, wd)).map(|d| d * 86400 + sod).filter(|&s| s >= o.start).collect()
                }
            };
            for s in onsets.into_iter().filter(|&s| s <= local) {
                if best.is_none_or(|(b, _)| s > b) {
                    best = Some((s, o.to_min));
                }
            }
        }
        best.map_or_else(|| obs.iter().min_by_key(|o| o.start).map_or(self.fallback_min, |o| o.from_min), |(_, m)| m)
    }
}

/// Days since 1970 of the `nth` (1-based; negative counts from the end)
/// weekday `wd` (0 = Sunday) of a month.
fn nth_weekday(year: i32, month: u8, nth: i8, wd: u8) -> Option<i64> {
    if !(1..=12).contains(&month) || nth == 0 {
        return None;
    }
    let first = days_from_civil(year, month, 1);
    let dim = days_in_month(year, month) as i64;
    let d = if nth > 0 {
        first + (wd as i64 - weekday_from_days(first) as i64).rem_euclid(7) + (nth as i64 - 1) * 7
    } else {
        let last = first + dim - 1;
        last - (weekday_from_days(last) as i64 - wd as i64).rem_euclid(7) - (-(nth as i64) - 1) * 7
    };
    (d >= first && d < first + dim).then_some(d)
}

// ------------------------------------------------------------ recurrence

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Clone, Debug, PartialEq)]
struct Rule {
    freq: Freq,
    interval: i64,
    count: Option<u32>,
    until: Option<Time>,
    /// (nth, weekday 0 = Sunday); nth 0 = every such weekday.
    by_day: Vec<(i8, u8)>,
    by_month_day: Vec<i8>,
    by_month: Vec<u8>,
}

impl Rule {
    /// `None` for rules this can't follow faithfully.
    fn parse(v: &str) -> Option<Rule> {
        let mut r = Rule { freq: Freq::Daily, interval: 1, count: None, until: None, by_day: vec![], by_month_day: vec![], by_month: vec![] };
        let mut freq = None;
        for part in v.trim().split(';').filter(|p| !p.is_empty()) {
            let (k, val) = part.split_once('=')?;
            match k.trim().to_ascii_uppercase().as_str() {
                "FREQ" => {
                    freq = Some(match val.trim().to_ascii_uppercase().as_str() {
                        "DAILY" => Freq::Daily,
                        "WEEKLY" => Freq::Weekly,
                        "MONTHLY" => Freq::Monthly,
                        "YEARLY" => Freq::Yearly,
                        _ => return None,
                    })
                }
                "INTERVAL" => r.interval = val.trim().parse::<i64>().ok().filter(|n| *n >= 1)?,
                "COUNT" => r.count = Some(val.trim().parse().ok()?),
                "UNTIL" => r.until = Some(parse_time(val, None)?),
                "BYDAY" => {
                    for d in val.split(',') {
                        let d = d.trim().to_ascii_uppercase();
                        if !d.is_ascii() {
                            return None;
                        }
                        let (n, name) = d.split_at(d.len().checked_sub(2)?);
                        let wd = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"].iter().position(|w| *w == name)? as u8;
                        let nth = if n.is_empty() { 0 } else { n.trim_start_matches('+').parse::<i8>().ok()? };
                        r.by_day.push((nth, wd));
                    }
                }
                "BYMONTHDAY" => {
                    for d in val.split(',') {
                        r.by_month_day.push(d.trim().parse::<i8>().ok().filter(|n| *n != 0 && n.abs() <= 31)?);
                    }
                }
                "BYMONTH" => {
                    for m in val.split(',') {
                        r.by_month.push(m.trim().parse::<u8>().ok().filter(|m| (1..=12).contains(m))?);
                    }
                }
                // Weeks start on Monday, as Google writes them.
                "WKST" => {}
                _ => return None,
            }
        }
        r.freq = freq?;
        Some(r)
    }

    /// Whether the rule, starting on day `start` (days since 1970), has a
    /// time on day `d`.
    fn on_day(&self, start: i64, d: i64) -> bool {
        if d < start {
            return false;
        }
        let (sy, sm, sd) = civil_from_days(start);
        let (y, m, day) = civil_from_days(d);
        let wd = weekday_from_days(d);
        let dim = days_in_month(y, m) as i64;
        let month_day_ok = |list: &[i8]| list.iter().any(|&md| if md > 0 { md as i64 == day as i64 } else { dim + md as i64 + 1 == day as i64 });
        let nth_ok = |nth: i8| match nth {
            0 => true,
            n if n > 0 => (day as i64 - 1) / 7 + 1 == n as i64,
            n => (dim - day as i64) / 7 + 1 == -(n as i64),
        };
        let by_day_ok = || self.by_day.iter().any(|&(nth, w)| w == wd && nth_ok(nth));
        if !self.by_month.is_empty() && !self.by_month.contains(&m) {
            return false;
        }
        match self.freq {
            Freq::Daily => {
                (d - start) % self.interval == 0 && (self.by_day.is_empty() || self.by_day.iter().any(|&(_, w)| w == wd)) && (self.by_month_day.is_empty() || month_day_ok(self.by_month_day.as_slice()))
            }
            Freq::Weekly => {
                let monday = |x: i64| x - (weekday_from_days(x) as i64 + 6) % 7;
                let weeks = (monday(d) - monday(start)) / 7;
                let days_ok = if self.by_day.is_empty() { wd == weekday_from_days(start) } else { self.by_day.iter().any(|&(_, w)| w == wd) };
                weeks % self.interval == 0 && days_ok
            }
            Freq::Monthly => {
                let months = (y as i64 * 12 + m as i64) - (sy as i64 * 12 + sm as i64);
                if months % self.interval != 0 {
                    return false;
                }
                match (self.by_day.is_empty(), self.by_month_day.is_empty()) {
                    (true, true) => day == sd,
                    (false, true) => by_day_ok(),
                    (true, false) => month_day_ok(self.by_month_day.as_slice()),
                    (false, false) => by_day_ok() && month_day_ok(self.by_month_day.as_slice()),
                }
            }
            Freq::Yearly => {
                if (y - sy) as i64 % self.interval != 0 {
                    return false;
                }
                if self.by_month.is_empty() && m != sm {
                    return false;
                }
                match (self.by_day.is_empty(), self.by_month_day.is_empty()) {
                    (true, true) => day == sd,
                    (false, _) => by_day_ok() && (self.by_month_day.is_empty() || month_day_ok(self.by_month_day.as_slice())),
                    (true, false) => month_day_ok(self.by_month_day.as_slice()),
                }
            }
        }
    }
}

/// The UTC starts of `rule`'s times (first at `start`) between `from` and `to`.
fn occurrences(rule: &Rule, start: &Time, zones: &Zones, from: i64, to: i64) -> Vec<i64> {
    // Days are counted on the event's own wall clock.
    let (start_local, tz) = match start {
        Time::Local { secs, tz } => (*secs, tz.clone()),
        // A UTC start repeats in UTC.
        Time::Utc(s) => (*s, Some(String::new())),
    };
    let to_utc = |local: i64| match &tz {
        Some(id) if id.is_empty() => local,
        _ => zones.utc(&Time::Local { secs: local, tz: tz.clone() }),
    };
    let (first_day, sod) = (start_local.div_euclid(86400), start_local.rem_euclid(86400));
    let until = rule.until.as_ref().map(|u| match u {
        Time::Utc(s) => *s,
        // A date alone: the whole of that day counts.
        Time::Local { secs, .. } if secs.rem_euclid(86400) == 0 => to_utc(secs + 86399),
        Time::Local { secs, .. } => to_utc(*secs),
    });
    // The window's days, a day wider each way for the zone's offset.
    let lo = (from.div_euclid(86400) - 1).max(first_day);
    let hi = to.div_euclid(86400) + 1;
    let mut out = Vec::new();
    // COUNT: how many times came before `lo`.
    let mut seen = 0u32;
    if let Some(count) = rule.count {
        let mut d = first_day;
        while d < lo && seen < count && d - first_day <= COUNT_SCAN_DAYS {
            seen += (d == first_day || rule.on_day(first_day, d)) as u32;
            d += 1;
        }
    }
    // The start itself is always a time, even if the rule wouldn't pick its day.
    let mut d = lo;
    while d <= hi {
        if d == first_day || rule.on_day(first_day, d) {
            if rule.count.is_some_and(|c| seen >= c) {
                break;
            }
            seen += 1;
            let s = to_utc(d * 86400 + sod);
            if until.is_some_and(|u| s > u) {
                break;
            }
            if s >= from && s < to {
                out.push(s);
            }
        }
        d += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UTC seconds of a wall-clock time at UTC+`off_h`.
    fn at(y: i32, mo: u8, d: u8, h: i64, mi: i64, off_h: i64) -> i64 {
        days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 - off_h * 3600
    }

    const JAKARTA: &str = "BEGIN:VTIMEZONE\r\nTZID:Asia/Jakarta\r\nX-LIC-LOCATION:Asia/Jakarta\r\nBEGIN:STANDARD\r\nTZOFFSETFROM:+0700\r\nTZOFFSETTO:+0700\r\nTZNAME:WIB\r\nDTSTART:19700101T000000\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n";
    const NEW_YORK: &str = "BEGIN:VTIMEZONE\r\nTZID:America/New_York\r\nBEGIN:DAYLIGHT\r\nTZOFFSETFROM:-0500\r\nTZOFFSETTO:-0400\r\nTZNAME:EDT\r\nDTSTART:19700308T020000\r\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU\r\nEND:DAYLIGHT\r\nBEGIN:STANDARD\r\nTZOFFSETFROM:-0400\r\nTZOFFSETTO:-0500\r\nTZNAME:EST\r\nDTSTART:19701101T020000\r\nRRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n";

    fn cal(zones: &str, events: &str) -> String {
        format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Google Inc//Google Calendar 70.9054//EN\r\nX-WR-CALNAME:rani@example.com\r\nX-WR-TIMEZONE:Asia/Jakarta\r\n{zones}{events}END:VCALENDAR\r\n")
    }

    fn event(lines: &str) -> String {
        format!("BEGIN:VEVENT\r\n{lines}END:VEVENT\r\n")
    }

    fn day(y: i32, mo: u8, d: u8) -> (i64, i64) {
        (at(y, mo, d, 0, 0, 7), at(y, mo, d, 0, 0, 7) + 86400)
    }

    #[test]
    fn links_people_copy_are_cleaned_and_others_refused() {
        let g = "https://calendar.google.com/calendar/ical/rani%40example.com/private-0123abcd/basic.ics";
        assert_eq!(clean_link(&format!("  {g}\n")).as_deref(), Some(g));
        assert_eq!(clean_link("webcal://p01-caldav.icloud.com/published/2/abc.ics").as_deref(), Some("https://p01-caldav.icloud.com/published/2/abc.ics"));
        assert_eq!(clean_link("http://calendar.google.com/calendar/ical/x/basic.ics"), None, "https only");
        assert_eq!(clean_link("https://calendar.google.com/calendar/u/0/r"), None, "the web app, not the file");
        assert_eq!(clean_link("media-user-token AbC"), None);
        assert_eq!(clean_link(""), None);
    }

    #[test]
    fn a_single_meeting_with_its_meet_link() {
        let ics = cal(
            JAKARTA,
            &event(
                "DTSTART:20261026T020000Z\r\nDTEND:20261026T023000Z\r\nUID:abc123@google.com\r\nSUMMARY:Weekly sync\\, product\r\nDESCRIPTION:Agenda\\nJoin with Google Meet: https://meet.google.com/abc-defg-hij\\n\r\n Or dial: (US) +1 555\r\nLOCATION:\r\nSTATUS:CONFIRMED\r\n",
            ),
        );
        let (from, to) = day(2026, 10, 26);
        let ms = meetings(&ics, from, to, 420);
        assert_eq!(ms.len(), 1);
        let m = &ms[0];
        assert_eq!(m.title, "Weekly sync, product");
        assert_eq!((m.start, m.end), (at(2026, 10, 26, 9, 0, 7), at(2026, 10, 26, 9, 30, 7)));
        assert_eq!(m.link.as_deref(), Some("https://meet.google.com/abc-defg-hij"));
        assert_eq!(notice_subtitle(m, m.start - 5 * 60, 420, true), "In 5 min \u{b7} 09:00");
        assert_eq!(notice_subtitle(m, m.start - 4 * 60 - 1, 420, false), "In 5 min \u{b7} 9:00 AM");
        assert_eq!(notice_subtitle(m, m.start + 10, 420, true), "Now \u{b7} 09:00");
    }

    #[test]
    fn the_conference_line_wins_and_other_links_are_never_taken() {
        let ics = cal(
            "",
            &event("DTSTART:20261026T020000Z\r\nDURATION:PT45M\r\nUID:z@x\r\nSUMMARY:Client call\r\nDESCRIPTION:Doc: https://docs.google.com/document/d/1\\nZoom: https://acme.zoom.us/j/123?pwd=xy.\r\nX-GOOGLE-CONFERENCE:https://meet.google.com/zzz-zzzz-zzz\r\n"),
        );
        let ms = meetings(&ics, 0, i64::MAX / 2, 420);
        assert_eq!(ms[0].link.as_deref(), Some("https://meet.google.com/zzz-zzzz-zzz"));
        assert_eq!(ms[0].end - ms[0].start, 45 * 60);
        assert_eq!(call_link("Doc: https://docs.google.com/x and https://acme.zoom.us/j/123?pwd=xy."), Some("https://acme.zoom.us/j/123?pwd=xy".into()));
        assert_eq!(call_link("<a href=\"https://teams.microsoft.com/l/meetup-join/19%3a\">Join</a>"), Some("https://teams.microsoft.com/l/meetup-join/19%3a".into()));
        assert_eq!(call_link("https://evil.example.com/meet.google.com"), None);
        assert_eq!(call_link("no links"), None);
    }

    #[test]
    fn all_day_cancelled_and_declined_events_are_left_out() {
        let ics = cal(
            JAKARTA,
            &[
                event("DTSTART;VALUE=DATE:20261026\r\nDTEND;VALUE=DATE:20261027\r\nUID:allday\r\nSUMMARY:Holiday\r\n"),
                event("DTSTART:20261026T030000Z\r\nUID:gone\r\nSUMMARY:Cancelled\r\nSTATUS:CANCELLED\r\n"),
                event("DTSTART:20261026T040000Z\r\nUID:no\r\nSUMMARY:Declined\r\nATTENDEE;CUTYPE=INDIVIDUAL;ROLE=REQ-PARTICIPANT;PARTSTAT=DECLINED;CN=rani@example.com;X-NUM-GUESTS=0:mailto:Rani@example.com\r\n"),
                event("DTSTART:20261026T050000Z\r\nUID:yes\r\nSUMMARY:Accepted\r\nATTENDEE;PARTSTAT=DECLINED;CN=\"Budi: PM\":mailto:budi@example.com\r\nATTENDEE;PARTSTAT=ACCEPTED:mailto:rani@example.com\r\n"),
            ]
            .concat(),
        );
        let (from, to) = day(2026, 10, 26);
        let titles: Vec<String> = meetings(&ics, from, to, 420).into_iter().map(|m| m.title).collect();
        assert_eq!(titles, ["Accepted"]);
    }

    #[test]
    fn a_weekly_meeting_in_its_zone_with_a_skipped_and_a_moved_week() {
        let ics = cal(
            JAKARTA,
            &[
                event("DTSTART;TZID=Asia/Jakarta:20260105T100000\r\nDTEND;TZID=Asia/Jakarta:20260105T103000\r\nRRULE:FREQ=WEEKLY;WKST=MO;BYDAY=MO,TH\r\nEXDATE;TZID=Asia/Jakarta:20261029T100000\r\nUID:standup\r\nSUMMARY:Standup\r\n"),
                event("DTSTART;TZID=Asia/Jakarta:20261102T140000\r\nDTEND;TZID=Asia/Jakarta:20261102T143000\r\nRECURRENCE-ID;TZID=Asia/Jakarta:20261102T100000\r\nUID:standup\r\nSUMMARY:Standup (moved)\r\n"),
            ]
            .concat(),
        );
        // Mon 26 Oct to Tue 3 Nov 2026, Jakarta.
        let ms = meetings(&ics, at(2026, 10, 26, 0, 0, 7), at(2026, 11, 4, 0, 0, 7), 420);
        let got: Vec<(i64, &str)> = ms.iter().map(|m| (m.start, m.title.as_str())).collect();
        assert_eq!(
            got,
            [
                (at(2026, 10, 26, 10, 0, 7), "Standup"),
                // Thu 29 Oct is skipped (EXDATE).
                (at(2026, 11, 2, 14, 0, 7), "Standup (moved)"),
            ]
        );
        assert_ne!(ms[0].key, ms[1].key);
    }

    #[test]
    fn daylight_saving_follows_the_files_zone_not_the_pc() {
        let ics = cal(NEW_YORK, &event("DTSTART;TZID=America/New_York:20260302T090000\r\nRRULE:FREQ=DAILY;INTERVAL=1\r\nUID:ny\r\nSUMMARY:NY standup\r\n"));
        // Before the switch (Sun 8 Mar 2026) 9:00 is UTC-5; after, UTC-4.
        let before = meetings(&ics, at(2026, 3, 6, 0, 0, 0), at(2026, 3, 7, 0, 0, 0), 420);
        assert_eq!(before.iter().map(|m| m.start).collect::<Vec<_>>(), [at(2026, 3, 6, 9, 0, -5)]);
        let after = meetings(&ics, at(2026, 3, 10, 0, 0, 0), at(2026, 3, 11, 0, 0, 0), 420);
        assert_eq!(after.iter().map(|m| m.start).collect::<Vec<_>>(), [at(2026, 3, 10, 9, 0, -4)]);
        // November: back to UTC-5.
        let nov = meetings(&ics, at(2026, 11, 3, 0, 0, 0), at(2026, 11, 4, 0, 0, 0), 420);
        assert_eq!(nov.iter().map(|m| m.start).collect::<Vec<_>>(), [at(2026, 11, 3, 9, 0, -5)]);
    }

    #[test]
    fn count_until_interval_and_monthly_rules() {
        let ev = |rule: &str| cal(JAKARTA, &event(&format!("DTSTART;TZID=Asia/Jakarta:20260105T090000\r\nRRULE:{rule}\r\nUID:r\r\nSUMMARY:R\r\n")));
        let starts = |rule: &str, from: i64, to: i64| meetings(&ev(rule), from, to, 420).into_iter().map(|m| m.start).collect::<Vec<_>>();
        let jan = (at(2026, 1, 1, 0, 0, 7), at(2026, 2, 1, 0, 0, 7));
        // Daily, 3 times: 5, 6, 7 Jan.
        assert_eq!(starts("FREQ=DAILY;COUNT=3", jan.0, jan.1), [5, 6, 7].map(|d| at(2026, 1, d, 9, 0, 7)));
        // Counted from the start even when the window opens later.
        assert_eq!(starts("FREQ=DAILY;COUNT=3", at(2026, 1, 7, 0, 0, 7), jan.1), [at(2026, 1, 7, 9, 0, 7)]);
        // Every other week, until the 2nd (a date alone takes the whole day).
        assert_eq!(starts("FREQ=WEEKLY;INTERVAL=2;UNTIL=20260202", jan.0, at(2026, 3, 1, 0, 0, 7)), [5, 19].map(|d| at(2026, 1, d, 9, 0, 7)).into_iter().chain([at(2026, 2, 2, 9, 0, 7)]).collect::<Vec<_>>());
        // The last Friday of each month, and the 15th.
        let q = (at(2026, 2, 1, 0, 0, 7), at(2026, 4, 1, 0, 0, 7));
        assert_eq!(starts("FREQ=MONTHLY;BYDAY=-1FR", q.0, q.1), [at(2026, 2, 27, 9, 0, 7), at(2026, 3, 27, 9, 0, 7)]);
        assert_eq!(starts("FREQ=MONTHLY;BYMONTHDAY=15", q.0, q.1), [at(2026, 2, 15, 9, 0, 7), at(2026, 3, 15, 9, 0, 7)]);
        // Yearly on its date.
        assert_eq!(starts("FREQ=YEARLY", at(2027, 1, 1, 0, 0, 7), at(2027, 2, 1, 0, 0, 7)), [at(2027, 1, 5, 9, 0, 7)]);
        // Rules it can't follow keep only their first time.
        assert_eq!(starts("FREQ=MONTHLY;BYDAY=MO,TU;BYSETPOS=-1", jan.0, at(2026, 4, 1, 0, 0, 7)), [at(2026, 1, 5, 9, 0, 7)]);
    }

    #[test]
    fn floating_times_use_the_pcs_offset() {
        let ics = cal("", &event("DTSTART:20261026T090000\r\nUID:f\r\nSUMMARY:Floating\r\n"));
        let ms = meetings(&ics, 0, i64::MAX / 2, 480);
        assert_eq!(ms[0].start, at(2026, 10, 26, 9, 0, 8));
    }

    #[test]
    fn the_reminder_comes_once_from_five_minutes_before() {
        let m = |key: &str, start: i64| Meeting { key: key.into(), title: key.into(), start, end: start + 1800, link: None };
        let list = [m("a", 1000), m("b", 5000)];
        assert_eq!(to_remind(&list, 1000 - 301, 5, &[]), None);
        assert_eq!(to_remind(&list, 1000 - 300, 5, &[]).map(|m| m.key.as_str()), Some("a"));
        assert_eq!(to_remind(&list, 1000 + START_GRACE_S - 1, 5, &[]).map(|m| m.key.as_str()), Some("a"));
        assert_eq!(to_remind(&list, 1000 + START_GRACE_S, 5, &[]), None, "too late to remind");
        assert_eq!(to_remind(&list, 1000, 5, &["a".into()]), None);
        assert_eq!(minutes_left(&list[0], 1000 - 241), 5);
        assert_eq!(minutes_left(&list[0], 1000 - 240), 4);
        assert_eq!(minutes_left(&list[0], 1000), 0);
    }

    #[test]
    fn the_page_lists_what_is_left_of_today_and_the_next_one_after() {
        let m = |key: &str, start: i64, mins: i64| Meeting { key: key.into(), title: key.into(), start, end: start + mins * 60, link: None };
        // Sun 25 Oct 2026, 14:00 in Jakarta.
        let now = at(2026, 10, 25, 14, 0, 7);
        let list = [
            m("done", at(2026, 10, 25, 9, 0, 7), 30),
            m("live", at(2026, 10, 25, 13, 30, 7), 60),
            m("later", at(2026, 10, 25, 16, 0, 7), 30),
            m("tomorrow", at(2026, 10, 26, 9, 0, 7), 30),
            m("tuesday", at(2026, 10, 27, 9, 0, 7), 30),
        ];
        let (rest, next) = today(&list, now, 420);
        assert_eq!(rest.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(), ["live", "later"]);
        assert!(live(rest[0], now) && !live(rest[1], now));
        assert_eq!(next.map(|m| m.key.as_str()), Some("tomorrow"));
        assert_eq!(span(rest[0], 420, true), "13:30 \u{2013} 14:30");
        assert_eq!(later_label(&list[3], now, 420, true), "Tomorrow 09:00");
        assert_eq!(later_label(&list[4], now, 420, false), "Tue 27 Oct 9:00 AM");
        // Just before midnight, tomorrow's first meeting isn't today's.
        let late = at(2026, 10, 25, 23, 50, 7);
        assert_eq!(today(&list, late, 420).0.len(), 0);
    }

    #[test]
    fn failed_fetches_back_off_to_the_usual_pace() {
        assert_eq!([0, 1, 2, 3, 4, 9].map(retry_after_s), [900, 120, 240, 480, 900, 900]);
    }

    #[test]
    fn nth_weekdays() {
        // Second Sunday of March 2026 is the 8th; last Sunday of October the 25th.
        assert_eq!(nth_weekday(2026, 3, 2, 0), Some(days_from_civil(2026, 3, 8)));
        assert_eq!(nth_weekday(2026, 10, -1, 0), Some(days_from_civil(2026, 10, 25)));
        assert_eq!(nth_weekday(2026, 2, 5, 1), None, "no fifth Monday in February 2026");
    }

    #[test]
    fn durations_and_offsets() {
        assert_eq!(parse_duration("PT1H30M"), Some(5400));
        assert_eq!(parse_duration("P1DT2H"), Some(93600));
        assert_eq!(parse_duration("P1W"), Some(604800));
        assert_eq!(parse_offset("+0700"), Some(420));
        assert_eq!(parse_offset("-0430"), Some(-270));
        assert_eq!(parse_offset("+053000"), Some(330));
    }
}
