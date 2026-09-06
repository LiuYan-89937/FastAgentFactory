use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use serde::Deserialize;
use serde_json::json;
use std::io::{BufRead, BufReader, Cursor, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tauri::AppHandle;
use uuid::Uuid;
use xcap::image::codecs::jpeg::JpegEncoder;
use xcap::image::imageops::FilterType;
use xcap::image::{DynamicImage, RgbaImage};

use crate::computer_accessibility::{
    capture_accessibility_tree, focus_accessibility_window, AccessibilitySnapshot,
};
use crate::computer_applications::{
    activate_target, list_applications, resolve_application, resolve_target_window,
    ApplicationTarget, WindowBounds,
};

const CAPTURE_INTERVAL: Duration = Duration::from_millis(50);
const OBSERVE_TIMEOUT: Duration = Duration::from_millis(900);
const MAIN_THREAD_TIMEOUT: Duration = Duration::from_secs(10);
const JPEG_QUALITY: u8 = 76;
const MAX_MODEL_FRAME_EDGE: u32 = 1536;
const STABLE_CHANGE_THRESHOLD: f32 = 0.012;
const REQUIRED_STABLE_FRAMES: u8 = 2;
const POST_ACTION_SETTLE_GRACE: Duration = Duration::from_millis(120);
const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(25);
const MAX_ACTIONS_PER_BATCH: usize = 16;

#[derive(Clone, Debug)]
pub struct ComputerHostEndpoint {
    pub address: String,
    pub token: String,
}

pub struct ComputerHost {
    endpoint: ComputerHostEndpoint,
    shutdown: Arc<AtomicBool>,
    server_thread: Option<JoinHandle<()>>,
    capture_thread: Option<JoinHandle<()>>,
}

impl ComputerHost {
    pub fn start(app_handle: AppHandle) -> Result<Self, Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let endpoint = ComputerHostEndpoint {
            address: listener.local_addr()?.to_string(),
            token: Uuid::new_v4().simple().to_string(),
        };
        let shutdown = Arc::new(AtomicBool::new(false));
        let session_state = Arc::new(Mutex::new(SessionState::default()));
        let frame_state = Arc::new((Mutex::new(CaptureState::default()), Condvar::new()));
        let main_thread = MainThreadExecutor::new(app_handle);

        let capture_thread = {
            let shutdown = Arc::clone(&shutdown);
            let session_state = Arc::clone(&session_state);
            let frame_state = Arc::clone(&frame_state);
            thread::Builder::new()
                .name("combo-computer-capture".into())
                .spawn(move || capture_loop(shutdown, session_state, frame_state))?
        };
        let server_thread = {
            let shutdown = Arc::clone(&shutdown);
            let session_state = Arc::clone(&session_state);
            let frame_state = Arc::clone(&frame_state);
            let token = endpoint.token.clone();
            thread::Builder::new()
                .name("combo-computer-host".into())
                .spawn(move || {
                    server_loop(
                        listener,
                        token,
                        shutdown,
                        session_state,
                        frame_state,
                        main_thread,
                    )
                })?
        };
        Ok(Self {
            endpoint,
            shutdown,
            server_thread: Some(server_thread),
            capture_thread: Some(capture_thread),
        })
    }

    pub fn endpoint(&self) -> ComputerHostEndpoint {
        self.endpoint.clone()
    }

    pub fn shutdown(&mut self) {
        if self.shutdown.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(thread) = self.server_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.capture_thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for ComputerHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Clone)]
struct MainThreadExecutor {
    app_handle: AppHandle,
}

impl MainThreadExecutor {
    fn new(app_handle: AppHandle) -> Self {
        Self { app_handle }
    }

    fn run<T, F>(&self, operation: &'static str, task: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, String> + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(1);
        self.app_handle
            .run_on_main_thread(move || {
                let _ = sender.send(task());
            })
            .map_err(|error| {
                format!("could not schedule {operation} on the main thread: {error}")
            })?;
        receiver
            .recv_timeout(MAIN_THREAD_TIMEOUT)
            .map_err(|_| format!("main-thread operation timed out: {operation}"))?
    }
}

#[derive(Clone)]
struct SessionLease {
    connection_id: Uuid,
    session_id: Uuid,
    cancelled: Arc<AtomicBool>,
}

#[derive(Default)]
struct SessionState {
    owner: Option<SessionLease>,
    target: Option<ApplicationTarget>,
}

