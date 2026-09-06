use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct ComputerPermissions {
    required: bool,
    accessibility: bool,
    screen_recording: bool,
}

impl ComputerPermissions {
    pub fn ready(&self) -> bool {
        !self.required || (self.accessibility && self.screen_recording)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerPermission {
    Accessibility,
    ScreenRecording,
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{ComputerPermission, ComputerPermissions};
    use enigo::{Enigo, Settings};
    use std::process::Command;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
    }

    pub fn status() -> ComputerPermissions {
        // Query permission only: do not capture the screen or simulate input.
        unsafe {
            ComputerPermissions {
                required: true,
                accessibility: AXIsProcessTrusted(),
                screen_recording: CGPreflightScreenCaptureAccess(),
            }
        }
    }

    pub fn request(permission: ComputerPermission) -> Result<(), String> {
        match permission {
            ComputerPermission::Accessibility if !status().accessibility => {
                // Enigo owns the existing Accessibility permission prompt.
                // Denial is reflected by status(), not treated as a host crash.
                let _ = Enigo::new(&Settings {
                    open_prompt_to_get_permissions: true,
                    ..Settings::default()
                });
                if !status().accessibility {
                    open_settings("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")?;
                }
            }
            ComputerPermission::ScreenRecording if !status().screen_recording => unsafe {
                CGRequestScreenCaptureAccess();
                if !status().screen_recording {
                    open_settings("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")?;
                }
            },
            _ => {}
        }
        Ok(())
    }

    fn open_settings(url: &str) -> Result<(), String> {
        let result = Command::new("open")
            .arg(url)
            .status()
            .map_err(|error| error.to_string())?;
        if result.success() {
            Ok(())
        } else {
            Err(format!("could not open macOS privacy settings: {result}"))
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::{ComputerPermission, ComputerPermissions};

    pub fn status() -> ComputerPermissions {
        // These are macOS privacy permissions. Other platforms do not expose
        // this authorization flow; OS/session input restrictions still apply.
        ComputerPermissions {
            required: false,
            accessibility: false,
            screen_recording: false,
        }
    }

    pub fn request(_: ComputerPermission) -> Result<(), String> {
        Ok(())
    }
}

#[tauri::command]
pub fn computer_permissions() -> ComputerPermissions {
    platform::status()
}

#[tauri::command]
pub async fn request_computer_permission(
    permission: ComputerPermission,
) -> Result<ComputerPermissions, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<ComputerPermissions, String> {
        platform::request(permission)?;
        Ok(platform::status())
    })
    .await
    .map_err(|error| error.to_string())?
}
