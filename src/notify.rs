//! Desktop notifications for feedback (recording started/stopped).
//! Via `org.freedesktop.Notifications` (session D-Bus).

use std::collections::HashMap;
use zvariant::Value;

/// Shows a notification. Errors are intentionally ignored (feedback only).
pub fn notify(conn: &zbus::blocking::Connection, summary: &str, body: &str, icon: &str) {
    let hints: HashMap<&str, Value> = HashMap::new();
    let actions: Vec<&str> = Vec::new();
    let _ = conn.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &(
            "stepshot", // app_name
            0u32,       // replaces_id
            icon,       // app_icon
            summary,    // summary
            body,       // body
            actions,    // actions
            hints,      // hints
            4000i32,    // timeout ms
        ),
    );
}
