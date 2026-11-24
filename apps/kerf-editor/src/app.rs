//! Opening the editor's windows: a project in a window of its own, or the
//! launcher when there is none; following kerf:// links to them; and what
//! Kerf says on its own at launch (a crash since the last notice, a newer
//! release).
//!
//! One window per project: opening or following a link to a project that
//! is open raises its window. What answers questions over a window
//! (`Window::prompt`) is drawn in Kerf's own look off macOS ([`install_prompts`]).

use crate::workspace::Workspace;
use editor_launcher::launcher::{LauncherEvent, LauncherView};
use editor_session::session::{hear_cloud, started_kerfd, BridgeEvent, Session};
use editor_state::folders::same_folder;
use editor_state::prefs::Prefs;
use editor_state::store::Change;
use editor_ui::motion;
use editor_ui::theme::{self, radius, text};
use editor_ui::ui;
use gpui::{
    actions, div, prelude::*, px, size, AnimationExt, App, AppContext, Bounds, Context, Entity,
    EventEmitter, FocusHandle, Focusable, FontWeight, Global, KeyBinding, Pixels, PromptButton,
    PromptHandle, PromptLevel, PromptResponse, RenderablePromptHandle, TitlebarOptions, WeakEntity,
    Window, WindowBounds, WindowHandle, WindowOptions,
};
use kerf_bridge::Event;
use serde_json::Value;
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// The options a project's window opens with, titled `title`: at
/// `layout::WINDOW` in the middle of the screen, or filling a screen too
/// small for that; or, for the first window of a run given `--window-size`
/// ([`set_window_size`]), that size there. The size asked is the first
/// window's alone: a window opened later in the run opens as it would
/// without it. A window that takes another's place opens where that one
/// was instead ([`options_at`]). See [`window_place`].
pub fn options(title: &str, cx: &mut App) -> WindowOptions {
    options_at(title, None, cx)
}

/// [`options`], in `place` when the window takes the place of another
/// (the window it follows, as `window.window_bounds()` gave it): where
/// that one was, at its size, maximized or full screen if it was, so
/// opening another project does not move the window from where the person
/// had it.
pub fn options_at(title: &str, place: Option<WindowBounds>, cx: &mut App) -> WindowOptions {
    opening(titlebar(title), place, cx)
}

/// The launcher's window options ([`options_at`]'s, in `place`), with
/// the system's title bar on macOS too: the launcher has no toolbar to be
/// its title bar, nor a stretch of one to move the window by.
pub fn launcher_options(place: Option<WindowBounds>, cx: &mut App) -> WindowOptions {
    let titlebar = TitlebarOptions {
        title: Some("Industrial Kerf".into()),
        ..Default::default()
    };
    opening(titlebar, place, cx)
}

/// A window's options with `titlebar`, where [`window_place`] puts it
/// on the primary display (the one a window opens on): in `place`, else
/// at the size `--window-size` asked for the run's first window, else at
/// `layout::WINDOW`.
fn opening(titlebar: TitlebarOptions, place: Option<WindowBounds>, cx: &mut App) -> WindowOptions {
    let asked = cx.try_global::<WindowSize>().and_then(|asked| asked.0);
    if asked.is_some() {
        cx.set_global(WindowSize(None));
    }
    let asked = asked.map(|(width, height)| size(px(width), px(height)));
    let screen = cx.primary_display().map(|display| display.bounds());
    let (least_width, least_height) = theme::layout::WINDOW_LEAST;
    WindowOptions {
        window_bounds: Some(window_place(place, asked, screen)),
        titlebar: Some(titlebar),
        app_id: Some("industrial-kerf".into()),
        window_min_size: Some(size(px(least_width), px(least_height))),
        ..Default::default()
    }
}

/// Where a window opens, on a screen whose bounds are `screen`:
///
/// - in `place` when it takes another window's place: where that one
///   was, at its size, maximized or full screen as it was;
/// - at `asked` (`--window-size`), windowed, in the screen's middle; one
///   bigger than the screen starts at its top left corner, never past it,
///   so that the toolbar is on the screen;
/// - else at `layout::WINDOW`, windowed in the screen's middle when it
///   fits there with what the screen keeps around a window
///   (`layout::SCREEN_KEPT`: its title bar, the menu bar, the Dock or a
///   taskbar). On a smaller screen it fills the screen (maximized), and
///   put back it is the most of `layout::WINDOW` that fits there, in its
///   middle, none of it off the screen.
///
/// With no screen known, windowed at the screen's origin.
pub fn window_place(
    place: Option<WindowBounds>,
    asked: Option<gpui::Size<Pixels>>,
    screen: Option<Bounds<Pixels>>,
) -> WindowBounds {
    use theme::layout::{SCREEN_KEPT, WINDOW, WINDOW_LEAST};
    if let Some(place) = place {
        return place;
    }
    let wanted = asked.unwrap_or_else(|| size(px(WINDOW.0), px(WINDOW.1)));
    let Some(screen) = screen else {
        return WindowBounds::Windowed(Bounds::new(gpui::point(px(0.0), px(0.0)), wanted));
    };
    let (screen_width, screen_height) =
        (f32::from(screen.size.width), f32::from(screen.size.height));
    // In the screen's middle, its top left corner never past the screen's.
    let middle = |width: f32, height: f32| {
        let left = f32::from(screen.origin.x) + ((screen_width - width) / 2.0).max(0.0);
        let top = f32::from(screen.origin.y) + ((screen_height - height) / 2.0).max(0.0);
        Bounds::new(gpui::point(px(left), px(top)), size(px(width), px(height)))
    };
    let (width, height) = (f32::from(wanted.width), f32::from(wanted.height));
    if asked.is_some() {
        return WindowBounds::Windowed(middle(width, height));
    }
    let room = (screen_width - SCREEN_KEPT.0, screen_height - SCREEN_KEPT.1);
    if width <= room.0 && height <= room.1 {
        WindowBounds::Windowed(middle(width, height))
    } else {
        WindowBounds::Maximized(middle(
            width.min(room.0).max(WINDOW_LEAST.0),
            height.min(room.1).max(WINDOW_LEAST.1),
        ))
    }
}

