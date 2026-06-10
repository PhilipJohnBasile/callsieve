//! Local indexing daemon: background refresh loop, on-disk state, and the
//! Unix-socket serving path that answers `agent-context` from the in-memory
//! index. Extracted from the monolithic CLI module.

use super::*;

#[derive(Debug, Serialize, serde::Deserialize)]
pub(super) struct DaemonState {
    pub(super) status: String,
    pub(super) root: String,
    pub(super) mode: String,
    pub(super) pid: u32,
    pub(super) lsp: bool,
    pub(super) interval_ms: u64,
    pub(super) started_at: u64,
    pub(super) last_indexed_at: u64,
    pub(super) last_change_at: u64,
    pub(super) index_generation: u64,
    pub(super) stale_files: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) last_error: Option<String>,
}

#[derive(Debug)]
pub(super) struct DaemonIndexSnapshot {
    pub(super) last_indexed_at: u64,
    pub(super) index_generation: u64,
    pub(super) stale_files: usize,
    pub(super) last_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct DaemonOutput {
    pub(super) command: &'static str,
    pub(super) state: DaemonState,
}

pub(super) fn daemon_state_is_usable(state: &DaemonState) -> bool {
    !matches!(
        state.status.as_str(),
        "missing" | "stopped" | "stop_requested" | "error"
    ) && state.last_error.is_none()
}

pub(super) fn daemon_index_snapshot(root: &Path) -> DaemonIndexSnapshot {
    let Some(index) = store::json_store::load_index(root).ok() else {
        return DaemonIndexSnapshot {
            last_indexed_at: 0,
            index_generation: 0,
            stale_files: 0,
            last_error: None,
        };
    };
    let stale_files = serde_json::to_value(query::index_status(root, Some(&index)))
        .ok()
        .and_then(|value| value.get("stale_files").and_then(serde_json::Value::as_u64))
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or_default();

    DaemonIndexSnapshot {
        last_indexed_at: index.metadata.indexed_at,
        index_generation: index.metadata.index_generation,
        stale_files,
        last_error: index.metadata.last_error,
    }
}

pub(super) fn run_daemon(
    root: &Path,
    lsp: bool,
    interval_ms: u64,
    foreground: bool,
    once: bool,
) -> Result<DaemonOutput> {
    let snapshot = daemon_index_snapshot(root);
    if !foreground && !once {
        clear_daemon_stop(root)?;
        let pid = if env::var_os("CALLSIEVE_TEST_BACKGROUND_NO_SPAWN").is_some() {
            0
        } else {
            let exe = env::current_exe().context("failed to resolve current executable")?;
            let mut command = ProcessCommand::new(exe);
            command
                .arg("daemon")
                .arg(root)
                .arg("--foreground")
                .arg("--interval-ms")
                .arg(interval_ms.to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if lsp {
                command.arg("--lsp");
            }
            #[cfg(windows)]
            {
                const DETACHED_PROCESS: u32 = 0x0000_0008;
                const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
                command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
            }
            command
                .spawn()
                .context("failed to spawn callsieve daemon")?
                .id()
        };
        let state = DaemonState {
            status: "starting".to_string(),
            root: root_label(root),
            mode: "background".to_string(),
            pid,
            lsp,
            interval_ms,
            started_at: now_unix_seconds(),
            last_indexed_at: snapshot.last_indexed_at,
            last_change_at: snapshot.last_indexed_at,
            index_generation: snapshot.index_generation,
            stale_files: snapshot.stale_files,
            last_error: snapshot.last_error,
        };
        save_daemon_state(root, &state)?;
        return Ok(DaemonOutput {
            command: "daemon",
            state,
        });
    }

    clear_daemon_stop(root)?;
    let mut state = DaemonState {
        status: if once { "indexing_once" } else { "running" }.to_string(),
        root: root_label(root),
        mode: if once { "once" } else { "foreground" }.to_string(),
        pid: std::process::id(),
        lsp,
        interval_ms,
        started_at: now_unix_seconds(),
        last_indexed_at: snapshot.last_indexed_at,
        last_change_at: snapshot.last_indexed_at,
        index_generation: snapshot.index_generation,
        stale_files: snapshot.stale_files,
        last_error: snapshot.last_error,
    };
    save_daemon_state(root, &state)?;

    // Foreground daemons hold the parsed index in memory and answer
    // agent-context requests over a local socket; once-mode skips serving.
    #[cfg(unix)]
    let shared_index: std::sync::Arc<
        std::sync::RwLock<Option<std::sync::Arc<store::CodeIndex>>>,
    > = std::sync::Arc::new(std::sync::RwLock::new(None));
    #[cfg(unix)]
    let freshness = std::sync::Arc::new(DaemonFreshness::new(interval_ms));
    #[cfg(unix)]
    let socket = if once {
        None
    } else {
        spawn_daemon_socket_listener(
            root.to_path_buf(),
            std::sync::Arc::clone(&shared_index),
            std::sync::Arc::clone(&freshness),
        )
    };
    loop {
        // With an in-memory index a stat-level freshness check (cheap) can
        // skip the full rebuild that refresh_watch_index performs; without
        // one (non-unix or first tick) the rebuild also primes the cache.
        #[cfg(unix)]
        let already_fresh = shared_index
            .read()
            .ok()
            .and_then(|guard| guard.as_ref().cloned())
            .is_some_and(|index| query::index_status(root, Some(&index)).is_fresh());
        #[cfg(not(unix))]
        let already_fresh = false;

        if already_fresh {
            #[cfg(unix)]
            freshness.mark_verified();
            state.status = if once { "indexed_once" } else { "running" }.to_string();
            state.last_error = None;
            state.stale_files = 0;
            save_daemon_state(root, &state)?;
            if once || daemon_stop_path(root).is_file() {
                state.status = if once { "indexed_once" } else { "stopped" }.to_string();
                save_daemon_state(root, &state)?;
                break;
            }
            thread::sleep(Duration::from_millis(interval_ms));
            continue;
        }

        match refresh_watch_index(root, "daemon", &state.mode, lsp) {
            Ok(output) => {
                let status_value = serde_json::to_value(&output.status)?;
                state.status = if once { "indexed_once" } else { "running" }.to_string();
                state.last_indexed_at = now_unix_seconds();
                state.last_change_at = state.last_indexed_at;
                state.index_generation = status_value
                    .get("index_generation")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or_default();
                state.stale_files = status_value
                    .get("stale_files")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok())
                    .unwrap_or_default();
                state.last_error = None;
            }
            Err(error) => {
                state.status = "error".to_string();
                state.last_error = Some(error.to_string());
            }
        }
        save_daemon_state(root, &state)?;

        // Prime/refresh the in-memory copy after a successful rebuild.
        #[cfg(unix)]
        if socket.is_some()
            && state.last_error.is_none()
            && let Ok(index) = store::json_store::load_index(root)
            && let Ok(mut guard) = shared_index.write()
        {
            *guard = Some(std::sync::Arc::new(index));
            drop(guard);
            freshness.mark_verified();
        }
        if once || daemon_stop_path(root).is_file() {
            state.status = if once { "indexed_once" } else { "stopped" }.to_string();
            save_daemon_state(root, &state)?;
            break;
        }

        thread::sleep(Duration::from_millis(interval_ms));
    }

    #[cfg(unix)]
    if let Some(socket) = socket {
        let _ = fs::remove_file(socket);
    }

    Ok(DaemonOutput {
        command: "daemon",
        state,
    })
}

pub(super) fn daemon_state_path(root: &Path) -> PathBuf {
    callsieve_dir(root).join("daemon.json")
}

#[derive(Debug, Serialize, serde::Deserialize)]
pub(super) struct DaemonContextResponse {
    ok: bool,
    #[serde(default)]
    rendered: String,
    #[serde(default)]
    error: String,
}

#[cfg(unix)]
pub(super) fn daemon_socket_path(root: &Path) -> PathBuf {
    callsieve_dir(root).join("daemon.sock")
}

/// Serve one agent-context request from the daemon's in-memory index. The
/// daemon refreshes on its poll interval, but a stat-level freshness check
/// still guards each response so a client never gets knowably stale context;
/// on any "not servable" condition the client falls back to direct load.
#[cfg(unix)]
pub(super) fn serve_daemon_connection(
    stream: &mut std::os::unix::net::UnixStream,
    root: &Path,
    shared_index: &std::sync::RwLock<Option<std::sync::Arc<store::CodeIndex>>>,
    freshness: &DaemonFreshness,
) {
    use std::io::{Read, Write};

    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let mut raw = String::new();
    if stream.read_to_string(&mut raw).is_err() {
        return;
    }
    let response = match serde_json::from_str::<AgentContextRequest>(&raw) {
        Ok(request) => daemon_context_response(root, shared_index, freshness, &request),
        Err(error) => DaemonContextResponse {
            ok: false,
            rendered: String::new(),
            error: format!("bad request: {error}"),
        },
    };
    if let Ok(encoded) = serde_json::to_string(&response) {
        let _ = stream.write_all(encoded.as_bytes());
    }
}

/// The daemon poll loop verifies index freshness every `interval_ms`. While
/// that verification is recent, serving skips the per-request stat walk —
/// the staleness window is the same one the daemon already promises. A
/// zeroed timestamp (or a stalled loop) falls back to the full check.
#[cfg(unix)]
pub(super) struct DaemonFreshness {
    verified_at_ms: std::sync::atomic::AtomicU64,
    interval_ms: u64,
}

#[cfg(unix)]
impl DaemonFreshness {
    fn new(interval_ms: u64) -> Self {
        Self {
            verified_at_ms: std::sync::atomic::AtomicU64::new(0),
            interval_ms,
        }
    }

