use tauri::webview::Webview;
use url::Url;

use crate::tabs::public_http_url;

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewFrame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Place a page inside the main webview's coordinate space.
/// `html` is top-left, y growing down. `main` is the main webview frame in its parent.
#[cfg(test)]
pub fn child_frame(html: ViewFrame, main: ViewFrame, flipped: bool) -> ViewFrame {
    if flipped {
        ViewFrame {
            x: main.x + html.x,
            y: main.y + html.y,
            w: html.w,
            h: html.h,
        }
    } else {
        ViewFrame {
            x: main.x + html.x,
            y: main.y + main.h - html.y - html.h,
            w: html.w,
            h: html.h,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryAction {
    Back,
    Forward,
    Reload,
    Stop,
}

pub fn typed_address(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("Enter a page address.".into());
    }
    if has_forbidden_scheme(trimmed) {
        return Err("The page URL must start with http:// or https://.".into());
    }
    if let Ok(url) = public_http_url(trimmed) {
        return Ok(url);
    }
    if trimmed.contains(' ') || trimmed.contains('\\') {
        return Err("Enter a full URL, like https://example.com.".into());
    }
    let candidate = if looks_local(trimmed) {
        format!("http://{trimmed}")
    } else {
        format!("https://{trimmed}")
    };
    public_http_url(&candidate)
}

fn has_forbidden_scheme(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "file:",
        "javascript:",
        "data:",
        "blob:",
        "about:",
        "tauri:",
        "mailto:",
        "ftp:",
        "ws:",
        "wss:",
    ]
    .iter()
    .any(|scheme| lower.starts_with(scheme))
}

pub fn navigation_allowed(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https" | "about")
}

pub fn page_url_from_engine(url: &str) -> Option<String> {
    let parsed = Url::parse(url.trim()).ok()?;
    if parsed.scheme() == "http" || parsed.scheme() == "https" {
        Some(parsed.to_string())
    } else {
        None
    }
}

/// Distance from the top of the parent view to the top of the main webview.
/// DOM coordinates start at the main webview. Child pages are positioned in the parent.
#[cfg(test)]
pub fn view_top_inset(parent_height: f64, origin_y: f64, height: f64, flipped: bool) -> f64 {
    let inset = if flipped {
        origin_y
    } else {
        parent_height - origin_y - height
    };
    if inset.is_finite() {
        inset.clamp(0.0, 64.0)
    } else {
        0.0
    }
}

/// Origin y for a page whose top sits `dom_y + main_top` points below the parent top.
pub fn page_origin_y(dom_y: f64, page_h: f64, parent_h: f64, main_top: f64, flipped: bool) -> f64 {
    let top = dom_y + main_top;
    if flipped {
        top
    } else {
        parent_h - top - page_h
    }
}

pub fn align_page(webview: &Webview, x: f64, y: f64, width: f64, height: f64) {
    #[cfg(target_os = "macos")]
    {
        let _ = webview.with_webview(move |platform| unsafe {
            place_under_address_bar(platform.inner(), x, y, width, height);
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (webview, x, y, width, height);
    }
}

#[cfg(target_os = "macos")]
unsafe fn place_under_address_bar(
    child_ptr: *mut std::ffi::c_void,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) {
    if child_ptr.is_null() {
        return;
    }
    unsafe {
        let child = &*child_ptr.cast::<objc2::runtime::AnyObject>();
        let parent_ptr: *mut objc2::runtime::AnyObject = objc2::msg_send![child, superview];
        if parent_ptr.is_null() {
            return;
        }
        let parent = &*parent_ptr;
        let parent_bounds: objc2_foundation::NSRect = objc2::msg_send![parent, bounds];
        let parent_flipped: objc2::runtime::Bool = objc2::msg_send![parent, isFlipped];
        let main = shell_sibling(parent, child);
        let (main_x, main_top) = if main.is_null() {
            (0.0, 0.0)
        } else {
            let frame: objc2_foundation::NSRect = objc2::msg_send![&*main, frame];
            let insets: objc2_foundation::NSEdgeInsets = objc2::msg_send![&*main, safeAreaInsets];
            let top = if parent_flipped.as_bool() {
                frame.origin.y
            } else {
                parent_bounds.size.height - frame.origin.y - frame.size.height
            };
            let top = if top.is_finite() && (0.0..=120.0).contains(&top) {
                top
            } else {
                0.0
            };
            let inset = if insets.top.is_finite() && (0.0..=80.0).contains(&insets.top) {
                insets.top
            } else {
                0.0
            };
            (frame.origin.x, top + inset)
        };
        let origin_y = page_origin_y(
            y,
            height,
            parent_bounds.size.height,
            main_top,
            parent_flipped.as_bool(),
        );
        let rect = objc2_foundation::NSRect::new(
            objc2_foundation::NSPoint::new(x + main_x, origin_y),
            objc2_foundation::NSSize::new(width.max(1.0), height.max(1.0)),
        );
        let _: () = objc2::msg_send![child, setFrame: rect];
        let _: () = objc2::msg_send![child, setAutoresizingMask: 0usize];
    }
}

#[cfg(target_os = "macos")]
unsafe fn shell_sibling(
    parent: &objc2::runtime::AnyObject,
    child: &objc2::runtime::AnyObject,
) -> *mut objc2::runtime::AnyObject {
    unsafe {
        let subviews: *mut objc2::runtime::AnyObject = objc2::msg_send![parent, subviews];
        if subviews.is_null() {
            return std::ptr::null_mut();
        }
        let count: usize = objc2::msg_send![subviews, count];
        let mut tallest: *mut objc2::runtime::AnyObject = std::ptr::null_mut();
        let mut tallest_height = -1.0;
        for index in 0..count {
            let view: *mut objc2::runtime::AnyObject =
                objc2::msg_send![subviews, objectAtIndex: index];
            if view.is_null() || std::ptr::eq(view, child) {
                continue;
            }
            let mask: usize = objc2::msg_send![&*view, autoresizingMask];
            if mask & 2 != 0 && mask & 16 != 0 {
                return view;
            }
            let frame: objc2_foundation::NSRect = objc2::msg_send![&*view, frame];
            if frame.size.height > tallest_height {
                tallest_height = frame.size.height;
                tallest = view;
            }
        }
        tallest
    }
}

pub fn parse_history_action(direction: &str) -> Result<HistoryAction, String> {
    match direction {
        "back" => Ok(HistoryAction::Back),
        "forward" => Ok(HistoryAction::Forward),
        "reload" => Ok(HistoryAction::Reload),
        "stop" => Ok(HistoryAction::Stop),
        _ => Err("Unknown history action.".into()),
    }
}

pub fn run_history(webview: &Webview, action: HistoryAction) -> Result<(), String> {
    match action {
        HistoryAction::Reload => webview
            .reload()
            .map_err(|err| format!("Could not reload the page: {err}")),
        HistoryAction::Stop => {
            engine_stop(webview);
            Ok(())
        }
        HistoryAction::Back | HistoryAction::Forward => {
            engine_history(webview, action);
            Ok(())
        }
    }
}

pub fn enable_gestures(webview: &Webview) {
    #[cfg(target_os = "macos")]
    {
        let _ = webview.with_webview(|platform| unsafe {
            wk_set_gestures(platform.inner(), true);
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = webview;
    }
}

pub fn read_history_buttons(webview: &Webview, done: impl FnOnce(bool, bool) + Send + 'static) {
    #[cfg(target_os = "macos")]
    {
        let _ = webview.with_webview(move |platform| {
            let (back, forward) = unsafe { wk_history_buttons(platform.inner()) };
            done(back, forward);
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = webview;
        done(true, true);
    }
}

fn looks_local(value: &str) -> bool {
    let host = value.split(['/', '?', '#']).next().unwrap_or(value);
    let host = host.split('@').next_back().unwrap_or(host);
    let name = host.rsplit_once(':').map(|(name, _)| name).unwrap_or(host);
    name == "localhost" || name.parse::<std::net::IpAddr>().is_ok()
}

fn engine_history(webview: &Webview, action: HistoryAction) {
    #[cfg(target_os = "macos")]
    {
        let _ = webview.with_webview(move |platform| unsafe {
            wk_history(platform.inner(), action);
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let script = match action {
            HistoryAction::Back => "history.back()",
            HistoryAction::Forward => "history.forward()",
            _ => return,
        };
        let _ = webview.eval(script);
    }
}

fn engine_stop(webview: &Webview) {
    #[cfg(target_os = "macos")]
    {
        let _ = webview.with_webview(|platform| unsafe {
            wk_stop(platform.inner());
        });
    }
    let _ = webview.eval("window.stop()");
}

#[cfg(target_os = "macos")]
unsafe fn wk_view(ptr: *mut std::ffi::c_void) -> Option<&'static objc2::runtime::AnyObject> {
    if ptr.is_null() {
        return None;
    }
    Some(unsafe { &*ptr.cast::<objc2::runtime::AnyObject>() })
}

#[cfg(target_os = "macos")]
unsafe fn wk_history(ptr: *mut std::ffi::c_void, action: HistoryAction) {
    let Some(view) = (unsafe { wk_view(ptr) }) else {
        return;
    };
    unsafe {
        match action {
            HistoryAction::Back => {
                let _: *mut objc2::runtime::AnyObject = objc2::msg_send![view, goBack];
            }
            HistoryAction::Forward => {
                let _: *mut objc2::runtime::AnyObject = objc2::msg_send![view, goForward];
            }
            _ => {}
        }
    }
}

#[cfg(target_os = "macos")]
unsafe fn wk_stop(ptr: *mut std::ffi::c_void) {
    let Some(view) = (unsafe { wk_view(ptr) }) else {
        return;
    };
    unsafe {
        let _: () = objc2::msg_send![view, stopLoading];
    }
}

#[cfg(target_os = "macos")]
unsafe fn wk_set_gestures(ptr: *mut std::ffi::c_void, enabled: bool) {
    let Some(view) = (unsafe { wk_view(ptr) }) else {
        return;
    };
    unsafe {
        let _: () = objc2::msg_send![view, setAllowsBackForwardNavigationGestures: enabled];
    }
}

#[cfg(target_os = "macos")]
unsafe fn wk_history_buttons(ptr: *mut std::ffi::c_void) -> (bool, bool) {
    let Some(view) = (unsafe { wk_view(ptr) }) else {
        return (false, false);
    };
    unsafe {
        let back: bool = objc2::msg_send![view, canGoBack];
        let forward: bool = objc2::msg_send![view, canGoForward];
        (back, forward)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_address_adds_https() {
        assert_eq!(
            typed_address("example.com/path").unwrap(),
            "https://example.com/path"
        );
        assert!(
            typed_address("https://example.com")
                .unwrap()
                .starts_with("https://example.com")
        );
        assert!(typed_address("").is_err());
        assert!(typed_address("javascript:alert(1)").is_err());
        assert!(typed_address("file:///tmp/x").is_err());
        assert!(typed_address("not a host").is_err());
    }

    #[test]
    fn typed_address_keeps_loopback_on_http() {
        assert!(
            typed_address("localhost:8080")
                .unwrap()
                .starts_with("http://localhost:8080")
        );
        assert!(
            typed_address("127.0.0.1")
                .unwrap()
                .starts_with("http://127.0.0.1")
        );
    }

    #[test]
    fn engine_urls_ignore_blank_documents() {
        assert_eq!(
            page_url_from_engine("https://example.com/app"),
            Some("https://example.com/app".into())
        );
        assert_eq!(page_url_from_engine("about:blank"), None);
        assert_eq!(page_url_from_engine("file:///tmp"), None);
        assert!(navigation_allowed(
            &Url::parse("https://a.example").unwrap()
        ));
        assert!(navigation_allowed(&Url::parse("about:blank").unwrap()));
        assert!(!navigation_allowed(
            &Url::parse("javascript:alert(1)").unwrap()
        ));
    }

    #[test]
    fn the_page_starts_below_the_toolbar_inside_the_main_webview() {
        let main = ViewFrame {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        };
        let html = ViewFrame {
            x: 12.0,
            y: 80.0,
            w: 500.0,
            h: 400.0,
        };
        let child = child_frame(html, main, false);
        assert_eq!(child.x, 12.0);
        assert_eq!(child.y, 120.0);
        assert_eq!(child.h, 400.0);
        let inset_main = ViewFrame {
            x: 0.0,
            y: 28.0,
            w: 800.0,
            h: 572.0,
        };
        let shifted = child_frame(html, inset_main, false);
        assert_eq!(shifted.y, 120.0);
    }

    #[test]
    fn the_page_origin_sits_under_the_address_bar() {
        let origin = page_origin_y(90.0, 500.0, 800.0, 0.0, false);
        assert_eq!(origin, 210.0);
        let shifted = page_origin_y(90.0, 500.0, 800.0, 28.0, false);
        assert_eq!(shifted, 182.0);
        assert_eq!(page_origin_y(90.0, 500.0, 800.0, 28.0, true), 118.0);
    }

    #[test]
    fn a_webview_inset_shifts_the_page_below_the_toolbar() {
        assert_eq!(view_top_inset(800.0, 0.0, 800.0, false), 0.0);
        assert_eq!(view_top_inset(800.0, 0.0, 772.0, false), 28.0);
        assert_eq!(view_top_inset(800.0, 28.0, 772.0, true), 28.0);
        assert_eq!(view_top_inset(800.0, f64::NAN, 10.0, false), 0.0);
    }

    #[test]
    fn history_actions_map_to_engine_commands() {
        assert_eq!(parse_history_action("back").unwrap(), HistoryAction::Back);
        assert_eq!(
            parse_history_action("reload").unwrap(),
            HistoryAction::Reload
        );
        assert_eq!(parse_history_action("stop").unwrap(), HistoryAction::Stop);
        assert!(parse_history_action("home").is_err());
    }
}