/// A window's title bar, titled `title`. On macOS the toolbar is the title
/// bar, so the window spends no line on a bar that holds only its title:
/// the window runs up under a transparent title bar with no title, and the
/// window buttons stand on the toolbar's line at its leading end
/// ([`window_buttons_at`]). Elsewhere the system's title bar stays.
pub fn titlebar(title: &str) -> TitlebarOptions {
    TitlebarOptions {
        title: Some(title.to_owned().into()),
        appears_transparent: cfg!(target_os = "macos"),
        traffic_light_position: cfg!(target_os = "macos").then(window_buttons_at),
    }
}

/// Where macOS's window buttons stand, from the window's top left: their
/// first one's left edge at `WINDOW_BUTTONS_LEFT`, on the middle of the
/// toolbar's height.
pub fn window_buttons_at() -> gpui::Point<Pixels> {
    use theme::layout::{TOOLBAR, WINDOW_BUTTON, WINDOW_BUTTONS_LEFT};
    gpui::point(px(WINDOW_BUTTONS_LEFT), px((TOOLBAR - WINDOW_BUTTON) / 2.0))
}

/// The size `--window-size` asked the first window to open at, until
/// that window takes it.
struct WindowSize(Option<(f32, f32)>);

impl Global for WindowSize {}

/// Opens the next window (the run's first, as `main` calls it) at `size`
/// (points) rather than edge to edge, for a capture; the windows after it
/// open as they would. `main` calls it with `--window-size`.
pub fn set_window_size(size: Option<(f32, f32)>, cx: &mut App) {
    if let Some(size) = size {
        cx.set_global(WindowSize(Some(size)));
    }
}

/// `--window-size`'s WxH ("1600x1000"): both sides at least 1.
pub fn parse_window_size(text: &str) -> Option<(f32, f32)> {
    let (width, height) = text.trim().split_once(['x', 'X'])?;
    let side = |text: &str| {
        text.trim()
            .parse::<u32>()
            .ok()
            .filter(|side| (1..=16384).contains(side))
    };
    Some((side(width)? as f32, side(height)? as f32))
}

/// `path` (PATH, `:` between folders) with the folders Homebrew and
/// `~/.local` keep tools in after it, each once: an app opened from Finder
/// or the Dock is given no shell PATH, and so would not find git, gh or
/// git-lfs. `main` sets it on macOS before anything runs a tool.
pub fn path_with_tool_folders(path: &str, home: &Path) -> String {
    let mut folders: Vec<String> = path
        .split(':')
        .filter(|folder| !folder.is_empty())
        .map(str::to_owned)
        .collect();
    let local = home
        .join(".local")
        .join("bin")
        .to_string_lossy()
        .into_owned();
    for folder in [
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/usr/local/bin",
        local.as_str(),
    ] {
        // A folder whose name has a colon cannot be written in a PATH.
        if !folder.contains(':') && !folders.iter().any(|known| known == folder) {
            folders.push(folder.to_owned());
        }
    }
    folders.join(":")
}

// ---------------------------------------------------------------- --screenshot

/// `--screenshot <file>`: once `after` has passed (`--quit-after-ms`, else
/// half a second), and the first window has drawn a frame since, writes a
/// PNG of that window to `file` and quits. On X11 the window is read back
/// from the X server; on macOS `screencapture` takes it; elsewhere
/// (Wayland, Windows) it says on stderr that it cannot, and quits. A window
/// that has drawn nothing by then (an X server with no window manager,
/// where GPUI waits for an event to draw) is woken (again each
/// `CAPTURE_REWAKE` while no frame comes), and given `CAPTURE_SETTLE` for
/// what comes in to be in place.
pub fn screenshot_then_quit(file: PathBuf, after: Duration, cx: &mut App) {
    let window = cx.windows().first().copied();
    // Whether the first window has begun a frame since it opened.
    let began = Rc::new(Cell::new(false));
    if let Some(window) = window {
        let began = began.clone();
        // A window that cannot be reached now counts as one that has drawn
        // nothing: it is woken below.
        let _ = window.update(cx, move |_, window, _| {
            window.on_next_frame(move |_, _| began.set(true));
        });
    }
    cx.spawn(async move |cx| {
        cx.background_executor().timer(after).await;
        let target = cx
            .update(|cx| match window {
                Some(window) => capture_target(window, cx),
                None => Err("no window is open".to_owned()),
            })
            .unwrap_or_else(|_| Err("the editor ended first".to_owned()));
        // A window that has begun no frame waits for an event to draw: it
        // is woken, as each frame is asked of it.
        let woken = !began.get();
        let wake = target.as_ref().ok().filter(|_| woken).cloned();
        // A frame drawn now, then (when the window was woken) another once
        // what came in has settled.
        let mut drawn = window.is_some() && frame_drawn(window, wake.clone(), cx).await;
        if drawn && woken {
            cx.background_executor()
                .timer(theme::motion::CAPTURE_SETTLE)
                .await;
            drawn = frame_drawn(window, wake, cx).await;
        }
        if window.is_some() && !drawn {
            log::warn!("--screenshot: the window drew no frame in time; reading what is there");
        }
        let written = match target {
            Ok(target) => {
                let file = file.clone();
                cx.background_executor()
                    .spawn(async move { capture(target, &file) })
                    .await
            }
            Err(message) => Err(message),
        };
        match written {
            Ok(()) => log::info!("--screenshot: wrote {}", file.display()),
            Err(message) => eprintln!("--screenshot {}: {message}", file.display()),
        }
        // Refused only once the editor has ended already.
        let _ = cx.update(|cx| cx.quit());
    })
    .detach();
}

