//! Native owner for the credential-bearing AI process. Only safe protocol
//! responses and display events enter the WebView.
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};
use tauri::{Emitter, Manager};

type Pending = Arc<Mutex<HashMap<u64, mpsc::Sender<Value>>>>;

#[derive(Default)]
pub struct AiState(Mutex<Option<Bridge>>);

impl AiState {
    pub fn shutdown(&self) {
        if let Ok(mut bridge) = self.0.lock() {
            *bridge = None;
        }
    }
}

struct Bridge {
    child: Child,
    stdin: ChildStdin,
    pending: Pending,
    next_id: u64,
}

impl Drop for Bridge {
    fn drop(&mut self) {
        // The sidecar, Codex/Claude, and MCP share this owned process group.
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        #[cfg(windows)]
        {
            let _ = Command::new("taskkill.exe")
                .args(["/PID", &self.child.id().to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn authorize_url(value: &str) -> bool {
    tauri::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
            && match url.host_str() {
                Some("auth.openai.com") => {
                    ["/api/accounts/authorize", "/oauth/authorize"].contains(&url.path())
                }
                Some("claude.ai") => ["/oauth/authorize", "/settings/usage"].contains(&url.path()),
                Some("chatgpt.com") => url.path() == "/settings/usage",
                Some("openrouter.ai") => url.path() == "/auth",
                Some("app.devin.ai") => url.path() == "/auth/cli/continue",
                Some("cursor.com") => url.path() == "/loginDeepControl",
                Some("app.kiro.dev") => url.path() == "/signin",
                Some("github.com") => url.path() == "/login/device",
                // Device issuers choose the verification path. Only these
                // fixed first-party HTTPS origins may open a browser.
                Some(
                    "auth.x.ai" | "accounts.x.ai" | "x.ai" | "auth.kimi.com" | "auth.kimi.ai"
                    | "kimi.com" | "kimi.ai" | "www.kimi.com" | "www.kimi.ai" | "auth.meta.com"
                    | "meta.com" | "www.meta.com" | "radius.pi.dev",
                ) => true,
                _ => false,
            }
    })
}

fn open_authorize(value: &str) {
    if !authorize_url(value) {
        return;
    }
    #[cfg(target_os = "linux")]
    let mut command = Command::new("xdg-open");
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("rundll32.exe");
        command.arg("url.dll,FileProtocolHandler");
        command
    };
    // No shell, no URL logging, no inherited stdout.
    let _ = command
        .arg(value)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

fn public_event(message: &Value) -> Option<&Value> {
    if message.get("method")? != "event" {
        return None;
    }
    let params = message.get("params")?;
    let event = params.get("type")?.as_str()?;
    let provider = params.get("provider")?.as_str()?;
    if ![
        "chatgpt",
        "claude",
        "github-copilot",
        "xai",
        "openrouter",
        "kimi-coding",
        "devin",
        "meta",
        "cursor",
        "radius",
        "kiro",
        "openai",
        "gemini",
        "deepseek",
        "mistral",
        "groq",
        "zai",
        "qwen",
        "minimax",
        "openrouter-key",
        "xai-key",
        "anthropic-key",
    ]
    .contains(&provider)
        || ![
            "auth",
            "device_code",
            "delta",
            "tool_call",
            "tool_result",
            "approval",
            "done",
            "error",
        ]
        .contains(&event)
    {
        return None;
    }
    Some(params)
}

fn sidecar_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ai/server.mjs");
    if cfg!(debug_assertions) && dev.is_file() {
        return Ok(dev);
    }
    let bundled = app
        .path()
        .resource_dir()
        .map_err(|_| "Cannot locate app resources")?
        .join("ai/server.mjs");
    if !bundled.is_file() {
        return Err("AI sidecar is not installed; install ai/ dependencies and rebuild.".into());
    }
    Ok(bundled)
}

impl Bridge {
    fn spawn(app: &tauri::AppHandle) -> Result<Self, String> {
        let script = sidecar_path(app)?;
        let config = app
            .path()
            .app_config_dir()
            .map_err(|_| "Cannot locate app config directory")?
            .join("ai");
        let mut command = Command::new("node");
        command
            .arg(&script)
            .current_dir(script.parent().ok_or("Cannot locate AI directory")?)
            .env("SIMREP_AI_CONFIG_DIR", config)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let mut child = command
            .spawn()
            .map_err(|_| "Cannot start AI bridge. Install Node.js 24+ and ai/ dependencies.")?;
        let stdin = child.stdin.take().ok_or("AI stdin unavailable")?;
        let stdout = child.stdout.take().ok_or("AI stdout unavailable")?;
        let pending: Pending = Arc::default();
        let readers = pending.clone();
        let handle = app.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if message.get("method") == Some(&json!("host.open_url")) {
                    if let Some(url) = message.pointer("/params/url").and_then(Value::as_str) {
                        open_authorize(url);
                    }
                } else if let Some(event) = public_event(&message) {
                    let _ = handle.emit("ai-event", event);
                } else if let Some(id) = message.get("id").and_then(Value::as_u64)
                    && let Ok(mut pending) = readers.lock()
                    && let Some(sender) = pending.remove(&id)
                {
                    let _ = sender.send(message);
                }
            }
            if let Ok(mut pending) = readers.lock() {
                // Dropping senders unblocks every in-flight request.
                pending.clear();
            }
        });
        Ok(Self {
            child,
            stdin,
            pending,
            next_id: 1,
        })
    }
}

