//! The window's chrome where the editor's own bars are its title bar
//! (macOS): the room the window buttons take at a bar's leading end, the
//! empty stretches of a bar that move the window, and the window's AppKit
//! side (the file it represents, its number for a capture).

use crate::theme;
use gpui::{prelude::*, Window};
use std::sync::atomic::{AtomicBool, Ordering};

/// What a bar along the window's top at its leading end (the sidebar's
/// top; the toolbar with the sidebar away) leaves for macOS's window
/// buttons in `window` (`WINDOW_BUTTONS_ROOM`, where its first control
/// starts); none elsewhere, or in full screen, where there are none
/// beside it. See [`title_bar_lead`].
pub fn window_buttons_room(window: &Window) -> f32 {
    if cfg!(target_os = "macos") && !window.is_fullscreen() {
        theme::layout::WINDOW_BUTTONS_ROOM
    } else {
        0.0
    }
}

/// Where the first control of a bar along the window's top edge at its
/// leading end starts (the sidebar's top; the toolbar with the sidebar
/// away): past macOS's window buttons ([`window_buttons_room`]), else 8 in.
pub fn title_bar_lead(window: &Window) -> f32 {
    lead_past(window_buttons_room(window))
}

/// A leading bar's first control's place, past `room` for the window
/// buttons and never nearer the edge than the bars' own 8.
pub fn lead_past(room: f32) -> f32 {
    room.max(8.0)
}

/// A press on a title bar's empty stretch, its drag still to come (macOS).
static TITLE_PRESSED: AtomicBool = AtomicBool::new(false);

/// `stretch`, an empty stretch of the toolbar or the sidebar's top (the
/// title bar on macOS): there, a press then a drag on it moves the window
/// and a double-click does what the system has a title bar's do (zoom, as
/// a rule). Elsewhere, where the system's title bar does that, it is left
/// as it is.
pub fn title_bar_stretch(stretch: gpui::Div) -> gpui::Div {
    if !cfg!(target_os = "macos") {
        return stretch;
    }
    stretch
        .on_mouse_down(
            gpui::MouseButton::Left,
            |event: &gpui::MouseDownEvent, window, _| {
                TITLE_PRESSED.store(event.click_count < 2, Ordering::Relaxed);
                if event.click_count == 2 {
                    window.titlebar_double_click();
                }
            },
        )
        .on_mouse_up(gpui::MouseButton::Left, |_, _, _| {
            TITLE_PRESSED.store(false, Ordering::Relaxed)
        })
        .on_mouse_move(|event: &gpui::MouseMoveEvent, window, _| {
            if event.pressed_button == Some(gpui::MouseButton::Left)
                && TITLE_PRESSED.swap(false, Ordering::Relaxed)
            {
                drag_window(window);
            }
        })
}

/// On macOS, hands a press on the title bar's empty stretch over to the
/// window server, which moves the window as the pointer drags (AppKit's
/// performWindowDragWithEvent with the event being handled); gpui 0.2.2's
/// own `start_window_move` does nothing there. Elsewhere nothing: the system's title bar moves the window.
pub fn drag_window(window: &Window) {
    #[cfg(target_os = "macos")]
    macos::drag(window);
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}

/// AppKit, through its safe bindings.
#[cfg(target_os = "macos")]
pub mod macos {
    use objc2::rc::Retained;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSView, NSWindow};
    use objc2_foundation::NSString;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::ffi::c_void;
    use std::path::Path;

    /// The NSWindow that holds `window`'s view (GPUI's, in its content view).
    fn ns_window(window: &impl HasWindowHandle) -> Option<Retained<NSWindow>> {
        let handle = window.window_handle().ok()?;
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return None;
        };
        let main = MainThreadMarker::new()?;
        let view = appkit.ns_view.as_ptr().cast_const();
        let is_view =
            |candidate: &Retained<NSView>| Retained::as_ptr(candidate).cast::<c_void>() == view;
        NSApplication::sharedApplication(main)
            .windows()
            .to_vec()
            .into_iter()
            .find(|ns_window| {
                ns_window.contentView().is_some_and(|content| {
                    is_view(&content) || content.subviews().to_vec().iter().any(&is_view)
                })
            })
    }

    /// Has the NSWindow that holds `window`'s view represent `file`.
    pub fn represent(window: &impl HasWindowHandle, file: &Path) {
        if let Some(ns_window) = ns_window(window) {
            ns_window.setRepresentedFilename(&NSString::from_str(&file.to_string_lossy()));
        }
    }

    /// Hands the press being handled to the window server, which moves the
    /// NSWindow that holds `window`'s view as the pointer drags.
    pub(super) fn drag(window: &impl HasWindowHandle) {
        let Some(main) = MainThreadMarker::new() else {
            return;
        };
        let Some(event) = NSApplication::sharedApplication(main).currentEvent() else {
            return;
        };
        if let Some(ns_window) = ns_window(window) {
            ns_window.performWindowDragWithEvent(&event);
        }
    }

    /// The window server's number for `window`'s NSWindow (what
    /// `screencapture -l` takes).
    pub fn window_number(window: &impl HasWindowHandle) -> Option<isize> {
        ns_window(window)
            .map(|ns_window| ns_window.windowNumber())
            .filter(|number| *number > 0)
    }
}
