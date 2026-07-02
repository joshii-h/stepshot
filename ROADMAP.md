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
- **Visible click** — the real cursor plus a translucent yellow highlight land
  in the image; the highlight is see-through, so text under it stays readable.
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
| R15 | **Windows backend** (hook + PrintWindow + UI Automation) | 🚧 implemented on `feature/windows-backend` (in sync with `main`); live testing pending |

## Platform support matrix

| Capability        | Linux / KDE (Wayland) | Windows           | macOS (help wanted, [#1](https://github.com/joshii-h/stepshot/issues/1)) |
|-------------------|-----------------------|-------------------|----------|
| Click capture     | evdev (`input` group) | `WH_MOUSE_LL` hook | `CGEventTap` |
| Window screenshot | KWin `ScreenShot2`    | `PrintWindow`      | `CGWindowListCreateImage` |
| Cursor + geometry | KWin script           | `GetCursorPos` + `GetWindowRect` | CGWindowList bounds |
| Element names     | AT-SPI                | UI Automation      | AX API |
| Tray              | ksni (StatusNotifierItem) | `Shell_NotifyIcon` | `NSStatusItem` |

The platform-specific parts sit behind traits — `ClickSource` and
`WindowCapturer` exist on `main` today; cursor tracking and element resolution
are still KDE-typed and get their traits with the 0.3 platform split (already
drafted as `platform.rs` on the `feature/windows-backend` branch). Each OS
provides one backend, while the shared parts (`model`, `report`, `annotate`,
`i18n`) stay platform-neutral.

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

### 0.4 — Step editing & redaction (shipped)
The clearest learning from the wider step-recorder field (BetterStepsRecorder,
OpenSteps, PSR++ — all of which ship an editor): the most-wanted post-capture
ability is fixing a recording without re-recording it — especially removing or
blurring something sensitive before sharing.

**Design (locked via a decision-tree interview):** editing happens in the
already-generated `report.html`, which gains a client-side JS editor (no GUI
toolkit, no new deps, reuses the report we already write). "Save" downloads a
small `edits.json`; `stepshot apply <session-dir> edits.json` then applies the
edits to disk and regenerates every enabled export. A tray entry "Edit last
session" opens the report as the native entry point. Each session also gains a
`session.json` (source of truth) so `apply` can rebuild all formats.

- [x] `session.json` per session (source of truth) + `stepshot apply
      <session-dir> [edits.json]` — rebuilds every enabled format; with no
      edits.json it just regenerates. The clicked element's AT-SPI box is stored
      at capture time (in image-pixel coords) for one-click redaction.
- [x] **apply engine** for the edits below (driven by an `edits.json`; the
      in-report editor that *produces* it is the next step):
  - [x] Redaction: **destructive** pixelation of `step-NNN.png` (original pixels
        gone) for any image-pixel box.
  - [x] Edit step descriptions (override the display text; the auto-generated
        one stays in `session.json` for revert).
  - [x] Delete steps + reorder (renumber and rewrite the PNGs; gap-free output).
- [x] **Native in-browser editor** — `stepshot edit <session-dir>` serves the
      editor from a tiny loopback HTTP server (zero deps, hand-rolled over
      `std::net`, `127.0.0.1` + one-time token). Draw redaction boxes or
      one-click the element box, edit text, delete/reorder; **"Apply" POSTs the
      edits straight back and rewrites the session in place** — no download, no
      manual `apply` step. Reachable from the tray ("Edit last session") or the
      CLI. (`stepshot apply` remains for scripting/CI.)
- [x] Insert manual steps in the editor (**+ Add step**): required text +
      optional image (file → copied in); rendered text-only when imageless.

### 0.5 — Capture polish (shipped)
- [x] Double-click merge: two rapid clicks of the same button within
      `[capture] double_click_ms` (default 400 ms) become one “double click”
      step instead of two near-identical ones.
- [x] Pause/resume from the tray menu (keeps the session; drops clicks while
      paused).
- [x] Click filtering: `[capture] buttons` selects which mouse buttons record.
- [x] Drag & drop as a step type: a press that moves ≥ `[capture] drag_min_px`
      (default 16) before release is recorded as a “Drag and drop” step with an
      arrow from start to drop. (Mouse/`EV_REL`; touchpad drags read as clicks.)
- [x] Process name (`resourceClass`, in addition to the window title) in the
      step metadata and reports.

### 0.6 — Configuration & export selection (shipped)
- [x] Config file (`~/.config/stepshot/config.toml`) — dependency-free
      TOML-subset parser; `--write-config` writes a commented starter.
- [x] Marker style: color, opacity, size (defaults = the current translucent
      yellow highlight).
- [x] Default output folder (`[general] output_dir`, `~` expanded; CLI wins).
- [x] **Export selection**: `[export] formats = […]` — the user picks which
      formats get written (default: all available). Honored by both the
      recording loop and `stepshot apply`.

### 0.7 — More export formats (shipped)
"Offer every export the user might want, they pick." Each renderer plugs into
the 0.6 export selection; the default is now **all** of them.
- [x] ODT (LibreOffice Writer) — hand-rolled store-only ZIP container
      (`zip.rs`), embedded screenshots, no dependency.
- [x] RTF (Word/WordPad) — screenshots embedded as `\pngblip` hex.
- [x] Plain text (.txt) — numbered steps, images referenced by filename.

### Later
- macOS backend (CGEventTap / CGWindowList / AX API) — **help wanted**, see
  [#1](https://github.com/joshii-h/stepshot/issues/1); I don't have a Mac running
  a current macOS to develop/test on, so this needs an external contributor.
- GNOME backend (portal screenshot, AT-SPI already shared).
- Keyboard-step capture (privacy-sensitive; needs a careful, summarized design).
- More languages (the i18n layer is built for it; EN/DE/FR/ES/IT ship today).

## Non-goals

- No background daemon, no auto-start, no cloud sync, no telemetry.
- No video recording — stills only (per-window; the full screen only when a
  panel/popup click demands it).
- No bundled browser engine just to render PDFs (exports stay native/pure-Rust).
