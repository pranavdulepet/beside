use crate::store::*;
use regex::Regex;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    ffi::{c_char, CStr, CString},
    sync::LazyLock,
};

extern "C" {
    fn beside_capture_front_elements(config: *const c_char) -> *mut c_char;
    fn beside_window_current(owner: *const c_char) -> i32;
    fn sumi_overlay_highlight_steps(request: *const c_char);
    fn sumi_overlay_clear();
    fn sumi_free(p: *mut c_char);
}

pub fn live_elements(s: &Store) -> Value {
    let Ok(config) = crate::activity::config(s) else {
        return Value::Null;
    };
    if config["enabled"] != true {
        return Value::Null;
    }
    let generation = config["generation"].clone();
    let config = CString::new(config.to_string()).unwrap();
    let ptr = unsafe { beside_capture_front_elements(config.as_ptr()) };
    if ptr.is_null() {
        return json!({"elements": []});
    }
    unsafe {
        let mut parsed = serde_json::from_slice::<Value>(CStr::from_ptr(ptr).to_bytes())
            .unwrap_or_else(|_| json!({"elements": []}));
        sumi_free(ptr);
        parsed["generation"] = generation;
        if usable(s, &parsed) {
            parsed
        } else {
            Value::Null
        }
    }
}

pub fn describe_front(s: &Store) -> Value {
    let mut snap = live_elements(s);
    if let Some(fields) = snap.as_object_mut() {
        fields.remove("image");
    }
    snap
}

fn owner(snap: &Value) -> Value {
    json!({"pid":snap["pid"],"wid":snap["wid"],"bundle":snap["bundle"],
        "window":snap.get("window").unwrap_or(&snap["windowFrame"])})
}

pub fn visual_frame<'a>(snap: &'a Value, brain: &Value) -> Option<&'a Value> {
    (brain["usedImage"] == true && snap["window"].is_object()).then_some(&snap["window"])
}

pub fn current(snap: &Value) -> bool {
    if !snap["wid"].is_number() || !snap["pid"].is_number() {
        return false;
    }
    CString::new(owner(snap).to_string())
        .is_ok_and(|c| unsafe { beside_window_current(c.as_ptr()) == 1 })
}

fn current_config(s: &Store, snap: &Value) -> Option<Value> {
    crate::activity::config(s).ok().filter(|cfg| {
        cfg["enabled"] == true && cfg["generation"] == snap["generation"] && current(snap)
    })
}

pub fn usable(s: &Store, snap: &Value) -> bool {
    current_config(s, snap).is_some()
}

pub fn live_image<'a>(s: &Store, snap: &'a Value) -> Option<&'a str> {
    current_config(s, snap)?;
    snap["image"].as_str().filter(|v| !v.is_empty())
}

pub fn present(s: &Store, snap: &Value, steps: &mut Vec<Value>) {
    steps.truncate(8);
    if steps.is_empty() {
        return;
    }
    let Some(config) = current_config(s, snap) else {
        steps.clear();
        return;
    };
    let target = owner(snap);
    let request = json!({"owner":target,"steps":steps,"config":config});
    if let Ok(c) = CString::new(request.to_string()) {
        unsafe { sumi_overlay_highlight_steps(c.as_ptr()) };
    }
    for step in steps {
        step["owner"] = target.clone();
    }
}

pub fn replay(s: &Store, step: &Value) -> Res<Value> {
    let snap = live_elements(s);
    if owner(&snap) != step["owner"] || !current(&snap) {
        return Err("The window changed. Ask Beside to point again.".into());
    }
    let item = snap["elements"]
        .as_array()
        .and_then(|els| {
            els.iter()
                .find(|v| v["label"] == step["label"] && v["x"] == step["x"] && v["y"] == step["y"])
        })
        .ok_or("That control moved. Ask Beside to point again.")?;
    let mut steps = vec![item.clone()];
    present(s, &snap, &mut steps);
    Ok(json!({"highlighted":!steps.is_empty()}))
}

pub fn clear() {
    unsafe { sumi_overlay_clear() };
}

