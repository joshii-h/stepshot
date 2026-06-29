//! Global click capture.
//!
//! On Wayland there is no protocol for system-wide input monitoring, so we read
//! the evdev devices in `/dev/input` directly. This works without root because
//! the user is in the `input` group.
//!
//! The `ClickSource` trait abstracts the platform; a future Windows backend
//! (low-level mouse hook) simply implements the same trait.

use crate::model::{Button, Click};
use crate::platform::ClickSource;
use anyhow::{Context, Result};
use std::sync::mpsc::Sender;
use std::thread;

/// evdev-based backend for Linux (Wayland & X11).
pub struct EvdevClickSource;

impl ClickSource for EvdevClickSource {
    fn start(&self, tx: Sender<Click>) -> Result<()> {
        let pointers = pointer_devices().context("could not enumerate mouse devices")?;
        anyhow::ensure!(
            !pointers.is_empty(),
            "no pointing device with mouse buttons found. Is the user in the `input` group?"
        );

        for (path, device) in pointers {
            let tx = tx.clone();
            // Each device gets its own thread with a blocking read.
            thread::Builder::new()
                .name(format!("evdev:{}", path))
                .spawn(move || device_loop(path, device, tx))
                .context("could not start input thread")?;
        }
        Ok(())
    }
}

/// All evdev devices that support mouse buttons (i.e. mice/touchpads).
fn pointer_devices() -> Result<Vec<(String, evdev::Device)>> {
    let mut out = Vec::new();
    for (path, device) in evdev::enumerate() {
        let is_pointer = device
            .supported_keys()
            .map(|keys| keys.contains(evdev::KeyCode::BTN_LEFT))
            .unwrap_or(false);
        if is_pointer {
            out.push((path.to_string_lossy().into_owned(), device));
        }
    }
    Ok(out)
}

/// Blocking read loop for a single device.
fn device_loop(path: String, mut device: evdev::Device, tx: Sender<Click>) {
    loop {
        let events = match device.fetch_events() {
            Ok(ev) => ev,
            Err(e) => {
                eprintln!("[stepshot] read error on {path}: {e} — thread exiting.");
                return;
            }
        };
        for ev in events {
            // Only button press (value == 1), not release/repeat.
            if ev.event_type() == evdev::EventType::KEY
                && ev.value() == 1
                && let Some(button) = Button::from_evdev_code(ev.code())
            {
                // Receiver gone = recording finished; exit cleanly.
                if tx.send(Click { button }).is_err() {
                    return;
                }
            }
        }
    }
}