/// Asks `window` for a frame and waits until it is on the screen (one
/// frame begun, and the next), at most `CAPTURE_FRAME_WAIT`; whether it
/// came. With `wake`, the window is woken once it has been asked, and again
/// each `CAPTURE_REWAKE` while no frame comes: a wake can go unheard.
async fn frame_drawn(
    window: Option<gpui::AnyWindowHandle>,
    wake: Option<CaptureTarget>,
    cx: &mut gpui::AsyncApp,
) -> bool {
    let drawn = Rc::new(Cell::new(false));
    let asked = window.and_then(|window| {
        let drawn = drawn.clone();
        cx.update(|cx| {
            window.update(cx, move |_, window, _| {
                window.refresh();
                window.on_next_frame(move |window, _| {
                    window.on_next_frame(move |_, _| drawn.set(true));
                });
            })
        })
        .ok()?
        .ok()
    });
    if asked.is_none() {
        return false;
    }
    let since = Instant::now();
    let mut woken_at: Option<Instant> = None;
    while !drawn.get() && since.elapsed() < theme::motion::CAPTURE_FRAME_WAIT {
        if let Some(target) = wake.clone()
            && woken_at.is_none_or(|at| at.elapsed() >= theme::motion::CAPTURE_REWAKE)
        {
            woken_at = Some(Instant::now());
            cx.background_executor()
                .spawn(async move { self::wake(&target) })
                .await;
        }
        cx.background_executor().timer(theme::motion::POLL).await;
    }
    drawn.get()
}

/// Wakes `target`'s window to draw where it waits for an event to (an X
/// server with no window manager); elsewhere nothing.
fn wake(target: &CaptureTarget) {
    match *target {
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        CaptureTarget::X11 { pid, size } => {
            if let Err(message) = x11::wake(pid, size) {
                log::warn!("--screenshot: {message}");
            }
        }
        #[cfg(target_os = "macos")]
        CaptureTarget::Mac(_) => {}
    }
}

/// How a window is read back (none: it cannot be, here).
#[derive(Clone)]
enum CaptureTarget {
    /// From the X server: the window of this process that shows, `size`
    /// pixels (the first made, if several do).
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    X11 { pid: u32, size: (u32, u32) },
    /// By `screencapture`, by its window number.
    #[cfg(target_os = "macos")]
    Mac(isize),
}

/// How `window` is read back here, or why it cannot be.
fn capture_target(window: gpui::AnyWindowHandle, cx: &mut App) -> Result<CaptureTarget, String> {
    #[cfg(target_os = "macos")]
    {
        window
            .update(cx, |_, window, _| {
                editor_ui::chrome::macos::window_number(window)
            })
            .ok()
            .flatten()
            .map(CaptureTarget::Mac)
            .ok_or_else(|| "the window has no number to capture it by".to_owned())
    }
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        match gpui::guess_compositor() {
            "X11" => {
                let size = window
                    .update(cx, |_, window, _| {
                        let size = window.viewport_size();
                        let scale = window.scale_factor();
                        (
                            (f32::from(size.width) * scale).round() as u32,
                            (f32::from(size.height) * scale).round() as u32,
                        )
                    })
                    .map_err(|_| "the window closed".to_owned())?;
                Ok(CaptureTarget::X11 {
                    pid: std::process::id(),
                    size,
                })
            }
            "Wayland" => Err(
                "not supported on Wayland; capture the window with the system's tools".to_owned(),
            ),
            other => Err(format!(
                "not supported here ({other}); capture the window with the system's tools"
            )),
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "freebsd")))]
    {
        let _ = (window, cx);
        Err("not supported on Windows; capture the window with the system's tools".to_owned())
    }
}

/// Reads `target` back and writes it to `file` as a PNG.
fn capture(target: CaptureTarget, file: &Path) -> Result<(), String> {
    // Unread where nothing can be captured (Windows): `CaptureTarget` has no kinds there.
    let _ = file;
    match target {
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        CaptureTarget::X11 { pid, size } => x11::read_window(pid, size)?
            .save_with_format(file, image::ImageFormat::Png)
            .map_err(|error| error.to_string()),
        #[cfg(target_os = "macos")]
        CaptureTarget::Mac(number) => {
            let status = std::process::Command::new("screencapture")
                .args(["-x", "-o", "-l", &number.to_string()])
                .arg(file)
                .status()
                .map_err(|error| format!("screencapture did not run: {error}"))?;
            if status.success() && file.is_file() {
                Ok(())
            } else {
                Err(format!("screencapture wrote nothing ({status})"))
            }
        }
    }
}

/// How an X server lays out a picture's pixels (`GetImage` in Z format):
/// bits a pixel and the bits a row is padded to (the pixmap format of its
/// depth), where red, green and blue are in a pixel (its visual), and
/// whether a pixel's bytes come most significant first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct X11Pixels {
    pub bits_per_pixel: u8,
    pub scanline_pad: u8,
    pub red_mask: u32,
    pub green_mask: u32,
    pub blue_mask: u32,
    pub msb_first: bool,
}

