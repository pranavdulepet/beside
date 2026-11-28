use std::{
    ffi::{c_char, CStr},
    sync::{Mutex, OnceLock},
};
use tauri::Emitter;

static APP: OnceLock<tauri::AppHandle> = OnceLock::new();
static TEXT: Mutex<Option<String>> = Mutex::new(None);
extern "C" {
    fn beside_register_service(callback: extern "C" fn(*const c_char));
}

pub fn enable(app: &tauri::AppHandle) {
    let _ = APP.set(app.clone());
    unsafe { beside_register_service(receive) }
}
extern "C" fn receive(raw: *const c_char) {
    if raw.is_null() {
        return;
    }
    let Some(app) = APP.get() else { return };
    let Ok(text) = unsafe { CStr::from_ptr(raw) }.to_str() else {
        return;
    };
    if text.trim().is_empty() || text.chars().count() > 12000 {
        return;
    }
    if let Ok(mut slot) = TEXT.lock() {
        *slot = Some(text.to_owned());
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::present_activity_sheet_with_origin(&app, serde_json::Value::Null, true);
        let _ = app.emit_to("activity", "selection-ready", ());
    });
}
pub fn take() -> Option<String> {
    TEXT.lock().ok()?.take()
}