pub fn wants_guidance(question: &str) -> bool {
    let q = question.to_lowercase();
    [
        "where is",
        "where's",
        "where’s",
        "where can i",
        "where do i",
        "where are",
        "how do i",
        "how can i",
        "how to ",
        "show me",
        "point to",
        "point at",
        "point me",
        "highlight",
        "which button",
        "which menu",
        "which setting",
        "which tab",
        "what button",
        "click",
        "walk me through",
        "guide me",
        "find the ",
        "find my ",
        "locate ",
        "take me to",
        "next step",
        "what next",
        "what's next",
        "whats next",
    ]
    .iter()
    .any(|cue| q.contains(cue))
}

static POINT_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\[point:([^|\]]+?)(?:\|([^\]]*))?\]\]").unwrap());

static QUOTED_LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"["“'`]([^"”'`]{2,42})["”'`]"#).unwrap());

fn parse_visual_box(target: &str, window_frame: Option<&Value>) -> Option<Value> {
    let wf = window_frame?;
    let wx = wf["x"].as_f64()?;
    let wy = wf["y"].as_f64()?;
    let ww = wf["w"].as_f64().filter(|v| *v > 20.0)?;
    let wh = wf["h"].as_f64().filter(|v| *v > 20.0)?;
    let nums: Vec<f64> = target
        .split(',')
        .map(|part| part.trim().trim_end_matches('%').parse::<f64>().ok())
        .collect::<Option<_>>()?;
    let (xp, yp, wp, hp) = match nums.as_slice() {
        [x, y] => (*x, *y, 6.0, 4.0),
        [x, y, w, h] => (*x, *y, *w, *h),
        _ => return None,
    };
    if !(0.0..=100.0).contains(&xp) || !(0.0..=100.0).contains(&yp) {
        return None;
    }
    let w = (wp.clamp(2.0, 80.0) / 100.0) * ww;
    let h = (hp.clamp(2.0, 80.0) / 100.0) * wh;
    let x = wx + (xp / 100.0) * ww - if nums.len() == 2 { w / 2.0 } else { 0.0 };
    let y = wy + (yp / 100.0) * wh - if nums.len() == 2 { h / 2.0 } else { 0.0 };
    Some(json!({
        "id": format!("visual-{}-{}", xp.round() as i64, yp.round() as i64),
        "role": "region",
        "label": "Screen target",
        "x": x.round(),
        "y": y.round(),
        "w": w.round(),
        "h": h.round()
    }))
}

#[allow(dead_code)]
pub fn extract_highlights(raw: &str, elements: &[Value]) -> (String, Vec<Value>) {
    extract_highlights_with_frame(raw, elements, None)
}

pub fn extract_highlights_with_frame(
    raw: &str,
    elements: &[Value],
    window_frame: Option<&Value>,
) -> (String, Vec<Value>) {
    let mut highlights = Vec::new();
    for cap in POINT_TAG.captures_iter(raw) {
        let target = cap[1].trim();
        let note = cap.get(2).map(|m| m.as_str().trim()).unwrap_or("");
        let found = elements
            .iter()
            .find(|el| {
                el["id"]
                    .as_str()
                    .is_some_and(|id| id.eq_ignore_ascii_case(target))
            })
            .or_else(|| {
                let needle = target.to_lowercase();
                elements.iter().find(|el| {
                    el["label"]
                        .as_str()
                        .is_some_and(|lbl| lbl.to_lowercase() == needle)
                })
            })
            .or_else(|| {
                let needle = target.to_lowercase();
                elements.iter().find(|el| {
                    el["label"]
                        .as_str()
                        .is_some_and(|lbl| lbl.to_lowercase().contains(&needle))
                })
            })
            .cloned()
            .or_else(|| parse_visual_box(target, window_frame));
        if let Some(el) = found {
            let label = if el["label"] == "Screen target" && !note.is_empty() {
                note.to_owned()
            } else {
                el["label"].as_str().unwrap_or("Control").to_owned()
            };
            highlights.push(json!({
                "id": el["id"],
                "role": el["role"],
                "label": label,
                "note": note,
                "region": el["region"],
                "x": el["x"],
                "y": el["y"],
                "w": el["w"],
                "h": el["h"]
            }));
        }
    }
    let cleaned = POINT_TAG
        .replace_all(raw, "")
        .replace(" .", ".")
        .replace(" ,", ",")
        .replace("  ", " ")
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned();
    (cleaned, highlights)
}

