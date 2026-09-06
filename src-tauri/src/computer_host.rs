use crate::computer_accessibility::{
    capture_accessibility_tree, perform_accessibility_element_action,
    set_accessibility_element_value, AccessibilitySnapshot,
};
use crate::computer_applications::{
    application_descriptor, list_applications, resolve_application_target, resolve_target_window,
    ApplicationTarget,
};
use serde::Deserialize;
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tauri::AppHandle;
use uuid::Uuid;

const MAIN_THREAD_TIMEOUT: Duration = Duration::from_secs(10);
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
        let main_thread = MainThreadExecutor::new(app_handle);
        let server_thread = {
            let shutdown = Arc::clone(&shutdown);
            let session_state = Arc::clone(&session_state);
            let token = endpoint.token.clone();
            thread::Builder::new()
                .name("combo-computer-host".into())
                .spawn(move || server_loop(listener, token, shutdown, session_state, main_thread))?
        };
        Ok(Self {
            endpoint,
            shutdown,
            server_thread: Some(server_thread),
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

#[derive(Debug, Deserialize)]
struct HostRequest {
    token: String,
    op: String,
    #[serde(default)]
    application_id: Option<String>,
    #[serde(default)]
    session_id: Option<Uuid>,
    #[serde(default)]
    actions: Vec<ComputerAction>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ComputerAction {
    PerformAction { element_id: u32, action: String },
    SetValue { element_id: u32, text: String },
    Wait { milliseconds: u64 },
}

fn server_loop(
    listener: TcpListener,
    token: String,
    shutdown: Arc<AtomicBool>,
    session_state: Arc<Mutex<SessionState>>,
    main_thread: MainThreadExecutor,
) {
    while !shutdown.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let token = token.clone();
                let shutdown = Arc::clone(&shutdown);
                let session_state = Arc::clone(&session_state);
                let main_thread = main_thread.clone();
                let _ = thread::Builder::new()
                    .name("combo-computer-client".into())
                    .spawn(move || {
                        if let Err(error) =
                            handle_connection(stream, &token, shutdown, session_state, main_thread)
                        {
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
            &main_thread,
            &mut observed,
        ) {
            write_error(&mut stream, &error)?;
        }
    }
    release_session(&session_state, connection_id, None);
    Ok(())
}

fn dispatch_request(
    stream: &mut TcpStream,
    request: HostRequest,
    connection_id: Uuid,
    session_state: &Arc<Mutex<SessionState>>,
    main_thread: &MainThreadExecutor,
    observed: &mut Option<ObservedWindow>,
) -> Result<(), String> {
    match request.op.as_str() {
        "start" => {
            if !crate::computer_permissions::computer_permissions().ready() {
                return Err("computer-use permission is required: grant Accessibility to Combo before starting a conversation".into());
            }
            let lease = {
                let mut session = session_state
                    .lock()
                    .map_err(|_| "computer session state is unavailable")?;
                session.acquire(connection_id)?
            };
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
            let application = application_descriptor(&application_id)?;
            let target = resolve_application_target(&application)?;
            {
                let mut session = session_state
                    .lock()
                    .map_err(|_| "computer session state is unavailable")?;
                session.require_owner(connection_id, session_id)?;
                session.target = Some(target.clone());
            }
            *observed = None;
            write_json_line(stream, &json!({ "ok": true, "target": target }))
                .map_err(|error| error.to_string())
        }
        "observe" => {
            let (target, lease) =
                require_target(session_state, connection_id, required_session_id(&request)?)?;
            ensure_session_active(&lease.cancelled)?;
            let (target, _) = resolve_target_window(&target)?;
            let accessibility = capture_accessibility_tree(&target);
            ensure_session_active(&lease.cancelled)?;
            {
                let mut session = session_state
                    .lock()
                    .map_err(|_| "computer session state is unavailable")?;
                session.require_owner(connection_id, lease.session_id)?;
                session.target = Some(target.clone());
            }
            *observed = Some(ObservedWindow {
                target: target.clone(),
                accessibility: accessibility.clone(),
            });
            write_json_line(
                stream,
                &json!({
                    "ok": true,
                    "target": target,
                    "accessibility": accessibility,
                }),
            )
            .map_err(|error| error.to_string())
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
            *observed = None;
            write_ok(stream)
        }
        "stop" => {
            release_session(
                session_state,
                connection_id,
                Some(required_session_id(&request)?),
            );
            *observed = None;
            write_ok(stream)
        }
        "cancel_session" => {
            let cancelled = session_state
                .lock()
                .map_err(|_| "computer session state is unavailable")?
                .cancel(required_session_id(&request)?);
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
            ensure_session_active(&cancelled)?;
            execute_accessibility_action(&action, &current_target, &accessibility)
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

fn execute_accessibility_action(
    action: &ComputerAction,
    target: &ApplicationTarget,
    accessibility: &AccessibilitySnapshot,
) -> Result<(), String> {
    match action {
        ComputerAction::PerformAction { element_id, action } => {
            perform_accessibility_element_action(target, accessibility, *element_id, action)
        }
        ComputerAction::SetValue { element_id, text } => {
            set_accessibility_element_value(target, accessibility, *element_id, text)
        }
        ComputerAction::Wait { .. } => unreachable!("wait actions execute outside the UI thread"),
    }
}
