use super::*;
use std::sync::Mutex;

extern "C" {
    fn sumi_workspace_windows(slot: usize) -> *mut c_char;
    fn sumi_workspace_available() -> i32;
}

struct Snapshot {
    generation: Value,
    at: i64,
    windows: Vec<Value>,
}

static SNAPSHOT: Mutex<Option<Snapshot>> = Mutex::new(None);

fn enabled(c: &Value) -> bool {
    c["enabled"] == true && c["scope"] == "displays" && c["openWindows"] == true
}

pub fn clear() {
    *SNAPSHOT.lock().unwrap() = None;
}

pub fn refresh(s: &Store, slot: usize) -> bool {
    let Ok(c) = config(s) else {
        clear();
        return false;
    };
    if !enabled(&c) {
        clear();
        return false;
    }
    let p = unsafe { sumi_workspace_windows(slot) };
    if p.is_null() {
        clear();
        return false;
    }
    let data = unsafe {
        let data = serde_json::from_slice::<Value>(CStr::from_ptr(p).to_bytes());
        sumi_free(p);
        data
    };
    let Ok(data) = data else {
        clear();
        return false;
    };
    let Some(rows) = data["windows"].as_array() else {
        clear();
        return false;
    };
    let _edit = s.activity_edit.lock().unwrap();
    if config(s).ok().as_ref() != Some(&c) {
        return false;
    }
    let now = Utc::now().timestamp();
    let mut snapshot = SNAPSHOT.lock().unwrap();
    let previous = snapshot
        .as_ref()
        .filter(|v| v.generation == c["generation"]);
    let mut source_changed = false;
    let mut windows = Vec::with_capacity(rows.len());
    for mut row in rows.iter().cloned() {
        if let Some(text) = row["text"].as_str() {
            row["text"] = json!(text.chars().take(2400).collect::<String>());
            row["readAt"] = json!(now);
            if matches!(row["app"].as_str(), Some("Notes" | "Mail" | "Microsoft Outlook"))
                && previous.is_none_or(|p| !p.windows.iter().any(|old|
                    ["id", "pid", "title", "text"].iter().all(|k| old[*k] == row[*k]))) {
                source_changed = true;
            }
        } else if let Some(old) = previous.and_then(|p| {
            p.windows.iter().find(|old| {
                ["id", "pid", "title", "app"]
                    .iter()
                    .all(|k| row[*k] == old[*k])
                    && old["readAt"].as_i64().is_some_and(|at| now - at < 90)
            })
        }) {
            row["text"] = old["text"].clone();
            row["readAt"] = old["readAt"].clone();
        }
        windows.push(row);
    }
    *snapshot = Some(Snapshot {
        generation: c["generation"].clone(),
        at: now,
        windows,
    });
    source_changed
}

pub fn windows(s: &Store) -> Vec<Value> {
    let Ok(c) = config(s) else { return vec![] };
    if !enabled(&c) || unsafe { sumi_workspace_available() } == 0 {
        return vec![];
    }
    let lock = SNAPSHOT.lock().unwrap();
    lock.as_ref()
        .filter(|v| v.generation == c["generation"] && Utc::now().timestamp() - v.at <= 15)
        .map(|v| v.windows.clone())
        .unwrap_or_default()
}

pub fn inventory(s: &Store) -> Vec<Value> {
    windows(s)
        .into_iter()
        .map(|mut w| {
            if let Some(m) = w.as_object_mut() {
                m.remove("text");
                m.remove("pid");
            }
            w
        })
        .collect()
}

pub fn attach(s: &Store, input: &mut Value, question: &str) {
    let terms = conversation::words(question);
    let mut rows = windows(s);
    if rows.is_empty() {
        return;
    }
    rows.sort_by_cached_key(|r| {
        let tokens = conversation::words(&format!("{} {} {}", r["app"], r["title"], r["text"]));
        std::cmp::Reverse((terms.intersection(&tokens).count(), r["front"] == true))
    });
    input["open_windows_on_this_mac"] = json!(rows.iter().take(40).enumerate().map(|(i,r)| {
        let mut w = json!({"app":r["app"],"title":r["title"],"position":if r["front"] == true {"frontmost at sample"} else {"also open"}});
        if i < 4 {
            w["excerpt"] = json!(r["text"].as_str().unwrap_or("").chars().take(750).collect::<String>());
            w["readAt"] = r["readAt"].clone();
        }
        w
    }).collect::<Vec<_>>());
}