/// `data`, `width` × `height` pixels as an X server gave them (`format`),
/// as opaque RGBA; None for a layout it cannot be (a pixel of other than
/// 16, 24 or 32 bits, or too few bytes).
pub fn x11_rgba(
    data: &[u8],
    width: u32,
    height: u32,
    format: X11Pixels,
) -> Option<image::RgbaImage> {
    let bytes = match format.bits_per_pixel {
        16 => 2,
        24 => 3,
        32 => 4,
        _ => return None,
    };
    let pad = u64::from(format.scanline_pad.max(8));
    let row_bits = u64::from(width) * u64::from(format.bits_per_pixel);
    let stride = usize::try_from(row_bits.div_ceil(pad) * pad / 8).ok()?;
    let needed = stride.checked_mul(height as usize)?;
    if data.len() < needed || width == 0 || height == 0 {
        return None;
    }
    // A channel's mask, as 0..255.
    let channel = |pixel: u32, mask: u32| -> u8 {
        if mask == 0 {
            return 0;
        }
        let shift = mask.trailing_zeros();
        let most = u64::from(mask >> shift);
        let value = u64::from((pixel & mask) >> shift);
        ((value * 255 + most / 2) / most) as u8
    };
    let mut picture = image::RgbaImage::new(width, height);
    for (y, row) in data.chunks(stride).take(height as usize).enumerate() {
        for x in 0..width as usize {
            let at = &row[x * bytes..x * bytes + bytes];
            let pixel = if format.msb_first {
                at.iter()
                    .fold(0u32, |pixel, byte| pixel << 8 | u32::from(*byte))
            } else {
                at.iter()
                    .rev()
                    .fold(0u32, |pixel, byte| pixel << 8 | u32::from(*byte))
            };
            picture.put_pixel(
                x as u32,
                y as u32,
                image::Rgba([
                    channel(pixel, format.red_mask),
                    channel(pixel, format.green_mask),
                    channel(pixel, format.blue_mask),
                    255,
                ]),
            );
        }
    }
    Some(picture)
}

/// Reading a window back from the X server (a GPUI window on X11 gives
/// no raw handle of its own): the one of this process that shows, found by
/// its `_NET_WM_PID`.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod x11 {
    use super::{x11_rgba, X11Pixels};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        AtomEnum, ConnectionExt, EventMask, ExposeEvent, ImageFormat, ImageOrder, MapState, Window,
        EXPOSE_EVENT,
    };

    fn said(error: impl std::fmt::Display) -> String {
        format!("the X server: {error}")
    }

    /// The window of process `pid` that shows (the one `size` pixels
    /// large, else the first made), and its size.
    fn find(
        connection: &impl Connection,
        root: Window,
        pid: u32,
        size: (u32, u32),
    ) -> Result<(Window, (u32, u32)), String> {
        let pid_atom = connection
            .intern_atom(true, b"_NET_WM_PID")
            .map_err(said)?
            .reply()
            .map_err(said)?
            .atom;
        // Every window that shows, top-level or under a window manager's
        // frame, whose process is this one.
        let mut found: Vec<(Window, (u32, u32))> = Vec::new();
        let mut parents = vec![root];
        while let Some(parent) = parents.pop() {
            let Ok(tree) = connection.query_tree(parent).map_err(said)?.reply() else {
                continue;
            };
            for window in tree.children {
                let Ok(attributes) = connection
                    .get_window_attributes(window)
                    .map_err(said)?
                    .reply()
                else {
                    continue;
                };
                if attributes.map_state != MapState::VIEWABLE {
                    continue;
                }
                parents.push(window);
                let owner = connection
                    .get_property(false, window, pid_atom, AtomEnum::CARDINAL, 0, 1)
                    .map_err(said)?
                    .reply()
                    .ok()
                    .and_then(|property| property.value32().and_then(|mut values| values.next()));
                if owner != Some(pid) {
                    continue;
                }
                if let Ok(geometry) = connection.get_geometry(window).map_err(said)?.reply() {
                    found.push((
                        window,
                        (u32::from(geometry.width), u32::from(geometry.height)),
                    ));
                }
            }
        }
        found.sort_by_key(|(window, _)| *window);
        found
            .iter()
            .find(|(_, found)| *found == size)
            .or_else(|| found.first())
            .copied()
            .ok_or_else(|| "no window of the editor shows on the X server".to_owned())
    }

    /// Sends the window of process `pid` an Expose, so a GPUI that waits
    /// for an event before it draws (no window manager to send one) draws.
    pub(super) fn wake(pid: u32, size: (u32, u32)) -> Result<(), String> {
        let (connection, screen) = x11rb::connect(None).map_err(said)?;
        let root = connection
            .setup()
            .roots
            .get(screen)
            .ok_or_else(|| "the X server has no such screen".to_owned())?
            .root;
        let (window, (width, height)) = find(&connection, root, pid, size)?;
        let expose = ExposeEvent {
            response_type: EXPOSE_EVENT,
            sequence: 0,
            window,
            x: 0,
            y: 0,
            width: width as u16,
            height: height as u16,
            count: 0,
        };
        // No mask: to the client that made the window, the editor.
        connection
            .send_event(false, window, EventMask::NO_EVENT, expose)
            .map_err(said)?;
        connection.flush().map_err(said)
    }

    /// The window of process `pid` that shows (the one `size` pixels
    /// large, else the first made), as RGBA.
    pub(super) fn read_window(pid: u32, size: (u32, u32)) -> Result<image::RgbaImage, String> {
        let (connection, screen) = x11rb::connect(None).map_err(said)?;
        let setup = connection.setup();
        let root = setup
            .roots
            .get(screen)
            .ok_or_else(|| "the X server has no such screen".to_owned())?
            .root;
        let (window, (width, height)) = find(&connection, root, pid, size)?;
        // The window itself; where it is partly off the screen (which X
        // cannot read), what of it is on the screen, from the root.
        let (picture, width, height) = match connection
            .get_image(
                ImageFormat::Z_PIXMAP,
                window,
                0,
                0,
                width as u16,
                height as u16,
                !0,
            )
            .map_err(said)?
            .reply()
        {
            Ok(picture) => (picture, width, height),
            Err(_) => {
                let screen = &setup.roots[screen];
                let at = connection
                    .translate_coordinates(window, root, 0, 0)
                    .map_err(said)?
                    .reply()
                    .map_err(said)?;
                let left = at.dst_x.max(0);
                let top = at.dst_y.max(0);
                let right =
                    (i32::from(at.dst_x) + width as i32).min(i32::from(screen.width_in_pixels));
                let bottom =
                    (i32::from(at.dst_y) + height as i32).min(i32::from(screen.height_in_pixels));
                let (width, height) = (
                    (right - i32::from(left)).max(0) as u32,
                    (bottom - i32::from(top)).max(0) as u32,
                );
                if width == 0 || height == 0 {
                    return Err("the window is off the screen".to_owned());
                }
                let picture = connection
                    .get_image(
                        ImageFormat::Z_PIXMAP,
                        root,
                        left,
                        top,
                        width as u16,
                        height as u16,
                        !0,
                    )
                    .map_err(said)?
                    .reply()
                    .map_err(said)?;
                (picture, width, height)
            }
        };
        let format = setup
            .pixmap_formats
            .iter()
            .find(|format| format.depth == picture.depth)
            .ok_or_else(|| format!("no pixel format for depth {}", picture.depth))?;
        let visual = setup
            .roots
            .iter()
            .flat_map(|screen| &screen.allowed_depths)
            .flat_map(|depth| &depth.visuals)
            .find(|visual| visual.visual_id == picture.visual)
            .ok_or_else(|| "the window's visual is not the server's".to_owned())?;
        let pixels = X11Pixels {
            bits_per_pixel: format.bits_per_pixel,
            scanline_pad: format.scanline_pad,
            red_mask: visual.red_mask,
            green_mask: visual.green_mask,
            blue_mask: visual.blue_mask,
            msb_first: setup.image_byte_order == ImageOrder::MSB_FIRST,
        };
        x11_rgba(&picture.data, width, height, pixels).ok_or_else(|| {
            format!(
                "pixels of {} bits at depth {} cannot be read",
                format.bits_per_pixel, picture.depth
            )
        })
    }
}