    fn mark_verified(&self) {
        self.verified_at_ms
            .store(now_epoch_ms(), std::sync::atomic::Ordering::Relaxed);
    }

    fn recently_verified(&self) -> bool {
        let verified = self
            .verified_at_ms
            .load(std::sync::atomic::Ordering::Relaxed);
        verified != 0
            && now_epoch_ms().saturating_sub(verified) <= self.interval_ms.saturating_mul(2)
    }
}

#[cfg(unix)]
pub(super) fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(unix)]
pub(super) fn daemon_context_response(
    root: &Path,
    shared_index: &std::sync::RwLock<Option<std::sync::Arc<store::CodeIndex>>>,
    freshness: &DaemonFreshness,
    request: &AgentContextRequest,
) -> DaemonContextResponse {
    let index = shared_index
        .read()
        .ok()
        .and_then(|guard| guard.as_ref().cloned());
    let Some(index) = index else {
        return DaemonContextResponse {
            ok: false,
            rendered: String::new(),
            error: "daemon index not loaded yet".to_string(),
        };
    };
    if !freshness.recently_verified() && !query::index_status(root, Some(&index)).is_fresh() {
        return DaemonContextResponse {
            ok: false,
            rendered: String::new(),
            error: "daemon index is stale".to_string(),
        };
    }
    match agent_context_output_for_index(
        root,
        &index,
        request,
        query::HybridOptions::embeddings(false),
        &[],
        0,
        false,
    )
    .and_then(|output| render_agent_context_output(&output, request))
    {
        Ok(rendered) => DaemonContextResponse {
            ok: true,
            rendered,
            error: String::new(),
        },
        Err(error) => DaemonContextResponse {
            ok: false,
            rendered: String::new(),
            error: error.to_string(),
        },
    }
}

