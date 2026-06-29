# stepshot

[![CI](https://github.com/joshii-h/stepshot/actions/workflows/rust.yml/badge.svg)](https://github.com/joshii-h/stepshot/actions/workflows/rust.yml)

> ⚠️ **Alpha** — works, but rough edges and breaking changes are expected.
> KDE Plasma / Wayland only for now.

A lean, open-source **step recorder** — the open-source answer to Windows
*Steps Recorder* (PSR), but better: on every mouse click it screenshots **exactly
the clicked window**, **marks the click**, names the **clicked UI element** (via
accessibility), and writes a **self-contained HTML report** describing each step.

It lives in the system tray; you start and stop recording from there.

## Features (v0.1, alpha)

- **Tray app**: runs in the system tray (camera icon, red dot while recording),
  start/stop from the tray menu — no terminal, no Ctrl+C needed.
- **Global click capture** without root — reads evdev directly (`input` group is enough).
- **Window screenshot** of the active window via `org.kde.KWin.ScreenShot2`
  (D-Bus, FD passing) — **no runtime dependency** like `spectacle`.
- **Click marker** + the real mouse cursor baked into the image (KWin `include-cursor`).
- **Element detection** via AT-SPI: “Left click on button ‘Save’ in window …”.
- **Notifications** on start/stop, **incremental report** (a crash/kill loses nothing),
  and a **self-contained** `report.html` (images embedded as base64) plus `report.md`.

## Requirements

| Purpose | Requirement |
|---------|-------------|
| Screenshot authorization | a `.desktop` file with `X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2` (created by `install.sh`) |
| Click capture | user in the `input` group |
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
```

## How authorization works (KDE)

KWin gates `org.kde.KWin.ScreenShot2`: a caller is only allowed if its executable
has an associated `.desktop` file declaring
`X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2` (KWin matches the
resolved executable path against `Exec=`). `install.sh` sets this up — which is
why it **copies** the binary instead of symlinking it.

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
  main.rs     tray app: event loop (start/stop/quit), sessions, incremental report
  tray.rs     tray icon/menu (ksni, StatusNotifierItem)
  icon.rs     camera icon drawn programmatically (red dot when active)
  notify.rs   desktop notifications (start/stop)
  input.rs    ClickSource trait    → EvdevClickSource (Linux)        [Win: LL mouse hook]
  capture.rs  WindowCapturer trait → KdeCapturer (KWin ScreenShot2)  [Win: PrintWindow]
  cursor.rs   KwinCursor: global cursor pos via a KWin script → zbus sink
  a11y.rs     Atspi: GetAccessibleAtPoint over the a11y bus (with deadline) [Win: UIA]
  annotate.rs draws the click marker into the image
  i18n.rs     minimal, dependency-free translations (English, German)
  model.rs    Step/Button + description logic
  report.rs   HTML + Markdown
```

The platform-specific parts sit behind traits — one backend per OS, while the
rest (`model`, `report`, `annotate`) stays shared. A Windows backend
(`SetWindowsHookEx` + `PrintWindow` + UI Automation) is the planned next step.

## Languages

UI, notifications and the report are localized. The language is auto-detected
from `LANGUAGE`/`LC_ALL`/`LC_MESSAGES`/`LANG` (defaults to English). Currently
**English** and **German** ship in `src/i18n.rs`.

Adding a language is deliberately simple and compiler-checked:

- **a new string**: add a field to `Strings` — every language `static` is a
  struct literal, so the compiler forces each language to provide it;
- **a new language**: add one `static XX: Strings = …` and one match arm in
  `strings_for`. Placeholders (`{n}`, `{title}`, …) are identical across
  languages.

## Roadmap

- Windows backend (the trait structure is ready for it)
- More languages (PRs welcome — see `src/i18n.rs`)
- Pause/resume, click filtering, PDF export

## License

[0BSD](LICENSE) — do whatever you want with it. No conditions, no attribution
required.