// ---------------------------------------------------------------- windows

/// A project open in a window of its own.
#[derive(Clone)]
struct OpenProject {
    project: PathBuf,
    window: WindowHandle<Workspace>,
    session: WeakEntity<Session>,
}

/// The projects open, one window each.
#[derive(Default)]
struct Opened(Vec<OpenProject>);

impl Global for Opened {}

/// The window open on `project`, if one is (and still there).
fn find_open(project: &Path, cx: &mut App) -> Option<OpenProject> {
    let live: Vec<_> = cx
        .windows()
        .iter()
        .map(|window| window.window_id())
        .collect();
    let opened = cx.default_global::<Opened>();
    opened
        .0
        .retain(|open| live.contains(&open.window.window_id()) && open.session.upgrade().is_some());
    opened
        .0
        .iter()
        .find(|open| same_folder(&open.project, project))
        .cloned()
}

/// Brings `open`'s window to the front, following `link` there.
fn raise(open: &OpenProject, link: Option<editor_state::links::Link>, cx: &mut App) {
    let _ = open
        .window
        .update(cx, |_, window, _| window.activate_window());
    if let Some(session) = open.session.upgrade() {
        session.update(cx, |session, cx| {
            if let Some(link) = link {
                session.follow(link);
            }
            cx.notify();
        });
    }
}

/// Opens `project` in a window, or the launcher with none. A project open
/// already has its window raised. With no `kerfd`, the one the editor was
/// started with ([`editor_session::session::set_kerfd`]) is kept.
pub fn open(project: Option<PathBuf>, kerfd: Option<PathBuf>, cx: &mut App) {
    open_at(project, kerfd, None, None, cx);
}

/// [`open`], in the place of a window `place` gives (its bounds, as
/// `window.window_bounds()` gave them): another project opened from a
/// window opens where that window was, at its size, maximized if it was,
/// so the window stays where the person had it. A project open already is
/// raised where it is.
pub fn open_in_place(
    project: PathBuf,
    kerfd: Option<PathBuf>,
    place: Option<WindowBounds>,
    cx: &mut App,
) {
    open_at(Some(project), kerfd, None, place, cx);
}

/// Follows a kerf:// link: the recent project it names, at its moment, in
/// its window if it is open. A link that cannot be followed is explained.
pub fn follow(text: &str, kerfd: Option<PathBuf>, cx: &mut App) {
    let recent = Prefs::load().recent;
    let found = editor_state::links::parse(text).and_then(|link| {
        let project = editor_state::links::resolve(&link, &recent)?;
        editor_state::links::openable(&project)?;
        Ok((project, link))
    });
    match found {
        Ok((project, link)) => open_at(Some(project), kerfd, Some(link), None, cx),
        Err(message) => {
            log::warn!("{text}: {message}");
            explain_link(text, &message, cx);
        }
    }
}

/// Tells the person why `link` was not followed: over a window open, else
/// in the launcher.
fn explain_link(link: &str, message: &str, cx: &mut App) {
    let window = cx.active_window().or_else(|| cx.windows().first().copied());
    let detail = format!("{message}\n\n{link}");
    let asked = window.and_then(|window| {
        window
            .update(cx, |_, window, cx| {
                window.prompt(
                    PromptLevel::Warning,
                    "Review link",
                    Some(&detail),
                    &[PromptButton::ok("OK")],
                    cx,
                )
            })
            .ok()
    });
    if asked.is_none() {
        open_launcher(Some(format!("{link}: {message}")), started_kerfd(cx), cx);
    }
}

