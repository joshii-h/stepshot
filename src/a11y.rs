//! Accessibility query via AT-SPI (over the a11y D-Bus).
//!
//! For a screen coordinate, returns the UI element located there (name + role),
//! e.g. button “Save”. Best effort: apps must expose their a11y tree (Qt with
//! accessibility, GTK, browsers with an active AT, …).
//!
//! Important: the tree traversal runs with a hard deadline on a separate thread —
//! a hung app (zbus timeout 25 s) must not block the recorder.

use anyhow::{Context, Result};
use std::sync::mpsc;
use std::time::Duration;
use zvariant::{OwnedObjectPath, OwnedValue, Value};

const REGISTRY: &str = "org.a11y.atspi.Registry";
const ROOT_PATH: &str = "/org/a11y/atspi/accessible/root";
const IFACE_ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const IFACE_COMPONENT: &str = "org.a11y.atspi.Component";
const NULL_PATH: &str = "/org/a11y/atspi/null";

/// Maximum time for an element_at query before we give up.
const QUERY_DEADLINE: Duration = Duration::from_millis(1500);

/// A detected UI element.
#[derive(Debug, Clone)]
pub struct Element {
    pub name: String,
    pub role: String,
}

impl Element {
    /// Description like “button ‘Save’” or just “text field”.
    pub fn describe(&self) -> String {
        match (self.role.trim(), self.name.trim()) {
            (r, n) if !r.is_empty() && !n.is_empty() => format!("{r} “{n}”"),
            (r, _) if !r.is_empty() => r.to_string(),
            (_, n) if !n.is_empty() => format!("“{n}”"),
            _ => crate::i18n::tr().element_generic.to_string(),
        }
    }
}

/// A reference to an AT-SPI object (bus name + object path).
type Ref = (String, String);

pub struct Atspi {
    probe: Probe,
    /// Session bus (for enabling/disabling a11y).
    session: zbus::blocking::Connection,
    /// Previous IsEnabled state, to restore it.
    prev_enabled: Option<bool>,
}

impl Atspi {
    /// Connects to the a11y bus (whose address the session provides).
    pub fn connect() -> Result<Self> {
        let session = zbus::blocking::Connection::session().context("session bus unreachable")?;
        let addr: String = session
            .call_method(
                Some("org.a11y.Bus"),
                "/org/a11y/bus",
                Some("org.a11y.Bus"),
                "GetAddress",
                &(),
            )
            .context("could not query a11y bus address")?
            .body()
            .deserialize()
            .context("could not read a11y bus address")?;

        let bus = zbus::blocking::connection::Builder::address(addr.as_str())
            .context("invalid a11y bus address")?
            .build()
            .context("could not connect to the a11y bus")?;

        Ok(Self {
            probe: Probe { bus },
            session,
            prev_enabled: None,
        })
    }

    /// Enables AT-SPI system-wide (remembering the previous state).
    pub fn enable(&mut self) {
        self.prev_enabled = self.get_status_bool("IsEnabled");
        self.set_status_bool("IsEnabled", true);
        self.set_status_bool("ScreenReaderEnabled", true);
    }

    /// Restores the previous a11y state.
    pub fn restore(&self) {
        if let Some(false) = self.prev_enabled {
            self.set_status_bool("IsEnabled", false);
            self.set_status_bool("ScreenReaderEnabled", false);
        }
    }

    /// Element at screen coordinate (x, y) — with a hard deadline.
    pub fn element_at(&self, x: i32, y: i32) -> Option<Element> {
        let bus = self.probe.bus.clone();
        let (tx, rx) = mpsc::channel();
        // Worker thread: if it blocks on a hung app, we still give up after the
        // deadline (the thread keeps running in the background).
        std::thread::spawn(move || {
            let probe = Probe { bus };
            let _ = tx.send(probe.element_at_inner(x, y));
        });
        rx.recv_timeout(QUERY_DEADLINE).ok().flatten()
    }

    /// Debug: print the tree (name + role) up to `max_depth`.
    pub fn debug_dump(&self, max_depth: u32) {
        let root = (REGISTRY.to_string(), ROOT_PATH.to_string());
        self.probe.dump(&root, 0, max_depth);
    }

    /// Debug: find the first named button in the tree → (name, center x, y).
    pub fn debug_first_button(&self) -> Option<(String, i32, i32)> {
        let root = (REGISTRY.to_string(), ROOT_PATH.to_string());
        self.probe.find_button(&root, 0)
    }

