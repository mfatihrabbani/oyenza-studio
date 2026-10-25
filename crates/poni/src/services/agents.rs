//! AI sessions (`poni_core::agents`). Claude Code's hooks and Codex's
//! `notify` run `poni.exe --agent claude|codex`; that short-lived process
//! reads what the tool said ([`forward`]), hands it to the running island's
//! helper window as a WM_COPYDATA message and exits, always with code 0 so
//! the tool carries on. The island answers with notices and the Agents tab;
//! Open brings the session's window to the front ([`bring_to_front`]).
//! Nothing runs between messages.

use crate::window::HELPER_CLASS;
use poni_core::agents::{self, Event};
use std::io::Read;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{BOOL, FALSE, HWND, LPARAM, TRUE, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowW, GetWindowTextLengthW, GetWindowTextW, IsIconic, IsWindow, IsWindowVisible, SendMessageW, SetForegroundWindow, ShowWindow,
    SW_RESTORE, WM_COPYDATA,
};

/// `--agent`: the argument that makes poni.exe a messenger.
pub const ARG: &str = "--agent";
/// WM_COPYDATA's tag for an agent event (ASCII "PONA").
pub const COPYDATA_TAG: usize = 0x504F_4E41;
/// What a hook may pass on stdin, at most.
const STDIN_MAX: u64 = 1 << 20;

/// Run as `poni.exe --agent <tool> [json]`: read the event, pass it on,
/// exit. Returns the exit code (always 0: a hook must never stop the tool).
pub fn forward(tool: &str, arg: Option<&str>) -> i32 {
    let event = match tool {
        // Claude Code writes the hook's details to stdin.
        "claude" => {
            let mut s = String::new();
            let _ = std::io::stdin().take(STDIN_MAX).read_to_string(&mut s);
            agents::parse_claude(&s)
        }
        // Codex passes them as the last argument.
        "codex" => arg.and_then(agents::parse_codex),
        _ => None,
    };
    let Some(event) = event else { return 0 };
    let Ok(json) = serde_json::to_string(&event) else { return 0 };
    unsafe {
        let Ok(helper) = FindWindowW(HELPER_CLASS, PCWSTR::null()) else { return 0 };
        if helper.is_invalid() {
            return 0;
        }
        let data = COPYDATASTRUCT { dwData: COPYDATA_TAG, cbData: json.len() as u32, lpData: json.as_ptr() as *mut _ };
        let _ = SendMessageW(helper, WM_COPYDATA, WPARAM(0), LPARAM(&data as *const COPYDATASTRUCT as isize));
    }
    0
}

/// The event in a WM_COPYDATA message's `lparam`, if it is one.
///
/// # Safety
/// `lp` must be the `lparam` of a WM_COPYDATA message being handled.
pub unsafe fn received(lp: LPARAM) -> Option<Event> {
    let data = &*(lp.0 as *const COPYDATASTRUCT);
    if data.dwData != COPYDATA_TAG || data.lpData.is_null() || data.cbData as u64 > STDIN_MAX {
        return None;
    }
    let bytes = std::slice::from_raw_parts(data.lpData as *const u8, data.cbData as usize);
    serde_json::from_str(std::str::from_utf8(bytes).ok()?).ok()
}

/// Bring the session's window forward: the one it started from, else the
/// first visible window whose title names its folder (VS Code and most
/// terminals show it). False when there's none.
pub fn bring_to_front(window: isize, title: &str) -> bool {
    let hwnd = HWND(window as *mut core::ffi::c_void);
    let target = if window != 0 && unsafe { IsWindow(hwnd) }.as_bool() { Some(hwnd) } else { titled(title) };
    let Some(h) = target else { return false };
    unsafe {
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        // Allowed: the click that asked for it was Poni's input.
        SetForegroundWindow(h).as_bool()
    }
}

/// The first visible top-level window whose title holds `part`.
fn titled(part: &str) -> Option<HWND> {
    if part.is_empty() {
        return None;
    }
    struct Search<'a> {
        part: &'a str,
        found: Option<HWND>,
    }
    unsafe extern "system" fn visit(hwnd: HWND, lp: LPARAM) -> BOOL {
        let search = &mut *(lp.0 as *mut Search);
        if !IsWindowVisible(hwnd).as_bool() {
            return TRUE;
        }
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return TRUE;
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, &mut buf).max(0) as usize;
        let title = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
        if title.to_lowercase().contains(&search.part.to_lowercase()) {
            search.found = Some(hwnd);
            return FALSE;
        }
        TRUE
    }
    let mut search = Search { part, found: None };
    // Stopping early makes EnumWindows report an error; the answer is in `search`.
    let _ = unsafe { EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize)) };
    search.found
}