fn open_at(
    project: Option<PathBuf>,
    kerfd: Option<PathBuf>,
    link: Option<editor_state::links::Link>,
    place: Option<WindowBounds>,
    cx: &mut App,
) {
    let kerfd = kerfd.or_else(|| started_kerfd(cx));
    let Some(project) = project else {
        open_launcher(None, kerfd, cx);
        return;
    };
    if let Some(open) = find_open(&project, cx) {
        raise(&open, link, cx);
        return;
    }
    let warnings = take_warnings(&project, cx);
    let mut prefs = Prefs::load();
    prefs.opened(&project);
    prefs.save();
    let title = project
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Industrial Kerf".into());
    let options = options_at(&title, place, cx);
    let mut made = None;
    let window = cx.open_window(options, |window, cx| {
        represent_project(window, &project);
        let session = Session::open(Some(project.clone()), kerfd, prefs, window, cx);
        // The account is the person's, whichever window heard of it.
        cx.subscribe(&session, |_, event: &BridgeEvent, cx| {
            if let Event::Cloud { .. } = &*event.0 {
                hear_cloud(&event.0, cx);
            }
        })
        .detach();
        if let Some(link) = link {
            session.update(cx, |session, _| session.follow(link));
        }
        say_when_up(&session, warnings, cx);
        made = Some(session.downgrade());
        cx.new(|cx| Workspace::new(session, window, cx))
    });
    match (window, made) {
        (Ok(window), Some(session)) => cx.default_global::<Opened>().0.push(OpenProject {
            project,
            window,
            session,
        }),
        (Err(error), _) => log::error!("could not open the editor: {error}"),
        (Ok(_), None) => {}
    }
}

/// The file a project's window represents: its project.kerf, the file
/// that makes its folder a project.
pub fn represented_file(project: &Path) -> PathBuf {
    project.join("project.kerf")
}

/// On macOS, has `window` represent `project`'s [`represented_file`], so
/// its title bar carries the file's proxy icon and the Window menu the
/// file; elsewhere nothing. A project's window
/// calls it as it opens: a window keeps its project.
pub fn represent_project(window: &Window, project: &Path) {
    #[cfg(target_os = "macos")]
    editor_ui::chrome::macos::represent(window, &represented_file(project));
    #[cfg(not(target_os = "macos"))]
    let _ = (window, project);
}

