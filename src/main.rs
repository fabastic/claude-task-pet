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
const PACKS: [&str; 3] = ["clawd", "clawd-pet", "mascot"];
const SIZES: [u32; 5] = [96, 128, 160, 220, 300];
const LABEL_W: u32 = 120; // extra width so the hover label has room
const LABEL_H: u32 = 60; // room under the pet for the task + model lines

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

// state -> [clawd (clawd-on-desk), clawd-pet (abderrahimghazali), mascot (claude-desktop-pet)].
// Each entry is a comma-separated list; one is picked at random every time the state starts.
const STATES: &[(&str, [&str; 3])] = &[
    ("idle", ["clawd-idle-living,clawd-idle-look,clawd-idle-follow,clawd-headphones-groove",
              "clawd-idle-living,clawd-smile,clawd-music,clawd-meditating", "idle,idle-breathe,idle-look-around"]),
    ("bored", ["clawd-idle-yawn,clawd-idle-doze,clawd-idle-bubble,clawd-coffee-hand,clawd-idle-low-battery",
               "clawd-bored,clawd-yawning,clawd-coffee,clawd-gaming,clawd-ice-cream", "gag-coffee,gag-nap,gag-dance,gag-robot"]),
    ("sleep", ["clawd-sleeping,clawd-collapse-sleep", "clawd-sleeping", "asleep"]),
    ("greet", ["clawd-wake,clawd-mini-enter", "clawd-waving,clawd-smile", "greeting"]),
    ("bye", ["clawd-mini-peek", "clawd-going-away,clawd-waving", "walking"]),
    ("thinking", ["clawd-working-thinking,clawd-working-ultrathink", "clawd-working-thinking,clawd-idea", "thinking"]),
    ("reading", ["clawd-idle-reading", "clawd-reading,clawd-studying", "reading"]),
    ("searching", ["clawd-working-debugger", "clawd-detective,clawd-working-debugger", "searching-code"]),
    ("web", ["clawd-working-wizard", "clawd-telescope,clawd-working-beacon", "searching-web"]),
    ("bash", ["clawd-working-building", "clawd-working-tool-calling,clawd-coding", "running-commands"]),
    ("testing", ["clawd-working-debugger", "clawd-working-testing", "testing"]),
    ("building", ["clawd-working-building", "clawd-working-building,clawd-crafting", "running-commands"]),
    ("installing", ["clawd-working-carrying", "clawd-loading,clawd-charging", "gag-buffering"]),
    ("git", ["clawd-working-carrying", "clawd-working-merging", "committing"]),
    ("push", ["clawd-working-carrying", "clawd-working-pushing,clawd-shipping", "committing"]),
    ("deploy", ["clawd-working-building", "clawd-working-deploying,clawd-rocket", "gag-deploy"]),
    ("server", ["clawd-headphones-groove", "clawd-working-oncall,clawd-dj", "server-running"]),
    ("writing", ["clawd-working-typing,clawd-working-typing-boss", "clawd-working-typing,clawd-coding", "writing,typing"]),
    ("designing", ["clawd-designing", "clawd-painting,clawd-crafting", "writing"]),
    ("reviewing", ["clawd-working-debugger", "clawd-working-reviewing,clawd-skeptical", "reading"]),
    ("agents", ["clawd-working-juggling", "clawd-working-conducting,clawd-working-juggling,clawd-working-pairing", "directing-helpers"]),
    ("planning", ["clawd-working-thinking", "clawd-working-meeting,clawd-idea", "plan-progress"]),
    ("skill", ["clawd-working-wizard", "clawd-magic", "using-tool"]),
    ("tool", ["clawd-working-carrying", "clawd-working-tool-calling", "using-tool"]),
    ("waiting", ["clawd-notification,clawd-mini-alert", "clawd-notification,clawd-mail", "waiting-for-you"]),
    ("permission", ["clawd-mini-alert,clawd-notification", "clawd-security,clawd-hopeful", "waiting-for-you"]),
    ("question", ["clawd-notification", "clawd-hopeful,clawd-confused", "waiting-for-you"]),
    ("done", ["clawd-happy,clawd-react-double-jump,clawd-heart-eyes", "clawd-celebrating,clawd-happy,clawd-trophy,clawd-clapping,clawd-200", "done"]),
    ("subdone", ["clawd-happy", "clawd-clapping,clawd-201", "done"]),
    ("compact", ["clawd-working-sweeping", "clawd-working-sweeping", "compacting"]),
    ("context", ["clawd-dizzy", "clawd-working-context-full,clawd-working-overheated", "context-nearly-full"]),
    ("error", ["clawd-error,clawd-dizzy,clawd-react-annoyed", "clawd-error,clawd-facepalm,clawd-500,clawd-working-firefighting", "error"]),
    ("ratelimit", ["clawd-idle-low-battery", "clawd-429,clawd-battery-low", "gag-tokens"]),
];

