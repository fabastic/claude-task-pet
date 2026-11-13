// Claude Task Pet: a transparent, frameless window that only plays an animation
// and shows what Claude is doing when you hover over it.
//
//   claude-task-pet.exe            run the pet
//   claude-task-pet.exe hook       (used by Claude Code hooks) forward stdin JSON to the pet
//   claude-task-pet.exe install    copy the exe to %LOCALAPPDATA%\ClaudeTaskPet and add the hooks
//   claude-task-pet.exe uninstall  remove the hooks
#![windows_subsystem = "windows"]

use include_dir::{include_dir, Dir};
use muda::{CheckMenuItem, ContextMenu, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use serde_json::{json, Value};
use std::borrow::Cow;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{env, fs, thread};
use tao::dpi::{LogicalSize, PhysicalPosition};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::windows::{WindowBuilderExtWindows, WindowExtWindows};
use tao::window::WindowBuilder;
use wry::http::{header::CONTENT_TYPE, Request, Response};
use wry::WebViewBuilder;

static WEB: Dir = include_dir!("$CARGO_MANIFEST_DIR/web");
const PORT: u16 = 23456;
const PACKS: [&str; 2] = ["clawd", "mascot"];
const SIZES: [u32; 5] = [96, 128, 160, 220, 300];
const LABEL_W: u32 = 120; // extra width so the hover label has room
const LABEL_H: u32 = 30;

enum UserEvent {
    Hook(Value),
    Timeout(u64, &'static str),
    Menu(String),
    Ipc(String),
}

// ---------- config ----------

struct Config { pack: String, size: u32, x: Option<i32>, y: Option<i32> }

fn config_path() -> PathBuf {
    PathBuf::from(env::var("APPDATA").unwrap_or_else(|_| ".".into())).join("claude-task-pet").join("pet.json")
}

fn load_config() -> Config {
    let v: Value = fs::read_to_string(config_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({}));
    let pack = v["pack"].as_str().filter(|p| PACKS.contains(p)).unwrap_or("clawd").to_string();
    Config {
        pack,
        size: v["size"].as_u64().map(|s| s as u32).filter(|s| (48..=600).contains(s)).unwrap_or(160),
        x: v["x"].as_i64().map(|n| n as i32),
        y: v["y"].as_i64().map(|n| n as i32),
    }
}

fn save_config(c: &Config) {
    let p = config_path();
    let _ = fs::create_dir_all(p.parent().unwrap());
    let _ = fs::write(p, json!({ "pack": c.pack, "size": c.size, "x": c.x, "y": c.y }).to_string());
}

// ---------- states & labels ----------

fn animation(pack: &str, state: &str) -> &'static str {
    let clawd = pack == "clawd";
    match state {
        "sleep" => if clawd { "clawd-sleeping" } else { "asleep" },
        "greet" => if clawd { "clawd-wake" } else { "greeting" },
        "thinking" => if clawd { "clawd-working-thinking" } else { "thinking" },
        "reading" => if clawd { "clawd-idle-reading" } else { "reading" },
        "searching" => if clawd { "clawd-working-debugger" } else { "searching-code" },
        "web" => if clawd { "clawd-working-wizard" } else { "searching-web" },
        "bash" => if clawd { "clawd-working-building" } else { "running-commands" },
        "writing" => if clawd { "clawd-working-typing" } else { "writing" },
        "agents" => if clawd { "clawd-working-juggling" } else { "directing-helpers" },
        "tool" => if clawd { "clawd-working-carrying" } else { "using-tool" },
        "waiting" => if clawd { "clawd-notification" } else { "waiting-for-you" },
        "done" => if clawd { "clawd-happy" } else { "done" },
        "compact" => if clawd { "clawd-working-sweeping" } else { "compacting" },
        "error" => if clawd { "clawd-error" } else { "error" },
        _ => if clawd { "clawd-idle-living" } else { "idle" },
    }
}

fn trunc(s: &str, n: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() > n { format!("{}…", s.chars().take(n).collect::<String>()) } else { s }
}

fn file_name(p: &str) -> String {
    Path::new(p).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string())
}

