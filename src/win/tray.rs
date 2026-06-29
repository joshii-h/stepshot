//! System-tray icon via `Shell_NotifyIcon`, with a right-click popup menu and
//! balloon notifications.
//!
//! A hidden helper window receives the tray callback message; the menu is shown
//! with `TPM_RETURNCMD` so the selected command is mapped to a [`Cmd`] and sent
//! to the main loop. The window procedure is a bare function, so the command
//! sender and the recording flag live in process-global state.

use anyhow::{Context, Result};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon,
    DestroyMenu, DestroyWindow, DispatchMessageW, GetCursorPos, HICON, IDI_APPLICATION, LoadIconW,
    MF_SEPARATOR, MF_STRING, MSG, PM_REMOVE, PeekMessageW, PostQuitMessage, RegisterClassW,
    SetForegroundWindow, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
    TranslateMessage, WINDOW_EX_STYLE, WM_APP, WM_CONTEXTMENU, WM_DESTROY, WM_LBUTTONUP,
    WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPED,
};
use windows::core::{PCWSTR, w};

/// Control commands from the tray to the main loop.
#[derive(Debug, Clone, Copy)]
pub enum Cmd {
    Start,
    Stop,
    OpenFolder,
    Quit,
}

const WM_TRAY: u32 = WM_APP + 1;
const ID_START: usize = 1;
const ID_STOP: usize = 2;
const ID_OPEN: usize = 3;
const ID_QUIT: usize = 4;
const TRAY_UID: u32 = 1;

static CMD_TX: OnceLock<Sender<Cmd>> = OnceLock::new();
static RECORDING: AtomicBool = AtomicBool::new(false);

pub struct Tray {
    hwnd: HWND,
    nid: NOTIFYICONDATAW,
}

impl Tray {
    pub fn new(tx: Sender<Cmd>) -> Result<Self> {
        let _ = CMD_TX.set(tx);
        unsafe {
            let hinst = HINSTANCE(GetModuleHandleW(None).context("GetModuleHandleW failed")?.0);
            let class = w!("stepshot_tray_wndclass");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinst,
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassW(&wc); // 0 if already registered — harmless

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                w!("stepshot"),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(hinst),
                None,
            )
            .context("could not create the tray helper window")?;

            let mut nid = NOTIFYICONDATAW {
                cbSize: size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: TRAY_UID,
                uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
                uCallbackMessage: WM_TRAY,
                hIcon: make_icon(false),
                ..Default::default()
            };
            put_wide(&mut nid.szTip, crate::i18n::tr().tt_ready);
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);

            Ok(Self { hwnd, nid })
        }
    }

    /// Drain and dispatch any pending window messages (non-blocking).
    pub fn pump(&mut self) {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    /// Update the icon + tooltip for the current recording state.
    pub fn set_recording(&mut self, recording: bool, steps: usize) {
        RECORDING.store(recording, Ordering::SeqCst);
        unsafe {
            let old = self.nid.hIcon;
            self.nid.hIcon = make_icon(recording);
            if !old.is_invalid() {
                let _ = DestroyIcon(old);
            }
            let tip = if recording {
                crate::i18n::tr()
                    .tt_recording
                    .replace("{n}", &steps.to_string())
            } else {
                crate::i18n::tr().tt_ready.to_string()
            };
            put_wide(&mut self.nid.szTip, &tip);
            self.nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
            let _ = Shell_NotifyIconW(NIM_MODIFY, &self.nid);
        }
    }

    /// Show a balloon notification (start/stop feedback).
    pub fn notify(&mut self, body: &str) {
        unsafe {
            put_wide(&mut self.nid.szInfoTitle, "stepshot");
            put_wide(&mut self.nid.szInfo, body);
            self.nid.uFlags = NIF_INFO;
            let _ = Shell_NotifyIconW(NIM_MODIFY, &self.nid);
        }
    }

    /// Remove the icon and destroy the helper window.
    pub fn remove(&mut self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.nid);
            if !self.hwnd.is_invalid() {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            let ev = lparam.0 as u32;
            if ev == WM_RBUTTONUP || ev == WM_LBUTTONUP || ev == WM_CONTEXTMENU {
                unsafe { show_menu(hwnd) };
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

unsafe fn show_menu(hwnd: HWND) {
    unsafe {
        let menu = match CreatePopupMenu() {
            Ok(m) => m,
            Err(_) => return,
        };
        let t = crate::i18n::tr();
        let recording = RECORDING.load(Ordering::SeqCst);

        // Bindings must outlive the AppendMenuW calls (which copy the text).
        let (toggle_id, toggle_text) = if recording {
            (ID_STOP, wide(t.menu_stop))
        } else {
            (ID_START, wide(t.menu_start))
        };
        let open_text = wide(t.menu_open_folder);
        let quit_text = wide(t.menu_quit);

        let _ = AppendMenuW(menu, MF_STRING, toggle_id, PCWSTR(toggle_text.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING, ID_OPEN, PCWSTR(open_text.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, PCWSTR(quit_text.as_ptr()));

        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        // Required so the menu dismisses when the user clicks elsewhere.
        let _ = SetForegroundWindow(hwnd);

        let chosen = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            p.x,
            p.y,
            None,
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu);

        let cmd = match chosen.0 as usize {
            ID_START => Some(Cmd::Start),
            ID_STOP => Some(Cmd::Stop),
            ID_OPEN => Some(Cmd::OpenFolder),
            ID_QUIT => Some(Cmd::Quit),
            _ => None,
        };
        if let (Some(cmd), Some(tx)) = (cmd, CMD_TX.get()) {
            let _ = tx.send(cmd);
        }
    }
}

/// Build the tray `HICON` from the shared camera icon; fall back to the stock
/// application icon if GDI icon creation fails.
fn make_icon(recording: bool) -> HICON {
    build_hicon(recording)
        .unwrap_or_else(|| unsafe { LoadIconW(None, IDI_APPLICATION).unwrap_or_default() })
}

fn build_hicon(recording: bool) -> Option<HICON> {
    let size = 32;
    let img = crate::icon::rgba(recording, size);
    let (w, h) = (img.width() as i32, img.height() as i32);
    let mut xor = img.into_raw(); // RGBA, top-down
    for px in xor.chunks_exact_mut(4) {
        px.swap(0, 2); // RGBA → BGRA (device order)
    }
    // Monochrome AND mask, rows padded to 16 bits; zero = use the color alpha.
    let and = vec![0u8; ((w as usize).div_ceil(16) * 2) * h as usize];
    unsafe { CreateIcon(None, w, h, 1, 32, and.as_ptr(), xor.as_ptr()) }.ok()
}

/// A NUL-terminated UTF-16 buffer that the caller keeps alive across the call.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Copy `s` (truncated, NUL-terminated) into a fixed-size UTF-16 field.
fn put_wide(dst: &mut [u16], s: &str) {
    dst.fill(0);
    let max = dst.len().saturating_sub(1);
    for (slot, ch) in dst.iter_mut().zip(s.encode_utf16().take(max)) {
        *slot = ch;
    }
}
