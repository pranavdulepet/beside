use crate::{activity, store::*};
use serde_json::{json, Value};
use std::{
    ffi::{c_char, CStr, CString},
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};
use tauri::{Emitter, Manager};

extern "C" {
    fn sumi_position_reply(pointer: *mut std::ffi::c_void);
    fn sumi_voice_key(down: i32, config: *const c_char, callback: extern "C" fn(*const c_char));
}
static APP: OnceLock<tauri::AppHandle> = OnceLock::new();
struct Gesture {
    id: String,
    payload: Value,
    pending: bool,
    time: Instant,
}
static LAST: Mutex<Option<Gesture>> = Mutex::new(None);

pub fn expire(clear: bool) {
    if let Ok(mut slot) = LAST.lock() {
        if clear
            || slot
                .as_ref()
                .is_some_and(|g| g.time.elapsed().as_secs() >= 600)
        {
            *slot = None;
        }
    }
}

pub fn key(app: &tauri::AppHandle, down: bool) {
    let _ = APP.set(app.clone());
    let s = app.state::<Arc<Store>>();
    let mut config = if down {
        s.get("activitySettings").unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    if config.is_object() {
        config["epoch"] = s.get("activityEpoch").unwrap_or(Value::Null);
    }
    let raw = CString::new(config.to_string()).unwrap();
    unsafe { sumi_voice_key(down as i32, raw.as_ptr(), receive) };
}
extern "C" fn receive(raw: *const c_char) {
    if raw.is_null() {
        return;
    }
    let Some(app) = APP.get().cloned() else {
        return;
    };
    let Ok(mut payload) = serde_json::from_slice::<Value>(unsafe { CStr::from_ptr(raw) }.to_bytes())
    else {
        return;
    };
    tauri::async_runtime::spawn_blocking(move || {
        let s = app.state::<Arc<Store>>();
        if payload["warm"] == true {
            if let Some(cfg) = activity::private_model(&s).filter(|cfg| cfg["runtime"] == "Sumi") {
                let _ = crate::bundled::warm(&cfg);
            }
            return;
        }
        if payload["tap"] == true {
            if crate::message_reply::capture(&s).unwrap_or(false) {
                crate::present_activity_sheet(&app, Value::Null);
                let _ = app.emit_to("activity", "message-reply-ready", ());
            } else {
                crate::activity_sheet(&app)
            }
            return;
        }
        let mut anchor = Value::Null;
        if let Ok(Some(id)) = activity::baggage::pin(&s, &payload["point"], &payload["generation"]) {
            anchor = json!({"id":id, "title":payload["point"]["bookingTitle"], "label":payload["point"]["app"]});
            payload["point"]["bookingId"] = json!(id);
        }
        if let Ok(mut slot) = LAST.lock() {
            *slot = Some(Gesture {
                id: id(),
                payload,
                pending: true,
                time: Instant::now(),
            });
        }
        // Deliver the destination before focusing an already-mounted composer.
        // A newly mounted sheet also checks the pending intent on startup.
        if app.get_webview_window("activity").is_some() {
            let _ = app.emit_to("activity", "voice-ready", ());
        }
        crate::present_activity_sheet(&app, anchor);
        if let Some(w) = app.get_webview_window("activity") {
            let inner = w.clone();
            let _ = w.run_on_main_thread(move || {
                if let Ok(ptr) = inner.ns_window() {
                    unsafe { sumi_position_reply(ptr) }
                }
            });
        }
        let _ = app.emit_to("activity", "voice-ready", ());
    });
}
pub fn intent(s: &Store) -> Res<Value> {
    let mut slot = LAST.lock().unwrap();
    let Some(g) = slot.as_mut() else {
        return Ok(Value::Null);
    };
    if !g.pending {
        return Ok(Value::Null);
    }
    g.pending = false;
    let p = &g.payload;
    let point = if p["epoch"] == s.get("activityEpoch")?
        && p["generation"] == s.get("activitySettings")?["generation"]
    {
        &p["point"]
    } else {
        &Value::Null
    };
    Ok(json!({
        "id": g.id,
        "typed": p["typed"] == true,
        "text": p["text"],
        "issue": p["issue"],
        "line": point["line"],
        "app": point["app"],
        "hasImage": point["fullImage"].as_str().is_some_and(|v| !v.is_empty())
            || point["image"].as_str().is_some_and(|v| !v.is_empty())
    }))
}
pub fn target(s: &Store, id: &str) -> Res<Value> {
    let slot = LAST.lock().unwrap();
    let g = slot
        .as_ref()
        .filter(|g| g.id == id && g.time.elapsed().as_secs() < 600)
        .ok_or("That pointer target expired. Point and hold again.")?;
    let p = &g.payload;
    if s.get("activityEpoch")? != p["epoch"]
        || s.get("activitySettings")?["generation"] != p["generation"]
    {
        return Err("Your sources changed. Point again.".into());
    }
    let point = &p["point"];
    if !activity::allowed(
        &s.get("activitySettings")?,
        point["source"].as_str().unwrap_or(""),
    ) {
        return Ok(Value::Null);
    }
    Ok(point.clone())
}
