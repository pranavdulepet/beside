use super::*;
use std::ffi::{c_char, CStr};

pub(super) fn permitted() -> bool {
    unsafe { beside_calendar_permitted() == 1 }
}
extern "C" {
    fn beside_calendar_permitted() -> i32;
    fn beside_calendar_allow() -> i32;
    fn beside_next_meeting() -> *mut c_char;
    fn sumi_free(p: *mut c_char);
}
pub(super) fn authorize() -> Res<()> {
    if unsafe { beside_calendar_allow() } == 1 {
        Ok(())
    } else {
        Err("Allow Calendar in System Settings to use meeting briefs.".into())
    }
}
fn upcoming() -> Option<Value> {
    unsafe {
        let p = beside_next_meeting();
        if p.is_null() {
            return None;
        }
        let row = serde_json::from_slice(CStr::from_ptr(p).to_bytes()).ok();
        sumi_free(p);
        row
    }
}
pub(super) fn candidate(_s: &Store, source: &str, app: &str, now: i64) -> Option<Value> {
    from_event(&upcoming()?, source, app, now)
}
fn from_event(event: &Value, source: &str, app: &str, now: i64) -> Option<Value> {
    let start = event["start"].as_i64()?;
    let title = event["title"].as_str()?;
    if !(0..=1200).contains(&(start - now)) {
        return None;
    }
    Some(
        json!({"id":format!("meeting:{}",hash(&format!("{}:{start}",event["id"]))),"kind":"meeting","badge":"Meeting soon",
        "title":title,"detail":format!("{title} starts in {} minutes.",(start-now+59)/60),"app":app,"source":source,"createdAt":now}),
    )
}
pub(super) fn read(s: &Store, args: &Value) -> Res<Value> {
    let cfg = settings(s)?;
    let n = latest(s);
    if cfg["enabled"] != true
        || cfg["calendarBriefs"] != true
        || n["kind"] != "meeting"
        || args["id"] != n["id"]
        || args["eventId"] != n["eventId"]
    {
        return Err("That meeting brief has expired.".into());
    }
    let current = candidate(
        s,
        n["source"].as_str().unwrap_or(""),
        n["app"].as_str().unwrap_or(""),
        Utc::now().timestamp(),
    )
    .filter(|v| v["id"] == n["id"])
    .ok_or("That meeting is no longer available.")?;
    let title = current["title"].as_str().unwrap_or("");
    let words = content_words(title);
    let terms: Vec<_> = words.iter().cloned().collect();
    let mut brief = current["detail"].as_str().unwrap_or(title).to_owned();
    if !terms.is_empty() {
        let mut lines = Vec::new();
        for row in crate::activity::imported::candidates(s, &terms, &terms)? {
            let name = row["title"].as_str().unwrap_or("");
            if let Some((score, line)) =
                note_passage(name, row["body"].as_str().unwrap_or(""), &words, &words, "")
            {
                lines.push((score, format!("{name} — {line}")));
            }
        }
        lines.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        for (_, line) in lines.into_iter().take(2) {
            brief.push_str("\n\n");
            brief.push_str(&line);
        }
    }
    if latest(s)["id"] != n["id"] || settings(s)?["calendarBriefs"] != true {
        return Err("That meeting brief has expired.".into());
    }
    Ok(json!({"text":brief}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn meeting_offer_is_time_bound_and_keeps_the_calendar_identifier_local() {
        let event = json!({"id":"private-calendar-row","start":1600,"title":"Editorial handover"});
        let offer = from_event(&event, "display", "Notes", 1000).unwrap();
        assert_eq!(offer["detail"], "Editorial handover starts in 10 minutes.");
        assert!(!offer.to_string().contains("private-calendar-row"));
        assert!(from_event(&event, "display", "Notes", 1601).is_none());
        assert!(from_event(&event, "display", "Notes", 100).is_none());
    }
}
