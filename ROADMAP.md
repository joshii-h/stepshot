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

- **Per-window capture** — never the whole desktop; only the active window.
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
| R15 | **Windows backend** (hook + PrintWindow + UI Automation) | 🚧 milestone 0.3 |

## Platform support matrix

| Capability        | Linux / KDE (Wayland) | Windows           | macOS (help wanted, [#1](https://github.com/joshii-h/stepshot/issues/1)) |
|-------------------|-----------------------|-------------------|----------|
| Click capture     | evdev (`input` group) | `WH_MOUSE_LL` hook | `CGEventTap` |
| Window screenshot | KWin `ScreenShot2`    | `PrintWindow`      | `CGWindowListCreateImage` |
| Cursor + geometry | KWin script           | `GetCursorPos` + `GetWindowRect` | CGWindowList bounds |
| Element names     | AT-SPI                | UI Automation      | AX API |
| Tray              | ksni (StatusNotifierItem) | `Shell_NotifyIcon` | `NSStatusItem` |

The platform-specific parts sit behind traits (`ClickSource`,
`WindowCapturer`, `CursorTracker`, `ElementResolver`); each OS provides one
backend, while the shared parts (`model`, `report`, `annotate`, `i18n`) stay
platform-neutral.

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

### 0.3 — Windows backend
- [ ] Low-level mouse hook (`SetWindowsHookEx` / `WH_MOUSE_LL`).
- [ ] Active-window screenshot (`PrintWindow` + `PW_RENDERFULLCONTENT`).
- [ ] Cursor + window geometry (`GetCursorPos`, `GetForegroundWindow`,
      `GetWindowRect`).
- [ ] Element names via UI Automation (`ElementFromPoint`).
- [ ] Tray via `Shell_NotifyIcon`.
- [ ] Cross-platform main loop selecting the backend by `cfg`.

### Later
- macOS backend (CGEventTap / CGWindowList / AX API) — **help wanted**, see
  [#1](https://github.com/joshii-h/stepshot/issues/1); the maintainer has no
  current-macOS Mac to develop/test on, so this needs an external contributor.
- GNOME backend (portal screenshot, AT-SPI already shared).
- Pause/resume, click filtering, keyboard-step capture.
- Redaction / blur of sensitive regions before export.
- More languages (the i18n layer is built for it).

## Non-goals

- No background daemon, no auto-start, no cloud sync, no telemetry.
- No full-desktop or video recording — single-window stills only.
- No bundled browser engine just to render PDFs (exports stay native/pure-Rust).