impl SessionState {
    fn acquire(&mut self, connection_id: Uuid) -> Result<SessionLease, String> {
        match self.owner.as_ref() {
            Some(owner) if owner.connection_id != connection_id => {
                Err("another Computer Use session is already active".into())
            }
            Some(owner) => Ok(owner.clone()),
            None => {
                let owner = SessionLease {
                    connection_id,
                    session_id: Uuid::new_v4(),
                    cancelled: Arc::new(AtomicBool::new(false)),
                };
                self.owner = Some(owner.clone());
                self.target = None;
                Ok(owner)
            }
        }
    }

    fn require_owner(&self, connection_id: Uuid, session_id: Uuid) -> Result<SessionLease, String> {
        let owner = self
            .owner
            .as_ref()
            .filter(|owner| owner.connection_id == connection_id && owner.session_id == session_id)
            .ok_or_else(|| "Computer Use session is no longer active".to_string())?;
        ensure_session_active(&owner.cancelled)?;
        Ok(owner.clone())
    }

    fn release(&mut self, connection_id: Uuid, session_id: Option<Uuid>) -> bool {
        let Some(owner) = self.owner.as_ref() else {
            return false;
        };
        if owner.connection_id != connection_id
            || session_id.is_some_and(|value| value != owner.session_id)
        {
            return false;
        }
        owner.cancelled.store(true, Ordering::SeqCst);
        self.owner = None;
        self.target = None;
        true
    }

    fn cancel(&mut self, session_id: Uuid) -> bool {
        let Some(owner) = self.owner.as_ref() else {
            return false;
        };
        if owner.session_id != session_id {
            return false;
        }
        owner.cancelled.store(true, Ordering::SeqCst);
        self.owner = None;
        self.target = None;
        true
    }
}

#[derive(Clone)]
struct WindowFrame {
    frame_id: u64,
    target: ApplicationTarget,
    image_width: u32,
    image_height: u32,
    jpeg: Arc<Vec<u8>>,
    stable_streak: u8,
    change_score: f32,
}

#[derive(Default)]
struct CaptureState {
    latest: Option<WindowFrame>,
    last_error: Option<String>,
    signature: Vec<u8>,
    next_frame_id: u64,
    last_action_at: Option<Instant>,
}

#[derive(Debug, Deserialize)]
struct HostRequest {
    token: String,
    op: String,
    #[serde(default)]
    application_id: Option<String>,
    #[serde(default)]
    session_id: Option<Uuid>,
    #[serde(default)]
    after_frame_id: Option<u64>,
    #[serde(default)]
    settle: bool,
    #[serde(default)]
    actions: Vec<ComputerAction>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ComputerAction {
    ClickElement {
        element_id: u32,
        #[serde(default = "default_button")]
        button: String,
    },
    SetValue {
        element_id: u32,
        text: String,
    },
    Click {
        x: f64,
        y: f64,
        #[serde(default = "default_button")]
        button: String,
    },
    DoubleClick {
        x: f64,
        y: f64,
        #[serde(default = "default_button")]
        button: String,
    },
    Drag {
        from_x: f64,
        from_y: f64,
        to_x: f64,
        to_y: f64,
        #[serde(default = "default_drag_duration_ms")]
        duration_ms: u64,
        #[serde(default = "default_button")]
        button: String,
    },
    Scroll {
        #[serde(default)]
        horizontal: i32,
        vertical: i32,
    },
    Type {
        text: String,
    },
    Key {
        key: String,
    },
    Hotkey {
        keys: Vec<String>,
    },
    Wait {
        milliseconds: u64,
    },
}

fn default_button() -> String {
    "left".into()
}
fn default_drag_duration_ms() -> u64 {
    160
}

fn server_loop(
    listener: TcpListener,
    token: String,
    shutdown: Arc<AtomicBool>,
    session_state: Arc<Mutex<SessionState>>,
    frame_state: Arc<(Mutex<CaptureState>, Condvar)>,
    main_thread: MainThreadExecutor,
) {
    while !shutdown.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let token = token.clone();
                let shutdown = Arc::clone(&shutdown);
                let session_state = Arc::clone(&session_state);
                let frame_state = Arc::clone(&frame_state);
                let main_thread = main_thread.clone();
                let _ = thread::Builder::new()
                    .name("combo-computer-client".into())
                    .spawn(move || {
                        if let Err(error) = handle_connection(
                            stream,
                            &token,
                            shutdown,
                            session_state,
                            frame_state,
                            main_thread,
                        ) {
                            eprintln!("Computer host connection failed: {error}");
                        }
                    });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => break,
        }
    }
}