fn tool_state(tool: &str, input: &Value) -> (&'static str, String) {
    let s = |k: &str| input[k].as_str().unwrap_or("").to_string();
    match tool {
        "Read" | "NotebookRead" => ("reading", format!("Reading {}", file_name(&s("file_path")))),
        "Grep" => ("searching", format!("Searching for \u{201c}{}\u{201d}", trunc(&s("pattern"), 40))),
        "Glob" | "LS" => ("searching", format!("Finding files {}", trunc(&s("pattern"), 40))),
        "WebSearch" => ("web", format!("Searching the web: {}", trunc(&s("query"), 40))),
        "WebFetch" => ("web", format!("Reading {}", trunc(&s("url"), 50))),
        "Bash" | "PowerShell" => {
            let d = s("description");
            ("bash", if d.is_empty() { format!("Running: {}", trunc(&s("command"), 50)) } else { trunc(&d, 60) })
        }
        "Edit" | "MultiEdit" | "NotebookEdit" => ("writing", format!("Editing {}", file_name(&s("file_path")))),
        "Write" => ("writing", format!("Writing {}", file_name(&s("file_path")))),
        "Task" | "Agent" => ("agents", format!("Delegating: {}", trunc(&s("description"), 50))),
        "" => ("tool", "Using a tool".into()),
        t => ("tool", format!("Using {}", trunc(t, 40))),
    }
}

// ---------- hook client / installer ----------

fn run_hook() {
    let mut body = String::new();
    let _ = std::io::stdin().read_to_string(&mut body);
    let addr: SocketAddr = ([127, 0, 0, 1], PORT).into();
    if let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(300)) {
        let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
        let _ = write!(s, "POST /event HTTP/1.1\r\nHost: pet\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
        let _ = s.read(&mut [0u8; 64]);
    }
}

const EVENTS: [&str; 10] = ["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "PostToolUseFailure",
    "Notification", "PreCompact", "SubagentStop", "Stop", "SessionEnd"];

fn is_pet_hook(group: &Value) -> bool {
    group["hooks"].as_array().is_some_and(|hs| hs.iter().any(|h| {
        let c = h["command"].as_str().unwrap_or("");
        c.contains("claude-task-pet") || (c.contains("pet") && c.contains("hook.js"))
    }))
}

fn edit_hooks(command: Option<String>) -> std::io::Result<()> {
    let file = PathBuf::from(env::var("USERPROFILE").unwrap_or_default()).join(".claude").join("settings.json");
    let mut s: Value = match fs::read_to_string(&file) {
        Ok(t) => { fs::write(file.with_extension("json.pet-backup"), &t)?; serde_json::from_str(&t).unwrap_or(json!({})) }
        Err(_) => json!({}),
    };
    if !s["hooks"].is_object() { s["hooks"] = json!({}); }
    for ev in EVENTS {
        let mut groups: Vec<Value> = s["hooks"][ev].as_array().cloned().unwrap_or_default();
        groups.retain(|g| !is_pet_hook(g));
        if let Some(cmd) = &command {
            groups.push(json!({ "matcher": "", "hooks": [{ "type": "command", "command": cmd, "timeout": 3 }] }));
        }
        if groups.is_empty() { s["hooks"].as_object_mut().unwrap().remove(ev); } else { s["hooks"][ev] = Value::Array(groups); }
    }
    fs::create_dir_all(file.parent().unwrap())?;
    fs::write(&file, serde_json::to_string_pretty(&s)?)
}

fn run_install() -> std::io::Result<()> {
    let dir = PathBuf::from(env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into())).join("ClaudeTaskPet");
    fs::create_dir_all(&dir)?;
    let dest = dir.join("claude-task-pet.exe");
    let me = env::current_exe()?;
    if fs::canonicalize(&me).ok() != fs::canonicalize(&dest).ok() { fs::copy(&me, &dest)?; }
    let cmd = format!("\"{}\" hook", dest.to_string_lossy().replace('\\', "/"));
    edit_hooks(Some(cmd))
}

// ---------- the pet ----------

struct Pet {
    cfg: Config,
    state: &'static str,
    label: String,
    prompt: String,
    generation: Arc<AtomicU64>,
    proxy: EventLoopProxy<UserEvent>,
}

impl Pet {
    fn script(&self) -> String {
        let src = format!("assets/{}/{}.svg", self.cfg.pack, animation(&self.cfg.pack, self.state));
        format!("document.documentElement.style.setProperty('--s','{}px');play({},{})",
            self.cfg.size, json!(src), json!(self.label))
    }