fn pick(list: &str) -> String {
    let opts: Vec<&str> = list.split(',').collect();
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0) as usize;
    opts[n % opts.len()].to_string()
}

fn animation(pack: &str, state: &str) -> String {
    let i = PACKS.iter().position(|p| *p == pack).unwrap_or(0);
    let row = STATES.iter().find(|(s, _)| *s == state).unwrap_or(&STATES[0]);
    pick(row.1[i])
}

fn bash_state(cmd: &str) -> &'static str {
    let c = format!(" {} ", cmd.to_lowercase());
    let has = |w: &[&str]| w.iter().any(|x| c.contains(x));
    if has(&["git push", "gh pr create", "gh release"]) { "push" }
    else if has(&["git commit", "git merge", "git rebase", "git cherry-pick"]) { "git" }
    else if has(&["deploy", "vercel", "netlify", "wrangler", "kubectl apply"]) { "deploy" }
    else if has(&[" test", "jest", "vitest", "pytest", "playwright", "cypress", " npm t "]) { "testing" }
    else if has(&["install", "npm i ", "pnpm add", "yarn add", "cargo add", "winget ", "brew "]) { "installing" }
    else if has(&["build", "cargo b ", " tsc", " make", "webpack", "vite build", "compile"]) { "building" }
    else if has(&["dev ", "serve", " start", "uvicorn", "runserver"]) { "server" }
    else { "bash" }
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
        "Bash" | "PowerShell" => (bash_state(&s("command")), format!("Running: {}", trunc(&s("command"), 50))),
        "Edit" | "MultiEdit" | "NotebookEdit" | "Write" => {
            let f = file_name(&s("file_path"));
            let design = [".css", ".scss", ".html", ".svg", ".png", ".figma"].iter().any(|x| f.to_lowercase().ends_with(x));
            (if design { "designing" } else { "writing" }, format!("{} {}", if tool == "Write" { "Writing" } else { "Editing" }, f))
        }
        "Task" | "Agent" | "Workflow" => ("agents", format!("Delegating: {}", trunc(&s("description"), 50))),
        "TodoWrite" | "EnterPlanMode" | "ExitPlanMode" | "TaskCreate" | "TaskUpdate" => ("planning", "Planning the next steps".into()),
        "AskUserQuestion" => ("question", "Has a question for you".into()),
        "Skill" => ("skill", format!("Using skill {}", trunc(&s("skill"), 40))),
        t if t.ends_with("update_plan") => ("planning", "Updating the task list".into()),
        t if t.contains("code-review") || t == "ReportFindings" => ("reviewing", "Reviewing code".into()),
        t if t.starts_with("mcp__") => ("tool", format!("Using {}", trunc(t.rsplit("__").next().unwrap_or(t), 40).replace('_', " "))),
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

