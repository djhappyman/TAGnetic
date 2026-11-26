use std::{collections::BTreeMap, fs, io, path::PathBuf, sync::Mutex};

#[cfg(windows)]
use tauri::PhysicalSize;
use tauri::{AppHandle, Manager, Monitor, PhysicalPosition, WebviewWindow};
#[cfg(not(windows))]
use tauri::{LogicalPosition, LogicalSize};

use super::AudioToolKind;

pub(super) const MIN_WIDTH: f64 = 420.0;
pub(super) const MIN_HEIGHT: f64 = 320.0;

// Windows exposes a global physical desktop. Cocoa and GTK express screen
// origins in logical coordinates, then Tao scales each monitor separately.
// Use the native desktop coordinates so unlike-DPI monitors cannot overlap
// in our reachability calculation.
#[cfg(windows)]
type ScreenPosition = PhysicalPosition<i32>;
#[cfg(windows)]
type ScreenSize = PhysicalSize<u32>;
#[cfg(not(windows))]
type ScreenPosition = LogicalPosition<i32>;
#[cfg(not(windows))]
type ScreenSize = LogicalSize<u32>;

#[cfg(windows)]
fn screen_position(position: PhysicalPosition<i32>, _: f64) -> ScreenPosition {
    position
}

#[cfg(not(windows))]
fn screen_position(position: PhysicalPosition<i32>, scale: f64) -> ScreenPosition {
    position.to_logical(scale)
}

#[derive(Default)]
pub(super) struct WindowPreferences(Mutex<Option<Profile>>);

