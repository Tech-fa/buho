use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Position, Rect, Size};

use crate::control::PAGE_WEBVIEW;
use crate::terminal::TERMINAL_WEBVIEW;

/// Window that holds the previewed page and the terminal. This label is not in the
/// capability list, so the page webview inside it cannot call shell commands.
pub const PREVIEW_WINDOW: &str = "preview";

pub const TERMINAL_BAR: f64 = 36.0;
pub const TERMINAL_MIN_PAGE: f64 = 180.0;
pub const TERMINAL_OPEN: f64 = 280.0;
const TERMINAL_SNAP: f64 = 48.0;
const PREVIEW_WIDTH: f64 = 1100.0;
const PREVIEW_HEIGHT: f64 = 760.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pane {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DockSplit {
    pub page: Pane,
    pub terminal: Pane,
    pub panel: f64,
    pub collapsed: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockView {
    pub panel: f64,
    pub window_height: f64,
    pub collapsed: bool,
    pub dragging: bool,
}

pub struct PreviewDock {
    height: Mutex<f64>,
    dragging: AtomicBool,
    applying: AtomicBool,
    apply_again: AtomicBool,
}

impl PreviewDock {
    pub fn new() -> Self {
        Self {
            height: Mutex::new(TERMINAL_BAR),
            dragging: AtomicBool::new(false),
            applying: AtomicBool::new(false),
            apply_again: AtomicBool::new(false),
        }
    }

    fn height(&self) -> f64 {
        *self
            .height
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn dragging(&self) -> bool {
        self.dragging.load(Ordering::SeqCst)
    }

    fn set(&self, height: f64, dragging: bool) {
        *self
            .height
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = height;
        self.dragging.store(dragging, Ordering::SeqCst);
    }
}

/// The terminal sits on the bottom of the preview. While the pointer is down it covers
/// the whole window, so the drag can move above the bar without leaving that webview.
pub fn dock_split(
    window_width: f64,
    window_height: f64,
    requested: f64,
    dragging: bool,
) -> DockSplit {
    let width = if window_width.is_finite() {
        window_width.max(1.0)
    } else {
        1.0
    };
    let height = if window_height.is_finite() {
        window_height.max(1.0)
    } else {
        1.0
    };
    let bar = TERMINAL_BAR.min(height);
    let max_terminal = (height - TERMINAL_MIN_PAGE).max(bar).min(height);
    let requested = if requested.is_finite() {
        requested
    } else {
        bar
    };
    let panel = requested.clamp(bar, max_terminal);
    let page_height = (height - panel).max(0.0);
    let terminal = if dragging {
        Pane {
            x: 0.0,
            y: 0.0,
            width,
            height,
        }
    } else {
        Pane {
            x: 0.0,
            y: page_height,
            width,
            height: panel,
        }
    };
    DockSplit {
        page: Pane {
            x: 0.0,
            y: 0.0,
            width,
            height: page_height,
        },
        terminal,
        panel,
        collapsed: panel <= bar + 0.5,
    }
}

pub fn set_height(
    app: &AppHandle,
    height: f64,
    dragging: bool,
    snap: bool,
) -> Result<DockView, String> {
    if !height.is_finite() {
        return Err("The terminal height is not a number.".into());
    }
    let mut height = height;
    if snap && !dragging && height < TERMINAL_BAR + TERMINAL_SNAP {
        height = TERMINAL_BAR;
    }
    let dock = app.state::<PreviewDock>();
    let was_open = dock.height() > TERMINAL_BAR + 1.0;
    dock.set(height, dragging);
    let view = apply(app)?;
    dock.set(view.panel, dragging);
    if was_open && view.collapsed && !view.dragging {
        if let Some(page) = app.get_webview(PAGE_WEBVIEW) {
            let _ = page.set_focus();
        }
    }
    Ok(view)
}

pub fn toggle(app: &AppHandle) -> Result<DockView, String> {
    let open = app.state::<PreviewDock>().height() <= TERMINAL_BAR + 1.0;
    let height = if open { TERMINAL_OPEN } else { TERMINAL_BAR };
    set_height(app, height, false, false)
}

pub fn apply(app: &AppHandle) -> Result<DockView, String> {
    let dock = app.state::<PreviewDock>();
    if dock.applying.swap(true, Ordering::SeqCst) {
        dock.apply_again.store(true, Ordering::SeqCst);
        return Ok(view_from(dock.height(), dock.dragging(), 0.0));
    }
    let mut result = apply_once(app);
    while dock.apply_again.swap(false, Ordering::SeqCst) {
        result = apply_once(app);
    }
    dock.applying.store(false, Ordering::SeqCst);
    result
}

fn view_from(panel: f64, dragging: bool, window_height: f64) -> DockView {
    let split = dock_split(1.0, window_height.max(panel), panel, dragging);
    DockView {
        panel: split.panel,
        window_height,
        collapsed: split.collapsed,
        dragging,
    }
}

fn apply_once(app: &AppHandle) -> Result<DockView, String> {
    let dock = app.state::<PreviewDock>();
    let page = app
        .get_webview(PAGE_WEBVIEW)
        .ok_or_else(|| "Open a page first.".to_string())?;
    let terminal = app
        .get_webview(TERMINAL_WEBVIEW)
        .ok_or_else(|| "The terminal is not open.".to_string())?;
    let window = page.window();
    let mut scale = window.scale_factor().unwrap_or(1.0);
    if !scale.is_finite() || scale <= 0.0 {
        scale = 1.0;
    }
    let size = window.inner_size().map_err(|err| err.to_string())?;
    let mut width = size.width as f64 / scale;
    let mut height = size.height as f64 / scale;
    if width < 1.0 {
        width = PREVIEW_WIDTH;
    }
    if height < 1.0 {
        height = PREVIEW_HEIGHT;
    }
    let split = dock_split(width, height, dock.height(), dock.dragging());
    page.set_bounds(pane_rect(split.page))
        .map_err(|err| format!("Could not resize the page: {err}"))?;
    terminal
        .set_bounds(pane_rect(split.terminal))
        .map_err(|err| format!("Could not resize the terminal: {err}"))?;
    Ok(DockView {
        panel: split.panel,
        window_height: height,
        collapsed: split.collapsed,
        dragging: dock.dragging(),
    })
}

fn pane_rect(pane: Pane) -> Rect {
    Rect {
        position: Position::Logical(LogicalPosition::new(pane.x, pane.y)),
        size: Size::Logical(LogicalSize::new(pane.width.max(1.0), pane.height.max(1.0))),
    }
}

#[cfg(test)]
mod tests {
    use super::{PREVIEW_WINDOW, TERMINAL_BAR, TERMINAL_MIN_PAGE, dock_split};

    #[test]
    fn the_page_window_is_separate_from_setup() {
        assert_ne!(crate::control::PAGE_WEBVIEW, "main");
        assert_ne!(PREVIEW_WINDOW, "main");
        assert_ne!(PREVIEW_WINDOW, crate::terminal::TERMINAL_WEBVIEW);
    }

    #[test]
    fn a_collapsed_terminal_sits_on_the_bottom() {
        let split = dock_split(1100.0, 760.0, TERMINAL_BAR, false);
        assert_eq!(split.page.height, 760.0 - TERMINAL_BAR);
        assert_eq!(split.terminal.y, 760.0 - TERMINAL_BAR);
        assert_eq!(split.terminal.height, TERMINAL_BAR);
        assert!(split.collapsed);
    }

    #[test]
    fn pulling_the_terminal_up_shrinks_the_page() {
        let split = dock_split(1100.0, 760.0, 280.0, false);
        assert_eq!(split.page.height, 480.0);
        assert_eq!(split.terminal.y, 480.0);
        assert_eq!(split.terminal.height, 280.0);
        assert!(!split.collapsed);
        assert_eq!(split.panel, 280.0);
    }

    #[test]
    fn the_page_keeps_a_minimum_height() {
        let split = dock_split(1100.0, 760.0, 900.0, false);
        assert_eq!(split.page.height, TERMINAL_MIN_PAGE);
        assert_eq!(split.terminal.height, 760.0 - TERMINAL_MIN_PAGE);
    }

    #[test]
    fn dragging_covers_the_window_so_the_pointer_stays_on_the_terminal() {
        let split = dock_split(1100.0, 760.0, 280.0, true);
        assert_eq!(split.terminal.y, 0.0);
        assert_eq!(split.terminal.height, 760.0);
        assert_eq!(split.page.height, 480.0);
        assert_eq!(split.panel, 280.0);
    }

    #[test]
    fn the_bar_never_grows_past_a_short_window() {
        let split = dock_split(400.0, 40.0, 280.0, false);
        assert!(split.terminal.height <= 40.0);
        assert!(split.page.height >= 0.0);
        assert!(split.page.height + split.terminal.height <= 40.0 + 0.01);
    }

    #[test]
    fn a_zero_window_does_not_produce_a_negative_pane() {
        let split = dock_split(0.0, 0.0, 10.0, false);
        assert!(split.page.width >= 1.0);
        assert!(split.page.height >= 0.0);
        assert!(split.terminal.height >= 0.0);
    }
}