fn valid_request(request: &Value) -> bool {
    request.get("jsonrpc") == Some(&json!("2.0"))
        && request
            .get("id")
            .is_some_and(|id| id.is_string() || id.is_i64() || id.is_u64())
        && request
            .get("method")
            .and_then(Value::as_str)
            .is_some_and(|method| {
                [
                    "auth.providers",
                    "auth.status",
                    "auth.accounts",
                    "auth.login",
                    "auth.logout",
                    "auth.openUsage",
                    "chat.start",
                    "chat.send",
                    "chat.cancel",
                    "approval.resolve",
                ]
                .contains(&method)
            })
}

#[tauri::command]
pub async fn ai_request(app: tauri::AppHandle, request: Value) -> Result<Value, String> {
    if !valid_request(&request) {
        return Err("Invalid AI protocol request".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let original_id = request["id"].clone();
        let (receiver, id, pending) = {
            let state = app.state::<AiState>();
            let mut guard = state.0.lock().map_err(|_| "AI state lock failed")?;
            if let Some(bridge) = guard.as_mut()
                && bridge
                    .child
                    .try_wait()
                    .map_err(|_| "AI process check failed")?
                    .is_some()
            {
                *guard = None;
            }
            if guard.is_none() {
                *guard = Some(Bridge::spawn(&app)?);
            }
            let bridge = guard.as_mut().ok_or("AI bridge unavailable")?;
            let id = bridge.next_id;
            bridge.next_id += 1;
            let (sender, receiver) = mpsc::channel();
            bridge
                .pending
                .lock()
                .map_err(|_| "AI request lock failed")?
                .insert(id, sender);
            let mut wire = request;
            wire["id"] = json!(id);
            if writeln!(bridge.stdin, "{wire}").is_err() {
                bridge
                    .pending
                    .lock()
                    .map_err(|_| "AI request lock failed")?
                    .remove(&id);
                return Err("AI process disconnected".into());
            }
            (receiver, id, bridge.pending.clone())
        };
        let result = receiver.recv_timeout(Duration::from_secs(120));
        pending
            .lock()
            .map_err(|_| "AI request lock failed")?
            .remove(&id);
        let mut response = result.map_err(|_| "AI request timed out or process disconnected")?;
        response["id"] = original_id;
        Ok(response)
    })
    .await
    .map_err(|_| "AI request worker failed")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_is_only_a_first_party_authorization_or_usage_endpoint() {
        assert!(authorize_url(
            "https://auth.openai.com/api/accounts/authorize?state=x"
        ));
        for value in [
            "https://claude.ai/oauth/authorize?state=x",
            "https://claude.ai/settings/usage",
            "https://chatgpt.com/settings/usage",
            "https://github.com/login/device",
            "https://openrouter.ai/auth",
            "https://app.devin.ai/auth/cli/continue",
            "https://cursor.com/loginDeepControl?uuid=x",
            "https://auth.x.ai/activate",
            "https://auth.kimi.ai/device",
            "https://auth.meta.com/device",
            "https://radius.pi.dev/device",
        ] {
            assert!(authorize_url(value), "{value}");
        }
        for value in [
            "http://auth.openai.com/api/accounts/authorize",
            "https://auth.openai.com.evil.test/api/accounts/authorize",
            "https://auth.openai.com:444/api/accounts/authorize",
            "https://x@auth.openai.com/api/accounts/authorize",
            "https://auth.openai.com/api/accounts/oauth/token",
            "https://claude.ai.evil.test/oauth/authorize",
            "https://github.com/login/device#fragment",
            "https://github.com/settings",
            "https://example.com/login/device",
            "file:///tmp/x",
        ] {
            assert!(!authorize_url(value), "{value}");
        }
    }

    #[test]
    fn credentials_and_host_urls_are_not_events() {
        assert!(
            public_event(&json!({"method":"host.open_url","params":{"url":"secret"}})).is_none()
        );
        assert!(
            public_event(
                &json!({"method":"event","params":{"type":"tokens","provider":"chatgpt"}})
            )
            .is_none()
        );
        assert!(public_event(&json!({"method":"event","params":{"type":"delta","provider":"chatgpt","text":"ok"}})).is_some());
        assert!(!valid_request(
            &json!({"jsonrpc":"2.0","id":1,"method":"host.open_url"})
        ));
        assert!(valid_request(
            &json!({"jsonrpc":"2.0","id":"a","method":"auth.status"})
        ));
        assert!(valid_request(
            &json!({"jsonrpc":"2.0","id":"a","method":"auth.providers"})
        ));
        assert!(valid_request(
            &json!({"jsonrpc":"2.0","id":"a","method":"auth.openUsage","params":{"provider":"claude"}})
        ));
        assert!(public_event(&json!({"method":"event","params":{"type":"device_code","provider":"github-copilot","userCode":"ABCD","verificationUri":"https://github.com/login/device","expiresIn":900}})).is_some());
    }
}
