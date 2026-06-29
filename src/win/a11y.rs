//! Element naming via UI Automation.
//!
//! `IUIAutomation::ElementFromPoint` gives the control under the cursor; its
//! name and localized control type become the element description (e.g. button
//! “Save”). UI Automation is always available, so `enable`/`restore` are no-ops.
//!
//! COM is initialized on the thread that constructs this resolver, and the
//! `IUIAutomation` pointer is used from that same thread (the main loop).

use crate::platform::{Element, ElementResolver};
use anyhow::{Context, Result};
use windows::Win32::Foundation::POINT;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};

pub struct UiaResolver {
    automation: IUIAutomation,
}

impl UiaResolver {
    pub fn connect() -> Result<Self> {
        unsafe {
            // Best effort: if COM is already initialized on this thread, this
            // returns a non-fatal code that we deliberately ignore.
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                    .context("could not create the UI Automation object")?;
            Ok(Self { automation })
        }
    }
}

impl ElementResolver for UiaResolver {
    fn enable(&mut self) {}
    fn restore(&self) {}

    fn element_at(&self, x: i32, y: i32) -> Option<Element> {
        unsafe {
            let el = self.automation.ElementFromPoint(POINT { x, y }).ok()?;
            let name = el
                .CurrentName()
                .map(|b| b.to_string())
                .unwrap_or_default()
                .trim()
                .to_string();
            let role = el
                .CurrentLocalizedControlType()
                .map(|b| b.to_string())
                .unwrap_or_default()
                .trim()
                .to_string();
            if name.is_empty() && role.is_empty() {
                None
            } else {
                Some(Element { name, role })
            }
        }
    }
}