    fn set(&mut self, state: &'static str, label: String) {
        self.state = state;
        self.label = label;
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        // Short states fall back to idle; idle dozes off after 5 minutes.
        let after = match state {
            "greet" | "done" | "error" => Some((5, "idle")),
            "idle" => Some((300, "sleep")),
            _ => None,
        };
        if let Some((secs, next)) = after {
            let proxy = self.proxy.clone();
            thread::spawn(move || { thread::sleep(Duration::from_secs(secs)); let _ = proxy.send_event(UserEvent::Timeout(generation, next)); });
        }
    }

    fn on_hook(&mut self, e: &Value) -> bool {
        let tool = e["tool_name"].as_str().unwrap_or("");
        let working = || if self.prompt.is_empty() { "Thinking…".to_string() } else { format!("Working on: {}", self.prompt) };
        let (state, label) = match e["hook_event_name"].as_str().unwrap_or("") {
            "SessionStart" => ("greet", "Hi! New Claude session".to_string()),
            "UserPromptSubmit" => {
                self.prompt = trunc(e["prompt"].as_str().unwrap_or(""), 50);
                ("thinking", format!("Thinking about: {}", self.prompt))
            }
            "PreToolUse" => tool_state(tool, &e["tool_input"]),
            "PostToolUse" | "SubagentStop" => ("thinking", working()),
            "PostToolUseFailure" => ("error", format!("{} failed", if tool.is_empty() { "A tool" } else { tool })),
            "Notification" => ("waiting", trunc(e["message"].as_str().unwrap_or("Waiting for you"), 70)),
            "PreCompact" => ("compact", "Compacting the conversation".into()),
            "Stop" => ("done", "Done! Your turn".into()),
            "SessionEnd" => ("idle", "Session ended".into()),
            _ => return false,
        };
        self.set(state, label);
        true
    }
}

fn idle_label(state: &str) -> String {
    if state == "sleep" { "Zzz… nothing to do".into() } else { "Idle, waiting for Claude".into() }
}

fn build_menu(cfg: &Config) -> Menu {
    let chars = Submenu::with_items("Character", true, &[]).unwrap();
    for p in PACKS { chars.append(&CheckMenuItem::with_id(format!("pack:{p}"), p, true, cfg.pack == p, None)).unwrap(); }
    let sizes = Submenu::with_items("Size", true, &[]).unwrap();
    for s in SIZES { sizes.append(&CheckMenuItem::with_id(format!("size:{s}"), format!("{s}px"), true, cfg.size == s, None)).unwrap(); }
    let menu = Menu::new();
    menu.append_items(&[&chars, &sizes, &PredefinedMenuItem::separator(), &MenuItem::with_id("quit", "Quit", true, None)]).unwrap();
    menu
}

fn serve(listener: TcpListener, proxy: EventLoopProxy<UserEvent>) {
    for stream in listener.incoming().flatten() {
        let mut s = stream;
        let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        let mut need: Option<usize> = None;
        loop {
            let n = match s.read(&mut chunk) { Ok(0) | Err(_) => break, Ok(n) => n };
            buf.extend_from_slice(&chunk[..n]);
            if need.is_none() {
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..i]).to_ascii_lowercase();
                    let len = head.lines().find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0))).unwrap_or(0);
                    need = Some(i + 4 + len);
                }
            }
            if need.is_some_and(|n| buf.len() >= n) || buf.len() > 4 << 20 { break; }
        }
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            if let Ok(v) = serde_json::from_slice::<Value>(&buf[i + 4..]) { let _ = proxy.send_event(UserEvent::Hook(v)); }
        }
    }
}

