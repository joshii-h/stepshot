//! Tray icon (StatusNotifierItem) — the app's control center.
//!
//! The app lives in the tray; recording is started/stopped from here. The icon
//! sits in the panel and is therefore never in the screenshots (we only
//! photograph the active window). Menu actions send commands to the main loop.

use crate::icon;
use ksni::menu::StandardItem;
use ksni::{MenuItem, Tray};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;

/// Control commands from the tray to the main loop.
#[derive(Debug, Clone, Copy)]
pub enum Cmd {
    Start,
    Stop,
    OpenFolder,
    Quit,
}

pub struct StepshotTray {
    pub tx: Sender<Cmd>,
    pub recording: Arc<AtomicBool>,
    pub steps: Arc<AtomicUsize>,
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
        let desc = if rec {
            t.tt_recording
                .replace("{n}", &self.steps.load(Ordering::SeqCst).to_string())
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

        let header = if rec {
            t.tray_recording
                .replace("{n}", &self.steps.load(Ordering::SeqCst).to_string())
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

        vec![
            StandardItem {
                label: header,
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            toggle,
            StandardItem {
                label: t.menu_open_folder.into(),
                icon_name: "folder-open".into(),
                activate: Box::new(|t: &mut StepshotTray| {
                    let _ = t.tx.send(Cmd::OpenFolder);
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
        ]
    }
}