    fn get_status_bool(&self, prop: &str) -> Option<bool> {
        let reply = self
            .session
            .call_method(
                Some("org.a11y.Bus"),
                "/org/a11y/bus",
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &("org.a11y.Status", prop),
            )
            .ok()?;
        let val: OwnedValue = reply.body().deserialize().ok()?;
        match &*val {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    fn set_status_bool(&self, prop: &str, value: bool) {
        let variant = Value::Bool(value);
        let _ = self.session.call_method(
            Some("org.a11y.Bus"),
            "/org/a11y/bus",
            Some("org.freedesktop.DBus.Properties"),
            "Set",
            &("org.a11y.Status", prop, &variant),
        );
    }
}

/// Read-only traversal on an a11y bus connection (usable from the worker thread).
struct Probe {
    bus: zbus::blocking::Connection,
}

impl Probe {
    /// Element at screen coordinate (x, y) — deepest node there.
    fn element_at_inner(&self, x: i32, y: i32) -> Option<Element> {
        let apps = self.children(&(REGISTRY.to_string(), ROOT_PATH.to_string()));
        for app in &apps {
            for frame in self.children(app) {
                if let Some(node) = self.descend(frame, x, y) {
                    let name = self.name(&node);
                    let role = self.role_name(&node);
                    if !name.is_empty() || !role.is_empty() {
                        return Some(Element { name, role });
                    }
                }
            }
        }
        None
    }

    /// Descend from the frame to the deepest element at the point.
    fn descend(&self, frame: Ref, x: i32, y: i32) -> Option<Ref> {
        let mut cur = frame;
        let mut steps = 0;
        loop {
            match self.at_point(&cur, x, y) {
                Some(child) if child != cur => {
                    cur = child;
                    steps += 1;
                    if steps > 40 {
                        break;
                    }
                }
                _ => break,
            }
        }
        // steps == 0 → the point was not inside this window.
        if steps == 0 { None } else { Some(cur) }
    }

    fn at_point(&self, r: &Ref, x: i32, y: i32) -> Option<Ref> {
        let reply = self
            .bus
            .call_method(
                Some(r.0.as_str()),
                r.1.as_str(),
                Some(IFACE_COMPONENT),
                "GetAccessibleAtPoint",
                &(x, y, 0u32), // 0 = screen coordinates
            )
            .ok()?;
        let (svc, path): (String, OwnedObjectPath) = reply.body().deserialize().ok()?;
        let path = path.as_str().to_string();
        if svc.is_empty() || path == NULL_PATH || path.is_empty() {
            None
        } else {
            Some((svc, path))
        }
    }

    fn children(&self, r: &Ref) -> Vec<Ref> {
        let reply = match self.bus.call_method(
            Some(r.0.as_str()),
            r.1.as_str(),
            Some(IFACE_ACCESSIBLE),
            "GetChildren",
            &(),
        ) {
            Ok(reply) => reply,
            Err(_) => return Vec::new(),
        };
        let list: Vec<(String, OwnedObjectPath)> = reply.body().deserialize().unwrap_or_default();
        list.into_iter()
            .map(|(s, p)| (s, p.as_str().to_string()))
            .filter(|(_, p)| p != NULL_PATH)
            .collect()
    }

    fn name(&self, r: &Ref) -> String {
        let reply = self.bus.call_method(
            Some(r.0.as_str()),
            r.1.as_str(),
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(IFACE_ACCESSIBLE, "Name"),
        );
        reply
            .ok()
            .and_then(|r| r.body().deserialize::<OwnedValue>().ok())
            .and_then(|v| match &*v {
                Value::Str(s) => Some(s.to_string()),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn role_name(&self, r: &Ref) -> String {
        self.bus
            .call_method(
                Some(r.0.as_str()),
                r.1.as_str(),
                Some(IFACE_ACCESSIBLE),
                "GetRoleName",
                &(),
            )
            .ok()
            .and_then(|reply| reply.body().deserialize::<String>().ok())
            .unwrap_or_default()
    }

    fn extents(&self, r: &Ref) -> Option<(i32, i32, i32, i32)> {
        let reply = self
            .bus
            .call_method(
                Some(r.0.as_str()),
                r.1.as_str(),
                Some(IFACE_COMPONENT),
                "GetExtents",
                &(0u32,), // 0 = screen
            )
            .ok()?;
        reply.body().deserialize::<(i32, i32, i32, i32)>().ok()
    }

    fn find_button(&self, r: &Ref, depth: u32) -> Option<(String, i32, i32)> {
        if depth > 25 {
            return None;
        }
        if self.role_name(r).contains("button") {
            let name = self.name(r);
            if !name.is_empty()
                && let Some((x, y, w, h)) = self.extents(r)
                && w > 0
                && h > 0
            {
                return Some((name, x + w / 2, y + h / 2));
            }
        }
        for ch in self.children(r) {
            if let Some(found) = self.find_button(&ch, depth + 1) {
                return Some(found);
            }
        }
        None
    }

    fn dump(&self, r: &Ref, depth: u32, max_depth: u32) {
        if depth > max_depth {
            return;
        }
        let role = self.role_name(r);
        let name = self.name(r);
        println!(
            "{}[{}] “{}”  ({})",
            "  ".repeat(depth as usize),
            role,
            name,
            r.0
        );
        for ch in self.children(r) {
            self.dump(&ch, depth + 1, max_depth);
        }
    }
}