fn handle_connection(
    mut stream: TcpStream,
    token: &str,
    shutdown: Arc<AtomicBool>,
    session_state: Arc<Mutex<SessionState>>,
    frame_state: Arc<(Mutex<CaptureState>, Condvar)>,
    main_thread: MainThreadExecutor,
) -> Result<(), Box<dyn std::error::Error>> {
    stream.set_nonblocking(false)?;
    stream.set_nodelay(true)?;
    let reader_stream = stream.try_clone()?;
    let mut reader = BufReader::new(reader_stream);
    let connection_id = Uuid::new_v4();
    let mut observed: Option<ObservedWindow> = None;
    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let request: HostRequest = match serde_json::from_str(line.trim()) {
            Ok(request) => request,
            Err(error) => {
                write_error(&mut stream, &format!("invalid request: {error}"))?;
                continue;
            }
        };
        if request.token != token {
            write_error(&mut stream, "unauthorized")?;
            continue;
        }
        if let Err(error) = dispatch_request(
            &mut stream,
            request,
            connection_id,
            &session_state,
            &frame_state,
            &main_thread,
            &mut observed,
        ) {
            write_error(&mut stream, &error)?;
        }
    }
    if release_session(&session_state, connection_id, None) {
        reset_capture_session(&frame_state);
    }
    Ok(())
}

fn dispatch_request(
    stream: &mut TcpStream,
    request: HostRequest,
    connection_id: Uuid,
    session_state: &Arc<Mutex<SessionState>>,
    frame_state: &Arc<(Mutex<CaptureState>, Condvar)>,
    main_thread: &MainThreadExecutor,
    observed: &mut Option<ObservedWindow>,
) -> Result<(), String> {
    match request.op.as_str() {
        "start" => {
            if !crate::computer_permissions::computer_permissions().ready() {
                return Err("computer-use permissions are required: grant Accessibility and Screen Recording in Combo before starting a conversation".into());
            }
            let (lease, acquired) = {
                let mut session = session_state
                    .lock()
                    .map_err(|_| "computer session state is unavailable")?;
                let was_unowned = session.owner.is_none();
                let lease = session.acquire(connection_id)?;
                (lease, was_unowned)
            };
            if acquired {
                reset_capture_session(frame_state);
            }
            write_json_line(
                stream,
                &json!({ "ok": true, "session_id": lease.session_id }),
            )
            .map_err(|error| error.to_string())
        }
        "list_applications" => {
            require_owner(session_state, connection_id, required_session_id(&request)?)?;
            let applications = list_applications()?;
            write_json_line(stream, &json!({ "ok": true, "applications": applications }))
                .map_err(|error| error.to_string())
        }
        "attach_application" => {
            let session_id = required_session_id(&request)?;
            require_owner(session_state, connection_id, session_id)?;
            let application_id = request
                .application_id
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "attach_application requires application_id".to_string())?;
            let target = resolve_application(&application_id)?;
            let activation_target = target.clone();
            main_thread.run("application activation", move || {
                activate_target(&activation_target)?;
                focus_accessibility_window(&activation_target)
            })?;
            {
                let mut session = session_state
                    .lock()
                    .map_err(|_| "computer session state is unavailable")?;
                session.require_owner(connection_id, session_id)?;
                session.target = Some(target.clone());
            }
            *observed = None;
            reset_capture_session(frame_state);
            write_json_line(stream, &json!({ "ok": true, "target": target }))
                .map_err(|error| error.to_string())
        }
        "observe" => {
            let (_, lease) =
                require_target(session_state, connection_id, required_session_id(&request)?)?;
            let (frame, accessibility) = consistent_observation(
                frame_state,
                request.after_frame_id,
                request.settle,
                &lease.cancelled,
            )?;
            *observed = Some(ObservedWindow {
                target: frame.target.clone(),
                accessibility: accessibility.clone(),
            });
            write_json_line(
                stream,
                &json!({
                    "ok": true,
                    "frame_id": frame.frame_id,
                    "width": frame.image_width,
                    "height": frame.image_height,
                    "mime_type": "image/jpeg",
                    "content_length": frame.jpeg.len(),
                    "stable": frame.stable_streak >= REQUIRED_STABLE_FRAMES,
                    "change_score": frame.change_score,
                    "target": frame.target,
                    "accessibility": accessibility,
                }),
            )
            .map_err(|error| error.to_string())?;
            stream
                .write_all(&frame.jpeg)
                .map_err(|error| error.to_string())?;
            stream.flush().map_err(|error| error.to_string())
        }
        "act" => {
            let session_id = required_session_id(&request)?;
            let (target, lease) = require_target(session_state, connection_id, session_id)?;
            let current = observed
                .as_ref()
                .ok_or_else(|| "act requires a current targeted observation".to_string())?;
            if current.target.application_id != target.application_id
                || current.target.window_id != target.window_id
            {
                return Err(
                    "target window changed after the current observation; observe again".into(),
                );
            }
            execute_actions(
                main_thread,
                &request.actions,
                &current.target,
                &current.accessibility,
                &lease.cancelled,
            )?;
            ensure_session_active(&lease.cancelled)?;
            if let Ok(mut state) = frame_state.0.lock() {
                state.last_action_at = Some(Instant::now());
                if let Some(latest) = state.latest.as_mut() {
                    latest.stable_streak = 0;
                }
            }
            *observed = None;
            write_ok(stream)
        }
        "stop" => {
            if release_session(
                session_state,
                connection_id,
                Some(required_session_id(&request)?),
            ) {
                reset_capture_session(frame_state);
            }
            *observed = None;
            write_ok(stream)
        }
        "cancel_session" => {
            let cancelled = session_state
                .lock()
                .map_err(|_| "computer session state is unavailable")?
                .cancel(required_session_id(&request)?);
            if cancelled {
                reset_capture_session(frame_state);
            }
            write_json_line(stream, &json!({ "ok": true, "cancelled": cancelled }))
                .map_err(|error| error.to_string())
        }
        _ => Err("unsupported operation".into()),
    }
}

