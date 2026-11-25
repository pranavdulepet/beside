mod activity;
mod activity_documents;
mod activity_files;
mod agent_link;
mod app;
mod attachments;
mod brain;
mod bundled;
mod catalog;
mod conversation;
mod correction;
mod desk_tools;
mod engine;
mod exports;
mod first_reading;
mod forget;
mod floating_chats;
mod ghost;
mod jobs;
mod local;
mod meetings;
mod memory;
mod message_reply;
mod mirror;
mod models;
mod notebook_data;
mod overlay;
mod plan;
mod point;
mod proactive;
mod providers;
mod scroll;
mod seal;
mod seed;
mod selection;
mod sources;
mod store;
mod transport;
mod web_search;
mod weights;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use store::*;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

#[tauri::command]
async fn notebook(
    window: tauri::WebviewWindow,
    store: tauri::State<'_, Arc<Store>>,
    operation: String,
    args: serde_json::Value,
) -> Res<serde_json::Value> {
    let s = store.inner().clone();
    if operation == "importMailExport" {
        let chosen = rfd::AsyncFileDialog::new().set_parent(&window)
            .set_title("Choose a Mail export folder").pick_folder().await;
        let Some(folder) = chosen else { return Ok(serde_json::Value::Null) };
        return tauri::async_runtime::spawn_blocking(move || activity::mail_archive::import(&s, folder.path())).await.msg()?;
    }
    tauri::async_runtime::spawn_blocking(move || {
        if operation == "voiceIntent" && window.label()=="activity" { return point::intent(&s); }
        if operation == "visitPublicSource" && window.label()=="activity" {
            let url=args["url"].as_str().filter(|url|web_search::public_url(url)).ok_or("Invalid web source.")?;
            std::process::Command::new("/usr/bin/open").arg(url).spawn().msg()?;
            return Ok(serde_json::Value::Null);
        }
        if operation == "messageReplyIntent" || operation == "draftMessageReply" {
            if window.label() != "activity" { return Err("Open the floating sheet.".into()); }
            if operation == "messageReplyIntent" { return message_reply::intent(&s); }
            return message_reply::draft(&s,args["id"].as_str().unwrap_or(""),&mut |delta| {
                let _=window.emit("activity-delta",serde_json::json!({"turn":args["id"],"delta":delta}));
            });
        }
        if operation == "takeSelection" {
            if window.label() != "activity" {return Ok(serde_json::Value::Null)}
            return Ok(serde_json::json!(selection::take()))
        }
        if operation == "dismissActivity" && window.label() == "activity" {
            overlay::clear();
            window.hide().msg()?;
            return Ok(serde_json::Value::Null);
        }
        if operation == "activitySize" && window.label() == "activity" {
            size_assistant(&window, args["compact"] == true, args["height"].as_i64().unwrap_or(160).clamp(120, 512) as i32, false)?;
            return Ok(serde_json::Value::Null);
        }
        if operation == "dragAssistant" && window.label() == "activity" {
            window.start_dragging().msg()?;
            return Ok(serde_json::Value::Null);
        }
        if operation == "openNotebook" {
            open_workspace(window.app_handle())?;
            if window.label() == "activity" {
                window.hide().msg()?;
            }
            return Ok(serde_json::Value::Null);
        }
        if operation == "openActivityFiles" {
            activity_files::sync(&s)?;
            if s.get("activitySettings")?["journalFiles"] != true {
                return Err("Turn on daily Markdown files first.".into());
            }
            std::process::Command::new("/usr/bin/open")
                .arg(activity_files::directory(&s))
                .spawn()
                .msg()?;
            return Ok(serde_json::Value::Null);
        }
        if operation == "openActivity" {
            activity_sheet(window.app_handle());
            return Ok(serde_json::Value::Null);
        }
        if operation == "trashNotebook" {
            notebook_data::request_removal(&s, &args)?;
            shutdown();
            window.app_handle().restart();
        }
        if operation == "activityAsk" {
            let answer = activity::ask_with_phases(
                &s,
                &args,
                &mut |delta| {
                    let _ = window.emit(
                        "activity-delta",
                        serde_json::json!({"turn":args["turn"],"delta":delta}),
                    );
                },
                &mut |stage| {
                    let _ = window.emit(
                        "activity-delta",
                        serde_json::json!({"turn":args["turn"],"phase":stage}),
                    );
                },
            )?;
            if args["seenAt"].is_i64() && std::env::args().any(|v| v == "--recall-screen") {
                let source = &answer["sources"][0];
                let quote = answer["quote"].as_str().unwrap_or("");
                let matched = if let Some(id) = source["id"].as_str() {
                    let db = s.db.lock().unwrap();
                    let body: String = db.query_row("SELECT body FROM activity WHERE id=?", [id], |r| r.get(0)).msg()?;
                    !quote.is_empty() && body.lines().any(|line| line.trim() == quote)
                } else { false };
                let mut report = serde_json::json!({"sourceTime":source["stamp"],"readPasses":answer["readPasses"],"quoteMatchesSource":matched,
                    "modelGenerated":answer["lineSelection"]==true,"localModel":activity::settings(&s)?["localModel"]["name"]});
                if matched && std::env::args().any(|v| v == "--print-recall") {
                    report["sourceTitle"] = source["title"].clone();
                    report["answer"] = answer["answer"].clone();
                    report["quote"] = answer["quote"].clone();
                }
                println!("{report}");
            }
            return Ok(answer);
        }
        if operation == "askSheet" {
            return conversation::ask_with(&s, &args, &mut |delta| {
                let _ = window.emit(
                    "reply-delta",
                    serde_json::json!({"turn":args["turn"],"delta":delta}),
                );
            });
        }
        dispatch(&s, &operation, &args)
    })
    .await
    .msg()?
}
fn dispatch(s: &Arc<Store>, op: &str, args: &serde_json::Value) -> Res<serde_json::Value> {
    if op == "beginFirstReading" {
        first_reading::start(s, Some(args["notify"] == true))?;
        return Ok(serde_json::Value::Null);
    }
    app::action(s, op, args)
}
fn show(app: &tauri::AppHandle) {
    if let Err(error) = open_workspace(app) {
        eprintln!("Beside could not open the notebook: {error}");
    }
}
fn open_workspace(app: &tauri::AppHandle) -> Res<()> {
    let window = app.get_webview_window("main").ok_or("The notebook window is unavailable. Reopen Beside.")?;
    window.unminimize().msg()?;
    window.show().msg()?;
    window.set_focus().msg()
}
fn size_assistant(
    w: &tauri::WebviewWindow,
    compact: bool,
    height: i32,
    initial: bool,
) -> Res<()> {
    extern "C" {
        fn sumi_size_panel(window: *mut std::ffi::c_void, layout: i32, initial: bool);
    }
    let window = w.clone();
    w.run_on_main_thread(move || {
        if let Ok(ptr) = window.ns_window() {
            let layout = if compact { 0 } else { height };
            unsafe { sumi_size_panel(ptr, layout, initial) };
        }
    })
    .msg()
}
fn activity_sheet(app: &tauri::AppHandle) {
    let store = app.state::<Arc<Store>>();
    let anchor = activity::anchor(&store).unwrap_or_default();
    present_activity_sheet(app, anchor);
}
fn present_activity_sheet(app: &tauri::AppHandle, anchor: serde_json::Value) {
    let _ = app
        .state::<Arc<Store>>()
        .set("activityAnchor", &anchor["id"]);
    if let Some(w) = app.get_webview_window("activity") {
        let _ = w.emit("activity-expand", ());
        let _ = w.emit("activity-anchor", &anchor);
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let window = tauri::WebviewWindowBuilder::new(
        app,
        "activity",
        tauri::WebviewUrl::App("index.html#activity".into()),
    )
    .title("Ask Beside")
    .title_bar_style(tauri::TitleBarStyle::Overlay)
    .hidden_title(true)
    .shadow(true)
    .visible(false)
    .inner_size(420., 144.)
    .min_inner_size(204., 46.)
    .always_on_top(true)
    .visible_on_all_workspaces(true)
    .center()
    .build();
    if let Ok(w) = window {
        let _ = size_assistant(&w, false, 144, true);
        let _ = w.show();
        let _ = w.set_focus();
    }
}
fn fresh_sheet(app: &tauri::AppHandle) {
    show(app);
    if app.state::<Arc<Store>>().get("onboarded").ok() == Some(serde_json::json!(true)) {
        let _ = app.emit("fresh-sheet", ());
    }
}
static QUITTING: AtomicBool = AtomicBool::new(false);
pub(crate) fn quitting() -> bool {
    QUITTING.load(Ordering::SeqCst)
}
fn shutdown() {
    QUITTING.store(true, Ordering::SeqCst);
    bundled::reap(true);
}
fn main() {
    let result = run();
    shutdown();
    if let Err(e) = result {
        eprintln!("Beside: {e}");
        std::process::exit(1)
    }
}
fn run() -> Res<()> {
    let args: Vec<String> = std::env::args().collect();
    let param = |k: &str| {
        args.iter()
            .position(|x| x == k)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let root = param("--notebook")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let home = sources::home();
            let legacy = home.join("Sumi");
            if legacy.join(".sumi/moments.db").is_file() { legacy } else { home.join("Beside") }
        });
    if let Some(path) = param("--import-mail-export") {
        if !root.join(".sumi/moments.db").is_file() { return Err("Open your notebook first.".into()); }
        let s = Store::open(root)?;
        println!("{}", activity::mail_archive::import(&s, std::path::Path::new(&path))?);
        return Ok(());
    }
    if let Some(topic) = param("--review-suggestion") {
        if !root.join(".sumi/moments.db").is_file() { return Err("Open your notebook first.".into()); }
        let s = Store::open(root)?;
        let through = param("--before").map(|v| v.parse::<i64>()).transpose().msg()?.unwrap_or_else(|| chrono::Utc::now().timestamp());
        println!("{}", proactive::review_saved(&s, &topic, through)?);
        return Ok(());
    }
    if args.iter().any(|x| x == "--screen-memory-status") {
        if !root.join(".sumi/moments.db").is_file() {
            return Err("Open your notebook first.".into());
        }
        let s = Store::open(root)?;
        let settings = activity::settings(&s)?;
        println!(
            "{}",
            serde_json::json!({
                "enabled": settings["enabled"], "scope":settings["scope"], "lastSavedAt":settings["lastSavedAt"],
                "observerScreenAccess":s.get("observerScreenAccess")?,"lastObservedAt":settings["lastObservedAt"], "captureStatus":settings["status"], "shortcutRegistered":settings["shortcutReady"],
                "lastShortcutAt":s.get("lastActivityShortcut")?, "localModel":settings["localModel"]["name"]
            })
        );
        return Ok(());
    }
    if args.iter().any(|x| x == "--mcp") {
        let f = param("--credential").ok_or("Missing agent credential.")?;
        let token = std::fs::read_to_string(f).msg()?;
        if transport::relay(&root, token.trim())? {
            return Ok(());
        }
        return seal::serve(&Store::open(root)?, token.trim());
    }
    #[cfg(debug_assertions)]
    if args.iter().any(|x| x == "--fixture-rpc") {
        return fixture_rpc(root, args.iter().any(|v| v == "--fixture-agents"));
    }
    let launch_model = param("--local-model").map(std::path::PathBuf::from);
    let seen_at = param("--seen-at").and_then(|v| v.parse::<i64>().ok());
    let request_screen = args.iter().any(|arg| arg == "--request-screen-access");
    let recall_on_open = args.iter().any(|arg| arg == "--recall-screen");
    let notebook_first = args.iter().any(|arg| arg == "--desk");
    notebook_data::finish_removal(&root)?;
    let s = Arc::new(Store::open(root)?);
    app::setup(&s)?;
    tauri::Builder::default()
        .manage(s.clone())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, e| {
                    if shortcut.matches(
                        tauri_plugin_global_shortcut::Modifiers::ALT
                            | tauri_plugin_global_shortcut::Modifiers::SHIFT,
                        tauri_plugin_global_shortcut::Code::Space,
                    ) {
                        let down = e.state() == ShortcutState::Pressed;
                        if down {
                            let _ = app.state::<Arc<Store>>().set(
                                "lastActivityShortcut",
                                &serde_json::json!(chrono::Utc::now().timestamp()),
                            );
                        }
                        point::key(app, down);
                    } else if e.state() == ShortcutState::Pressed {
                        fresh_sheet(app);
                    }
                })
                .build(),
        )
        .setup(move |app| {
            selection::enable(app.handle());
            let show_item = MenuItem::with_id(app, "open", "Open notebook", true, None::<&str>)?;
            let fresh = MenuItem::with_id(app, "fresh", "New Desk Sheet", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Beside", true, None::<&str>)?;
            let ask = MenuItem::with_id(
                app,
                "askactivity",
                "Ask about this Mac   ⌥⇧Space",
                true,
                None::<&str>,
            )?;
            let toggle = MenuItem::with_id(app, "observe", "Activity paused", false, None::<&str>)?;
            let exclude = MenuItem::with_id(
                app,
                "exclude-active",
                "Exclude current source",
                false,
                None::<&str>,
            )?;
            let controls = MenuItem::with_id(
                app,
                "activity-controls",
                "Activity sources and privacy…",
                true,
                None::<&str>,
            )?;
            let forget = MenuItem::with_id(
                app,
                "forget-hour",
                "Forget the last hour…",
                true,
                None::<&str>,
            )?;
            let suggestion =
                MenuItem::with_id(app, "suggestion", "View suggestion", true, None::<&str>)?;
            let end_meeting = MenuItem::with_id(
                app,
                "end-meeting",
                "Stop meeting recording",
                false,
                None::<&str>,
            )?;
            let menu = Menu::with_items(
                app,
                &[
                    &ask,
                    &end_meeting,
                    &toggle,
                    &exclude,
                    &controls,
                    &forget,
                    &show_item,
                    &fresh,
                    &quit,
                ],
            )?;
            TrayIconBuilder::with_id("sumi")
                .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?)
                .icon_as_template(true)
                .tooltip("Beside")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "askactivity" => activity_sheet(app),
                    "observe" => {
                        let s = app.state::<Arc<Store>>();
                        let enabled = activity::settings(&s)
                            .ok()
                            .is_some_and(|c| c["enabled"] == true);
                        let _ = activity::configure(&s, &serde_json::json!({"enabled":!enabled}));
                        let _ = app.emit("activity-changed", ());
                    }
                    "exclude-active" => {
                        let s = app.state::<Arc<Store>>();
                        if let Ok(c) = activity::settings(&s) {
                            if let Some(source) = activity::observed_source(&c) {
                                let _ = activity::configure(
                                    &s,
                                    &serde_json::json!({"source":source,"allow":false}),
                                );
                            }
                        }
                    }
                    "suggestion" => {
                        let _ = app.state::<Arc<Store>>().set("activityControls", &serde_json::json!("suggestion"));
                        activity_sheet(app);
                        let _ = app.emit_to("activity", "activity-controls", ());
                    }
                    "activity-controls" | "forget-hour" => {
                        let _ = app
                            .state::<Arc<Store>>()
                            .set("activityControls", &serde_json::json!(event.id.as_ref()));
                        activity_sheet(app);
                        let _ = app.emit_to(
                            "activity",
                            "activity-controls",
                            event.id.as_ref() == "forget-hour",
                        );
                    }
                    "end-meeting" => {
                        let _ = meetings::control(&app.state::<Arc<Store>>(), "stop", false);
                    }
                    "open" => show(app),
                    "fresh" => fresh_sheet(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            s.set(
                "globalShortcutReady",
                &serde_json::json!(app.global_shortcut().register("Alt+Space").is_ok()),
            )?;
            s.set(
                "activityShortcutReady",
                &serde_json::json!(app.global_shortcut().register("Alt+Shift+Space").is_ok()),
            )?;
            let workspace_db = s.clone();
            std::thread::spawn(move || {
                crate::local::background_thread();
                for slot in 0.. {
                    if quitting() { break; }
                    let new_source = activity::workspace::refresh(&workspace_db, slot);
                    if new_source || slot % 6 == 0 { let _ = activity::revisit_front(&workspace_db); }
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
            });
            let capture_db = s.clone();
            let capture_app = app.handle().clone();
            std::thread::spawn(move || {
                crate::local::background_thread();
                let mut last_sync = std::time::Instant::now();
                let mut showing_suggestion = false;
                let mut published = serde_json::Value::Null;
                let mut identity = activity::capture_identity(&capture_db);
                let mut front_changed = false;
                loop {
                    if last_sync.elapsed() >= std::time::Duration::from_secs(30) {
                        if let Err(err) = activity_files::sync(&capture_db) {
                            let _ = capture_db.set("journalError", &serde_json::json!(err));
                        } else {
                            if capture_db.get("journalError").is_ok_and(|v| !v.is_null()) {
                                let _ = capture_db.set("journalError", &serde_json::Value::Null);
                            }
                        }
                        last_sync = std::time::Instant::now();
                    }
                    if quitting() {
                        break;
                    }
                    if let Err(error) = activity::sample(&capture_db) {
                        let _ = capture_db.set("activityStatus", &serde_json::json!(error));
                    }
                    if front_changed && proactive::latest().is_null() {
                        let _ = activity::revisit_front(&capture_db);
                    }
                    let next_nudge = proactive::latest();
                    if next_nudge["id"] != published["id"] || next_nudge["eventId"] != published["eventId"] {
                        published = next_nudge.clone();
                        let _ = capture_app.emit("proactive-nudge", &next_nudge);
                        if !next_nudge["sourceId"].is_null() {
                            if let Some(model) = activity::private_model(&capture_db).filter(|m| m["runtime"] == "Sumi") {
                                std::thread::spawn(move || { let _ = bundled::warm(&model); });
                            }
                        }
                    }
                    if let Ok(mut c) = activity::config(&capture_db) {
                        c["status"] = capture_db.get("activityStatus").unwrap_or_default();
                        let watching = c["enabled"] == true;
                        let suggested = watching && !next_nudge.is_null();
                        if suggested != showing_suggestion {
                            let updated = if suggested {
                                menu.insert(&suggestion, 1)
                            } else {
                                menu.remove(&suggestion)
                            };
                            if updated.is_ok() {
                                showing_suggestion = suggested;
                            }
                        }
                        let current = activity::observed_source(&c);
                        let _ = exclude.set_enabled(current.is_some());
                        let _ = exclude.set_text(
                            current
                                .map(|v| {
                                    format!(
                                        "Exclude {}",
                                        c["names"][v].as_str().unwrap_or(activity::label(v))
                                    )
                                })
                                .unwrap_or_else(|| "Exclude current source".into()),
                        );
                        let _ = toggle.set_enabled(
                            c["scope"] == "displays"
                                || c["sources"].as_array().is_some_and(|v| !v.is_empty()),
                        );
                        let _ = toggle.set_text(if watching {
                            "Pause observation"
                        } else {
                            "Resume observation"
                        });
                        let recording = meetings::control(&capture_db, "status", false)
                            .is_ok_and(|v| v["phase"] == "recording");
                        let _ = end_meeting.set_enabled(recording);
                        if let Some(tray) = capture_app.tray_by_id("sumi") {
                            let state = if suggested {
                                "Suggestion available"
                            } else if watching {
                                c["status"].as_str().unwrap_or("Observation on")
                            } else {
                                "Observation paused"
                            };
                            let _ = tray.set_tooltip(Some(format!("Beside · {state}")));
                            let mark = if recording {
                                "● REC"
                            } else if suggested {
                                "●"
                            } else if watching {
                                ""
                            } else if capture_db.agent_waiting().unwrap_or(false) {
                                "●"
                            } else {
                                "Ⅱ"
                            };
                            let _ = tray.set_title(Some(mark));
                        }
                    }
                    front_changed = false;
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                    while std::time::Instant::now() < deadline && !quitting() {
                        std::thread::sleep(std::time::Duration::from_millis(300));
                        let next = activity::capture_identity(&capture_db);
                        if next != identity {
                            if let Some(row) = &next {
                                proactive::follow_front(row["source"].as_str().unwrap_or(""), row["title"].as_str().unwrap_or(""));
                            }
                            identity = next;
                            front_changed = true;
                            break;
                        }
                        // A covered Notes or Mail window may finish indexing during this wait.
                        let current = proactive::latest();
                        if current["id"] != published["id"] || current["eventId"] != published["eventId"] {
                            break;
                        }
                    }
                }
            });
            let dream_db = s.clone();
            std::thread::spawn(move || {
                crate::local::background_thread();
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(300));
                    if quitting() {
                        break;
                    }
                    let _ = activity::prune(&dream_db);
                    if crate::local::idle() {
                        let _ = activity::dream(&dream_db);
                    }
                }
            });
            if let Err(e) = transport::start_socket(s.clone()) {
                let _ = s.set("backgroundIssue", &serde_json::json!(e));
            }
            if let Err(e) = transport::start(s.clone()) {
                let _ = s.set("backgroundIssue", &serde_json::json!(e));
            }
            first_reading::start(&s, None).map_err(std::io::Error::other)?;
            jobs::start(s.clone());
            let handle = app.handle().clone();
            let db = s.clone();
            std::thread::spawn(move || {
                let mut last_revision = (0, 0);
                let mut disk_edits = scroll::DiskEdits::default();

                let mut arrival = exports::Arrival::default();
                let downloads = sources::home().join("Downloads");
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(15));
                    bundled::reap(false);
                    point::expire(false);
                    let _ = arrival.check(&db, &downloads);
                    if let Ok(_g) = db.edit.try_lock() {
                        let _ = disk_edits.check(&db);
                    }
                    let revision = {
                        let c = db.db.lock().unwrap();
                        (
                            c.total_changes(),
                            c.query_row("PRAGMA data_version", [], |r| r.get::<_, i64>(0))
                                .unwrap_or(0),
                        )
                    };
                    if revision != last_revision {
                        last_revision = revision;
                        let _ = handle.emit("notebook-changed", ());
                    }
                }
            });

            if let Some(path) = launch_model {
                let db = s.clone();
                std::thread::spawn(move || match bundled::use_file(&db, &path) {
                    Ok(_) => {
                        let _ = db.set("activityModelIssue", &serde_json::Value::Null);
                    }
                    Err(e) => {
                        let _ = db.set("activityModelIssue", &serde_json::json!(e));
                    }
                });
            }
            if request_screen {
                let db = s.clone();
                std::thread::spawn(move || {
                    if let Ok(c) = activity::settings(&db) {
                        if c["enabled"] == true
                            && c["scope"] == "displays"
                            && c["screenPermission"] != true
                        {
                            let _ = activity::permission("apps");
                        }
                    }
                });
            }
            if recall_on_open {
                s.set("activityControls", &serde_json::json!({"recall":seen_at}))?;
            }
            if notebook_first {
                show(app.handle());
            } else {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn_blocking(move || activity_sheet(&handle));
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "activity" {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![notebook])
        .build(tauri::generate_context!())
        .msg()?
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { ref api, .. } = event {
                let db = app.state::<Arc<Store>>();
                let phase = meetings::control(&db, "status", false)
                    .ok()
                    .and_then(|s| s["phase"].as_str().map(str::to_owned));
                if matches!(phase.as_deref(), Some("starting" | "recording" | "saving")) {
                    api.prevent_exit();
                    static FINISHING: AtomicBool = AtomicBool::new(false);
                    if !FINISHING.swap(true, Ordering::SeqCst) {
                        let handle = app.clone();
                        std::thread::spawn(move || {
                            let store = handle.state::<Arc<Store>>();
                            for _ in 0..120 {
                                if let Ok(state) = meetings::control(&store, "stop", false) {
                                    if !matches!(
                                        state["phase"].as_str(),
                                        Some("starting" | "recording" | "saving")
                                    ) {
                                        FINISHING.store(false, Ordering::SeqCst);
                                        handle.exit(0);
                                        return;
                                    }
                                }
                                std::thread::sleep(std::time::Duration::from_millis(250));
                            }
                            FINISHING.store(false, Ordering::SeqCst);
                        });
                    }
                }
            }
            if matches!(event, tauri::RunEvent::Exit) {
                shutdown();
            }
            if let tauri::RunEvent::Reopen {
                has_visible_windows: false,
                ..
            } = event
            {
                activity_sheet(app);
            }
        });
    Ok(())
}
#[cfg(debug_assertions)]
fn fixture_rpc(root: std::path::PathBuf, agents: bool) -> Res<()> {
    use std::io::{BufRead, Write};
    if !root.starts_with(std::env::temp_dir()) && !root.starts_with("/tmp") {
        return Err("Fixture notebooks require a temporary folder.".into());
    }
    let s = Arc::new(Store::with_key(root, &[19; 32])?);
    if std::env::args().any(|v| v == "--fixture-predictions") {
        let predictions: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(s.root.join("predictions.json")).msg()?).msg()?;
        for v in predictions {
            s.put("prediction", &v)?;
        }
    }
    if agents {
        transport::start_socket(s.clone())?;
    }
    app::setup(&s)?;
    first_reading::start(&s, None)?;
    let mut arrival = exports::Arrival::default();
    let downloads = s.root.join("Downloads");
    for line in std::io::stdin().lock().lines() {
        let req: serde_json::Value = serde_json::from_str(&line.msg()?).msg()?;
        let op = req["operation"].as_str().unwrap_or("");
        // Keep a fixture connection inside its temporary notebook even if a UI test
        // clicks a real agent's Connect button.
        if op == "state" {
            arrival.check(&s, &downloads)?;
        }
        let result = if op == "fixtureActivity" {
            let config = activity::settings(&s)?;
            let mut row = req["args"].clone();
            if let Some(input) = row["fixtureImage"].as_str() {
                let decoded = attachments::read(
                    &serde_json::json!({"name":"Synthetic window.png","data":input}),
                )?;
                row["body"] = decoded["text"].clone();
                if let Some(preview) = decoded["preview"].as_str() {
                    row["preview"] =
                        serde_json::json!(preview.trim_start_matches("data:image/jpeg;base64,"));
                }
            }
            activity::ingest(
                &s,
                &config,
                &row,
                chrono::Utc::now().timestamp() - row["age"].as_i64().unwrap_or(0),
            )
            .map(|_| serde_json::Value::Null)
        } else if op == "fixtureDocumentFolder" {
            let folder = s.root.join("example-documents").canonicalize().msg()?;
            s.set("activityFolders", &serde_json::json!([folder]))
                .map(|_| serde_json::Value::Null)
        } else if op == "fixtureActivityDream" {
            activity::dream(&s).map(|_| serde_json::Value::Null)
        } else if op == "fixtureInkling" {
            let file = req["args"]["path"]
                .as_str()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("../release/Inkling-2-Mini-UD-IQ3_XXS.gguf")
                });
            bundled::use_file(&s, &file)
        } else if op == "activityAsk" {
            activity::ask_with_phases(
                &s,
                &req["args"],
                &mut |delta| {
                    if req["stream"] == true {
                        let _ = writeln!(
                            std::io::stdout(),
                            "{}",
                            serde_json::json!({"event":"activity-delta","payload":{"turn":req["args"]["turn"],"delta":delta}})
                        );
                    }
                },
                &mut |stage| {
                    if req["stream"] == true {
                        let _ = writeln!(
                            std::io::stdout(),
                            "{}",
                            serde_json::json!({"event":"activity-delta","payload":{"turn":req["args"]["turn"],"phase":stage}})
                        );
                    }
                },
            )
        } else if op == "overlayCaptureFor" {
            Ok(overlay::live_elements_for(&s,
                req["args"]["hint"].as_str().unwrap_or(""),
            ))
        } else if op == "askSheet" && req["stream"] == true {
            conversation::ask_with(&s, &req["args"], &mut |delta| {
                let _ = writeln!(
                    std::io::stdout(),
                    "{}",
                    serde_json::json!({"event":"reply-delta","payload":{"turn":req["args"]["turn"],"delta":delta}})
                );
            })
        } else if op == "exportNotebook" {
            let dest = std::env::temp_dir().join(format!("sumi-export-{}.zip", crate::store::id()));
            crate::notebook_data::export(&s, &dest).map(|()| serde_json::json!(dest))
        } else if op == "trashNotebook" {
            Err("Fixture notebooks cannot operate on the Mac Trash.".into())
        } else if op == "exportLink" {
            exports::request(&s)
        } else if op == "exports" {
            exports::find(&s, &downloads)
        } else if op == "acceptExport" {
            exports::accept(&s, &downloads, req["args"]["path"].as_str().unwrap_or(""))
        } else if op == "connect" {
            seal::connect(
                &s,
                req["args"]["name"].as_str().unwrap_or(""),
                &s.root.join("agent-config"),
                &std::env::current_exe().msg()?,
            )
        } else if op == "discover" {
            Ok(serde_json::json!([]))
        } else if op == "pickSource"
            && req["args"]["kind"] == "export"
            && s.root.join("conversations.json").is_file()
        {
            app::register(
                &s,
                "fixture-export",
                &s.root.join("conversations.json"),
                "Studio journal · example conversations",
            )
        } else {
            dispatch(&s, op, &req["args"])
        };
        writeln!(
            std::io::stdout(),
            "{}",
            match result {
                Ok(v) => serde_json::json!({"ok":v}),
                Err(e) => serde_json::json!({"error":e}),
            }
        )
        .msg()?
    }
    bundled::reap(true);
    Ok(())
}

#[cfg(test)]
mod checks;
