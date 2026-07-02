# stepshot

[![CI](https://github.com/joshii-h/stepshot/actions/workflows/rust.yml/badge.svg)](https://github.com/joshii-h/stepshot/actions/workflows/rust.yml)

> ⚠️ **Alpha** — works, but rough edges and breaking changes are expected.
> KDE Plasma / Wayland only for now.

A lean, open-source **step recorder** — the open-source answer to Windows
*Steps Recorder* (PSR), but better: on every mouse click it screenshots **the
clicked window** (or the whole screen when the click lands on the panel or in
a popup menu), **marks the click**, names the **clicked UI element** (via
accessibility), and writes a **self-contained HTML report** describing each step.

It lives in the system tray; you start and stop recording from there.

## Features (alpha)

- **Tray app**: runs in the system tray (camera icon, red dot while recording),
  start/stop from the tray menu — no terminal, no Ctrl+C needed.
- **Global click capture** without root — reads evdev directly (`input` group is
  enough); mice plugged in while running are picked up automatically.
- **Window screenshot** of the active window via `org.kde.KWin.ScreenShot2`
  (D-Bus, FD passing) — **no runtime dependency** like `spectacle`. Clicks that
  don't land in the active window (panel, start menu, desktop) or hit a **popup
  menu** (a separate Wayland surface) get a **full-screen capture** instead, so
  what you clicked is always in the picture.
- **Click marker**: a translucent yellow highlight — text under it stays
  readable — plus the real mouse cursor baked into the image (KWin `include-cursor`).
- **Own clicks stay out**: the clicks that operate stepshot's tray menu
  (start/stop/quit) are not recorded as steps.
- **Element detection** via AT-SPI: “Left click on button ‘Save’ in window …”.
- **Notifications** on start/stop, **incremental report** (a crash/kill loses nothing),
  and a **self-contained** `report.html` (images embedded as base64) plus `report.md`.
- **Exports**: on stop you also get `report.pdf` (paginated), `report.docx` (Word),
  `report.odt` (LibreOffice), `report.rtf` and `report.txt` — screenshots embedded,
  pure-Rust, no external tools. Pick which formats via `[export] formats`.

## Requirements

| Purpose | Requirement |
|---------|-------------|
| Screenshot authorization | a `.desktop` file with `X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2` (created by `install.sh`) |
| Click capture | user in the `input` group — `sudo usermod -aG input "$USER"`, then **reboot** (see note below) |
| Element detection (Qt/KDE) | **qtbase built with the `accessibility` USE flag** (Gentoo) / the Qt AT-SPI bridge |
| Element detection (GTK) | `at-spi2-atk` / `libatk-bridge` (usually present) |
| Element detection (Firefox) | activates automatically once an AT is detected |
| Element detection (Chrome/Electron) | launch with `--force-renderer-accessibility` |

Without accessibility the description gracefully falls back to the window level
(“Left click in window …”). Games / canvas apps expose nothing.

## Build & install

```sh
./install.sh   # builds the release, installs the binary + icon + .desktop, refreshes caches
```

Then launch **stepshot from your application menu** (or `stepshot` in a terminal).
It appears as a camera icon in the system tray:

1. Click the tray icon → **“Start recording”**
2. Click around as usual — every click is documented (red dot = active)
3. **“Stop recording & write report”** → notification + finished report
4. **“Open last report folder”** opens the result

```sh
stepshot ~/path/to/output    # optional: custom output base folder
```

Sessions are written to `~/Pictures/stepshot/session-<timestamp>/`.

### Debug / test modes

```sh
STEPSHOT_ONESHOT=1 stepshot   # capture a single step (pipeline self-test)
STEPSHOT_DEBUG=1   stepshot   # extra diagnostics on stderr
STEPSHOT_ICON=1    stepshot   # render the tray icon to /tmp for inspection
STEPSHOT_ATTREE=3  stepshot   # dump the AT-SPI tree (to the given depth)
STEPSHOT_ATDUMP=1  stepshot   # find the first named button and resolve it back
```

`stepshot --help` / `--version` work as expected.

## Editing & redaction

After a recording you can fix it without re-recording — **blur out something
sensitive**, reword a step, delete or reorder steps:

```sh
stepshot edit ~/Pictures/stepshot/session-<timestamp>
```

This opens a **native in-browser editor** (a tiny local server on `127.0.0.1`,
no dependencies, no data leaves your machine). Drag across a screenshot to
**redact** an area — or one-click **“Redact clicked element”** to blank the
button/field that was clicked — edit the description text, delete or reorder
steps, or **＋ Add step** to insert a manual step (text, optional image), then
**Apply**. The edits are written straight back to the session and
every enabled export is regenerated; there is nothing to download. Redaction is
**destructive** (the original pixels in `step-NNN.png` are gone).

The tray menu’s **“Edit last session”** opens the same editor for your most
recent recording. For scripting/CI there is also a headless form:

```sh
stepshot apply <session-dir> [edits.json]   # apply an editor's edits.json,
                                            # or, with none, just regenerate
```

Each session stores a `session.json` (the source of truth) that both commands
rebuild every format from.

## Configuration

stepshot works with **no config file** — everything below is optional. To
customize, write a commented starter file and edit it:

```sh
stepshot --write-config   # → ~/.config/stepshot/config.toml (never overwrites)
```

```toml
[general]
# output_dir = "~/Pictures/stepshot"   # a path given on the command line still wins

[marker]                # the translucent click highlight
# fill = "#FFE13C"      # highlighter yellow
# fill_alpha = 0.35     # see-through so text under it stays readable
# rim = "#FFA500"       # amber outline
# rim_alpha = 0.75
# radius = 20.0         # marker size in pixels

[export]                # which report formats to write (default: all)
# formats = ["html", "md", "pdf", "docx", "odt", "rtf", "txt"]

[capture]               # click handling
# buttons = ["left", "right", "middle"]   # which buttons record (default: all)
# double_click_ms = 400 # merge two rapid same-button clicks into one step (0 = off)
```

Colors are `"#RRGGBB"`, alpha is `0.0`–`1.0`. A missing or malformed file falls
back to the built-in defaults (with a warning), never blocking startup.

## How authorization works (KDE)

KWin gates `org.kde.KWin.ScreenShot2`: a caller is only allowed if its executable
has an associated `.desktop` file declaring
`X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2` (KWin matches the
resolved executable path against `Exec=`). `install.sh` sets this up — which is
why it **copies** the binary instead of symlinking it.

> **Just joined the `input` group? Reboot — don't just re-log in.** On systemd
> systems a logout/login does *not* restart the per-user `systemd --user` manager,
> which launches your tray apps and keeps its *old* group set. So menu-launched
> stepshot still sees no input device (tray shows, but every recording yields 0
> steps). A full reboot — or `loginctl terminate-user "$USER"` — fixes it.
>
> Tempted to shortcut with `newgrp input` / `sg input`? Don't: the gid switch
> makes the process *non-dumpable* (e.g. Fedora's `suid_dumpable=2`), so KWin
> can't read its `/proc/<pid>/exe` to find the `.desktop` and refuses the
> screenshot with `NoAuthorized`. Reboot instead.

## Permissions & privacy

stepshot is deliberately privileged and shows **no Wayland permission prompt** —
not even on first run — because it bypasses the sanctioned (prompting) paths:

- **Screenshots** go straight to KWin's privileged `org.kde.KWin.ScreenShot2`
  D-Bus interface instead of the `xdg-desktop-portal` screen-share picker (the
  "remember this choice" popup you may know from browsers/OBS). KWin authorizes
  the call **statically** via the install-time `.desktop` declaration above and
  checks it silently on every call — the same way Spectacle captures without
  nagging. There is no runtime consent dialog.
- **Global clicks** are read directly from `/dev/input` (evdev), which sits
  *below* Wayland's input isolation entirely — Wayland has no global input API to
  prompt for. The only gate is OS-level `input` group membership.

So while it runs, stepshot can see every click and silently screenshot any
window — the same capabilities a keylogger or screen recorder would need. In
exchange it is **local-only** (no network), writes solely to the session folder,
and toggles accessibility (AT-SPI) **only while recording**. Still: only run
builds you trust, and remove the `.desktop` file to revoke screenshot access.

## Architecture

```
src/
  main.rs     startup + tray event loop (start/stop/quit), wiring
  session.rs  recording session: per-click capture step + final report
  selftest.rs env-driven debug/self-test modes (ONESHOT, ICON, ATTREE, ATDUMP)
  tray.rs     tray icon/menu (ksni, StatusNotifierItem)
  icon.rs     camera icon drawn programmatically (red dot when active)
  notify.rs   desktop notifications (start/stop)
  input.rs    ClickSource trait    → EvdevClickSource (Linux)        [Win: LL mouse hook]
  capture.rs  WindowCapturer trait → KdeCapturer (KWin ScreenShot2)  [Win: PrintWindow]
  cursor.rs   KwinCursor: global cursor pos via a KWin script → zbus sink
  a11y.rs     Atspi: GetAccessibleAtPoint over the a11y bus (with deadline) [Win: UIA]
  annotate.rs draws the click marker into the image
  i18n.rs     minimal, dependency-free translations (one file per language)
  i18n/       en.rs, de.rs, fr.rs, es.rs, it.rs — string tables (one per language)
  config.rs   ~/.config/stepshot/config.toml (marker, export selection, output dir)
  model.rs    Step/Button + description logic
  report.rs   HTML + Markdown (honors the export selection)
  export_pdf.rs / export_docx.rs  paginated PDF and Word, screenshots embedded
  export_odt.rs / export_rtf.rs / export_txt.rs  ODT, RTF and plain-text exports
  zip.rs      minimal dependency-free store-only ZIP writer (ODT container)
  session.rs  session.json (source of truth) + per-click capture step
  apply.rs    stepshot apply — rebuild a session from session.json (+ edits.json)
  edit.rs     stepshot edit — native in-browser editor (loopback HTTP server)
  json.rs     minimal dependency-free JSON reader/writer (session.json/edits.json)
```

The platform-specific parts sit behind traits — one backend per OS, while the
rest (`model`, `report`, `annotate`) stays shared. A Windows backend
(`SetWindowsHookEx` + `PrintWindow` + UI Automation) is the planned next step.

## Languages

UI, notifications and the report are localized. The language is auto-detected
from `LANGUAGE`/`LC_ALL`/`LC_MESSAGES`/`LANG` (defaults to English). Currently
**English**, **German**, **French**, **Spanish** and **Italian** ship (one file
each under `src/i18n/`), so adding a language is a self-contained PR. French,
Spanish and Italian are machine-assisted translations — native-speaker review
is welcome.

Adding a language is deliberately simple and compiler-checked:

- **a new string**: add a field to `Strings` (in `src/i18n.rs`) — every language
  file is a struct literal, so the compiler forces each language to provide it;
- **a new language**: add `src/i18n/xx.rs` with `pub static STRINGS: Strings =
  …`, then register it in `src/i18n.rs` (`mod xx;`, a `Lang` variant, a
  `strings_for` arm, and its locale prefix in `Lang::detect`). Placeholders
  (`{n}`, `{title}`, …) are identical across languages.

## Roadmap

See [ROADMAP.md](ROADMAP.md) for the full vision, requirements and milestones,
or the [project board](https://github.com/joshii-h/stepshot/projects). PDF + DOCX
export (roadmap milestone 0.2) has already shipped; in short, next up:

- **Windows backend** — mouse hook + `PrintWindow` + UI Automation (milestone 0.3);
  implemented on the `feature/windows-backend` branch, kept in sync with main and
  compiled in CI — what's left is runtime testing on a real Windows machine
- **Step editing & redaction** (milestone 0.4) — fix a recording without
  re-recording: blur sensitive regions, edit descriptions, delete steps
- **Capture polish** (milestone 0.5) — double-click merge, pause/resume,
  click filtering, drag & drop steps
- **Configuration & export selection** (milestone 0.6) — config file with marker
  style options; the user picks which export formats get written
- **More export formats** (milestone 0.7) — ODT, RTF, plain text
- **macOS backend** — **help wanted** ([#1](https://github.com/joshii-h/stepshot/issues/1)):
  I don't have a Mac running a current macOS, so this needs an external contributor
- More languages (PRs welcome — add a file under `src/i18n/`)

## License

[0BSD](LICENSE) — do whatever you want with it. No conditions, no attribution
required.