fn require_owner(
    session_state: &Arc<Mutex<SessionState>>,
    connection_id: Uuid,
    session_id: Uuid,
) -> Result<SessionLease, String> {
    session_state
        .lock()
        .map_err(|_| "computer session state is unavailable".to_string())?
        .require_owner(connection_id, session_id)
}

fn require_target(
    session_state: &Arc<Mutex<SessionState>>,
    connection_id: Uuid,
    session_id: Uuid,
) -> Result<(ApplicationTarget, SessionLease), String> {
    let session = session_state
        .lock()
        .map_err(|_| "computer session state is unavailable".to_string())?;
    let lease = session.require_owner(connection_id, session_id)?;
    let target = session
        .target
        .clone()
        .ok_or_else(|| "Computer Use has not attached an application".to_string())?;
    Ok((target, lease))
}

fn release_session(
    session_state: &Arc<Mutex<SessionState>>,
    connection_id: Uuid,
    session_id: Option<Uuid>,
) -> bool {
    session_state
        .lock()
        .map(|mut session| session.release(connection_id, session_id))
        .unwrap_or(false)
}

fn required_session_id(request: &HostRequest) -> Result<Uuid, String> {
    request
        .session_id
        .ok_or_else(|| "computer host request requires session_id".into())
}

fn ensure_session_active(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::SeqCst) {
        Err("Computer Use session was cancelled".into())
    } else {
        Ok(())
    }
}

fn reset_capture_session(frame_state: &Arc<(Mutex<CaptureState>, Condvar)>) {
    if let Ok(mut state) = frame_state.0.lock() {
        state.latest = None;
        state.signature.clear();
        state.last_error = None;
        state.last_action_at = None;
        frame_state.1.notify_all();
    }
}

fn write_json_line<T: serde::Serialize>(
    stream: &mut TcpStream,
    payload: &T,
) -> Result<(), Box<dyn std::error::Error>> {
    serde_json::to_writer(&mut *stream, payload)?;
    stream.write_all(b"\n")?;
    Ok(())
}

fn write_ok(stream: &mut TcpStream) -> Result<(), String> {
    write_json_line(stream, &json!({ "ok": true })).map_err(|error| error.to_string())
}

fn write_error(stream: &mut TcpStream, error: &str) -> Result<(), Box<dyn std::error::Error>> {
    write_json_line(stream, &json!({ "ok": false, "error": error }))
}