/// Try a running daemon first; any failure (no socket, connect refused,
/// stale index, protocol error) silently returns None and the caller loads
/// directly. Non-unix targets always load directly.
pub(super) fn try_daemon_agent_context(
    root: &Path,
    request: &AgentContextRequest,
) -> Option<String> {
    #[cfg(unix)]
    {
        use std::io::{Read, Write};
        use std::net::Shutdown;

        let socket = daemon_socket_path(root);
        if !socket.exists() {
            return None;
        }
        let mut stream = std::os::unix::net::UnixStream::connect(&socket).ok()?;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
        stream
            .write_all(serde_json::to_string(request).ok()?.as_bytes())
            .ok()?;
        stream.shutdown(Shutdown::Write).ok()?;
        let mut raw = String::new();
        stream.read_to_string(&mut raw).ok()?;
        let response: DaemonContextResponse = serde_json::from_str(&raw).ok()?;
        response.ok.then_some(response.rendered)
    }
    #[cfg(not(unix))]
    {
        let _ = (root, request);
        None
    }
}

#[cfg(unix)]
pub(super) fn spawn_daemon_socket_listener(
    root: PathBuf,
    shared_index: std::sync::Arc<std::sync::RwLock<Option<std::sync::Arc<store::CodeIndex>>>>,
    freshness: std::sync::Arc<DaemonFreshness>,
) -> Option<PathBuf> {
    let socket = daemon_socket_path(&root);
    let _ = fs::remove_file(&socket);
    let listener = match std::os::unix::net::UnixListener::bind(&socket) {
        Ok(listener) => listener,
        Err(_) => return None,
    };
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(mut stream) => {
                    serve_daemon_connection(&mut stream, &root, &shared_index, &freshness)
                }
                Err(_) => break,
            }
        }
    });
    Some(socket)
}

pub(super) fn daemon_stop_path(root: &Path) -> PathBuf {
    callsieve_dir(root).join("daemon.stop")
}

pub(super) fn save_daemon_state(root: &Path, state: &DaemonState) -> Result<()> {
    let dir = callsieve_dir(root);
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    fs::write(daemon_state_path(root), serde_json::to_vec_pretty(state)?)
        .with_context(|| format!("failed to write daemon state for {}", root.display()))
}

pub(super) fn load_daemon_state(root: &Path) -> Option<DaemonState> {
    fs::read(daemon_state_path(root))
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
}

pub(super) fn write_daemon_stop(root: &Path) -> Result<()> {
    let dir = callsieve_dir(root);
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    fs::write(daemon_stop_path(root), b"stop")
        .with_context(|| format!("failed to write daemon stop marker for {}", root.display()))
}