const EVENTS: [&str; 12] = ["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "PostToolUseFailure",
    "PermissionRequest", "Notification", "PreCompact", "SubagentStop", "Stop", "StopFailure", "SessionEnd"];

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
    anim: String,
    label: String,
    prompt: String,
    said: String,
    model: String,
    effort: String,
    mode: String,
    generation: Arc<AtomicU64>,
    proxy: EventLoopProxy<UserEvent>,
}

impl Pet {
    fn script(&self) -> String {
        let src = format!("assets/{}/{}.svg", self.cfg.pack, self.anim);
        let info: Vec<&str> = [self.model.as_str(), self.effort.as_str(), self.mode.as_str()]
            .into_iter().filter(|x| !x.is_empty()).collect();
        format!("document.documentElement.style.setProperty('--s','{}px');play({},{},{})",
            self.cfg.size, json!(src), json!(self.label), json!(info.join(" \u{00b7} ")))
    }

    fn set(&mut self, state: &'static str, label: String) {
        // Same state again (e.g. two edits in a row): keep the current animation running.
        if state != self.state || self.anim.is_empty() { self.anim = animation(&self.cfg.pack, state); }
        self.state = state;
        self.label = label;
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        // Short reactions fall back to idle; idle gets bored after 2 minutes, then falls asleep.
        let after = match state {
            "greet" | "done" | "subdone" | "error" | "bye" => Some((6, "idle")),
            "idle" => Some((120, "bored")),
            "bored" => Some((180, "sleep")),
            _ => None,
        };
        if let Some((secs, next)) = after {
            let proxy = self.proxy.clone();
            thread::spawn(move || { thread::sleep(Duration::from_secs(secs)); let _ = proxy.send_event(UserEvent::Timeout(generation, next)); });
        }
    }