fn capture_loop(
    shutdown: Arc<AtomicBool>,
    session_state: Arc<Mutex<SessionState>>,
    frame_state: Arc<(Mutex<CaptureState>, Condvar)>,
) {
    while !shutdown.load(Ordering::Relaxed) {
        let capture_target = session_state.lock().ok().and_then(|session| {
            let owner = session.owner.as_ref()?;
            let target = session.target.clone()?;
            (!owner.cancelled.load(Ordering::SeqCst)).then_some((
                owner.session_id,
                Arc::clone(&owner.cancelled),
                target,
            ))
        });
        let Some((session_id, cancelled, target)) = capture_target else {
            thread::sleep(Duration::from_millis(80));
            continue;
        };
        let started = Instant::now();
        match capture_target_window(&target) {
            Ok((resolved, image)) => {
                if !cancelled.load(Ordering::SeqCst)
                    && update_resolved_target(&session_state, session_id, &target, &resolved)
                {
                    update_frame(&frame_state, resolved, image);
                }
            }
            Err(error) => set_capture_error(&frame_state, error),
        }
        let elapsed = started.elapsed();
        if elapsed < CAPTURE_INTERVAL {
            thread::sleep(CAPTURE_INTERVAL - elapsed);
        }
    }
}

fn capture_target_window(
    target: &ApplicationTarget,
) -> Result<(ApplicationTarget, RgbaImage), String> {
    let (resolved, window) = resolve_target_window(target)?;
    let image = window.capture_image().map_err(|error| error.to_string())?;
    Ok((resolved, image))
}

fn update_resolved_target(
    session_state: &Arc<Mutex<SessionState>>,
    session_id: Uuid,
    previous: &ApplicationTarget,
    resolved: &ApplicationTarget,
) -> bool {
    if let Ok(mut session) = session_state.lock() {
        let owner_is_current = session
            .owner
            .as_ref()
            .map(|owner| owner.session_id == session_id && !owner.cancelled.load(Ordering::SeqCst))
            .unwrap_or(false);
        if owner_is_current
            && session
                .target
                .as_ref()
                .map(|target| {
                    target.application_id == previous.application_id
                        && target.window_id == previous.window_id
                })
                .unwrap_or(false)
        {
            session.target = Some(resolved.clone());
            return true;
        }
    }
    false
}

fn update_frame(
    frame_state: &Arc<(Mutex<CaptureState>, Condvar)>,
    target: ApplicationTarget,
    image: RgbaImage,
) {
    let signature = visual_signature(&image);
    let model_image = model_frame(&image);
    let jpeg = match encode_jpeg(model_image.clone()) {
        Ok(value) => value,
        Err(error) => {
            set_capture_error(frame_state, error.to_string());
            return;
        }
    };
    let (lock, ready) = &**frame_state;
    if let Ok(mut state) = lock.lock() {
        let same_geometry = state
            .latest
            .as_ref()
            .map(|frame| {
                frame.target.window_id == target.window_id && frame.target.bounds == target.bounds
            })
            .unwrap_or(false);
        let change_score = if same_geometry {
            signature_change_score(&state.signature, &signature)
        } else {
            1.0
        };
        let stable_streak = state
            .latest
            .as_ref()
            .filter(|_| same_geometry)
            .map(|frame| {
                if change_score <= STABLE_CHANGE_THRESHOLD {
                    frame.stable_streak.saturating_add(1)
                } else {
                    0
                }
            })
            .unwrap_or(0);
        state.next_frame_id = state.next_frame_id.saturating_add(1);
        state.signature = signature;
        state.last_error = None;
        state.latest = Some(WindowFrame {
            frame_id: state.next_frame_id,
            target,
            image_width: model_image.width(),
            image_height: model_image.height(),
            jpeg: Arc::new(jpeg),
            stable_streak,
            change_score,
        });
        ready.notify_all();
    }
}

fn consistent_observation(
    frame_state: &Arc<(Mutex<CaptureState>, Condvar)>,
    after_frame_id: Option<u64>,
    settle: bool,
    cancelled: &AtomicBool,
) -> Result<(WindowFrame, AccessibilitySnapshot), String> {
    let deadline = Instant::now() + OBSERVE_TIMEOUT;
    let mut frame = wait_for_frame(frame_state, after_frame_id, settle, cancelled)?;
    loop {
        ensure_session_active(cancelled)?;
        let accessibility = capture_accessibility_tree(&frame.target);
        let latest = latest_frame(frame_state)?;
        if latest.target.window_id == frame.target.window_id
            && latest.target.bounds == frame.target.bounds
        {
            return Ok((frame, accessibility));
        }
        if Instant::now() >= deadline {
            return Err("target window kept moving while creating an observation".into());
        }
        frame = latest;
    }
}

