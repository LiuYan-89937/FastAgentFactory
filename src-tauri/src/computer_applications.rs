use serde::Serialize;
use std::collections::BTreeMap;
use xcap::Window;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct WindowBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl WindowBounds {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        let right = i64::from(self.x) + i64::from(self.width);
        let bottom = i64::from(self.y) + i64::from(self.height);
        i64::from(x) >= i64::from(self.x)
            && i64::from(x) < right
            && i64::from(y) >= i64::from(self.y)
            && i64::from(y) < bottom
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ApplicationWindowDescriptor {
    pub window_id: u32,
    pub title: String,
    pub bounds: WindowBounds,
    pub focused: bool,
    pub minimized: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ApplicationDescriptor {
    pub application_id: String,
    pub display_name: String,
    pub process_id: u32,
    pub windows: Vec<ApplicationWindowDescriptor>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ApplicationTarget {
    pub application_id: String,
    pub display_name: String,
    pub process_id: u32,
    pub window_id: u32,
    pub window_title: String,
    pub bounds: WindowBounds,
}

pub fn list_applications() -> Result<Vec<ApplicationDescriptor>, String> {
    let windows = Window::all().map_err(|error| error.to_string())?;
    let mut applications = BTreeMap::<u32, ApplicationDescriptor>::new();
    for window in windows {
        let Ok(process_id) = window.pid() else {
            continue;
        };
        let Ok(width) = window.width() else {
            continue;
        };
        let Ok(height) = window.height() else {
            continue;
        };
        if width == 0 || height == 0 {
            continue;
        }
        let display_name = window.app_name().unwrap_or_default();
        if display_name.trim().is_empty() {
            continue;
        }
        let descriptor = ApplicationWindowDescriptor {
            window_id: match window.id() {
                Ok(value) => value,
                Err(_) => continue,
            },
            title: window.title().unwrap_or_default(),
            bounds: WindowBounds {
                x: window.x().unwrap_or_default(),
                y: window.y().unwrap_or_default(),
                width,
                height,
            },
            focused: window.is_focused().unwrap_or(false),
            minimized: window.is_minimized().unwrap_or(false),
        };
        applications
            .entry(process_id)
            .or_insert_with(|| ApplicationDescriptor {
                application_id: format!("process:{process_id}"),
                display_name,
                process_id,
                windows: Vec::new(),
            })
            .windows
            .push(descriptor);
    }
    let mut result = applications.into_values().collect::<Vec<_>>();
    for application in &mut result {
        application.windows.sort_by_key(window_priority);
    }
    result.sort_by(|left, right| {
        left.display_name
            .to_lowercase()
            .cmp(&right.display_name.to_lowercase())
            .then_with(|| left.process_id.cmp(&right.process_id))
    });
    Ok(result)
}

pub fn resolve_application(application_id: &str) -> Result<ApplicationTarget, String> {
    let application = list_applications()?
        .into_iter()
        .find(|candidate| candidate.application_id == application_id)
        .ok_or_else(|| format!("application is no longer available: {application_id}"))?;
    target_from_application(application)
}

pub fn resolve_target_window(
    target: &ApplicationTarget,
) -> Result<(ApplicationTarget, Window), String> {
    let windows = Window::all().map_err(|error| error.to_string())?;
    let mut candidates = windows
        .into_iter()
        .filter(|window| window.pid().ok() == Some(target.process_id))
        .filter_map(|window| {
            let window_id = window.id().ok()?;
            let width = window.width().ok()?;
            let height = window.height().ok()?;
            (width > 0 && height > 0).then_some((window_id, width, height, window))
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(window_id, width, height, window)| {
        let descriptor = ApplicationWindowDescriptor {
            window_id: *window_id,
            title: window.title().unwrap_or_default(),
            bounds: WindowBounds {
                x: window.x().unwrap_or_default(),
                y: window.y().unwrap_or_default(),
                width: *width,
                height: *height,
            },
            focused: window.is_focused().unwrap_or(false),
            minimized: window.is_minimized().unwrap_or(false),
        };
        (
            u8::from(*window_id != target.window_id),
            window_priority(&descriptor),
        )
    });
    let (window_id, width, height, window) = candidates.into_iter().next().ok_or_else(|| {
        format!(
            "target application has no capturable window: {}",
            target.display_name
        )
    })?;
    let resolved = ApplicationTarget {
        application_id: target.application_id.clone(),
        display_name: target.display_name.clone(),
        process_id: target.process_id,
        window_id,
        window_title: window.title().unwrap_or_default(),
        bounds: WindowBounds {
            x: window.x().map_err(|error| error.to_string())?,
            y: window.y().map_err(|error| error.to_string())?,
            width,
            height,
        },
    };
    Ok((resolved, window))
}

pub fn activate_target(target: &ApplicationTarget) -> Result<(), String> {
    platform::activate(target)
}

fn target_from_application(
    application: ApplicationDescriptor,
) -> Result<ApplicationTarget, String> {
    let window = application.windows.into_iter().next().ok_or_else(|| {
        format!(
            "application has no capturable window: {}",
            application.display_name
        )
    })?;
    Ok(ApplicationTarget {
        application_id: application.application_id,
        display_name: application.display_name,
        process_id: application.process_id,
        window_id: window.window_id,
        window_title: window.title,
        bounds: window.bounds,
    })
}

fn window_priority(window: &ApplicationWindowDescriptor) -> (u8, u8, std::cmp::Reverse<u64>) {
    (
        u8::from(!window.focused),
        u8::from(window.minimized),
        std::cmp::Reverse(u64::from(window.bounds.width) * u64::from(window.bounds.height)),
    )
}

#[cfg(target_os = "macos")]
mod platform {
    use super::ApplicationTarget;
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};

    pub fn activate(target: &ApplicationTarget) -> Result<(), String> {
        let process_id = target.process_id;
        let application = NSRunningApplication::runningApplicationWithProcessIdentifier(
            process_id
                .try_into()
                .map_err(|_| format!("invalid application process id: {process_id}"))?,
        )
        .ok_or_else(|| format!("application process is no longer running: {process_id}"))?;
        application.unhide();
        if !application.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows) {
            return Err(format!(
                "macOS refused to activate application process: {process_id}"
            ));
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::ApplicationTarget;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetForegroundWindow, ShowWindowAsync, SW_RESTORE,
    };
    pub fn activate(target: &ApplicationTarget) -> Result<(), String> {
        let handle = HWND(target.window_id as isize as *mut _);
        unsafe {
            let _ = ShowWindowAsync(handle, SW_RESTORE);
            if !SetForegroundWindow(handle).as_bool() {
                return Err(format!(
                    "Windows refused to activate application process: {}",
                    target.process_id
                ));
            }
        }
        Ok(())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use super::ApplicationTarget;

    pub fn activate(_: &ApplicationTarget) -> Result<(), String> {
        Err("application activation is not supported on this platform".into())
    }
}