/// Opens the launcher, saying `problem` when there is one (a link that
/// could not be followed). `kerfd` is the one the editor was started with:
/// a project made from a preset imports its media with it, and the window
/// the launcher opens keeps it.
fn open_launcher(problem: Option<String>, kerfd: Option<PathBuf>, cx: &mut App) {
    let prefs = Prefs::load();
    let options = launcher_options(None, cx);
    let window = cx.open_window(options, |window, cx| {
        let launcher = cx.new(|cx| LauncherView::new(prefs, window, cx).with_kerfd(kerfd.clone()));
        if let Some(problem) = problem {
            launcher.update(cx, |launcher, cx| launcher.say(problem, cx));
        }
        let handle = window.window_handle();
        cx.subscribe(&launcher, move |_, event: &LauncherEvent, cx| match event {
            LauncherEvent::Open {
                project,
                starts,
                warnings,
            } => {
                warn_when_open(project, warnings.clone(), cx);
                // A folder that was no project yet brings its media in; its
                // window takes the launcher's place.
                let place = handle
                    .update(cx, |_, window, _| window.window_bounds())
                    .ok();
                crate::workspace::open_folder_in(
                    project.clone(),
                    *starts,
                    kerfd.clone(),
                    place,
                    cx,
                );
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
            // The launcher the editor starts with has no editor to give back.
            LauncherEvent::Cancel => {}
        })
        .detach();
        launcher
    });
    if let Err(error) = window {
        log::error!("could not open the launcher: {error}");
    }
}

/// Renders the preview of the first project open once kerfd has it
/// (`--render-preview`): the registry's `sequence.renderPreview`, as View ▸
/// Render Preview runs it.
pub fn render_preview_when_up(cx: &mut App) {
    let Some(session) = cx
        .try_global::<Opened>()
        .and_then(|opened| opened.0.first())
        .and_then(|open| open.session.upgrade())
    else {
        return;
    };
    fn render(session: &Entity<Session>, cx: &mut App) {
        session.update(cx, |session, cx| {
            session.run_command("sequence.renderPreview", serde_json::json!({}));
            cx.notify();
        });
    }
    if session.read(cx).state.opened_or_failed() {
        render(&session, cx);
        return;
    }
    let mut waiting = true;
    cx.subscribe(&session, move |session, _: &Change, cx| {
        if waiting && session.read(cx).state.opened_or_failed() {
            waiting = false;
            render(&session, cx);
        }
    })
    .detach();
}

/// Opens the shot's graph `start` names (`--graph`, `--graph-node`) in the
/// first project's window, once its project has said that media
/// ([`editor_making::making::open_when_ready`]).
pub fn open_graph_when_up(start: editor_making::making::GraphStart, cx: &mut App) {
    let Some(open) = cx
        .try_global::<Opened>()
        .and_then(|opened| opened.0.first())
        .cloned()
    else {
        return;
    };
    let Some(session) = open.session.upgrade() else {
        return;
    };
    let window = open.window;
    editor_making::making::open_when_ready(
        start,
        &session,
        move |media, cx| {
            let _ = window.update(cx, |workspace, window, cx| {
                workspace.open_shot_graph(media, window, cx)
            });
        },
        cx,
    );
}

/// What making or cloning a project warned of, until its window is open.
#[derive(Default)]
struct Warnings(Vec<(PathBuf, Vec<String>)>);

impl Global for Warnings {}

/// Says `warnings` (from making or cloning `project`: Git LFS missing, a
/// push that failed) at the foot of its window once kerfd has it open, so
/// they are there to be read. What opens the project the launcher chose
/// calls it first ([`LauncherEvent::Open`]); the window opened for it, now
/// or next, takes them.
pub fn warn_when_open(project: &Path, warnings: Vec<String>, cx: &mut App) {
    if warnings.is_empty() {
        return;
    }
    match find_open(project, cx).and_then(|open| open.session.upgrade()) {
        Some(session) => say_when_up(&session, warnings, cx),
        None => cx
            .default_global::<Warnings>()
            .0
            .push((project.to_path_buf(), warnings)),
    }
}

/// The warnings kept for `project`, taken.
fn take_warnings(project: &Path, cx: &mut App) -> Vec<String> {
    let Some(kept) = cx.try_global::<Warnings>() else {
        return Vec::new();
    };
    if kept.0.is_empty() {
        return Vec::new();
    }
    let kept = &mut cx.default_global::<Warnings>().0;
    let mut taken = Vec::new();
    kept.retain(|(folder, warnings)| {
        if same_folder(folder, project) {
            taken.extend(warnings.iter().cloned());
            false
        } else {
            true
        }
    });
    taken
}

/// Says `warnings` in `session`'s window once its project is up (its
/// first snapshot), or kerfd could not be reached
/// ([`EditorState::opened_or_failed`](editor_state::store::EditorState::opened_or_failed)):
/// at once when it is already, else at the first change after which it is.
/// Not while it only connects: a notice goes after a while, and a large
/// project can take longer than that to open.
fn say_when_up(session: &Entity<Session>, warnings: Vec<String>, cx: &mut App) {
    if warnings.is_empty() {
        return;
    }
    fn say(session: &Entity<Session>, warnings: &[String], cx: &mut App) {
        session.update(cx, |session, cx| {
            for warning in warnings {
                session.state.notify(warning, true);
            }
            cx.emit(Change::Notice);
            cx.notify();
        });
    }
    if session.read(cx).state.opened_or_failed() {
        say(session, &warnings, cx);
        return;
    }
    let mut warnings = Some(warnings);
    cx.subscribe(session, move |session, _: &Change, cx| {
        if warnings.is_none() || !session.read(cx).state.opened_or_failed() {
            return;
        }
        if let Some(warnings) = warnings.take() {
            say(&session, &warnings, cx);
        }
    })
    .detach();
}

// ---------------------------------------------------------------- at launch

/// What the crash notice says of `reports` (unseen, newest first).
pub fn crash_notice_text(reports: &[Value]) -> String {
    let Some(latest) = reports.first() else {
        return String::new();
    };
    let component =
        editor_launcher::settings::component_name(latest["component"].as_str().unwrap_or("editor"));
    let mut text = if reports.len() == 1 {
        format!("Kerf stopped unexpectedly: {component} hit an error it could not recover from.")
    } else {
        format!(
            "Kerf stopped unexpectedly {} times since it last said so. The latest was in {component}.",
            reports.len()
        )
    };
    if let Some(message) = latest["message"]
        .as_str()
        .and_then(|message| message.lines().next())
        .filter(|message| !message.trim().is_empty())
    {
        text.push_str(&format!("\n\n“{}”", message.trim()));
    }
    text.push_str(
        "\n\nA report of each is in the crash folder, on this computer only. Attaching one to a bug report shows where it happened.",
    );
    text
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or(0)
}

/// The window a notice goes over: the one in front, else the first.
fn front_window(cx: &App) -> Option<gpui::AnyWindowHandle> {
    cx.active_window().or_else(|| cx.windows().first().copied())
}

/// What Kerf says on its own at launch, once a window is open: a crash
/// since the last notice, then, with update notices on and the last
/// question to GitHub 20 h ago or more, a newer release not skipped.
pub fn launch_notices(cx: &mut App) {
    cx.spawn(async move |cx| {
        if let Ok(Some((answer, folder))) = cx.update(crash_notice)
            && matches!(answer.await, Ok(1))
        {
            let _ = cx.update(|cx| {
                if let Err(error) = std::fs::create_dir_all(&folder) {
                    log::warn!(
                        "{}: the crash folder could not be made: {error}",
                        folder.display()
                    );
                }
                cx.reveal_path(&folder);
            });
        }
        let due = cx
            .update(|_| {
                let mut prefs = Prefs::load();
                let now = now_ms();
                if !prefs.update_due(now) {
                    return false;
                }
                // Counted when asked, so a failing network is not asked again all day.
                prefs.update_checked_ms = now;
                prefs.save();
                true
            })
            .unwrap_or(false);
        if !due {
            return;
        }
        let answer = cx
            .background_executor()
            .spawn(async { kerf_bridge::update_check() })
            .await;
        let Ok(Some((answer, version, url))) = cx.update(|cx| offer_update(&answer, cx)) else {
            return;
        };
        match answer.await {
            Ok(0) => {
                let _ = cx.update(|cx| cx.open_url(&url));
            }
            Ok(1) => {
                let _ = cx.update(|_| {
                    let mut prefs = Prefs::load();
                    prefs.update_skipped = version;
                    prefs.save();
                });
            }
            _ => {}
        }
    })
    .detach();
}

/// Says so once when Kerf crashed since the last notice: Close (0) or Show
/// Crash Folder (1), and the folder.
fn crash_notice(
    cx: &mut App,
) -> Option<(
    impl std::future::Future<Output = Result<usize, impl Sized + use<>>> + use<>,
    PathBuf,
)> {
    let listed = kerf_bridge::crash_reports();
    let mut prefs = Prefs::load();
    let unseen = prefs.unseen_crashes(&listed);
    if unseen.is_empty() {
        return None;
    }
    let folder = PathBuf::from(listed["folder"].as_str()?);
    let window = front_window(cx)?;
    prefs.saw_crashes(&unseen);
    prefs.save();
    let text = crash_notice_text(&unseen);
    let answer = window
        .update(cx, |_, window, cx| {
            window.prompt(
                PromptLevel::Warning,
                "Kerf stopped unexpectedly",
                Some(&text),
                // Esc answers Close: the folder opens only when asked for.
                &[
                    PromptButton::cancel("Close"),
                    PromptButton::new("Show Crash Folder"),
                ],
                cx,
            )
        })
        .ok()?;
    Some((answer, folder))
}

/// Offers the release `answer` names when it is newer and not skipped:
/// Download (0), Skip This Version (1) or Not Now (2).
fn offer_update(
    answer: &Value,
    cx: &mut App,
) -> Option<(
    impl std::future::Future<Output = Result<usize, impl Sized + use<>>> + use<>,
    String,
    String,
)> {
    let (version, url) = Prefs::load().update_offer(answer)?;
    let window = front_window(cx)?;
    let detail = format!(
        "Industrial Kerf {version} is available. Download it from its release page, and unpack it over this one or beside it."
    );
    let answer = window
        .update(cx, |_, window, cx| {
            window.prompt(
                PromptLevel::Info,
                "A newer Kerf is out",
                Some(&detail),
                &[
                    PromptButton::ok("Download"),
                    PromptButton::new("Skip This Version"),
                    PromptButton::cancel("Not Now"),
                ],
                cx,
            )
        })
        .ok()?;
    Some((answer, version, url))
}

// ---------------------------------------------------------------- prompts

actions!(prompt, [PromptDefault, PromptCancel]);

/// The key context a question over the window answers in.
const PROMPT: &str = "KerfPrompt";
/// A question's width.
const PROMPT_WIDTH: f32 = 440.0;

/// Questions over a window (`Window::prompt`) in Kerf's look: off macOS,
/// where the system draws none of its own.
pub fn install_prompts(cx: &mut App) {
    if cfg!(target_os = "macos") {
        return;
    }
    cx.set_prompt_builder(kerf_prompt);
}

fn kerf_prompt(
    level: PromptLevel,
    message: &str,
    detail: Option<&str>,
    actions: &[PromptButton],
    handle: PromptHandle,
    window: &mut Window,
    cx: &mut App,
) -> RenderablePromptHandle {
    // Binding a keymap clears these: bound again whenever they are missing.
    if !editor_launcher::settings::escape_bound(cx, &PromptCancel) {
        cx.bind_keys([
            KeyBinding::new("escape", PromptCancel, Some(PROMPT)),
            KeyBinding::new("enter", PromptDefault, Some(PROMPT)),
        ]);
    }
    static ASKED: AtomicU64 = AtomicU64::new(0);
    let view = cx.new(|cx| KerfPrompt {
        level,
        message: message.to_owned(),
        detail: detail.map(str::to_owned),
        actions: actions.to_vec(),
        focus: cx.focus_handle(),
        asked: ASKED.fetch_add(1, Ordering::Relaxed),
    });
    handle.with_view(view, window, cx)
}

/// A question over the dimmed window, its answers in a row at its foot:
/// the first, the default, at the end and filled.
struct KerfPrompt {
    level: PromptLevel,
    message: String,
    detail: Option<String>,
    actions: Vec<PromptButton>,
    focus: FocusHandle,
    /// Which question this is, so each rises once.
    asked: u64,
}

impl EventEmitter<PromptResponse> for KerfPrompt {}

impl Focusable for KerfPrompt {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl KerfPrompt {
    /// Esc: the Cancel answer, else the last.
    fn cancel(&mut self, cx: &mut Context<Self>) {
        let index = self
            .actions
            .iter()
            .position(|action| matches!(action, PromptButton::Cancel(_)))
            .unwrap_or(self.actions.len().saturating_sub(1));
        cx.emit(PromptResponse(index));
    }
}

impl Render for KerfPrompt {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mark = match self.level {
            PromptLevel::Critical => theme::neg(),
            PromptLevel::Warning => theme::warn(),
            PromptLevel::Info => theme::accent(),
        };
        let answers = self
            .actions
            .iter()
            .enumerate()
            .rev()
            .map(|(index, action)| {
                let button = ui::button(("prompt-answer", index), action.label().clone())
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(PromptResponse(index))));
                if index == 0 {
                    button.primary()
                } else {
                    button
                }
            });
        let body = div()
            .w(px(PROMPT_WIDTH))
            .flex()
            .flex_col()
            .gap(px(theme::space::S12))
            .p(px(20.0))
            .rounded(px(radius::PANEL))
            .bg(theme::chrome())
            .border_1()
            .border_color(theme::line2())
            .shadow_2xl()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme::space::S8))
                    .child(ui::dot(mark, 8.0))
                    .child(ui::label(self.message.clone(), text::TITLE, theme::ink())),
            )
            .children(self.detail.clone().map(|detail| {
                div().flex().flex_col().gap(px(theme::space::S4)).children(
                    detail
                        .split("\n\n")
                        .map(|paragraph| ui::label(paragraph.to_owned(), text::BODY, theme::ink2()))
                        .collect::<Vec<_>>(),
                )
            }))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(theme::space::S8))
                    .pt(px(theme::space::S4))
                    .children(answers),
            )
            // It comes up into place, as what one goes into does.
            .with_animation(
                ("prompt-rise", self.asked),
                motion::animation(theme::motion::PAGE),
                |body, t| {
                    body.relative()
                        .top(px(theme::motion::PAGE_RISE * (1.0 - t)))
                        .opacity(0.2 + 0.8 * t)
                },
            );
        div()
            .key_context(PROMPT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|_, _: &PromptDefault, _, cx| cx.emit(PromptResponse(0))))
            .on_action(cx.listener(|this, _: &PromptCancel, _, cx| this.cancel(cx)))
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .font(theme::ui_font(FontWeight::NORMAL))
            // The window dims at once, and nothing under the dim answers.
            .bg(theme::with_alpha(theme::scrim(), theme::motion::SCRIM))
            .occlude()
            .child(body)
    }
}