    fn on_hook(&mut self, e: &Value) -> bool {
        let tool = e["tool_name"].as_str().unwrap_or("");
        if let Some(m) = e["permission_mode"].as_str() { self.mode = pretty_mode(m); }
        if let Some(x) = e["effort"].as_str().or(e["effort_level"].as_str()) { self.effort = capitalize(x); }
        if let Some(m) = e["model"].as_str().or(e["model"]["id"].as_str()) { self.model = pretty_model(m); }
        if let Some(t) = e["transcript_path"].as_str() {
            let (model, said) = read_transcript(t);
            if !model.is_empty() { self.model = pretty_model(&model); }
            if !said.is_empty() { self.said = said; }
        }
        let working = || if !self.said.is_empty() { trunc(&self.said, 90) }
            else if !self.prompt.is_empty() { format!("Working on: {}", self.prompt) }
            else { "Deciding what to do next".to_string() };
        let (state, label) = match e["hook_event_name"].as_str().unwrap_or("") {
            "SessionStart" => ("greet", "Hi! New Claude session".to_string()),
            "UserPromptSubmit" => {
                self.said.clear();
                self.prompt = trunc(e["prompt"].as_str().unwrap_or(""), 70);
                ("thinking", format!("Thinking about: {}", self.prompt))
            }
            "PreToolUse" => {
                // Same text the Claude app shows for the step, when the tool call carries one.
                let (state, fallback) = tool_state(tool, &e["tool_input"]);
                let desc = e["tool_input"]["description"].as_str().unwrap_or("").trim();
                (state, if desc.is_empty() { fallback } else { trunc(desc, 90) })
            }
            // Keep showing the step that just ran until the next one starts.
            "PostToolUse" => ("thinking", if self.label.is_empty() { working() } else { self.label.clone() }),
            "SubagentStop" => ("subdone", "A helper agent finished".into()),
            "PostToolUseFailure" => ("error", format!("{} failed", if tool.is_empty() { "A tool" } else { tool })),
            "PermissionRequest" => ("permission", format!("Needs your OK to use {}", if tool.is_empty() { "a tool" } else { tool })),
            "Notification" => {
                let m = e["message"].as_str().unwrap_or("Waiting for you");
                let l = m.to_lowercase();
                let state = if l.contains("rate limit") || l.contains("usage limit") { "ratelimit" }
                    else if l.contains("permission") { "permission" } else { "waiting" };
                (state, trunc(m, 70))
            }
            "StopFailure" => {
                let l = e.to_string().to_lowercase();
                if l.contains("rate") || l.contains("limit") || l.contains("429") { ("ratelimit", "Hit a usage limit".into()) }
                else { ("error", "The turn stopped with an error".into()) }
            }
            "PreCompact" if e["trigger"] == "auto" => ("context", "Context is full, compacting".into()),
            "PreCompact" => ("compact", "Compacting the conversation".into()),
            "Stop" => ("done", if self.said.is_empty() { "Done! Your turn".into() } else { format!("Done: {}", trunc(&self.said, 80)) }),
            "SessionEnd" => ("bye", "Session ended. Bye!".into()),
            _ => return false,
        };
        self.set(state, label);
        true
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

fn pretty_mode(m: &str) -> String {
    match m {
        "default" => "Ask".into(), "acceptEdits" => "Accept edits".into(), "plan" => "Plan".into(),
        "bypassPermissions" => "Bypass".into(), "dontAsk" => "Don't ask".into(), o => capitalize(o),
    }
}

// "claude-opus-5-5" -> "Opus 5.5"; "claude-sonnet-4-5-20250929" -> "Sonnet 4.5"
fn pretty_model(id: &str) -> String {
    let id = id.trim_start_matches("claude-");
    let parts: Vec<&str> = id.split(['-', '[']).collect();
    let name = parts.iter().find(|p| !p.is_empty() && p.chars().all(|c| c.is_alphabetic())).map(|p| capitalize(p)).unwrap_or_default();
    let nums: Vec<&str> = parts.iter().copied().filter(|p| !p.is_empty() && p.len() <= 2 && p.chars().all(|c| c.is_ascii_digit())).collect();
    if name.is_empty() { id.to_string() } else if nums.is_empty() { name } else { format!("{} {}", name, nums.join(".")) }
}

// Latest model id and the latest thing the agent said, from the tail of the session transcript.
fn read_transcript(path: &str) -> (String, String) {
    use std::io::{Seek, SeekFrom};
    let Ok(mut f) = fs::File::open(path) else { return Default::default() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(len.saturating_sub(512 * 1024)));
    let mut buf = Vec::new();
    let _ = f.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    let (mut model, mut said) = (String::new(), String::new());
    for line in text.lines().rev() {
        if !line.contains("\"assistant\"") { continue; }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if v["type"] != "assistant" || v["isSidechain"] == true { continue; }
        if model.is_empty() {
            if let Some(m) = v["message"]["model"].as_str() { if !m.starts_with('<') { model = m.into(); } }
        }
        if said.is_empty() {
            if let Some(blocks) = v["message"]["content"].as_array() {
                let last = blocks.iter().rev().filter(|b| b["type"] == "text")
                    .filter_map(|b| b["text"].as_str()).find(|t| !t.trim().is_empty());
                if let Some(t) = last {
                    // first sentence, without markdown symbols
                    let clean = t.replace(['*', '`', '#'], "");
                    said = clean.trim().split_inclusive(['.', '!', '?', '\n']).next().unwrap_or("").trim().to_string();
                }
            }
        }
        if !model.is_empty() && !said.is_empty() { break; }
    }
    (model, said)
}

fn idle_label(state: &str) -> String {
    match state {
        "sleep" => "Zzz… nothing to do".into(),
        "bored" => "Bored… give Claude something to do".into(),
        _ => "Idle, waiting for Claude".into(),
    }
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

    let mut pet = Pet { cfg, state: "idle", anim: String::new(), label: idle_label("idle"), prompt: String::new(), said: String::new(), model: String::new(), effort: String::new(), mode: String::new(), generation: Arc::new(AtomicU64::new(0)), proxy };
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
                if let Some(p) = id.strip_prefix("pack:") { pet.cfg.pack = p.to_string(); pet.anim = animation(p, pet.state); }
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