#[derive(Default)]
struct Profile {
    path: Option<PathBuf>,
    entries: BTreeMap<String, Preference>,
    dirty: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub(super) struct Preference {
    pub(super) pinned: bool,
    geometry: Option<Geometry>,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
struct Geometry {
    x: i32,
    y: i32,
    width: f64,
    height: f64,
}

impl WindowPreferences {
    pub(super) fn get(&self, app: &AppHandle, kind: AudioToolKind) -> Preference {
        self.with_profile(app, |profile| {
            profile
                .entries
                .get(kind.label())
                .copied()
                .unwrap_or_default()
        })
    }

    pub(super) fn save(
        &self,
        app: &AppHandle,
        windows: impl IntoIterator<Item = (AudioToolKind, Option<WebviewWindow>)>,
        pinned: Option<bool>,
    ) {
        // Query native windows before taking the profile mutex. A window
        // callback can run while an asynchronous command awaits the event loop.
        let placements: Vec<_> = windows
            .into_iter()
            .map(|(kind, window)| (kind, window.as_ref().and_then(capture)))
            .collect();
        self.with_profile(app, |profile| {
            for (kind, geometry) in placements {
                profile.remember(kind, geometry, pinned);
            }
            if let Err(error) = profile.flush() {
                eprintln!("Could not save audio tool window preferences: {error}");
            }
        });
    }

    fn with_profile<T>(&self, app: &AppHandle, f: impl FnOnce(&mut Profile) -> T) -> T {
        let mut profile = self.0.lock().unwrap_or_else(|error| error.into_inner());
        f(profile.get_or_insert_with(|| {
            Profile::load(
                app.path()
                    .app_config_dir()
                    .ok()
                    .map(|directory| directory.join("audio-tool-windows.json")),
            )
        }))
    }
}

impl Profile {
    fn load(path: Option<PathBuf>) -> Self {
        let entries = path
            .as_ref()
            .and_then(|path| fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            path,
            entries,
            dirty: false,
        }
    }

    fn remember(&mut self, kind: AudioToolKind, geometry: Option<Geometry>, pinned: Option<bool>) {
        let previous = self.entries.get(kind.label()).copied().unwrap_or_default();
        let next = Preference {
            pinned: pinned.unwrap_or(previous.pinned),
            geometry: geometry.or(previous.geometry),
        };
        if next != previous {
            self.entries.insert(kind.label().to_owned(), next);
            self.dirty = true;
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        let Some(path) = self.path.as_ref().filter(|_| self.dirty) else {
            return Ok(());
        };
        if let Some(directory) = path.parent() {
            fs::create_dir_all(directory)?;
        }
        let pending = path.with_extension("json.next");
        fs::write(&pending, serde_json::to_vec(&self.entries)?)?;
        fs::rename(pending, path)?;
        self.dirty = false;
        Ok(())
    }
}

fn capture(window: &WebviewWindow) -> Option<Geometry> {
    if !window.is_visible().ok()?
        || window.is_minimized().ok()?
        || window.is_maximized().ok()?
        || window.is_fullscreen().ok()?
    {
        return None;
    }
    let scale = window.scale_factor().ok()?;
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let position = screen_position(window.outer_position().ok()?, scale);
    let size = window.inner_size().ok()?.to_logical::<f64>(scale);
    (size.width.is_finite() && size.height.is_finite() && size.width > 0.0 && size.height > 0.0)
        .then_some(Geometry {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        })
}

#[derive(Clone, Copy)]
struct WorkArea {
    origin: ScreenPosition,
    size: ScreenSize,
    scale: f64,
}

impl WorkArea {
    fn from_monitor(monitor: &Monitor) -> Option<Self> {
        let rect = monitor.work_area();
        let (origin, size) = if rect.size.width > 0 && rect.size.height > 0 {
            (rect.position, rect.size)
        } else {
            (*monitor.position(), *monitor.size())
        };
        let scale = monitor.scale_factor();
        if size.width == 0 || size.height == 0 || !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let origin = screen_position(origin, scale);
        #[cfg(not(windows))]
        let (size, scale) = (size.to_logical(scale), 1.0);
        Some(Self {
            origin,
            size,
            scale,
        })
    }

    fn contains(self, x: i32, y: i32) -> bool {
        let relative_x = i64::from(x) - i64::from(self.origin.x);
        let relative_y = i64::from(y) - i64::from(self.origin.y);
        (0..i64::from(self.size.width)).contains(&relative_x)
            && (0..i64::from(self.size.height)).contains(&relative_y)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Placement {
    position: ScreenPosition,
    inner: ScreenSize,
}

fn fit_placement(
    geometry: Geometry,
    areas: &[WorkArea],
    fallback: WorkArea,
    frame: (f64, f64),
) -> Placement {
    let matching_area = areas
        .iter()
        .find(|area| area.contains(geometry.x, geometry.y));
    let area = matching_area.copied().unwrap_or(fallback);
    let fit = |desired: f64, frame: f64, minimum: f64, available: u32| {
        let frame = (frame * area.scale).ceil() as u32;
        let inner = ((desired.max(minimum) * area.scale).round() as u32)
            .clamp(1, available.saturating_sub(frame).max(1));
        (inner, available.saturating_sub(inner.saturating_add(frame)))
    };
    let (width, remaining_x) = fit(geometry.width, frame.0, MIN_WIDTH, area.size.width);
    let (height, remaining_y) = fit(geometry.height, frame.1, MIN_HEIGHT, area.size.height);
    let origin = |saved, start, remaining| {
        let start = i64::from(start);
        let remaining = i64::from(remaining);
        let value = if matching_area.is_some() {
            i64::from(saved).clamp(start, start + remaining)
        } else {
            start + remaining / 2
        };
        value.min(i64::from(i32::MAX)) as i32
    };
    Placement {
        position: ScreenPosition::new(
            origin(geometry.x, area.origin.x, remaining_x),
            origin(geometry.y, area.origin.y, remaining_y),
        ),
        inner: ScreenSize::new(width, height),
    }
}

fn available_work_areas(main: &WebviewWindow) -> Vec<WorkArea> {
    main.available_monitors()
        .unwrap_or_default()
        .iter()
        .filter_map(WorkArea::from_monitor)
        .collect()
}

fn place(window: &WebviewWindow, main: &WebviewWindow, geometry: Geometry, areas: &[WorkArea]) {
    let fallback = main
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| main.primary_monitor().ok().flatten());
    let Some(fallback) = fallback
        .as_ref()
        .and_then(WorkArea::from_monitor)
        .or_else(|| areas.first().copied())
    else {
        // Let the compositor make the placement if monitor details are unavailable.
        let _ = window.center();
        return;
    };
    let frame = window
        .outer_size()
        .ok()
        .zip(window.inner_size().ok())
        .zip(
            window
                .scale_factor()
                .ok()
                .filter(|scale| scale.is_finite() && *scale > 0.0),
        )
        .map(|((outer, inner), scale)| {
            (
                f64::from(outer.width.saturating_sub(inner.width)) / scale,
                f64::from(outer.height.saturating_sub(inner.height)) / scale,
            )
        })
        .unwrap_or_default();
    // Decorations may be unavailable before the first show. Allow space for
    // the titlebar even when the underlying compositor has not measured it.
    let placement = fit_placement(geometry, areas, fallback, (frame.0, frame.1.max(36.0)));
    #[cfg(windows)]
    let _ = window.set_position(placement.position);
    let _ = window.set_size(placement.inner);
    #[cfg(not(windows))]
    let _ = window.set_position(placement.position);
}

impl Preference {
    pub(super) fn restore(self, window: &WebviewWindow, main: &WebviewWindow) {
        if let Some(geometry) = self.geometry {
            place(window, main, geometry, &available_work_areas(main));
        }
    }

    pub(super) fn recover_if_offscreen(self, window: &WebviewWindow, main: &WebviewWindow) {
        let areas = available_work_areas(main);
        if let (Ok(position), Ok(scale)) = (window.outer_position(), window.scale_factor()) {
            if !scale.is_finite() || scale <= 0.0 {
                return;
            }
            let position = screen_position(position, scale);
            if !areas.is_empty()
                && !areas
                    .iter()
                    .any(|area| area.contains(position.x, position.y))
            {
                if let Some(geometry) = self.geometry {
                    place(window, main, geometry, &areas);
                } else {
                    let _ = window.center();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: WorkArea = WorkArea {
        origin: ScreenPosition { x: 0, y: 30 },
        size: ScreenSize {
            width: 1200,
            height: 870,
        },
        scale: 1.0,
    };
    const LEFT: WorkArea = WorkArea {
        origin: ScreenPosition { x: -1600, y: -250 },
        size: ScreenSize {
            width: 1600,
            height: 1200,
        },
        scale: 2.0,
    };
    const SAVED: Geometry = Geometry {
        x: -1500,
        y: -190,
        width: 720.0,
        height: 520.0,
    };

    #[test]
    fn restores_negative_monitor_offsets_and_logical_size() {
        let result = fit_placement(SAVED, &[MAIN, LEFT], MAIN, (0.0, 38.0));
        assert_eq!(result.position, ScreenPosition::new(-1500, -190));
        assert_eq!(result.inner, ScreenSize::new(1440, 1040));
    }

    #[test]
    fn centers_a_disconnected_display_placement_in_the_current_work_area() {
        let result = fit_placement(SAVED, &[MAIN], MAIN, (0.0, 38.0));
        assert_eq!(result.position, ScreenPosition::new(240, 186));
        assert_eq!(result.inner, ScreenSize::new(720, 520));
    }

    #[test]
    fn fits_oversize_saved_bounds_inside_the_live_work_area() {
        let geometry = Geometry {
            x: 1150,
            y: 800,
            width: 2400.0,
            height: 1800.0,
        };
        let result = fit_placement(geometry, &[MAIN], MAIN, (8.0, 38.0));
        assert_eq!(result.position, MAIN.origin);
        assert_eq!(result.inner, ScreenSize::new(1192, 832));
    }

    #[test]
    fn profiles_keep_independent_pins_and_geometry() {
        let mut profile = Profile::default();
        profile.remember(AudioToolKind::Eq, Some(SAVED), Some(true));
        profile.remember(AudioToolKind::Repair, None, Some(true));
        profile.remember(AudioToolKind::Eq, None, Some(false));
        let persisted = serde_json::to_vec(&profile.entries).unwrap();
        let reloaded: BTreeMap<String, Preference> = serde_json::from_slice(&persisted).unwrap();
        assert_eq!(
            reloaded[AudioToolKind::Eq.label()],
            Preference {
                pinned: false,
                geometry: Some(SAVED)
            }
        );
        assert_eq!(
            reloaded[AudioToolKind::Repair.label()],
            Preference {
                pinned: true,
                geometry: None
            }
        );
        assert!(!reloaded.contains_key(AudioToolKind::Dynamics.label()));
    }
}
