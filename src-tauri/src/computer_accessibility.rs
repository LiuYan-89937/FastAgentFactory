use serde::Serialize;
use std::collections::BTreeMap;

use crate::computer_applications::{ApplicationTarget, WindowBounds};

const MAX_ACCESSIBILITY_NODES: usize = 320;
const MAX_ACCESSIBILITY_RAW_NODES: usize = 2_048;
const MAX_ACCESSIBILITY_DEPTH: usize = 32;
const MAX_ACCESSIBILITY_TEXT_CHARS: usize = 240;

#[derive(Clone, Debug, Serialize)]
pub struct AccessibilitySnapshot {
    pub available: bool,
    pub usable: bool,
    pub complete: bool,
    pub application: String,
    pub window_title: String,
    pub nodes: Vec<AccessibilityNode>,
    pub actionable_node_count: usize,
    pub named_node_count: usize,
    pub quality_score: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip)]
    element_paths: BTreeMap<u32, Vec<usize>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AccessibilityNode {
    pub element_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<u32>,
    pub role: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub subrole: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub value: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub identifier: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub placeholder: String,
    pub enabled: bool,
    pub focused: bool,
    pub focusable: bool,
    pub selected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expanded: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,
    pub value_settable: bool,
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

struct RawAccessibilityNode {
    parent_index: Option<usize>,
    path: Vec<usize>,
    node: AccessibilityNode,
}

impl AccessibilitySnapshot {
    pub fn unavailable(error: impl Into<String>) -> Self {
        Self {
            available: false,
            usable: false,
            complete: false,
            application: String::new(),
            window_title: String::new(),
            nodes: Vec::new(),
            actionable_node_count: 0,
            named_node_count: 0,
            quality_score: 0.0,
            error: Some(error.into()),
            element_paths: BTreeMap::new(),
        }
    }

    fn captured(
        target: &ApplicationTarget,
        window_title: String,
        nodes: Vec<AccessibilityNode>,
        element_paths: BTreeMap<u32, Vec<usize>>,
        complete: bool,
    ) -> Self {
        let actionable_node_count = nodes
            .iter()
            .filter(|node| is_directly_actionable(node))
            .count();
        let named_node_count = nodes
            .iter()
            .filter(|node| {
                !node.name.is_empty() || !node.value.is_empty() || !node.placeholder.is_empty()
            })
            .count();
        let semantic_nodes = nodes.iter().filter(|node| is_semantic_node(node)).count();
        let usable = !nodes.is_empty() && semantic_nodes > 0;
        let denominator = nodes.len().max(1) as f64;
        let semantic_ratio = (semantic_nodes.min(nodes.len()) as f64) / denominator;
        let quality_score = if usable {
            (0.35 + semantic_ratio * 0.5 + if complete { 0.15 } else { 0.0 }).min(1.0)
        } else {
            0.0
        };
        Self {
            available: true,
            usable,
            complete,
            application: limited_text(target.display_name.clone()),
            window_title: limited_text(window_title),
            nodes,
            actionable_node_count,
            named_node_count,
            quality_score,
            error: (!usable)
                .then(|| "Accessibility returned no semantic controls or content".into()),
            element_paths,
        }
    }

    fn element_path(&self, element_id: u32) -> Result<&[usize], String> {
        self.element_paths
            .get(&element_id)
            .map(Vec::as_slice)
            .ok_or_else(|| format!("accessibility element has no native locator: {element_id}"))
    }
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

fn is_semantic_node(node: &AccessibilityNode) -> bool {
    !node.name.is_empty()
        || !node.value.is_empty()
        || !node.placeholder.is_empty()
        || node.role == "AXWebArea"
        || is_directly_actionable(node)
}

fn is_directly_actionable(node: &AccessibilityNode) -> bool {
    node.enabled && node.role != "AXGroup" && (node.value_settable || !node.actions.is_empty())
}

fn compact_nodes(
    raw_nodes: Vec<RawAccessibilityNode>,
) -> (Vec<AccessibilityNode>, BTreeMap<u32, Vec<usize>>, bool) {
    let selected_indices = raw_nodes
        .iter()
        .enumerate()
        .filter_map(|(index, raw)| (index == 0 || is_semantic_node(&raw.node)).then_some(index))
        .collect::<Vec<_>>();
    let projection_truncated = selected_indices.len() > MAX_ACCESSIBILITY_NODES;
    let selected_indices = selected_indices
        .into_iter()
        .take(MAX_ACCESSIBILITY_NODES)
        .collect::<Vec<_>>();
    let identifiers = selected_indices
        .iter()
        .enumerate()
        .map(|(position, raw_index)| (*raw_index, position as u32 + 1))
        .collect::<BTreeMap<_, _>>();
    let mut nodes = Vec::with_capacity(selected_indices.len());
    let mut paths = BTreeMap::new();
    for raw_index in selected_indices {
        let raw = &raw_nodes[raw_index];
        let element_id = identifiers[&raw_index];
        let mut ancestor = raw.parent_index;
        let parent_id = loop {
            let Some(index) = ancestor else {
                break None;
            };
            if let Some(parent_id) = identifiers.get(&index) {
                break Some(*parent_id);
            }
            ancestor = raw_nodes[index].parent_index;
        };
        let mut node = raw.node.clone();
        node.element_id = element_id;
        node.parent_id = parent_id;
        paths.insert(element_id, raw.path.clone());
        nodes.push(node);
    }
    (nodes, paths, projection_truncated)
}

pub fn capture_accessibility_tree(target: &ApplicationTarget) -> AccessibilitySnapshot {
    platform::capture(target).unwrap_or_else(AccessibilitySnapshot::unavailable)
}

pub fn perform_accessibility_element_action(
    target: &ApplicationTarget,
    snapshot: &AccessibilitySnapshot,
    element_id: u32,
    action: &str,
) -> Result<(), String> {
    platform::perform_action(target, snapshot, element_id, action)
}

pub fn set_accessibility_element_value(
    target: &ApplicationTarget,
    snapshot: &AccessibilitySnapshot,
    element_id: u32,
    text: &str,
) -> Result<(), String> {
    platform::set_value(target, snapshot, element_id, text)
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use core_foundation::base::{CFGetTypeID, CFRelease, CFTypeID, CFTypeRef, TCFType};
    use core_foundation::string::{CFString, CFStringGetTypeID, CFStringRef};
    use core_foundation_sys::array::{
        CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex, CFArrayRef,
    };
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
        fn AXUIElementCopyActionNames(element: AXUIElementRef, names: *mut CFArrayRef) -> AXError;
        fn AXUIElementIsAttributeSettable(
            element: AXUIElementRef,
            attribute: CFStringRef,
            settable: *mut bool,
        ) -> AXError;
        fn AXUIElementSetAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: CFTypeRef,
        ) -> AXError;
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
            let window =
                target_window(application.0 as AXUIElementRef, target).ok_or_else(|| {
                    "target window is unavailable from macOS Accessibility".to_string()
                })?;
            let root = &window;
            let window_title = string_attribute(root.0 as AXUIElementRef, "AXTitle");
            let mut raw_nodes = Vec::new();
            let mut traversal_truncated = false;
            visit(
                root.0 as AXUIElementRef,
                None,
                Vec::new(),
                0,
                target.bounds,
                &mut raw_nodes,
                &mut traversal_truncated,
            );
            let (nodes, element_paths, projection_truncated) = compact_nodes(raw_nodes);
            Ok(AccessibilitySnapshot::captured(
                target,
                window_title,
                nodes,
                element_paths,
                !traversal_truncated && !projection_truncated,
            ))
        }
    }

    pub fn perform_action(
        target: &ApplicationTarget,
        snapshot: &AccessibilitySnapshot,
        element_id: u32,
        action: &str,
    ) -> Result<(), String> {
        let action = action.trim();
        if action.is_empty() {
            return Err("accessibility action must not be empty".into());
        }
        let node = snapshot
            .nodes
            .iter()
            .find(|node| node.element_id == element_id)
            .ok_or_else(|| {
                format!("accessibility element is not in the current observation: {element_id}")
            })?;
        if !node.actions.iter().any(|candidate| candidate == action) {
            return Err(format!(
                "accessibility action is not available for element {element_id}: {action}"
            ));
        }
        unsafe {
            let element = resolve_observed_element(target, snapshot.element_path(element_id)?)?;
            let action_name = CFString::new(action);
            let result = AXUIElementPerformAction(
                element.0 as AXUIElementRef,
                action_name.as_concrete_TypeRef(),
            );
            if result == AX_SUCCESS {
                Ok(())
            } else {
                Err(format!(
                    "macOS Accessibility action {action} failed for element {element_id}: {result}"
                ))
            }
        }
    }

    pub fn set_value(
        target: &ApplicationTarget,
        snapshot: &AccessibilitySnapshot,
        element_id: u32,
        text: &str,
    ) -> Result<(), String> {
        let node = snapshot
            .nodes
            .iter()
            .find(|node| node.element_id == element_id)
            .ok_or_else(|| {
                format!("accessibility element is not in the current observation: {element_id}")
            })?;
        if !node.value_settable {
            return Err(format!(
                "accessibility element value is not settable: {element_id}"
            ));
        }
        unsafe {
            let element = resolve_observed_element(target, snapshot.element_path(element_id)?)?;
            let attribute = CFString::new("AXValue");
            let value = CFString::new(text);
            let result = AXUIElementSetAttributeValue(
                element.0 as AXUIElementRef,
                attribute.as_concrete_TypeRef(),
                value.as_concrete_TypeRef().cast(),
            );
            if result == AX_SUCCESS {
                Ok(())
            } else {
                Err(format!(
                    "macOS Accessibility could not set element {element_id} value: {result}"
                ))
            }
        }
    }

    unsafe fn visit(
        element: AXUIElementRef,
        parent_index: Option<usize>,
        path: Vec<usize>,
        depth: usize,
        surface: WindowBounds,
        nodes: &mut Vec<RawAccessibilityNode>,
        truncated: &mut bool,
    ) {
        if depth > MAX_ACCESSIBILITY_DEPTH || nodes.len() >= MAX_ACCESSIBILITY_RAW_NODES {
            *truncated = true;
            return;
        }
        let raw_index = nodes.len();
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
        nodes.push(RawAccessibilityNode {
            parent_index,
            path: path.clone(),
            node: AccessibilityNode {
                element_id: 0,
                parent_id: None,
                role: limited_text(role),
                subrole: limited_text(subrole),
                name: limited_text(name),
                value: limited_text(value),
                identifier: limited_text(string_attribute(element, "AXIdentifier")),
                placeholder: limited_text(string_attribute(element, "AXPlaceholderValue")),
                enabled: bool_attribute(element, "AXEnabled").unwrap_or(true),
                focused: bool_attribute(element, "AXFocused").unwrap_or(false),
                focusable: attribute_settable(element, "AXFocused"),
                selected: bool_attribute(element, "AXSelected").unwrap_or(false),
                expanded: bool_attribute(element, "AXExpanded"),
                actions: action_names(element),
                value_settable: attribute_settable(element, "AXValue"),
                bounds: rect.and_then(|rect| {
                    normalized_bounds(
                        rect.origin.x,
                        rect.origin.y,
                        rect.size.width,
                        rect.size.height,
                        surface,
                    )
                }),
            },
        });
        let Some(children) = copy_attribute(element, "AXChildren") else {
            return;
        };
        if CFGetTypeID(children.0) != CFArrayGetTypeID() {
            return;
        }
        let count = CFArrayGetCount(children.0.cast());
        for index in 0..count {
            if nodes.len() >= MAX_ACCESSIBILITY_RAW_NODES {
                *truncated = true;
                break;
            }
            let child = CFArrayGetValueAtIndex(children.0.cast(), index) as AXUIElementRef;
            if !child.is_null() {
                let mut child_path = path.clone();
                child_path.push(index as usize);
                visit(
                    child,
                    Some(raw_index),
                    child_path,
                    depth + 1,
                    surface,
                    nodes,
                    truncated,
                );
            }
        }
    }

    unsafe fn resolve_observed_element(
        target: &ApplicationTarget,
        path: &[usize],
    ) -> Result<OwnedValue, String> {
        let application = OwnedValue(AXUIElementCreateApplication(
            target
                .process_id
                .try_into()
                .map_err(|_| "application process id is outside the macOS AX range")?,
        ) as CFTypeRef);
        if application.0.is_null() {
            return Err("target application is unavailable from macOS Accessibility".into());
        }
        let mut current = target_window(application.0 as AXUIElementRef, target)
            .ok_or_else(|| "target window is unavailable from macOS Accessibility".to_string())?;
        for child_index in path {
            let children = copy_attribute(current.0 as AXUIElementRef, "AXChildren")
                .ok_or_else(|| "accessibility element path is no longer available".to_string())?;
            if CFGetTypeID(children.0) != CFArrayGetTypeID() {
                return Err("accessibility element path no longer resolves to children".into());
            }
            let count = CFArrayGetCount(children.0.cast());
            if *child_index >= count as usize {
                return Err("accessibility element path changed after observation".into());
            }
            let child =
                CFArrayGetValueAtIndex(children.0.cast(), *child_index as isize) as AXUIElementRef;
            current = retain_element(child)
                .ok_or_else(|| "accessibility element disappeared after observation".to_string())?;
        }
        Ok(current)
    }

    unsafe fn target_window(
        application: AXUIElementRef,
        target: &ApplicationTarget,
    ) -> Option<OwnedValue> {
        let windows = copy_attribute(application, "AXWindows")?;
        if CFGetTypeID(windows.0) != CFArrayGetTypeID() {
            return copy_attribute(application, "AXFocusedWindow");
        }
        let mut best: Option<(f64, f64, bool, AXUIElementRef)> = None;
        let count = CFArrayGetCount(windows.0.cast());
        for index in 0..count {
            let candidate = CFArrayGetValueAtIndex(windows.0.cast(), index) as AXUIElementRef;
            if candidate.is_null() {
                continue;
            }
            let title_matches = !target.window_title.is_empty()
                && string_attribute(candidate, "AXTitle") == target.window_title;
            let (overlap, geometry_delta) = element_rect(candidate)
                .map(|rect| {
                    (
                        overlap_area(rect, target.bounds),
                        geometry_delta(rect, target.bounds),
                    )
                })
                .unwrap_or((0.0, f64::INFINITY));
            let is_better = best
                .map(|(best_overlap, best_delta, best_title_matches, _)| {
                    overlap > best_overlap
                        || (overlap == best_overlap && geometry_delta < best_delta)
                        || (overlap == best_overlap
                            && geometry_delta == best_delta
                            && title_matches
                            && !best_title_matches)
                })
                .unwrap_or(true);
            if is_better {
                best = Some((overlap, geometry_delta, title_matches, candidate));
            }
        }
        best.filter(|(overlap, _, title_matches, _)| *overlap > 0.0 || *title_matches)
            .and_then(|(_, _, _, element)| retain_element(element))
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

    fn geometry_delta(rect: CGRect, bounds: WindowBounds) -> f64 {
        (rect.origin.x - f64::from(bounds.x)).abs()
            + (rect.origin.y - f64::from(bounds.y)).abs()
            + (rect.size.width - f64::from(bounds.width)).abs()
            + (rect.size.height - f64::from(bounds.height)).abs()
    }

    unsafe fn copy_attribute(element: AXUIElementRef, name: &str) -> Option<OwnedValue> {
        let attribute = CFString::new(name);
        let mut value: CFTypeRef = ptr::null();
        (AXUIElementCopyAttributeValue(element, attribute.as_concrete_TypeRef(), &mut value)
            == AX_SUCCESS
            && !value.is_null())
        .then_some(OwnedValue(value))
    }

    unsafe fn action_names(element: AXUIElementRef) -> Vec<String> {
        let mut values: CFArrayRef = ptr::null();
        if AXUIElementCopyActionNames(element, &mut values) != AX_SUCCESS || values.is_null() {
            return Vec::new();
        }
        let values = OwnedValue(values.cast());
        if CFGetTypeID(values.0) != CFArrayGetTypeID() {
            return Vec::new();
        }
        let count = CFArrayGetCount(values.0.cast());
        (0..count)
            .filter_map(|index| {
                let value = CFArrayGetValueAtIndex(values.0.cast(), index);
                if value.is_null() || CFGetTypeID(value) != CFStringGetTypeID() {
                    return None;
                }
                let text = CFString::wrap_under_get_rule(value as CFStringRef).to_string();
                (!text.is_empty()).then_some(limited_text(text))
            })
            .collect()
    }

    unsafe fn attribute_settable(element: AXUIElementRef, name: &str) -> bool {
        let attribute = CFString::new(name);
        let mut settable = false;
        AXUIElementIsAttributeSettable(element, attribute.as_concrete_TypeRef(), &mut settable)
            == AX_SUCCESS
            && settable
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
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTreeWalker,
    };

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
            let mut raw_nodes = Vec::new();
            let mut traversal_truncated = false;
            visit(
                &walker,
                &root,
                None,
                Vec::new(),
                0,
                target.bounds,
                &mut raw_nodes,
                &mut traversal_truncated,
            );
            let (nodes, element_paths, projection_truncated) = compact_nodes(raw_nodes);
            let window_title = root
                .CurrentName()
                .map(|value| value.to_string())
                .unwrap_or_default();
            Ok(AccessibilitySnapshot::captured(
                target,
                window_title,
                nodes,
                element_paths,
                !traversal_truncated && !projection_truncated,
            ))
        }
    }

    pub fn perform_action(
        _: &ApplicationTarget,
        _: &AccessibilitySnapshot,
        _: u32,
        _: &str,
    ) -> Result<(), String> {
        Err("accessibility actions are not implemented on Windows".into())
    }

    pub fn set_value(
        _: &ApplicationTarget,
        _: &AccessibilitySnapshot,
        _: u32,
        _: &str,
    ) -> Result<(), String> {
        Err("accessibility value setting is not implemented on Windows".into())
    }

    unsafe fn visit(
        walker: &IUIAutomationTreeWalker,
        element: &IUIAutomationElement,
        parent_index: Option<usize>,
        path: Vec<usize>,
        depth: usize,
        surface: WindowBounds,
        nodes: &mut Vec<RawAccessibilityNode>,
        truncated: &mut bool,
    ) {
        if depth > MAX_ACCESSIBILITY_DEPTH || nodes.len() >= MAX_ACCESSIBILITY_RAW_NODES {
            *truncated = true;
            return;
        }
        let raw_index = nodes.len();
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
        nodes.push(RawAccessibilityNode {
            parent_index,
            path: path.clone(),
            node: AccessibilityNode {
                element_id: 0,
                parent_id: None,
                role: limited_text(role),
                subrole: String::new(),
                name: limited_text(name),
                value: limited_text(value),
                identifier: element
                    .CurrentAutomationId()
                    .map(|value| limited_text(value.to_string()))
                    .unwrap_or_default(),
                placeholder: String::new(),
                enabled: element
                    .CurrentIsEnabled()
                    .map(|value| value.as_bool())
                    .unwrap_or(true),
                focused: element
                    .CurrentHasKeyboardFocus()
                    .map(|value| value.as_bool())
                    .unwrap_or(false),
                focusable: element
                    .CurrentIsKeyboardFocusable()
                    .map(|value| value.as_bool())
                    .unwrap_or(false),
                selected: false,
                expanded: None,
                actions: Vec::new(),
                value_settable: false,
                bounds,
            },
        });
        let mut child = walker.GetFirstChildElement(element).ok();
        let mut child_index = 0usize;
        while let Some(current) = child {
            let mut child_path = path.clone();
            child_path.push(child_index);
            visit(
                walker,
                &current,
                Some(raw_index),
                child_path,
                depth + 1,
                surface,
                nodes,
                truncated,
            );
            if nodes.len() >= MAX_ACCESSIBILITY_RAW_NODES {
                *truncated = true;
                break;
            }
            child = walker.GetNextSiblingElement(&current).ok();
            child_index += 1;
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use super::*;

    pub fn capture(_: &ApplicationTarget) -> Result<AccessibilitySnapshot, String> {
        Err("accessibility tree is not supported on this platform".into())
    }

    pub fn perform_action(
        _: &ApplicationTarget,
        _: &AccessibilitySnapshot,
        _: u32,
        _: &str,
    ) -> Result<(), String> {
        Err("accessibility actions are not supported on this platform".into())
    }

    pub fn set_value(
        _: &ApplicationTarget,
        _: &AccessibilitySnapshot,
        _: u32,
        _: &str,
    ) -> Result<(), String> {
        Err("accessibility value setting is not supported on this platform".into())
    }
}