pub(super) fn clear_daemon_stop(root: &Path) -> Result<()> {
    let path = daemon_stop_path(root);
    if path.is_file() {
        fs::remove_file(&path).with_context(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
#[cfg(unix)]
mod daemon_socket_tests {
    use super::*;

    fn request(task: &str) -> AgentContextRequest {
        AgentContextRequest {
            task: task.to_string(),
            limit: 3,
            snippets_per_file: 0,
            why_debug: false,
            profile: "skim".to_string(),
            token_budget: query::DEFAULT_AGENT_CONTEXT_TOKEN_BUDGET,
            format: "json".to_string(),
            git_boost: false,
            pretty: false,
        }
    }

    #[test]
    fn daemon_socket_serves_output_identical_to_direct_path() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::write(
            root.join("session.ts"),
            "export function createSession() {}\n",
        )
        .unwrap();
        let index = indexer::build_index(&root).unwrap();
        store::json_store::save_index(&root, &index).unwrap();

        let shared: std::sync::Arc<std::sync::RwLock<Option<std::sync::Arc<store::CodeIndex>>>> =
            std::sync::Arc::new(std::sync::RwLock::new(Some(std::sync::Arc::new(
                index.clone(),
            ))));
        let socket = spawn_daemon_socket_listener(
            root.clone(),
            std::sync::Arc::clone(&shared),
            std::sync::Arc::new(DaemonFreshness::new(1000)),
        )
        .unwrap();
        assert!(socket.exists());

        let request = request("where is createSession handled");
        let via_daemon = try_daemon_agent_context(&root, &request)
            .expect("daemon should serve a fresh in-memory index");

        let direct = agent_context_output_for_index(
            &root,
            &index,
            &request,
            query::HybridOptions::embeddings(false),
            &[],
            0,
            false,
        )
        .and_then(|output| render_agent_context_output(&output, &request))
        .unwrap();

        assert_eq!(via_daemon, direct, "daemon and direct output must match");
        let _ = fs::remove_file(socket);
    }

    #[test]
    fn daemon_socket_refuses_stale_index_so_client_falls_back() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::write(
            root.join("session.ts"),
            "export function createSession() {}\n",
        )
        .unwrap();
        let index = indexer::build_index(&root).unwrap();
        store::json_store::save_index(&root, &index).unwrap();

        let shared: std::sync::Arc<std::sync::RwLock<Option<std::sync::Arc<store::CodeIndex>>>> =
            std::sync::Arc::new(std::sync::RwLock::new(Some(std::sync::Arc::new(index))));
        let socket = spawn_daemon_socket_listener(
            root.clone(),
            std::sync::Arc::clone(&shared),
            std::sync::Arc::new(DaemonFreshness::new(1000)),
        )
        .unwrap();

        // Mutate a source file: the served index is now stale and the daemon
        // must refuse so the client falls back to a direct load.
        fs::write(
            root.join("session.ts"),
            "export function createSession() { return 1; }\n",
        )
        .unwrap();

        assert!(try_daemon_agent_context(&root, &request("anything")).is_none());
        let _ = fs::remove_file(socket);
    }

    #[test]
    fn recently_verified_daemon_skips_the_per_request_stat_walk() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::write(
            root.join("session.ts"),
            "export function createSession() {}\n",
        )
        .unwrap();
        let index = indexer::build_index(&root).unwrap();
        store::json_store::save_index(&root, &index).unwrap();

        let shared: std::sync::RwLock<Option<std::sync::Arc<store::CodeIndex>>> =
            std::sync::RwLock::new(Some(std::sync::Arc::new(index)));

        // Within the poll loop's verification window, a mutation is served
        // until the next tick — the same staleness contract the daemon's
        // refresh interval already promises.
        let freshness = DaemonFreshness::new(60_000);
        freshness.mark_verified();
        fs::write(
            root.join("session.ts"),
            "export function createSession() { return 1; }\n",
        )
        .unwrap();
        let response = daemon_context_response(&root, &shared, &freshness, &request("anything"));
        assert!(response.ok, "{}", response.error);

        // A zeroed (never-verified) timestamp falls back to the full check
        // and refuses the stale index.
        let unverified = DaemonFreshness::new(60_000);
        let response = daemon_context_response(&root, &shared, &unverified, &request("anything"));
        assert!(!response.ok);
        assert!(response.error.contains("stale"), "{}", response.error);
    }

    #[test]
    fn try_daemon_returns_none_without_a_socket() {
        let temp = tempfile::tempdir().unwrap();
        assert!(try_daemon_agent_context(temp.path(), &request("anything")).is_none());
    }
}