fn latest_frame(frame_state: &Arc<(Mutex<CaptureState>, Condvar)>) -> Result<WindowFrame, String> {
    frame_state
        .0
        .lock()
        .map_err(|_| "capture state is unavailable".to_string())?
        .latest
        .clone()
        .ok_or_else(|| "target window frame is unavailable".into())
}

fn set_capture_error(frame_state: &Arc<(Mutex<CaptureState>, Condvar)>, error: String) {
    if let Ok(mut state) = frame_state.0.lock() {
        state.last_error = Some(error);
        frame_state.1.notify_all();
    }
}

fn wait_for_frame(
    frame_state: &Arc<(Mutex<CaptureState>, Condvar)>,
    after_frame_id: Option<u64>,
    settle: bool,
    cancelled: &AtomicBool,
) -> Result<WindowFrame, String> {
    let deadline = Instant::now() + OBSERVE_TIMEOUT;
    let (lock, ready) = &**frame_state;
    let mut state = lock
        .lock()
        .map_err(|_| "capture state poisoned".to_string())?;
    loop {
        ensure_session_active(cancelled)?;
        if let Some(frame) = state.latest.as_ref() {
            let is_new = after_frame_id.map(|id| frame.frame_id > id).unwrap_or(true);
            let grace_elapsed = state
                .last_action_at
                .map(|at| at.elapsed() >= POST_ACTION_SETTLE_GRACE)
                .unwrap_or(true);
            let is_settled =
                !settle || (grace_elapsed && frame.stable_streak >= REQUIRED_STABLE_FRAMES);
            if is_new && is_settled {
                return Ok(frame.clone());
            }
        }
        if Instant::now() >= deadline {
            if let Some(frame) = state.latest.as_ref() {
                let is_new = after_frame_id.map(|id| frame.frame_id > id).unwrap_or(true);
                if is_new {
                    return Ok(frame.clone());
                }
            }
            return Err(state
                .last_error
                .clone()
                .unwrap_or_else(|| "target window capture timed out".into()));
        }
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .min(CANCELLATION_POLL_INTERVAL);
        let (next, _) = ready
            .wait_timeout(state, remaining)
            .map_err(|_| "capture state poisoned".to_string())?;
        state = next;
    }
}

#[derive(Clone)]
struct ObservedWindow {
    target: ApplicationTarget,
    accessibility: AccessibilitySnapshot,
}

fn execute_actions(
    main_thread: &MainThreadExecutor,
    actions: &[ComputerAction],
    target: &ApplicationTarget,
    accessibility: &AccessibilitySnapshot,
    cancelled: &Arc<AtomicBool>,
) -> Result<(), String> {
    if actions.len() > MAX_ACTIONS_PER_BATCH {
        return Err(format!(
            "computer action batch exceeds {MAX_ACTIONS_PER_BATCH} actions"
        ));
    }
    for action in actions {
        ensure_session_active(cancelled)?;
        if let ComputerAction::Wait { milliseconds } = action {
            interruptible_wait(Duration::from_millis((*milliseconds).min(5_000)), cancelled)?;
            continue;
        }
        let action = action.clone();
        let observed_target = target.clone();
        let accessibility = accessibility.clone();
        let cancelled = Arc::clone(cancelled);
        main_thread.run("computer input", move || {
            ensure_session_active(&cancelled)?;
            let (current_target, _) = resolve_target_window(&observed_target)?;
            validate_observation_geometry(&observed_target, &current_target)?;
            activate_target(&current_target)?;
            focus_accessibility_window(&current_target)?;
            ensure_session_active(&cancelled)?;
            execute_input_action(&action, current_target.bounds, &accessibility)
        })?;
    }
    Ok(())
}

fn interruptible_wait(duration: Duration, cancelled: &AtomicBool) -> Result<(), String> {
    let deadline = Instant::now() + duration;
    loop {
        ensure_session_active(cancelled)?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(());
        }
        thread::sleep(remaining.min(CANCELLATION_POLL_INTERVAL));
    }
}

fn validate_observation_geometry(
    observed: &ApplicationTarget,
    current: &ApplicationTarget,
) -> Result<(), String> {
    if observed.application_id != current.application_id
        || observed.process_id != current.process_id
        || observed.window_id != current.window_id
    {
        return Err("target window changed after observation; observe again".into());
    }
    if observed.bounds.width != current.bounds.width
        || observed.bounds.height != current.bounds.height
    {
        return Err("target window was resized after observation; observe again".into());
    }
    Ok(())
}

