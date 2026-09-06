use serde::Serialize;

use crate::computer_applications::{ApplicationTarget, WindowBounds};

const MAX_ACCESSIBILITY_NODES: usize = 320;
const MAX_ACCESSIBILITY_DEPTH: usize = 12;
const MAX_ACCESSIBILITY_TEXT_CHARS: usize = 240;

#[derive(Clone, Debug, Serialize)]
pub struct AccessibilitySnapshot {
    pub available: bool,
    pub application: String,
    pub window_title: String,
    pub nodes: Vec<AccessibilityNode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AccessibilityNode {
    pub element_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<u32>,
    pub role: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub value: String,
    pub enabled: bool,
    pub focusable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounds: Option<NormalizedBounds>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct NormalizedBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl AccessibilitySnapshot {
    pub fn unavailable(error: impl Into<String>) -> Self {
        Self {
            available: false,
            application: String::new(),
            window_title: String::new(),
            nodes: Vec::new(),
            error: Some(error.into()),
        }
    }

    pub fn element_target(
        &self,
        element_id: u32,
        current_window: WindowBounds,
    ) -> Result<(i32, i32), String> {
        let node = self
            .nodes
            .iter()
            .find(|node| node.element_id == element_id)
            .ok_or_else(|| {
                format!("accessibility element is not in the current observation: {element_id}")
            })?;
        let bounds = node.bounds.ok_or_else(|| {
            format!("accessibility element has no actionable bounds: {element_id}")
        })?;
        let x = bounds.x + bounds.width / 2.0;
        let y = bounds.y + bounds.height / 2.0;
        normalized_window_point(current_window, x, y)
    }
}

fn normalized_window_point(window: WindowBounds, x: f64, y: f64) -> Result<(i32, i32), String> {
    let px = f64::from(window.x) + x * f64::from(window.width.saturating_sub(1));
    let py = f64::from(window.y) + y * f64::from(window.height.saturating_sub(1));
    let point = (px.round() as i32, py.round() as i32);
    window
        .contains(point.0, point.1)
        .then_some(point)
        .ok_or_else(|| "accessibility target is outside the current window".into())
}

fn normalized_bounds(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    surface: WindowBounds,
) -> Option<NormalizedBounds> {
    if width <= 0.0 || height <= 0.0 || surface.width == 0 || surface.height == 0 {
        return None;
    }
    let surface_width = f64::from(surface.width);
    let surface_height = f64::from(surface.height);
    let left = ((x - f64::from(surface.x)) / surface_width).clamp(0.0, 1.0);
    let top = ((y - f64::from(surface.y)) / surface_height).clamp(0.0, 1.0);
    let right = ((x + width - f64::from(surface.x)) / surface_width).clamp(0.0, 1.0);
    let bottom = ((y + height - f64::from(surface.y)) / surface_height).clamp(0.0, 1.0);
    (right > left && bottom > top).then_some(NormalizedBounds {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

fn limited_text(value: String) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    normalized
        .chars()
        .take(MAX_ACCESSIBILITY_TEXT_CHARS)
        .collect()
}

pub fn capture_accessibility_tree(target: &ApplicationTarget) -> AccessibilitySnapshot {
    platform::capture(target).unwrap_or_else(AccessibilitySnapshot::unavailable)
}

pub fn focus_accessibility_window(target: &ApplicationTarget) -> Result<(), String> {
    platform::focus(target)
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use core_foundation::base::{CFGetTypeID, CFRelease, CFTypeID, CFTypeRef, TCFType};
    use core_foundation::string::{CFString, CFStringGetTypeID, CFStringRef};
    use core_foundation_sys::array::{CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex};
    use std::ffi::c_void;
    use std::ptr;

    type AXUIElementRef = *const c_void;
    type AXValueRef = *const c_void;
    type AXError = i32;

    const AX_SUCCESS: AXError = 0;
    const AX_VALUE_CG_POINT: u32 = 1;
    const AX_VALUE_CG_SIZE: u32 = 2;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> AXError;
        fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> AXError;
        fn AXValueGetType(value: AXValueRef) -> u32;
        fn AXValueGetValue(value: AXValueRef, value_type: u32, output: *mut c_void) -> bool;
    }

    struct OwnedValue(CFTypeRef);

    impl Drop for OwnedValue {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) };
            }
        }
    }

    pub fn capture(target: &ApplicationTarget) -> Result<AccessibilitySnapshot, String> {
        unsafe {
            let application = OwnedValue(AXUIElementCreateApplication(
                target
                    .process_id
                    .try_into()
                    .map_err(|_| "application process id is outside the macOS AX range")?,
            ) as CFTypeRef);
            if application.0.is_null() {
                return Err("target application is unavailable from macOS Accessibility".into());
            }
            let window = target_window(application.0 as AXUIElementRef, target);
            let root = window.as_ref().unwrap_or(&application);
            let window_title = string_attribute(root.0 as AXUIElementRef, "AXTitle");
            let mut nodes = Vec::new();
            visit(root.0 as AXUIElementRef, None, 0, target.bounds, &mut nodes);
            Ok(AccessibilitySnapshot {
                available: true,
                application: limited_text(target.display_name.clone()),
                window_title: limited_text(window_title),
                nodes,
                error: None,
            })
        }
    }

    pub fn focus(target: &ApplicationTarget) -> Result<(), String> {
        unsafe {
            let application = OwnedValue(AXUIElementCreateApplication(
                target
                    .process_id
                    .try_into()
                    .map_err(|_| "application process id is outside the macOS AX range")?,
            ) as CFTypeRef);
            if application.0.is_null() {
                return Err("target application is unavailable from macOS Accessibility".into());
            }
            let window =
                target_window(application.0 as AXUIElementRef, target).ok_or_else(|| {
                    "target window is unavailable from macOS Accessibility".to_string()
                })?;
            let action = CFString::new("AXRaise");
            if AXUIElementPerformAction(window.0 as AXUIElementRef, action.as_concrete_TypeRef())
                != AX_SUCCESS
            {
                return Err("macOS Accessibility could not raise the target window".into());
            }
            Ok(())
        }
    }

    unsafe fn visit(
        element: AXUIElementRef,
        parent_id: Option<u32>,
        depth: usize,
        surface: WindowBounds,
        nodes: &mut Vec<AccessibilityNode>,
    ) {
        if depth > MAX_ACCESSIBILITY_DEPTH || nodes.len() >= MAX_ACCESSIBILITY_NODES {
            return;
        }
        let element_id = nodes.len() as u32 + 1;
        let role = string_attribute(element, "AXRole");
        let subrole = string_attribute(element, "AXSubrole");
        let title = string_attribute(element, "AXTitle");
        let description = string_attribute(element, "AXDescription");
        let help = string_attribute(element, "AXHelp");
        let name = [title, description, help]
            .into_iter()
            .find(|value| !value.is_empty())
            .unwrap_or_default();
        let value = if subrole == "AXSecureTextField" {
            String::new()
        } else {
            string_attribute(element, "AXValue")
        };
        let rect = element_rect(element);
        nodes.push(AccessibilityNode {
            element_id,
            parent_id,
            role: limited_text(role),
            name: limited_text(name),
            value: limited_text(value),
            enabled: bool_attribute(element, "AXEnabled").unwrap_or(true),
            focusable: bool_attribute(element, "AXFocused").is_some(),
            bounds: rect.and_then(|rect| {
                normalized_bounds(
                    rect.origin.x,
                    rect.origin.y,
                    rect.size.width,
                    rect.size.height,
                    surface,
                )
            }),
        });
        let Some(children) = copy_attribute(element, "AXChildren") else {
            return;
        };
        if CFGetTypeID(children.0) != CFArrayGetTypeID() {
            return;
        }
        let count = CFArrayGetCount(children.0.cast());
        for index in 0..count {
            if nodes.len() >= MAX_ACCESSIBILITY_NODES {
                break;
            }
            let child = CFArrayGetValueAtIndex(children.0.cast(), index) as AXUIElementRef;
            if !child.is_null() {
                visit(child, Some(element_id), depth + 1, surface, nodes);
            }
        }
    }

    unsafe fn target_window(
        application: AXUIElementRef,
        target: &ApplicationTarget,
    ) -> Option<OwnedValue> {
        let windows = copy_attribute(application, "AXWindows")?;
        if CFGetTypeID(windows.0) != CFArrayGetTypeID() {
            return copy_attribute(application, "AXFocusedWindow");
        }
        let mut best: Option<(f64, AXUIElementRef)> = None;
        let count = CFArrayGetCount(windows.0.cast());
        for index in 0..count {
            let candidate = CFArrayGetValueAtIndex(windows.0.cast(), index) as AXUIElementRef;
            if candidate.is_null() {
                continue;
            }
            if !target.window_title.is_empty()
                && string_attribute(candidate, "AXTitle") == target.window_title
            {
                return retain_element(candidate);
            }
            let overlap = element_rect(candidate)
                .map(|rect| overlap_area(rect, target.bounds))
                .unwrap_or_default();
            if best.map(|(area, _)| overlap > area).unwrap_or(true) {
                best = Some((overlap, candidate));
            }
        }
        best.and_then(|(_, element)| retain_element(element))
            .or_else(|| copy_attribute(application, "AXFocusedWindow"))
    }

    unsafe fn retain_element(element: AXUIElementRef) -> Option<OwnedValue> {
        let retained = core_foundation_sys::base::CFRetain(element.cast()) as CFTypeRef;
        (!retained.is_null()).then_some(OwnedValue(retained))
    }

    fn overlap_area(rect: CGRect, bounds: WindowBounds) -> f64 {
        let left = rect.origin.x.max(f64::from(bounds.x));
        let top = rect.origin.y.max(f64::from(bounds.y));
        let right =
            (rect.origin.x + rect.size.width).min(f64::from(bounds.x) + f64::from(bounds.width));
        let bottom =
            (rect.origin.y + rect.size.height).min(f64::from(bounds.y) + f64::from(bounds.height));
        (right - left).max(0.0) * (bottom - top).max(0.0)
    }

    unsafe fn copy_attribute(element: AXUIElementRef, name: &str) -> Option<OwnedValue> {
        let attribute = CFString::new(name);
        let mut value: CFTypeRef = ptr::null();
        (AXUIElementCopyAttributeValue(element, attribute.as_concrete_TypeRef(), &mut value)
            == AX_SUCCESS
            && !value.is_null())
        .then_some(OwnedValue(value))
    }

    unsafe fn string_attribute(element: AXUIElementRef, name: &str) -> String {
        let Some(value) = copy_attribute(element, name) else {
            return String::new();
        };
        if CFGetTypeID(value.0) != CFStringGetTypeID() {
            return String::new();
        }
        CFString::wrap_under_get_rule(value.0 as CFStringRef).to_string()
    }

    unsafe fn bool_attribute(element: AXUIElementRef, name: &str) -> Option<bool> {
        let value = copy_attribute(element, name)?;
        let type_id: CFTypeID = CFGetTypeID(value.0);
        if type_id != core_foundation_sys::number::CFBooleanGetTypeID() {
            return None;
        }
        Some(core_foundation_sys::number::CFBooleanGetValue(
            value.0.cast(),
        ))
    }

    unsafe fn element_rect(element: AXUIElementRef) -> Option<CGRect> {
        let position_value = copy_attribute(element, "AXPosition")?;
        let size_value = copy_attribute(element, "AXSize")?;
        let position_ax = position_value.0 as AXValueRef;
        let size_ax = size_value.0 as AXValueRef;
        if AXValueGetType(position_ax) != AX_VALUE_CG_POINT
            || AXValueGetType(size_ax) != AX_VALUE_CG_SIZE
        {
            return None;
        }
        let mut origin = CGPoint::default();
        let mut size = CGSize::default();
        if !AXValueGetValue(
            position_ax,
            AX_VALUE_CG_POINT,
            (&mut origin as *mut CGPoint).cast(),
        ) || !AXValueGetValue(size_ax, AX_VALUE_CG_SIZE, (&mut size as *mut CGSize).cast())
        {
            return None;
        }
        Some(CGRect { origin, size })
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation, IUIAutomationElement};

    use windows::Win32::Foundation::HWND;

    pub fn capture(target: &ApplicationTarget) -> Result<AccessibilitySnapshot, String> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                    .map_err(|error| error.to_string())?;
            let root = automation
                .ElementFromHandle(HWND(target.window_id as isize as *mut _))
                .map_err(|error| error.to_string())?;
            let walker = automation
                .ControlViewWalker()
                .map_err(|error| error.to_string())?;
            let mut nodes = Vec::new();
            visit(&walker, &root, None, 0, target.bounds, &mut nodes);
            let window_title = root
                .CurrentName()
                .map(|value| value.to_string())
                .unwrap_or_default();
            Ok(AccessibilitySnapshot {
                available: true,
                application: limited_text(target.display_name.clone()),
                window_title: limited_text(window_title),
                nodes,
                error: None,
            })
        }
    }

    pub fn focus(_: &ApplicationTarget) -> Result<(), String> {
        Ok(())
    }

    unsafe fn visit(
        walker: &windows::Win32::UI::Accessibility::IUIAutomationTreeWalker,
        element: &IUIAutomationElement,
        parent_id: Option<u32>,
        depth: usize,
        surface: WindowBounds,
        nodes: &mut Vec<AccessibilityNode>,
    ) {
        if depth > MAX_ACCESSIBILITY_DEPTH || nodes.len() >= MAX_ACCESSIBILITY_NODES {
            return;
        }
        let element_id = nodes.len() as u32 + 1;
        let bounds = element.CurrentBoundingRectangle().ok().and_then(|rect| {
            normalized_bounds(
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                surface,
            )
        });
        let role = element
            .CurrentLocalizedControlType()
            .map(|value| value.to_string())
            .unwrap_or_default();
        let name = element
            .CurrentName()
            .map(|value| value.to_string())
            .unwrap_or_default();
        let value = element
            .CurrentHelpText()
            .map(|value| value.to_string())
            .unwrap_or_default();
        nodes.push(AccessibilityNode {
            element_id,
            parent_id,
            role: limited_text(role),
            name: limited_text(name),
            value: limited_text(value),
            enabled: element
                .CurrentIsEnabled()
                .map(|value| value.as_bool())
                .unwrap_or(true),
            focusable: element
                .CurrentIsKeyboardFocusable()
                .map(|value| value.as_bool())
                .unwrap_or(false),
            bounds,
        });
        let mut child = walker.GetFirstChildElement(element).ok();
        while let Some(current) = child {
            visit(
                walker,
                &current,
                Some(element_id),
                depth + 1,
                surface,
                nodes,
            );
            if nodes.len() >= MAX_ACCESSIBILITY_NODES {
                break;
            }
            child = walker.GetNextSiblingElement(&current).ok();
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use super::*;

    pub fn capture(_: &ApplicationTarget) -> Result<AccessibilitySnapshot, String> {
        Err("accessibility tree is not supported on this platform".into())
    }

    pub fn focus(_: &ApplicationTarget) -> Result<(), String> {
        Ok(())
    }
}
