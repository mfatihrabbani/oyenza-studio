//! Meetings from the calendar link saved on this PC (Google Calendar's secret
//! iCal address, sealed in `paths::calendar_link_path`). The app asks for a
//! fetch (`App::calendar_tick`: every `poni_core::calendar::FETCH_EVERY_S`
//! while someone is at the PC); each one runs on its own short-lived thread,
//! parses the file there and reports the meetings ahead. Nothing runs
//! between fetches, and the link never reaches the log.

use crate::events::{AppEvent, CalendarEvent, EventSink};
use poni_core::calendar;
use std::time::Duration;

const USER_AGENT: &str = concat!("OyenzaStudio/", env!("CARGO_PKG_VERSION"), " (+https://github.com/lucacadalora/oyenza-studio)");
/// A busy calendar's file can be a few megabytes.
const TIMEOUT: Duration = Duration::from_secs(20);
/// Meetings that started up to this long ago are kept (one under way).
const BEHIND_S: i64 = 3600;

/// Fetch and read the calendar; the answer arrives as `AppEvent::Calendar`.
/// `now` is UTC seconds and `local_offset_min` the PC's offset from UTC.
pub fn fetch(sink: EventSink, now: i64, local_offset_min: i32) {
    let tell = sink.clone();
    let spawned = std::thread::Builder::new().name("calendar".into()).spawn(move || {
        sink.send(AppEvent::Calendar(read(now, local_offset_min)));
    });
    if let Err(e) = spawned {
        tell.send(AppEvent::Calendar(CalendarEvent::Failed(format!("no thread ({e})"))));
    }
}

fn read(now: i64, local_offset_min: i32) -> CalendarEvent {
    let Some(link) = super::secret::load(&crate::paths::calendar_link_path()) else {
        return CalendarEvent::Failed("the saved link can't be read".into());
    };
    match super::http::get_full(&link, USER_AGENT, TIMEOUT) {
        Ok(r) if r.status == 200 && r.body.contains("BEGIN:VCALENDAR") => {
            CalendarEvent::Meetings(calendar::meetings(&r.body, now - BEHIND_S, now + calendar::AHEAD_S, local_offset_min))
        }
        Ok(r) if r.status == 200 => CalendarEvent::Refused(200),
        // Google answers 404 once the secret address is reset.
        Ok(r) if matches!(r.status, 401 | 403 | 404 | 410) => CalendarEvent::Refused(r.status),
        Ok(r) => CalendarEvent::Failed(format!("HTTP {}", r.status)),
        Err(e) => CalendarEvent::Failed(e.to_string()),
    }
}