fn execute_input_action(
    action: &ComputerAction,
    bounds: WindowBounds,
    accessibility: &AccessibilitySnapshot,
) -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings {
        open_prompt_to_get_permissions: false,
        ..Settings::default()
    })
    .map_err(|error| error.to_string())?;
    match action {
        ComputerAction::ClickElement { element_id, button } => {
            let (x, y) = accessibility.element_target(*element_id, bounds)?;
            ensure_point_in_window(bounds, x, y)?;
            enigo
                .move_mouse(x, y, Coordinate::Abs)
                .map_err(input_error)?;
            enigo
                .button(parse_button(button)?, Direction::Click)
                .map_err(input_error)?;
        }
        ComputerAction::SetValue { element_id, text } => {
            let (x, y) = accessibility.element_target(*element_id, bounds)?;
            ensure_point_in_window(bounds, x, y)?;
            enigo
                .move_mouse(x, y, Coordinate::Abs)
                .map_err(input_error)?;
            enigo
                .button(Button::Left, Direction::Click)
                .map_err(input_error)?;
            select_all(&mut enigo)?;
            enigo.text(text).map_err(input_error)?;
        }
        ComputerAction::Click { x, y, button } => {
            move_to(&mut enigo, bounds, *x, *y)?;
            enigo
                .button(parse_button(button)?, Direction::Click)
                .map_err(input_error)?;
        }
        ComputerAction::DoubleClick { x, y, button } => {
            move_to(&mut enigo, bounds, *x, *y)?;
            let button = parse_button(button)?;
            enigo
                .button(button, Direction::Click)
                .map_err(input_error)?;
            thread::sleep(Duration::from_millis(70));
            enigo
                .button(button, Direction::Click)
                .map_err(input_error)?;
        }
        ComputerAction::Drag {
            from_x,
            from_y,
            to_x,
            to_y,
            duration_ms,
            button,
        } => {
            move_to(&mut enigo, bounds, *from_x, *from_y)?;
            let button = parse_button(button)?;
            enigo
                .button(button, Direction::Press)
                .map_err(input_error)?;
            let (start_x, start_y) = normalized_point(bounds, *from_x, *from_y)?;
            let (end_x, end_y) = normalized_point(bounds, *to_x, *to_y)?;
            let steps = ((*duration_ms / 16).clamp(2, 60)) as i32;
            for step in 1..=steps {
                let x = start_x + (end_x - start_x) * step / steps;
                let y = start_y + (end_y - start_y) * step / steps;
                enigo
                    .move_mouse(x, y, Coordinate::Abs)
                    .map_err(input_error)?;
                thread::sleep(Duration::from_millis((*duration_ms / steps as u64).max(1)));
            }
            enigo
                .button(button, Direction::Release)
                .map_err(input_error)?;
        }
        ComputerAction::Scroll {
            horizontal,
            vertical,
        } => {
            move_to(&mut enigo, bounds, 0.5, 0.5)?;
            if *horizontal != 0 {
                enigo
                    .scroll(*horizontal, Axis::Horizontal)
                    .map_err(input_error)?;
            }
            if *vertical != 0 {
                enigo
                    .scroll(*vertical, Axis::Vertical)
                    .map_err(input_error)?;
            }
        }
        ComputerAction::Type { text } => enigo.text(text).map_err(input_error)?,
        ComputerAction::Key { key } => enigo
            .key(parse_key(key)?, Direction::Click)
            .map_err(input_error)?,
        ComputerAction::Hotkey { keys } => execute_hotkey(&mut enigo, keys)?,
        ComputerAction::Wait { .. } => unreachable!("wait actions execute outside the UI thread"),
    }
    Ok(())
}

fn input_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn ensure_point_in_window(bounds: WindowBounds, x: i32, y: i32) -> Result<(), String> {
    bounds
        .contains(x, y)
        .then_some(())
        .ok_or_else(|| "computer action target is outside the attached window".into())
}

fn select_all(enigo: &mut Enigo) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let modifier = Key::Meta;
    #[cfg(not(target_os = "macos"))]
    let modifier = Key::Control;
    enigo.key(modifier, Direction::Press).map_err(input_error)?;
    enigo
        .key(Key::Unicode('a'), Direction::Click)
        .map_err(input_error)?;
    enigo
        .key(modifier, Direction::Release)
        .map_err(input_error)?;
    Ok(())
}

fn move_to(enigo: &mut Enigo, bounds: WindowBounds, x: f64, y: f64) -> Result<(), String> {
    let (x, y) = normalized_point(bounds, x, y)?;
    enigo.move_mouse(x, y, Coordinate::Abs).map_err(input_error)
}