fn main() {
    match env::args().nth(1).as_deref() {
        Some("hook") => return run_hook(),
        Some("install") => { if let Err(e) = run_install() { eprintln!("{e}"); std::process::exit(1) } return; }
        Some("uninstall") => { if let Err(e) = edit_hooks(None) { eprintln!("{e}"); std::process::exit(1) } return; }
        _ => {}
    }

    // Only one pet: if the port is taken, another pet is already running.
    let Ok(listener) = TcpListener::bind(("127.0.0.1", PORT)) else { return };

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let cfg = load_config();

    let (w, h) = (cfg.size + LABEL_W, cfg.size + LABEL_H);
    let mut wb = WindowBuilder::new()
        .with_title("Claude Task Pet")
        .with_decorations(false).with_transparent(true).with_always_on_top(true).with_resizable(false)
        .with_skip_taskbar(true).with_undecorated_shadow(false)
        .with_inner_size(LogicalSize::new(w, h));
    if let (Some(x), Some(y)) = (cfg.x, cfg.y) {
        wb = wb.with_position(PhysicalPosition::new(x, y));
    } else if let Some(m) = event_loop.primary_monitor() {
        let (sz, sc) = (m.size(), m.scale_factor());
        let x = sz.width as i32 - ((w + 10) as f64 * sc) as i32;
        let y = sz.height as i32 - ((h + 60) as f64 * sc) as i32;
        wb = wb.with_position(PhysicalPosition::new(x, y));
    }
    let window = wb.build(&event_loop).expect("window");

    let ipc_proxy = Mutex::new(proxy.clone());
    let webview = WebViewBuilder::new()
        .with_transparent(true)
        .with_initialization_script("window.PET_BASE='http://pet.localhost/'")
        .with_custom_protocol("pet".into(), |_id, req: Request<Vec<u8>>| {
            let path = req.uri().path().trim_start_matches('/');
            match WEB.get_file(path) {
                Some(f) => {
                    let mime = if path.ends_with(".svg") { "image/svg+xml" } else { "text/html; charset=utf-8" };
                    Response::builder().header(CONTENT_TYPE, mime).body(Cow::Borrowed(f.contents())).unwrap()
                }
                None => Response::builder().status(404).body(Cow::Borrowed(&b""[..])).unwrap(),
            }
        })
        .with_ipc_handler(move |req: Request<String>| { let _ = ipc_proxy.lock().unwrap().send_event(UserEvent::Ipc(req.body().clone())); })
        .with_url("http://pet.localhost/index.html")
        .build(&window)
        .expect("webview (WebView2 runtime required)");

    let menu_proxy = Mutex::new(proxy.clone());
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| { let _ = menu_proxy.lock().unwrap().send_event(UserEvent::Menu(e.id.0)); }));

    let server_proxy = proxy.clone();
    thread::spawn(move || serve(listener, server_proxy));

    let mut pet = Pet { cfg, state: "idle", label: idle_label("idle"), prompt: String::new(), generation: Arc::new(AtomicU64::new(0)), proxy };
    pet.set("idle", idle_label("idle"));
    let mut _menu_alive: Option<Menu> = None;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        let mut refresh = false;
        match event {
            Event::WindowEvent { event: WindowEvent::Moved(p), .. } => { pet.cfg.x = Some(p.x); pet.cfg.y = Some(p.y); save_config(&pet.cfg); }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => *control_flow = ControlFlow::Exit,
            Event::UserEvent(UserEvent::Hook(v)) => refresh = pet.on_hook(&v),
            Event::UserEvent(UserEvent::Timeout(g, next)) if g == pet.generation.load(Ordering::SeqCst) => {
                pet.set(next, idle_label(next)); refresh = true;
            }
            Event::UserEvent(UserEvent::Ipc(m)) => match m.as_str() {
                "ready" => refresh = true,
                "drag" => { let _ = window.drag_window(); }
                "menu" => {
                    let menu = build_menu(&pet.cfg);
                    unsafe { menu.show_context_menu_for_hwnd(window.hwnd() as _, None); }
                    _menu_alive = Some(menu);
                }
                _ => {}
            },
            Event::UserEvent(UserEvent::Menu(id)) => {
                if id == "quit" { *control_flow = ControlFlow::Exit; return; }
                if let Some(p) = id.strip_prefix("pack:") { pet.cfg.pack = p.to_string(); }
                if let Some(s) = id.strip_prefix("size:").and_then(|s| s.parse().ok()) {
                    pet.cfg.size = s;
                    window.set_inner_size(LogicalSize::new(s + LABEL_W, s + LABEL_H));
                }
                save_config(&pet.cfg);
                refresh = true;
            }
            _ => {}
        }
        if refresh { let _ = webview.evaluate_script(&pet.script()); }
    });
}
