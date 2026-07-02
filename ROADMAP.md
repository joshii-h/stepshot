# Roadmap & Goals

This document is the north star for stepshot: what it is, what it must do, and
where it is going. It captures the original brief as concrete, checkable goals so
the project can be driven feature by feature.

## Vision

A lean, **cross-platform, open-source step recorder** — the spiritual successor
to Windows *Steps Recorder* (PSR, being retired), but with output that is
actually pleasant to read and share. On every click it screenshots **exactly the
window that was clicked**, marks the click, names the **UI element** under the
cursor, and produces a **self-contained, shareable report**.

It is deliberately lean: no daemon, no cloud, no telemetry, local-only, and it
lives in the system tray so a single binary does the whole job.

## Core principles

- **Per-window capture** — the active window by default; the whole screen only
  when the click doesn't land in it (panel, desktop, popup menus — otherwise
  the clicked thing wouldn't be in the picture).
- **Visible click** — the real cursor plus a drawn marker land in the image.
- **Element-level description** — “Left click on button ‘Save’ in window …”,
  resolved from the accessibility tree (AT-SPI on Linux, UI Automation on
  Windows), with a graceful fall-back to the window level.
- **Self-contained output** — images embedded; one file you can send.
- **Tray-driven** — start/stop from the tray, no terminal, no `Ctrl+C`.
- **Privacy** — local-only, no network; accessibility is toggled on only while
  recording.

## Requirements (from the original brief)

These are the explicit asks that define “done” for the foundation. All of the
v0.1 items below are shipped in the alpha.

| # | Requirement | Status |
|---|-------------|--------|
| R1 | KDE Plasma / Wayland client, first cut | ✅ shipped |
| R2 | Screenshot **only the relevant window** on click | ✅ shipped |
| R3 | Represent the click **visually** in the image | ✅ shipped |
| R4 | **Describe** in words what was done per step | ✅ shipped |
| R5 | Resolve the clicked element’s **id / text** into the description | ✅ shipped (AT-SPI) |
| R6 | Work for **browsers** (Firefox/Chrome) and **Flatpaks** | ✅ shipped (a11y bridge) |
| R7 | Bundle required libraries as install dependencies | ✅ documented in README |
| R8 | Run as a **tray app**, start/stop from the tray (no `Ctrl+C`) | ✅ shipped |
| R9 | **Notifications** on start/stop (green ✓ / red ✗ style) | ✅ shipped |
| R10 | **Embed images** in the report (inline, self-contained) | ✅ shipped |
| R11 | Custom **SVG logo** (camera, red dot when active) — no emoji | ✅ shipped |
| R12 | **i18n**, simple to extend (English + German) | ✅ shipped |
| R13 | Released as **alpha**, English UI, permissive (0BSD) | ✅ shipped |
| R14 | **Export** to PDF and Word (like comparable tools) | ✅ shipped (0.2) |
| R15 | **Windows backend** (hook + PrintWindow + UI Automation) | 🚧 implemented on `feature/windows-backend`; rebase + runtime testing pending |

## Platform support matrix

| Capability        | Linux / KDE (Wayland) | Windows           | macOS (help wanted, [#1](https://github.com/joshii-h/stepshot/issues/1)) |
|-------------------|-----------------------|-------------------|----------|
| Click capture     | evdev (`input` group) | `WH_MOUSE_LL` hook | `CGEventTap` |
| Window screenshot | KWin `ScreenShot2`    | `PrintWindow`      | `CGWindowListCreateImage` |
| Cursor + geometry | KWin script           | `GetCursorPos` + `GetWindowRect` | CGWindowList bounds |
| Element names     | AT-SPI                | UI Automation      | AX API |
| Tray              | ksni (StatusNotifierItem) | `Shell_NotifyIcon` | `NSStatusItem` |

The platform-specific parts sit behind the traits in `platform.rs`
(`ClickSource`, `WindowCapturer`, `CursorTracker`, `ElementResolver`); each OS
provides one backend, while the shared parts (`model`, `report`, `annotate`,
`i18n`, the session logic) stay platform-neutral.

## Milestones

### 0.1 — Alpha (shipped)
KDE/Wayland foundation: tray app, per-window capture, click marker, AT-SPI
element naming, self-contained HTML + Markdown report, notifications, i18n.

### 0.2 — Exports (shipped)
- [x] **PDF** export (paginated, embedded screenshots, no external runtime).
- [x] **DOCX** export (Word-compatible, embedded screenshots).
- [x] Reports module emits HTML + Markdown + PDF + DOCX on finalize.

### 0.2.x — Capture refinements (shipped)
- [x] Graceful start without an input device: keep the tray alive + notify
      instead of exiting silently.
- [x] Full-screen fallback for invisible active windows (Xwayland video bridge,
      bare desktop), capturing the **monitor under the cursor** (multi-monitor).
- [x] One file per language under `src/i18n/`; `main.rs` split into
      `session.rs` + `selftest.rs`.
- [x] Full-screen capture when the click doesn't land in the active window —
      panel/desktop clicks (start menu, taskbar) and clicks inside **popups**
      (context menus are separate Wayland surfaces, invisible in a window
      capture of their parent).
- [x] The stop gesture (tray icon + stop menu item) no longer ends up in the
      report; its steps and screenshots are trimmed on stop/quit.
- [x] Input hotplug: mice plugged in (or re-plugged) while running are picked
      up by a periodic `/dev/input` rescan.
- [x] Code-review hardening: KWin script in `XDG_RUNTIME_DIR`, capture read
      deadline, AT-SPI screen-reader flag restored correctly, `--help`/
      `--version`, weekly `cargo audit` in CI, unit tests for the pure helpers.

### 0.3 — Windows backend
Implemented on the [`feature/windows-backend`](https://github.com/joshii-h/stepshot/tree/feature/windows-backend)
branch (`platform.rs` traits + `src/win/`, compiled by a Windows CI job) and
kept in sync with `main` (full-screen capture, stop-gesture trim, hotplug).
Before it lands: functional testing on a live Windows desktop.
- [x] Low-level mouse hook (`SetWindowsHookEx` / `WH_MOUSE_LL`).
- [x] Active-window screenshot (`PrintWindow` + `PW_RENDERFULLCONTENT`).
- [x] Full-screen capture (`BitBlt` of the virtual screen) for panel/popup clicks.
- [x] Cursor + window geometry (`GetCursorPos`, `GetForegroundWindow`,
      `GetWindowRect`), popup detection via `WindowFromPoint`.
- [x] Element names via UI Automation (`ElementFromPoint`).
- [x] Tray via `Shell_NotifyIcon`.
- [x] Cross-platform main loop selecting the backend by `cfg`.
- [ ] Live-desktop testing + polishing (DPI/scale, balloon timing, marker fit).

### Later
- macOS backend (CGEventTap / CGWindowList / AX API) — **help wanted**, see
  [#1](https://github.com/joshii-h/stepshot/issues/1); I don't have a Mac running
  a current macOS to develop/test on, so this needs an external contributor.
- GNOME backend (portal screenshot, AT-SPI already shared).
- Pause/resume, click filtering, keyboard-step capture.
- Redaction / blur of sensitive regions before export.
- More languages (the i18n layer is built for it).

## Non-goals

- No background daemon, no auto-start, no cloud sync, no telemetry.
- No video recording — stills only (per-window; the full screen only when a
  panel/popup click demands it).
- No bundled browser engine just to render PDFs (exports stay native/pure-Rust).
