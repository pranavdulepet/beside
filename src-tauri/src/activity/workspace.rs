use super::*;
use std::sync::Mutex;

extern "C" {
    fn beside_workspace_windows(slot: usize) -> *mut c_char;
    fn beside_workspace_available() -> i32;
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
    let p = unsafe { beside_workspace_windows(slot) };
    if p.is_null() {
        clear();
        return false;
    }
    let data = unsafe {
        let data = serde_json::from_slice::<Value>(CStr::from_ptr(p).to_bytes());
        beside_free(p);
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
    if !enabled(&c) || unsafe { beside_workspace_available() } == 0 {
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
    let mut scored: Vec<(usize, Value)> = rows
        .drain(..)
        .map(|r| {
            let tokens = conversation::words(&format!("{} {} {}", r["app"], r["title"], r["text"]));
            (terms.intersection(&tokens).count(), r)
        })
        .collect();
    scored.sort_by(|a, b| (b.0, b.1["front"] == true).cmp(&(a.0, a.1["front"] == true)));
    let mut matched_excerpts = 0usize;
    input["open_windows_on_this_mac"] = json!(scored.into_iter().take(12).map(|(overlap, r)| {
        let front = r["front"] == true;
        let mut w = json!({"app":r["app"],"title":r["title"],"position":if front {"frontmost at sample"} else {"also open"}});
        let include_excerpt = front || (overlap > 0 && matched_excerpts < 2);
        if include_excerpt {
            if !front {
                matched_excerpts += 1;
            }
            w["excerpt"] = json!(r["text"].as_str().unwrap_or("").chars().take(420).collect::<String>());
            w["readAt"] = r["readAt"].clone();
        }
        w
    }).collect::<Vec<_>>());
}