fn guidance_tokens(text: &str, app_name: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "where", "can", "could", "would", "should", "what", "which", "when", "how", "see", "find",
        "show", "point", "locate", "open", "view", "access", "check", "click", "press", "tap",
        "button", "menu", "tab", "icon", "option", "section", "panel", "screen", "window", "page",
        "app", "here", "there", "this", "that", "the", "for", "with", "from", "into", "onto",
        "about", "under", "inside", "your", "you", "my", "our", "me", "in", "on", "at", "to", "of",
        "is", "are", "do", "does", "did", "all", "get", "go",
    ];
    let stop: HashSet<&str> = STOP.iter().copied().collect();
    let app_lower = app_name.to_lowercase();
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3 && !stop.contains(w) && *w != app_lower)
        .map(str::to_owned)
        .collect()
}

/// Deterministic fallback matcher: when a user asks a visual guidance question
/// ("where can i see my automations?") and a smaller local model forgets to emit
/// `[[point:E#]]`, match the exact visible UI label on screen so Beside ALWAYS points.
pub fn resolve_highlights(
    question: &str,
    raw_answer: &str,
    elements: &[Value],
    window_frame: Option<&Value>,
    app_name: &str,
) -> (String, Vec<Value>) {
    let (cleaned, mut highlights) =
        extract_highlights_with_frame(raw_answer, elements, window_frame);
    if !highlights.is_empty() || elements.is_empty() {
        return (cleaned, highlights);
    }

    let q_tokens = guidance_tokens(question, app_name);
    let quoted: Vec<String> = QUOTED_LABEL
        .captures_iter(&cleaned)
        .map(|c| c[1].trim().to_lowercase())
        .filter(|s| s.len() >= 2 && s.to_lowercase() != app_name.to_lowercase())
        .collect();

    let mut best: Option<(i32, &Value)> = None;
    for el in elements {
        let Some(lbl_raw) = el["label"].as_str() else {
            continue;
        };
        let lbl = lbl_raw.trim().to_lowercase();
        if lbl.len() < 2 || lbl == app_name.to_lowercase() {
            continue;
        }
        let mut score: i32 = 0;

        for q in &quoted {
            if lbl == *q {
                score += 120;
            } else if lbl.contains(q.as_str()) || q.contains(lbl.as_str()) {
                score += 70;
            }
        }
        for tok in &q_tokens {
            if lbl == *tok {
                score += 110;
            } else if lbl.split(|c: char| !c.is_alphanumeric()).any(|w| w == tok) {
                score += 80;
            } else if lbl.contains(tok.as_str()) && tok.len() >= 4 {
                score += 45;
            }
        }
        if score > 0 {
            // Prefer compact UI controls (sidebar/toolbar buttons) over long prose lines
            if lbl.len() <= 24 {
                score += 18;
            } else if lbl.len() > 40 {
                score -= 20;
            }
            if matches!(
                el["region"].as_str(),
                Some(
                    "left sidebar" | "top bar" | "top-left bar" | "top-right bar" | "right sidebar"
                )
            ) {
                score += 15;
            }
            if best.is_none_or(|(b, _)| score > b) {
                best = Some((score, el));
            }
        }
    }

    if let Some((score, el)) = best.filter(|(s, _)| *s >= 60) {
        let label = el["label"].as_str().unwrap_or("Control");
        let region = el["region"].as_str().unwrap_or("");
        let note = if region.is_empty() || region == "window" {
            String::new()
        } else {
            format!("In the {region}")
        };
        let _ = score;
        highlights.push(json!({
            "id": el["id"],
            "role": el["role"],
            "label": label,
            "note": note,
            "region": el["region"],
            "x": el["x"],
            "y": el["y"],
            "w": el["w"],
            "h": el["h"]
        }));
    }

    (cleaned, highlights)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_need_actual_pixels_and_visible_ids_keep_their_order() {
        let snap = json!({"window":{"x":100,"y":35,"w":600,"h":420}});
        let raw = "First, choose a tab. [[point:10,12,24,15|First tab]]";
        assert!(extract_highlights_with_frame(
            raw,
            &[],
            visual_frame(&snap, &json!({"usedImage":false}))
        )
        .1
        .is_empty());
        assert_eq!(
            extract_highlights_with_frame(
                raw,
                &[],
                visual_frame(&snap, &json!({"usedImage":true}))
            )
            .1
            .len(),
            1
        );
        let controls = vec![
            json!({"id":"E1","label":"Review","x":1,"y":1,"w":20,"h":20}),
            json!({"id":"E2","label":"Summary","x":2,"y":2,"w":20,"h":20}),
        ];
        let (_, steps) = extract_highlights_with_frame(
            "Read both. [[point:E2|Start]] [[point:E1|Finish]]",
            &controls,
            None,
        );
        assert_eq!(
            steps.iter().map(|v| &v["label"]).collect::<Vec<_>>(),
            vec!["Summary", "Review"]
        );
    }

    #[test]
    fn parses_point_tags_by_id_and_label_and_strips_markup() {
        let elements = vec![
            json!({"id": "E1", "role": "button", "label": "Export", "x": 120.0, "y": 40.0, "w": 64.0, "h": 28.0}),
            json!({"id": "E2", "role": "checkbox", "label": "Include notes", "x": 120.0, "y": 82.0, "w": 110.0, "h": 22.0}),
        ];
        let raw = "Click Export in the top bar [[point:E1|Save as PDF]], then enable Include notes [[point:Include notes|Keep comments]].";
        let (clean, hits) = extract_highlights(raw, &elements);
        assert_eq!(
            clean,
            "Click Export in the top bar, then enable Include notes."
        );
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0]["id"], "E1");
        assert_eq!(hits[0]["note"], "Save as PDF");
        assert_eq!(hits[1]["id"], "E2");
        assert_eq!(hits[1]["x"], 120.0);
    }

    #[test]
    fn maps_visual_percent_coordinates_when_accessibility_elements_are_absent() {
        let frame = json!({"x": 100.0, "y": 50.0, "w": 1000.0, "h": 800.0});
        let raw = "Click the Share button in the upper right [[point:85,5,10,5|Share]].";
        let (clean, hits) = extract_highlights_with_frame(raw, &[], Some(&frame));
        assert_eq!(clean, "Click the Share button in the upper right.");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["x"], 950.0);
        assert_eq!(hits[0]["y"], 90.0);
        assert_eq!(hits[0]["w"], 100.0);
        assert_eq!(hits[0]["h"], 40.0);
    }

    #[test]
    fn detects_visual_guidance_questions() {
        assert!(wants_guidance("How do I export this file?"));
        assert!(wants_guidance("Show me where the mute button is"));
        assert!(wants_guidance(
            "where can i see my automations in antigravity?"
        ));
        assert!(!wants_guidance("Summarize this email"));
    }

    #[test]
    fn resolves_visible_ui_target_even_without_explicit_point_tag() {
        let elements = vec![
            json!({"id": "E1", "role": "ui", "label": "New Conversation", "region": "left sidebar", "x": 24.0, "y": 72.0, "w": 120.0, "h": 24.0}),
            json!({"id": "E2", "role": "ui", "label": "Automations", "region": "left sidebar", "x": 24.0, "y": 108.0, "w": 96.0, "h": 22.0}),
            json!({"id": "E3", "role": "ui", "label": "Customizations", "region": "left sidebar", "x": 24.0, "y": 140.0, "w": 110.0, "h": 22.0}),
        ];
        let (_, hits) = resolve_highlights(
            "where can i see my automations in antigravity?",
            "Click \"Automations\" in the left sidebar.",
            &elements,
            None,
            "Antigravity",
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["id"], "E2");
        assert_eq!(hits[0]["label"], "Automations");
        assert_eq!(hits[0]["x"], 24.0);
    }
}
