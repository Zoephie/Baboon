//! Persistent native main-window placement.
//!
//! Window state deliberately lives outside Baboon's portable/project storage:
//! it describes this user's desktop, so it always belongs in the platform
//! configuration directory.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use atomic_write_file::AtomicWriteFile;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

pub(crate) const DEFAULT_INNER_SIZE: [f32; 2] = [1280.0, 800.0];
pub(crate) const MIN_INNER_SIZE: [f32; 2] = [520.0, 360.0];

const SCHEMA_VERSION: u32 = 1;
const SETTLE_DELAY: Duration = Duration::from_millis(750);
const CONTINUOUS_SAVE_INTERVAL: Duration = Duration::from_secs(5);
const MIN_VISIBLE_WIDTH: f32 = 64.0;
const MIN_VISIBLE_TITLE_HEIGHT: f32 = 32.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Point {
    x: f32,
    y: f32,
}

impl Point {
    fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Size {
    width: f32,
    height: f32,
}

impl Size {
    fn is_valid(self) -> bool {
        self.width.is_finite() && self.height.is_finite() && self.width > 0.0 && self.height > 0.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
struct PixelRect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl PixelRect {
    fn from_position_size(position: Point, size: Size) -> Self {
        Self {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        }
    }

    fn is_valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.width > 0.0
            && self.height > 0.0
    }

    fn right(self) -> f32 {
        self.x + self.width
    }

    fn bottom(self) -> f32 {
        self.y + self.height
    }

    fn intersection(self, other: Self) -> Option<Self> {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (right > left && bottom > top).then_some(Self {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        })
    }

    fn area(self) -> f32 {
        self.width * self.height
    }

    fn contains(self, point: Point) -> bool {
        point.x >= self.x && point.x < self.right() && point.y >= self.y && point.y < self.bottom()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WindowMode {
    #[default]
    Normal,
    Maximized,
    Fullscreen,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct NormalWindowBounds {
    inner_position_px: Option<Point>,
    outer_position_px: Option<Point>,
    inner_size_logical: Size,
    outer_size_logical: Size,
    native_scale_factor: f32,
    monitor_name: Option<String>,
    monitor_bounds_px: Option<PixelRect>,
}

impl NormalWindowBounds {
    fn validate(&self) -> bool {
        self.inner_position_px.is_none_or(Point::is_finite)
            && self.outer_position_px.is_none_or(Point::is_finite)
            && self.inner_size_logical.is_valid()
            && self.outer_size_logical.is_valid()
            && self.outer_size_logical.width >= self.inner_size_logical.width
            && self.outer_size_logical.height >= self.inner_size_logical.height
            && self.native_scale_factor.is_finite()
            && self.native_scale_factor > 0.0
            && self.monitor_bounds_px.is_none_or(PixelRect::is_valid)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct PersistedWindowState {
    schema_version: u32,
    mode: WindowMode,
    normal: NormalWindowBounds,
}

impl PersistedWindowState {
    fn validate(&self) -> bool {
        self.schema_version == SCHEMA_VERSION && self.normal.validate()
    }
}

#[derive(Clone, Debug, PartialEq)]
struct MonitorGeometry {
    name: Option<String>,
    is_primary: bool,
    bounds_px: PixelRect,
    work_area_px: PixelRect,
    scale_factor: f32,
}

impl MonitorGeometry {
    fn validate(&self) -> bool {
        self.bounds_px.is_valid()
            && self.work_area_px.is_valid()
            && self.scale_factor.is_finite()
            && self.scale_factor > 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RestoredViewport {
    pub(crate) position: Option<[f32; 2]>,
    pub(crate) inner_size: [f32; 2],
    pub(crate) maximized: bool,
}

pub(crate) struct StartupWindowState {
    pub(crate) restored: Option<RestoredViewport>,
    pub(crate) tracker: WindowStateTracker,
}

pub(crate) fn load_startup_state() -> StartupWindowState {
    let path = window_state_path();
    let monitors = discover_monitors();
    let loaded = path.as_deref().and_then(load_from_path);
    let restored = loaded
        .as_ref()
        .and_then(|state| restore_viewport(state, &monitors));

    StartupWindowState {
        restored,
        tracker: WindowStateTracker::new(path, monitors, loaded),
    }
}

fn window_state_path() -> Option<PathBuf> {
    ProjectDirs::from("", "", "Baboon")
        .map(|directories| directories.config_dir().join("window-state.json"))
}

fn load_from_path(path: &Path) -> Option<PersistedWindowState> {
    let text = fs::read_to_string(path).ok()?;
    parse_state(&text)
}

fn parse_state(text: &str) -> Option<PersistedWindowState> {
    let state = serde_json::from_str::<PersistedWindowState>(text).ok()?;
    state.validate().then_some(state)
}

fn write_state_atomic(path: &Path, state: &PersistedWindowState) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create window-state directory: {error}"))?;
    }
    let text = serde_json::to_string_pretty(state)
        .map_err(|error| format!("could not encode window state: {error}"))?;
    let mut file = AtomicWriteFile::open(path)
        .map_err(|error| format!("could not open atomic window-state file: {error}"))?;
    file.write_all(text.as_bytes())
        .map_err(|error| format!("could not write window state: {error}"))?;
    file.write_all(b"\n")
        .map_err(|error| format!("could not finish window state: {error}"))?;
    file.commit()
        .map_err(|error| format!("could not commit window state: {error}"))
}

fn restore_viewport(
    state: &PersistedWindowState,
    monitors: &[MonitorGeometry],
) -> Option<RestoredViewport> {
    if !state.validate() || monitors.is_empty() {
        return None;
    }

    let normal = &state.normal;
    let saved_outer_size_px = Size {
        width: normal.outer_size_logical.width * normal.native_scale_factor,
        height: normal.outer_size_logical.height * normal.native_scale_factor,
    };
    let saved_outer_rect = normal
        .outer_position_px
        .map(|position| PixelRect::from_position_size(position, saved_outer_size_px));
    let visible_target = saved_outer_rect.and_then(|saved_outer_rect| {
        monitors
            .iter()
            .enumerate()
            .filter_map(|(index, monitor)| {
                let intersection = saved_outer_rect.intersection(monitor.work_area_px)?;
                let title_area = PixelRect {
                    x: saved_outer_rect.x,
                    y: saved_outer_rect.y,
                    width: saved_outer_rect.width,
                    height: (MIN_VISIBLE_TITLE_HEIGHT * monitor.scale_factor)
                        .min(saved_outer_rect.height),
                };
                let visible_title = title_area.intersection(monitor.work_area_px)?;
                (visible_title.width >= MIN_VISIBLE_WIDTH * monitor.scale_factor
                    && visible_title.height > 0.0)
                    .then_some((
                        index,
                        monitor.name.as_ref() == normal.monitor_name.as_ref(),
                        intersection.area(),
                    ))
            })
            .max_by(|left, right| {
                left.1
                    .cmp(&right.1)
                    .then_with(|| left.2.total_cmp(&right.2))
            })
            .map(|(index, _, _)| index)
    });
    let primary = monitors
        .iter()
        .position(|monitor| monitor.is_primary)
        .unwrap_or(0);
    let target_index = visible_target.unwrap_or(primary);
    let target = &monitors[target_index];

    let frame_width = (normal.outer_size_logical.width - normal.inner_size_logical.width).max(0.0);
    let frame_height =
        (normal.outer_size_logical.height - normal.inner_size_logical.height).max(0.0);
    let available_logical = Size {
        width: target.work_area_px.width / target.scale_factor,
        height: target.work_area_px.height / target.scale_factor,
    };
    let max_inner = Size {
        width: (available_logical.width - frame_width).max(1.0),
        height: (available_logical.height - frame_height).max(1.0),
    };
    let inner_size = Size {
        width: normal
            .inner_size_logical
            .width
            .max(MIN_INNER_SIZE[0])
            .min(max_inner.width),
        height: normal
            .inner_size_logical
            .height
            .max(MIN_INNER_SIZE[1])
            .min(max_inner.height),
    };
    let outer_size_px = Size {
        width: (inner_size.width + frame_width) * target.scale_factor,
        height: (inner_size.height + frame_height) * target.scale_factor,
    };

    let builder_position = normal.outer_position_px.map(|saved_outer_position| {
        let mut outer_position = if visible_target.is_some() {
            saved_outer_position
        } else {
            Point {
                x: target.work_area_px.x + (target.work_area_px.width - outer_size_px.width) * 0.5,
                y: target.work_area_px.y
                    + (target.work_area_px.height - outer_size_px.height) * 0.5,
            }
        };
        outer_position.x = clamp_axis(
            outer_position.x,
            target.work_area_px.x,
            target.work_area_px.right() - outer_size_px.width,
        );
        outer_position.y = clamp_axis(
            outer_position.y,
            target.work_area_px.y,
            target.work_area_px.bottom() - outer_size_px.height,
        );

        #[cfg(target_os = "macos")]
        {
            let saved_offset_logical = normal
                .inner_position_px
                .map(|inner| Point {
                    x: (inner.x - saved_outer_position.x) / normal.native_scale_factor,
                    y: (inner.y - saved_outer_position.y) / normal.native_scale_factor,
                })
                .unwrap_or_default();
            Point {
                x: outer_position.x / target.scale_factor + saved_offset_logical.x,
                y: outer_position.y / target.scale_factor + saved_offset_logical.y,
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            Point {
                x: outer_position.x / target.scale_factor,
                y: outer_position.y / target.scale_factor,
            }
        }
    });

    Some(RestoredViewport {
        position: builder_position.map(|position| [position.x, position.y]),
        inner_size: [inner_size.width, inner_size.height],
        maximized: state.mode == WindowMode::Maximized,
    })
}

fn clamp_axis(value: f32, minimum: f32, maximum: f32) -> f32 {
    if maximum < minimum {
        minimum
    } else {
        value.clamp(minimum, maximum)
    }
}

pub(crate) struct WindowStateTracker {
    path: Option<PathBuf>,
    monitors: Vec<MonitorGeometry>,
    current: Option<PersistedWindowState>,
    last_persisted: Option<PersistedWindowState>,
    dirty_since: Option<Instant>,
    last_write: Instant,
    startup_mode_pending: Option<(WindowMode, u8)>,
}

impl WindowStateTracker {
    fn new(
        path: Option<PathBuf>,
        monitors: Vec<MonitorGeometry>,
        state: Option<PersistedWindowState>,
    ) -> Self {
        let startup_mode_pending = state
            .as_ref()
            .map(|state| state.mode)
            .filter(|mode| *mode != WindowMode::Normal)
            .map(|mode| (mode, 0));
        Self {
            path,
            monitors,
            current: state.clone(),
            last_persisted: state,
            dirty_since: None,
            last_write: Instant::now(),
            startup_mode_pending,
        }
    }

    pub(crate) fn observe(&mut self, ctx: &eframe::egui::Context) {
        let viewport = ctx.input(|input| input.viewport().clone());
        if let Some((mode, attempts)) = self.startup_mode_pending {
            let restored = match mode {
                WindowMode::Normal => true,
                WindowMode::Maximized => viewport.maximized == Some(true),
                WindowMode::Fullscreen => viewport.fullscreen == Some(true),
            };
            if restored {
                self.startup_mode_pending = None;
            } else if attempts < 3 {
                self.startup_mode_pending = Some((mode, attempts + 1));
                // eframe keeps its native viewport hidden for the first app
                // update, so these bounded retries can apply a special mode
                // without a visible normal-window flash.
                match mode {
                    WindowMode::Normal => {}
                    WindowMode::Maximized => {
                        ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Maximized(true));
                    }
                    WindowMode::Fullscreen => {
                        ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Fullscreen(true));
                    }
                }
                ctx.request_repaint();
                return;
            } else {
                self.startup_mode_pending = None;
            }
        }

        let mode = if viewport.fullscreen == Some(true) {
            WindowMode::Fullscreen
        } else if viewport.maximized == Some(true) {
            WindowMode::Maximized
        } else {
            WindowMode::Normal
        };
        let previous_mode = self.current.as_ref().map(|state| state.mode);
        let minimized = viewport.minimized == Some(true);

        let normal = if mode == WindowMode::Normal && !minimized {
            let previous = self.current.as_ref().map(|state| &state.normal);
            capture_normal_bounds(ctx, &viewport, &self.monitors, previous)
        } else {
            None
        };
        self.record_observation(mode, normal);

        let Some(current) = &self.current else {
            return;
        };
        if self.last_persisted.as_ref() == Some(current) {
            self.dirty_since = None;
            return;
        }

        let now = Instant::now();
        let mode_changed = previous_mode.is_some_and(|previous| previous != mode);
        let dirty_since = *self.dirty_since.get_or_insert(now);
        if mode_changed
            || now.duration_since(dirty_since) >= SETTLE_DELAY
            || now.duration_since(self.last_write) >= CONTINUOUS_SAVE_INTERVAL
        {
            self.persist_now();
        } else {
            let until_settled =
                SETTLE_DELAY.saturating_sub(now.saturating_duration_since(dirty_since));
            let until_continuous = CONTINUOUS_SAVE_INTERVAL
                .saturating_sub(now.saturating_duration_since(self.last_write));
            ctx.request_repaint_after(until_settled.min(until_continuous));
        }
    }

    fn record_observation(&mut self, mode: WindowMode, normal: Option<NormalWindowBounds>) {
        if mode == WindowMode::Normal {
            if let Some(normal) = normal {
                self.current = Some(PersistedWindowState {
                    schema_version: SCHEMA_VERSION,
                    mode,
                    normal,
                });
            }
        } else if let Some(state) = &mut self.current {
            state.mode = mode;
        }
    }

    pub(crate) fn persist_now(&mut self) {
        let (Some(path), Some(current)) = (&self.path, &self.current) else {
            return;
        };
        if write_state_atomic(path, current).is_ok() {
            self.last_persisted = Some(current.clone());
            self.dirty_since = None;
            self.last_write = Instant::now();
        }
    }
}

fn capture_normal_bounds(
    ctx: &eframe::egui::Context,
    viewport: &eframe::egui::ViewportInfo,
    monitors: &[MonitorGeometry],
    previous: Option<&NormalWindowBounds>,
) -> Option<NormalWindowBounds> {
    let native_scale = viewport.native_pixels_per_point?;
    let pixels_per_point = ctx.pixels_per_point();
    if !native_scale.is_finite()
        || native_scale <= 0.0
        || !pixels_per_point.is_finite()
        || pixels_per_point <= 0.0
    {
        return None;
    }
    let logical_per_point = pixels_per_point / native_scale;
    let fallback_inner = ctx.screen_rect();
    let inner = viewport.inner_rect.unwrap_or(fallback_inner);
    let outer = viewport.outer_rect.unwrap_or(inner);
    let inner_size_logical = Size {
        width: round_half(inner.width() * logical_per_point),
        height: round_half(inner.height() * logical_per_point),
    };
    let mut outer_size_logical = Size {
        width: round_half(outer.width() * logical_per_point),
        height: round_half(outer.height() * logical_per_point),
    };
    if viewport.outer_rect.is_none() {
        let decoration = previous
            .map(|normal| Size {
                width: (normal.outer_size_logical.width - normal.inner_size_logical.width).max(0.0),
                height: (normal.outer_size_logical.height - normal.inner_size_logical.height)
                    .max(0.0),
            })
            .unwrap_or_default();
        outer_size_logical.width += decoration.width;
        outer_size_logical.height += decoration.height;
    }
    if !inner_size_logical.is_valid() || !outer_size_logical.is_valid() {
        return None;
    }

    let inner_position_px = viewport.inner_rect.map(|inner| {
        round_point(Point {
            x: inner.min.x * pixels_per_point,
            y: inner.min.y * pixels_per_point,
        })
    });
    let outer_position_px = viewport.outer_rect.map(|outer| {
        round_point(Point {
            x: outer.min.x * pixels_per_point,
            y: outer.min.y * pixels_per_point,
        })
    });
    let monitor = outer_position_px
        .map(|position| Point {
            x: position.x + outer_size_logical.width * native_scale * 0.5,
            y: position.y + outer_size_logical.height * native_scale * 0.5,
        })
        .and_then(|center| {
            monitors
                .iter()
                .find(|monitor| monitor.bounds_px.contains(center))
        })
        .or_else(|| {
            previous.and_then(|previous| {
                previous.monitor_name.as_ref().and_then(|name| {
                    monitors
                        .iter()
                        .find(|monitor| monitor.name.as_ref() == Some(name))
                })
            })
        })
        .or_else(|| monitors.iter().find(|monitor| monitor.is_primary));

    Some(NormalWindowBounds {
        inner_position_px: inner_position_px
            .or_else(|| previous.and_then(|state| state.inner_position_px)),
        outer_position_px: outer_position_px
            .or_else(|| previous.and_then(|state| state.outer_position_px)),
        inner_size_logical,
        outer_size_logical,
        native_scale_factor: native_scale,
        monitor_name: monitor.and_then(|monitor| monitor.name.clone()),
        monitor_bounds_px: monitor.map(|monitor| monitor.bounds_px),
    })
}

fn round_half(value: f32) -> f32 {
    (value * 2.0).round() * 0.5
}

fn round_point(point: Point) -> Point {
    Point {
        x: point.x.round(),
        y: point.y.round(),
    }
}

fn discover_monitors() -> Vec<MonitorGeometry> {
    let Ok(displays) = display_info::DisplayInfo::all() else {
        return Vec::new();
    };
    #[cfg(target_os = "linux")]
    let x11_work_area = x11_work_area();
    #[cfg(target_os = "macos")]
    let primary_height_logical = displays
        .iter()
        .find(|display| display.is_primary)
        .map_or(0.0, |display| display.height as f32);

    displays
        .into_iter()
        .filter_map(|display| {
            let scale = if display.scale_factor.is_finite() && display.scale_factor > 0.0 {
                display.scale_factor
            } else {
                1.0
            };
            #[cfg(target_os = "macos")]
            let bounds = PixelRect {
                x: display.x as f32 * scale,
                y: display.y as f32 * scale,
                width: display.width as f32 * scale,
                height: display.height as f32 * scale,
            };
            #[cfg(not(target_os = "macos"))]
            let bounds = PixelRect {
                x: display.x as f32,
                y: display.y as f32,
                width: display.width as f32,
                height: display.height as f32,
            };

            #[cfg(windows)]
            let work_area = windows_work_area(display.raw_handle).unwrap_or(bounds);
            #[cfg(target_os = "macos")]
            let work_area = macos_work_area(display.raw_handle, scale, primary_height_logical)
                .unwrap_or(bounds);
            #[cfg(target_os = "linux")]
            let work_area = x11_work_area
                .and_then(|area| area.intersection(bounds))
                .unwrap_or(bounds);
            #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
            let work_area = bounds;

            let monitor = MonitorGeometry {
                name: Some(if display.friendly_name.is_empty() {
                    display.name
                } else {
                    display.friendly_name
                }),
                is_primary: display.is_primary,
                bounds_px: bounds,
                work_area_px: work_area,
                scale_factor: scale,
            };
            monitor.validate().then_some(monitor)
        })
        .collect()
}

#[cfg(windows)]
fn windows_work_area(monitor: windows::Win32::Graphics::Gdi::HMONITOR) -> Option<PixelRect> {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO};

    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetMonitorInfoW(monitor, &mut info) }.ok().is_err() {
        return None;
    }
    Some(PixelRect {
        x: info.rcWork.left as f32,
        y: info.rcWork.top as f32,
        width: (info.rcWork.right - info.rcWork.left) as f32,
        height: (info.rcWork.bottom - info.rcWork.top) as f32,
    })
}

#[cfg(target_os = "macos")]
fn macos_work_area(display_id: u32, scale: f32, primary_height_logical: f32) -> Option<PixelRect> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSScreen;
    use objc2_foundation::{NSNumber, NSString};

    let screens = NSScreen::screens(unsafe { MainThreadMarker::new_unchecked() });
    for screen in screens {
        let number = screen
            .deviceDescription()
            .objectForKey(&NSString::from_str("NSScreenNumber"))?
            .downcast::<NSNumber>()
            .ok()?
            .unsignedIntValue();
        if number != display_id {
            continue;
        }
        let visible = screen.visibleFrame();
        return Some(PixelRect {
            x: visible.origin.x as f32 * scale,
            y: (primary_height_logical - visible.size.height as f32 - visible.origin.y as f32)
                * scale,
            width: visible.size.width as f32 * scale,
            height: visible.size.height as f32 * scale,
        });
    }
    None
}

#[cfg(target_os = "linux")]
fn x11_work_area() -> Option<PixelRect> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var_os("XDG_SESSION_TYPE")
            .is_some_and(|value| value.to_string_lossy().eq_ignore_ascii_case("wayland"))
    {
        return None;
    }

    use xcb::x;

    let (connection, screen_number) = xcb::Connection::connect(None).ok()?;
    let root = connection
        .get_setup()
        .roots()
        .nth(screen_number as usize)?
        .root();
    let current_desktop = x11_cardinals(&connection, root, b"_NET_CURRENT_DESKTOP")?
        .first()
        .copied()
        .unwrap_or(0) as usize;
    let work_areas = x11_cardinals(&connection, root, b"_NET_WORKAREA")?;
    let values = work_areas.get(current_desktop * 4..current_desktop * 4 + 4)?;
    let area = PixelRect {
        x: values[0] as i32 as f32,
        y: values[1] as i32 as f32,
        width: values[2] as f32,
        height: values[3] as f32,
    };
    area.is_valid().then_some(area)
}

#[cfg(target_os = "linux")]
fn x11_cardinals(
    connection: &xcb::Connection,
    root: xcb::x::Window,
    name: &[u8],
) -> Option<Vec<u32>> {
    use xcb::x;

    let atom_cookie = connection.send_request(&x::InternAtom {
        only_if_exists: true,
        name,
    });
    let atom = connection.wait_for_reply(atom_cookie).ok()?.atom();
    if atom == x::ATOM_NONE {
        return None;
    }
    let property_cookie = connection.send_request(&x::GetProperty {
        delete: false,
        window: root,
        property: atom,
        r#type: x::ATOM_CARDINAL,
        long_offset: 0,
        long_length: 1024,
    });
    let reply = connection.wait_for_reply(property_cookie).ok()?;
    Some(reply.value::<u32>().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(
        name: &str,
        primary: bool,
        bounds: [f32; 4],
        work: [f32; 4],
        scale: f32,
    ) -> MonitorGeometry {
        MonitorGeometry {
            name: Some(name.to_owned()),
            is_primary: primary,
            bounds_px: PixelRect {
                x: bounds[0],
                y: bounds[1],
                width: bounds[2],
                height: bounds[3],
            },
            work_area_px: PixelRect {
                x: work[0],
                y: work[1],
                width: work[2],
                height: work[3],
            },
            scale_factor: scale,
        }
    }

    fn state(mode: WindowMode, position: [f32; 2], size: [f32; 2]) -> PersistedWindowState {
        PersistedWindowState {
            schema_version: SCHEMA_VERSION,
            mode,
            normal: NormalWindowBounds {
                inner_position_px: Some(Point {
                    x: position[0] + 8.0,
                    y: position[1] + 32.0,
                }),
                outer_position_px: Some(Point {
                    x: position[0],
                    y: position[1],
                }),
                inner_size_logical: Size {
                    width: size[0],
                    height: size[1],
                },
                outer_size_logical: Size {
                    width: size[0] + 16.0,
                    height: size[1] + 40.0,
                },
                native_scale_factor: 1.0,
                monitor_name: Some("primary".to_owned()),
                monitor_bounds_px: Some(PixelRect {
                    x: 0.0,
                    y: 0.0,
                    width: 1920.0,
                    height: 1080.0,
                }),
            },
        }
    }

    fn primary_monitor() -> MonitorGeometry {
        monitor(
            "primary",
            true,
            [0.0, 0.0, 1920.0, 1080.0],
            [0.0, 0.0, 1920.0, 1040.0],
            1.0,
        )
    }

    #[test]
    fn schema_round_trips_every_window_mode() {
        for mode in [
            WindowMode::Normal,
            WindowMode::Maximized,
            WindowMode::Fullscreen,
        ] {
            let original = state(mode, [120.0, 80.0], [1000.0, 700.0]);
            let encoded = serde_json::to_string(&original).unwrap();
            assert_eq!(parse_state(&encoded), Some(original));
        }
    }

    #[test]
    fn corrupt_partial_future_and_non_finite_states_fall_back() {
        assert!(parse_state("not json").is_none());
        assert!(parse_state(r#"{"schema_version":1}"#).is_none());
        let future = serde_json::to_string(&PersistedWindowState {
            schema_version: 2,
            ..state(WindowMode::Normal, [0.0, 0.0], [800.0, 600.0])
        })
        .unwrap();
        assert!(parse_state(&future).is_none());

        let invalid = r#"{
            "schema_version": 1,
            "mode": "normal",
            "normal": {
                "inner_position_px": {"x": 0.0, "y": 0.0},
                "outer_position_px": {"x": 0.0, "y": 0.0},
                "inner_size_logical": {"width": -1.0, "height": 600.0},
                "outer_size_logical": {"width": 816.0, "height": 640.0},
                "native_scale_factor": 1.0,
                "monitor_name": null,
                "monitor_bounds_px": null
            }
        }"#;
        assert!(parse_state(invalid).is_none());
        assert!(
            !Point {
                x: f32::NAN,
                y: 0.0
            }
            .is_finite()
        );
        let mut invalid_values = state(WindowMode::Normal, [0.0, 0.0], [800.0, 600.0]);
        invalid_values.normal.native_scale_factor = f32::INFINITY;
        assert!(!invalid_values.validate());
        invalid_values.normal.native_scale_factor = 1.0;
        invalid_values.normal.inner_size_logical.width = f32::NAN;
        assert!(!invalid_values.validate());
        invalid_values.normal.inner_size_logical.width = 900.0;
        invalid_values.normal.outer_size_logical.width = 800.0;
        assert!(!invalid_values.validate());
    }

    #[test]
    fn restored_dimensions_obey_minimum_and_work_area() {
        let tiny = state(WindowMode::Normal, [10.0, 10.0], [100.0, 100.0]);
        let restored = restore_viewport(&tiny, &[primary_monitor()]).unwrap();
        assert_eq!(restored.inner_size, MIN_INNER_SIZE);

        let huge = state(WindowMode::Normal, [10.0, 10.0], [4000.0, 3000.0]);
        let restored = restore_viewport(&huge, &[primary_monitor()]).unwrap();
        assert_eq!(restored.inner_size, [1904.0, 1000.0]);
    }

    #[test]
    fn negative_coordinate_monitor_is_restored_without_primary_fallback() {
        let monitors = [
            monitor(
                "left",
                false,
                [-1600.0, 0.0, 1600.0, 900.0],
                [-1600.0, 0.0, 1600.0, 860.0],
                1.0,
            ),
            primary_monitor(),
        ];
        let mut saved = state(WindowMode::Normal, [-1400.0, 100.0], [900.0, 600.0]);
        saved.normal.monitor_name = Some("left".to_owned());
        let restored = restore_viewport(&saved, &monitors).unwrap();
        assert_eq!(restored.position, Some([-1400.0, 100.0]));
    }

    #[test]
    fn disconnected_or_barely_visible_window_is_centered_on_primary() {
        let disconnected = state(WindowMode::Normal, [4000.0, 300.0], [1000.0, 700.0]);
        let restored = restore_viewport(&disconnected, &[primary_monitor()]).unwrap();
        assert_eq!(restored.position, Some([452.0, 150.0]));

        let barely_visible = state(WindowMode::Normal, [1910.0, 100.0], [1000.0, 700.0]);
        let restored = restore_viewport(&barely_visible, &[primary_monitor()]).unwrap();
        assert_eq!(restored.position, Some([452.0, 150.0]));

        let secondary = monitor(
            "secondary",
            false,
            [1920.0, 0.0, 1920.0, 1080.0],
            [1920.0, 0.0, 1920.0, 1040.0],
            1.0,
        );
        let mut stale_secondary = state(WindowMode::Normal, [5000.0, 300.0], [1000.0, 700.0]);
        stale_secondary.normal.monitor_name = Some("secondary".to_owned());
        let restored = restore_viewport(&stale_secondary, &[primary_monitor(), secondary]).unwrap();
        assert_eq!(restored.position, Some([452.0, 150.0]));
    }

    #[test]
    fn dpi_change_preserves_logical_size_and_clamps_physical_placement() {
        let mut saved = state(WindowMode::Normal, [100.0, 100.0], [900.0, 600.0]);
        saved.normal.native_scale_factor = 1.0;
        let scaled = monitor(
            "primary",
            true,
            [0.0, 0.0, 2560.0, 1440.0],
            [0.0, 0.0, 2560.0, 1400.0],
            2.0,
        );
        let restored = restore_viewport(&saved, &[scaled]).unwrap();
        assert_eq!(restored.inner_size, [900.0, 600.0]);
        assert_eq!(restored.position, Some([50.0, 50.0]));
    }

    #[test]
    fn special_modes_restore_without_losing_normal_bounds() {
        for (mode, maximized) in [
            (WindowMode::Normal, false),
            (WindowMode::Maximized, true),
            (WindowMode::Fullscreen, false),
        ] {
            let restored = restore_viewport(
                &state(mode, [120.0, 80.0], [1000.0, 700.0]),
                &[primary_monitor()],
            )
            .unwrap();
            assert_eq!(restored.position, Some([120.0, 80.0]));
            assert_eq!(restored.inner_size, [1000.0, 700.0]);
            assert_eq!(restored.maximized, maximized);
        }
    }

    #[test]
    fn missing_platform_position_restores_size_and_leaves_placement_to_the_os() {
        let mut saved = state(WindowMode::Normal, [120.0, 80.0], [1000.0, 700.0]);
        saved.normal.inner_position_px = None;
        saved.normal.outer_position_px = None;
        let restored = restore_viewport(&saved, &[primary_monitor()]).unwrap();
        assert_eq!(restored.position, None);
        assert_eq!(restored.inner_size, [1000.0, 700.0]);
    }

    #[test]
    fn tracker_preserves_normal_bounds_through_special_modes() {
        let initial = state(WindowMode::Normal, [120.0, 80.0], [1000.0, 700.0]);
        let initial_normal = initial.normal.clone();
        let changed_normal = state(WindowMode::Normal, [200.0, 160.0], [900.0, 600.0]).normal;
        let mut tracker = WindowStateTracker::new(None, vec![primary_monitor()], Some(initial));

        tracker.record_observation(WindowMode::Maximized, Some(changed_normal.clone()));
        assert_eq!(
            tracker.current.as_ref().unwrap().normal,
            initial_normal,
            "maximized frames must not replace the OS restore rectangle"
        );
        tracker.record_observation(WindowMode::Fullscreen, Some(changed_normal.clone()));
        assert_eq!(tracker.current.as_ref().unwrap().normal, initial_normal);
        tracker.record_observation(WindowMode::Normal, Some(changed_normal.clone()));
        assert_eq!(tracker.current.as_ref().unwrap().normal, changed_normal);
    }

    #[test]
    fn atomic_writer_replaces_complete_state() {
        let temp = std::env::temp_dir().join(format!(
            "baboon-window-state-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&temp).unwrap();
        let path = temp.join("window-state.json");
        fs::write(&path, "old").unwrap();
        let saved = state(WindowMode::Maximized, [20.0, 30.0], [900.0, 600.0]);
        write_state_atomic(&path, &saved).unwrap();
        assert_eq!(
            parse_state(&fs::read_to_string(&path).unwrap()),
            Some(saved)
        );
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn uncommitted_atomic_write_preserves_previous_file() {
        let temp = std::env::temp_dir().join(format!(
            "baboon-window-state-interrupted-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&temp).unwrap();
        let path = temp.join("window-state.json");
        fs::write(&path, "previous").unwrap();
        {
            let mut file = AtomicWriteFile::open(&path).unwrap();
            file.write_all(b"incomplete").unwrap();
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "previous");
        fs::remove_dir_all(temp).unwrap();
    }
}
