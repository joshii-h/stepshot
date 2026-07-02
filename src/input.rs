//! Global click capture.
//!
//! On Wayland there is no protocol for system-wide input monitoring, so we read
//! the evdev devices in `/dev/input` directly. This works without root because
//! the user is in the `input` group.
//!
//! Hotplug: a supervisor thread rescans `/dev/input` periodically, so a mouse
//! plugged in (or re-plugged) after startup is picked up; a device whose read
//! loop dies simply gets re-adopted on the next scan.
//!
//! The `ClickSource` trait abstracts the platform; a future Windows backend
//! (low-level mouse hook) simply implements the same trait.

use crate::model::{Button, Click};
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// How often the supervisor rescans `/dev/input` for new/re-plugged devices.
const RESCAN_INTERVAL: Duration = Duration::from_secs(3);

/// A source of global clicks. Reports every button press over the channel.
pub trait ClickSource {
    /// Starts capturing. Clicks are delivered asynchronously over `tx`.
    fn start(&self, tx: Sender<Click>) -> Result<()>;
}

/// evdev-based backend for Linux (Wayland & X11).
pub struct EvdevClickSource;

impl ClickSource for EvdevClickSource {
    fn start(&self, tx: Sender<Click>) -> Result<()> {
        let pointers = pointer_devices();
        anyhow::ensure!(
            !pointers.is_empty(),
            "no pointing device with mouse buttons found. Is the user in the `input` group?"
        );

        // Paths that currently have a reader thread; a thread removes its own
        // path when it exits, so the supervisor can re-adopt the device.
        let active: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

        for (path, device) in pointers {
            spawn_device_thread(path, device, tx.clone(), active.clone())?;
        }

        // Supervisor: rescan for devices that appeared (or died) after startup.
        thread::Builder::new()
            .name("evdev-rescan".into())
            .spawn(move || {
                loop {
                    thread::sleep(RESCAN_INTERVAL);
                    for (path, device) in pointer_devices() {
                        let known = active.lock().map(|a| a.contains(&path)).unwrap_or(true);
                        if !known
                            && spawn_device_thread(path, device, tx.clone(), active.clone())
                                .is_err()
                        {
                            return; // thread spawning is broken; give up quietly
                        }
                    }
                }
            })
            .context("could not start rescan thread")?;
        Ok(())
    }
}

/// Registers `path` as active and starts its blocking read loop.
fn spawn_device_thread(
    path: String,
    device: evdev::Device,
    tx: Sender<Click>,
    active: Arc<Mutex<HashSet<String>>>,
) -> Result<()> {
    if let Ok(mut a) = active.lock() {
        a.insert(path.clone());
    }
    thread::Builder::new()
        .name(format!("evdev:{}", path))
        .spawn(move || {
            device_loop(&path, device, tx);
            if let Ok(mut a) = active.lock() {
                a.remove(&path);
            }
        })
        .context("could not start input thread")?;
    Ok(())
}

/// All evdev devices that support mouse buttons (i.e. mice/touchpads).
fn pointer_devices() -> Vec<(String, evdev::Device)> {
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
    out
}

/// Blocking read loop for a single device. Returns when the device goes away
/// (unplug) or the receiver is gone; the supervisor re-adopts on re-plug.
fn device_loop(path: &str, mut device: evdev::Device, tx: Sender<Click>) {
    loop {
        let events = match device.fetch_events() {
            Ok(ev) => ev,
            Err(e) => {
                eprintln!("[stepshot] read error on {path}: {e} — device dropped.");
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