fn normalized_point(bounds: WindowBounds, x: f64, y: f64) -> Result<(i32, i32), String> {
    if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
        return Err("computer coordinates must be normalized to 0..1".into());
    }
    let px = f64::from(bounds.x) + x * f64::from(bounds.width.saturating_sub(1));
    let py = f64::from(bounds.y) + y * f64::from(bounds.height.saturating_sub(1));
    let point = (px.round() as i32, py.round() as i32);
    ensure_point_in_window(bounds, point.0, point.1)?;
    Ok(point)
}

fn parse_button(value: &str) -> Result<Button, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "left" => Ok(Button::Left),
        "right" => Ok(Button::Right),
        "middle" => Ok(Button::Middle),
        _ => Err(format!("unsupported mouse button: {value}")),
    }
}

fn execute_hotkey(enigo: &mut Enigo, keys: &[String]) -> Result<(), String> {
    if keys.is_empty() || keys.len() > 5 {
        return Err("hotkey requires between 1 and 5 keys".into());
    }
    let parsed = keys
        .iter()
        .map(|key| parse_key(key))
        .collect::<Result<Vec<_>, _>>()?;
    for key in &parsed {
        enigo.key(*key, Direction::Press).map_err(input_error)?;
    }
    for key in parsed.iter().rev() {
        enigo.key(*key, Direction::Release).map_err(input_error)?;
    }
    Ok(())
}

fn parse_key(value: &str) -> Result<Key, String> {
    let normalized = value.trim().to_ascii_lowercase();
    let key = match normalized.as_str() {
        "ctrl" | "control" => Key::Control,
        "shift" => Key::Shift,
        "alt" | "option" => Key::Alt,
        "meta" | "cmd" | "command" | "win" | "windows" => Key::Meta,
        "enter" | "return" => Key::Return,
        "tab" => Key::Tab,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "escape" | "esc" => Key::Escape,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "left" => Key::LeftArrow,
        "right" => Key::RightArrow,
        "up" => Key::UpArrow,
        "down" => Key::DownArrow,
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        _ => {
            let mut chars = value.chars();
            match (chars.next(), chars.next()) {
                (Some(character), None) => Key::Unicode(character),
                _ => return Err(format!("unsupported key: {value}")),
            }
        }
    };
    Ok(key)
}

fn model_frame(image: &RgbaImage) -> RgbaImage {
    let width = image.width();
    let height = image.height();
    let longest = width.max(height);
    if longest <= MAX_MODEL_FRAME_EDGE {
        return image.clone();
    }
    let scale = f64::from(MAX_MODEL_FRAME_EDGE) / f64::from(longest);
    let target_width = ((f64::from(width) * scale).round() as u32).max(1);
    let target_height = ((f64::from(height) * scale).round() as u32).max(1);
    xcap::image::imageops::resize(image, target_width, target_height, FilterType::Triangle)
}

fn encode_jpeg(image: RgbaImage) -> Result<Vec<u8>, xcap::image::ImageError> {
    let rgb = DynamicImage::ImageRgba8(image).into_rgb8();
    let mut output = Cursor::new(Vec::new());
    JpegEncoder::new_with_quality(&mut output, JPEG_QUALITY).encode_image(&rgb)?;
    Ok(output.into_inner())
}

fn visual_signature(image: &RgbaImage) -> Vec<u8> {
    let width = image.width().max(1);
    let height = image.height().max(1);
    let grid = 24u32;
    let mut signature = Vec::with_capacity((grid * grid) as usize);
    for gy in 0..grid {
        for gx in 0..grid {
            let x = ((gx * width) / grid).min(width - 1);
            let y = ((gy * height) / grid).min(height - 1);
            let pixel = image.get_pixel(x, y).0;
            let luminance =
                ((u16::from(pixel[0]) * 54 + u16::from(pixel[1]) * 183 + u16::from(pixel[2]) * 19)
                    >> 8) as u8;
            signature.push(luminance);
        }
    }
    signature
}

fn signature_change_score(previous: &[u8], current: &[u8]) -> f32 {
    if previous.len() != current.len() || current.is_empty() {
        return 1.0;
    }
    let total: u32 = previous
        .iter()
        .zip(current)
        .map(|(left, right)| u32::from(left.abs_diff(*right)))
        .sum();
    total as f32 / (current.len() as f32 * 255.0)
}
