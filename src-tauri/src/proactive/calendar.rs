use super::*;
use std::ffi::{c_char, CStr};

const LOOKAHEAD_SECONDS: i64 = 20 * 60;
const START_GRACE_SECONDS: i64 = 3 * 60;

pub(super) fn permitted() -> bool {
    unsafe { beside_calendar_permitted() == 1 }
}
extern "C" {
    fn beside_calendar_permitted() -> i32;
    fn beside_calendar_allow() -> i32;
    fn beside_meetings_between(earliest: i64, until: i64) -> *mut c_char;
    fn sumi_free(p: *mut c_char);
}
pub(super) fn authorize() -> Res<()> {
    if unsafe { beside_calendar_allow() } == 1 {
        Ok(())
    } else {
        Err("Allow Calendar in System Settings to use meeting briefs.".into())
    }
}
fn upcoming(now: i64) -> Option<Vec<Value>> {
    unsafe {
        let p = beside_meetings_between(now - START_GRACE_SECONDS, now + LOOKAHEAD_SECONDS + 1);
        if p.is_null() {
            return None;
        }
        let row = serde_json::from_slice(CStr::from_ptr(p).to_bytes()).ok();
        sumi_free(p);
        row
    }
}
pub(super) fn candidate(
    source: &str,
    app: &str,
    now: i64,
    wanted_id: Option<&str>,
) -> Option<Value> {
    select(&upcoming(now)?, source, app, now, wanted_id)
}

fn select(
    events: &[Value],
    source: &str,
    app: &str,
    now: i64,
    wanted_id: Option<&str>,
) -> Option<Value> {
    events.iter().find_map(|event| {
        let start = event["start"].as_i64()?;
        let end = event["end"].as_i64()?;
        let delta = start.checked_sub(now)?;
        if !(-START_GRACE_SECONDS..=LOOKAHEAD_SECONDS).contains(&delta)
            || end <= now
            || end <= start
        {
            return None;
        }
        // Keep the same opaque identifier as existing offers, including recurring occurrences.
        let event_id = event.get("id").filter(|id| id.is_string())?;
        let id = format!("meeting:{}", hash(&format!("{event_id}:{start}")));
        if wanted_id.is_some_and(|wanted| wanted != id) {
            return None;
        }
        let title = event["title"].as_str()?;
        let (badge, when) = match delta {
            0 => ("Starting now", "is starting now".to_owned()),
            1..=60 => ("Meeting soon", "starts in 1 minute".to_owned()),
            61.. => (
                "Meeting soon",
                format!("starts in {} minutes", (delta + 59) / 60),
            ),
            -59..=-1 => ("Meeting started", "started just now".to_owned()),
            -119..=-60 => ("Meeting started", "started 1 minute ago".to_owned()),
            _ => (
                "Meeting started",
                format!("started {} minutes ago", -delta / 60),
            ),
        };
        Some(json!({"id":id,"kind":"meeting","badge":badge,"title":title,
                    "detail":format!("{title} {when}."),"app":app,"source":source,"createdAt":now}))
    })
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
        n["source"].as_str().unwrap_or(""),
        n["app"].as_str().unwrap_or(""),
        Utc::now().timestamp(),
        Some(n["id"].as_str().ok_or("That meeting brief has expired.")?),
    )
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
    fn meeting_offer_tracks_its_own_start_and_end() {
        let events = [
            json!({"id":"private-calendar-row","start":1600,"end":3400,"title":"Editorial handover"}),
        ];
        let at = |now| select(&events, "display", "Notes", now, None);
        let offer = at(1000).unwrap();
        assert_eq!(offer["detail"], "Editorial handover starts in 10 minutes.");
        assert!(!offer.to_string().contains("private-calendar-row"));
        assert!(at(399).is_none());
        assert!(at(400).is_some());
        assert_eq!(
            at(1540).unwrap()["detail"],
            "Editorial handover starts in 1 minute."
        );
        assert_eq!(
            at(1600).unwrap()["detail"],
            "Editorial handover is starting now."
        );
        assert_eq!(
            at(1605).unwrap()["detail"],
            "Editorial handover started just now."
        );
        assert_eq!(
            at(1780).unwrap()["detail"],
            "Editorial handover started 3 minutes ago."
        );
        assert!(at(1781).is_none());
        let short = [json!({"id":"short","title":"Quick call","start":1600,"end":1610})];
        assert!(select(&short, "display", "Notes", 1609, None).is_some());
        assert!(select(&short, "display", "Notes", 1610, None).is_none());
    }

    #[test]
    fn click_after_start_revalidates_the_offered_occurrence_among_overlapping_meetings() {
        let meeting =
            json!({"id":"recurring","title":"Editorial handover","start":1600,"end":3400});
        let offer = select(
            std::slice::from_ref(&meeting),
            "display",
            "Notes",
            1540,
            None,
        )
        .unwrap();
        let wanted = offer["id"].as_str();
        let other = json!({"id":"recurring","title":"Separate occurrence","start":1590,"end":1800});
        assert!(select(
            std::slice::from_ref(&other),
            "display",
            "Notes",
            1605,
            wanted
        )
        .is_none());
        let clicked = select(&[other, meeting], "display", "Notes", 1605, wanted).unwrap();
        assert_eq!(clicked["id"], offer["id"]);
        assert_eq!(clicked["detail"], "Editorial handover started just now.");
    }
}
