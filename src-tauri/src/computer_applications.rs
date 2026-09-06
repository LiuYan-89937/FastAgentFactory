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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_identifier: Option<String>,
    pub process_id: u32,
    pub windows: Vec<ApplicationWindowDescriptor>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ApplicationTarget {
    pub application_id: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_identifier: Option<String>,
    pub process_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_data_url: Option<String>,
    pub window_id: u32,
    pub window_title: String,
    pub bounds: WindowBounds,
}

pub fn list_applications() -> Result<Vec<ApplicationDescriptor>, String> {
    let mut applications = BTreeMap::<u32, ApplicationDescriptor>::new();
    for application in platform::running_applications()? {
        applications.insert(application.process_id, application);
    }
    let windows = match Window::all() {
        Ok(windows) => windows,
        Err(error) if !applications.is_empty() => {
            eprintln!("Could not enrich running applications with windows: {error}");
            Vec::new()
        }
        Err(error) => return Err(error.to_string()),
    };
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
                bundle_identifier: None,
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

pub fn application_descriptor(application_id: &str) -> Result<ApplicationDescriptor, String> {
    list_applications()?
        .into_iter()
        .find(|candidate| candidate.application_id == application_id)
        .ok_or_else(|| format!("application is no longer available: {application_id}"))
}

pub fn resolve_application_target(
    application: &ApplicationDescriptor,
) -> Result<ApplicationTarget, String> {
    let refreshed = list_applications()?
        .into_iter()
        .find(|candidate| candidate.application_id == application.application_id)
        .ok_or_else(|| {
            format!(
                "application exited while attaching: {}",
                application.display_name
            )
        })?;
    target_from_application(refreshed)
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
            "target application has no available window: {}",
            target.display_name
        )
    })?;
    let resolved = ApplicationTarget {
        application_id: target.application_id.clone(),
        display_name: target.display_name.clone(),
        bundle_identifier: target.bundle_identifier.clone(),
        process_id: target.process_id,
        icon_data_url: target.icon_data_url.clone(),
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

#[cfg(not(target_os = "macos"))]
pub fn activate_target(target: &ApplicationTarget) -> Result<(), String> {
    platform::activate(target.process_id, Some(target.window_id))
}

fn target_from_application(
    application: ApplicationDescriptor,
) -> Result<ApplicationTarget, String> {
    let window = application.windows.into_iter().next().ok_or_else(|| {
        format!(
            "application has no available window: {}",
            application.display_name
        )
    })?;
    Ok(ApplicationTarget {
        application_id: application.application_id,
        display_name: application.display_name,
        bundle_identifier: application.bundle_identifier,
        process_id: application.process_id,
        icon_data_url: platform::application_icon_data_url(application.process_id),
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
    use super::ApplicationDescriptor;
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use objc2_app_kit::NSRunningApplication;
    use objc2_app_kit::{NSApplicationActivationPolicy, NSWorkspace};
    use std::io::Cursor;
    use std::ptr::NonNull;
    use xcap::image::ImageFormat;

    pub fn running_applications() -> Result<Vec<ApplicationDescriptor>, String> {
        let workspace = NSWorkspace::sharedWorkspace();
        let running = workspace.runningApplications();
        let mut applications = Vec::new();
        for index in 0..running.count() {
            let application = running.objectAtIndex(index);
            if application.isTerminated()
                || application.activationPolicy() != NSApplicationActivationPolicy::Regular
            {
                continue;
            }
            let process_id = application.processIdentifier();
            let Some(display_name) = application.localizedName() else {
                continue;
            };
            let display_name = display_name.to_string();
            if process_id <= 0 || display_name.trim().is_empty() {
                continue;
            }
            let process_id = process_id as u32;
            let bundle_identifier = application
                .bundleIdentifier()
                .map(|value| value.to_string())
                .filter(|value| !value.trim().is_empty());
            applications.push(ApplicationDescriptor {
                application_id: bundle_identifier
                    .as_ref()
                    .map(|value| format!("bundle:{value}"))
                    .unwrap_or_else(|| format!("process:{process_id}")),
                display_name,
                bundle_identifier,
                process_id,
                windows: Vec::new(),
            });
        }
        Ok(applications)
    }

    pub fn application_icon_data_url(process_id: u32) -> Option<String> {
        let process_id = i32::try_from(process_id).ok()?;
        let application =
            NSRunningApplication::runningApplicationWithProcessIdentifier(process_id)?;
        let icon = application.icon()?;
        let representation = icon.TIFFRepresentation()?;
        let length = representation.length();
        if length == 0 {
            return None;
        }
        let mut tiff = vec![0_u8; length];
        let buffer = NonNull::new(tiff.as_mut_ptr().cast())?;
        unsafe { representation.getBytes_length(buffer, length) };
        let image = xcap::image::load_from_memory(&tiff).ok()?.thumbnail(64, 64);
        let mut png = Cursor::new(Vec::new());
        image.write_to(&mut png, ImageFormat::Png).ok()?;
        Some(format!(
            "data:image/png;base64,{}",
            STANDARD.encode(png.into_inner())
        ))
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::ApplicationDescriptor;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetForegroundWindow, ShowWindowAsync, SW_RESTORE,
    };
    pub fn running_applications() -> Result<Vec<ApplicationDescriptor>, String> {
        Ok(Vec::new())
    }

    pub fn application_icon_data_url(_: u32) -> Option<String> {
        None
    }

    pub fn activate(process_id: u32, window_id: Option<u32>) -> Result<(), String> {
        let window_id =
            window_id.ok_or_else(|| format!("application process has no window: {process_id}"))?;
        let handle = HWND(window_id as isize as *mut _);
        unsafe {
            let _ = ShowWindowAsync(handle, SW_RESTORE);
            if !SetForegroundWindow(handle).as_bool() {
                return Err(format!(
                    "Windows refused to activate application process: {}",
                    process_id
                ));
            }
        }
        Ok(())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use super::ApplicationDescriptor;

    pub fn running_applications() -> Result<Vec<ApplicationDescriptor>, String> {
        Ok(Vec::new())
    }

    pub fn application_icon_data_url(_: u32) -> Option<String> {
        None
    }

    pub fn activate(_: u32, _: Option<u32>) -> Result<(), String> {
        Err("application activation is not supported on this platform".into())
    }
}
