//! Tray icon (StatusNotifierItem) — the app's control center.
//!
//! The app lives in the tray; recording is started/stopped from here. Menu
//! actions send commands to the main loop. The clicks that operate this menu
//! are kept out of the recording: on Start the queued clicks are drained, on
//! Stop/Quit the trailing gesture steps are trimmed (`trim_stop_gesture`).

use crate::icon;
use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{MenuItem, Tray};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;

/// Control commands from the tray to the main loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    Start,
    Stop,
    /// Toggle pause while recording (stop capturing clicks, keep the session).
    TogglePause,
    /// Toggle keyboard-step capture (opt-in, see `keys`).
    ToggleKeyboard,
    OpenFolder,
    /// Open the in-browser editor for the last recorded session.
    EditLast,
    /// Quit initiated from the tray menu — the clicks of that gesture are
    /// trimmed from the recording.
    Quit,
    /// Quit initiated by a signal (Ctrl+C) — no tray clicks to trim.
    Terminate,
}

pub struct StepshotTray {
    pub tx: Sender<Cmd>,
    pub recording: Arc<AtomicBool>,
    pub paused: Arc<AtomicBool>,
    pub steps: Arc<AtomicUsize>,
    pub keyboard: Arc<AtomicBool>,
}

impl Tray for StepshotTray {
    fn id(&self) -> String {
        "org.stepshot.Stepshot".into()
    }

    fn title(&self) -> String {
        "stepshot".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![icon::tray_icon(self.recording.load(Ordering::SeqCst))]
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let t = crate::i18n::tr();
        let rec = self.recording.load(Ordering::SeqCst);
        let paused = self.paused.load(Ordering::SeqCst);
        let n = self.steps.load(Ordering::SeqCst).to_string();
        let desc = if rec && paused {
            t.tt_paused.replace("{n}", &n)
        } else if rec {
            t.tt_recording.replace("{n}", &n)
        } else {
            t.tt_ready.to_string()
        };
        ksni::ToolTip {
            icon_name: String::new(),
            icon_pixmap: vec![icon::tray_icon(rec)],
            title: "stepshot".into(),
            description: desc,
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let t = crate::i18n::tr();
        let rec = self.recording.load(Ordering::SeqCst);
        let paused = self.paused.load(Ordering::SeqCst);
        let n = self.steps.load(Ordering::SeqCst).to_string();

        let header = if rec && paused {
            t.tray_paused.replace("{n}", &n)
        } else if rec {
            t.tray_recording.replace("{n}", &n)
        } else {
            t.tray_ready.to_string()
        };

        let toggle: MenuItem<Self> = if rec {
            StandardItem {
                label: t.menu_stop.into(),
                icon_name: "media-playback-stop".into(),
                activate: Box::new(|t: &mut StepshotTray| {
                    let _ = t.tx.send(Cmd::Stop);
                }),
                ..Default::default()
            }
            .into()
        } else {
            StandardItem {
                label: t.menu_start.into(),
                icon_name: "media-record".into(),
                activate: Box::new(|t: &mut StepshotTray| {
                    let _ = t.tx.send(Cmd::Start);
                }),
                ..Default::default()
            }
            .into()
        };

        let mut items: Vec<MenuItem<Self>> = vec![
            StandardItem {
                label: header,
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            toggle,
        ];

        // Pause/resume only makes sense while a recording is running.
        if rec {
            let (label, icon_name) = if paused {
                (t.menu_resume, "media-playback-start")
            } else {
                (t.menu_pause, "media-playback-pause")
            };
            items.push(
                StandardItem {
                    label: label.into(),
                    icon_name: icon_name.into(),
                    activate: Box::new(|t: &mut StepshotTray| {
                        let _ = t.tx.send(Cmd::TogglePause);
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }

        items.push(
            CheckmarkItem {
                label: t.menu_keyboard.into(),
                checked: self.keyboard.load(Ordering::SeqCst),
                activate: Box::new(|t: &mut StepshotTray| {
                    let _ = t.tx.send(Cmd::ToggleKeyboard);
                }),
                ..Default::default()
            }
            .into(),
        );

        items.extend([
            StandardItem {
                label: t.menu_open_folder.into(),
                icon_name: "folder-open".into(),
                activate: Box::new(|t: &mut StepshotTray| {
                    let _ = t.tx.send(Cmd::OpenFolder);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: t.menu_edit_last.into(),
                icon_name: "document-edit".into(),
                activate: Box::new(|t: &mut StepshotTray| {
                    let _ = t.tx.send(Cmd::EditLast);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: t.menu_quit.into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|t: &mut StepshotTray| {
                    let _ = t.tx.send(Cmd::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]);
        items
    }
}
