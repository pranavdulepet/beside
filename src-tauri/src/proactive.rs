use crate::{conversation, memory, scroll, store::*};
use chrono::Utc;
use regex::Regex;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    sync::{LazyLock, Mutex},
};

#[derive(Default)]
struct Watcher {
    last_nudge_at: i64,
    last_digest: String,
    active: Option<Value>,
    dismissed: HashSet<String>,
}

static WATCHER: LazyLock<Mutex<Watcher>> = LazyLock::new(|| Mutex::new(Watcher::default()));

static ERROR_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:error\[E\d+\]|error:|exception:|typeerror:|referenceerror:|syntaxerror:|panic(?:ked)? at\b|command not found\b|permission denied\b|exit code [1-9]\b|failed to compile\b|404 not found\b|500 internal server error\b|fatal:)")
        .unwrap()
});

pub fn settings(s: &Store) -> Res<Value> {
    let v = s.get("proactiveSettings")?;
    if v.is_object() {
        Ok(v)
    } else {
        Ok(json!({
            "enabled": true,
            "cooldownSeconds": 90,
            "mutedApps": []
        }))
    }
}

pub fn configure(s: &Store, a: &Value) -> Res<Value> {
    let mut cfg = settings(s)?;
    if let Some(enabled) = a["enabled"].as_bool() {
        cfg["enabled"] = json!(enabled);
        if !enabled {
            clear_active();
        }
    }
    if let Some(cd) = a["cooldownSeconds"].as_i64() {
        cfg["cooldownSeconds"] = json!(cd.clamp(15, 1800));
    }
    if let Some(app) = a["muteApp"].as_str().filter(|v| !v.trim().is_empty()) {
        let muted = cfg["mutedApps"]
            .as_array_mut()
            .ok_or("Invalid muted apps.")?;
        if !muted.iter().any(|v| v == app) {
            muted.push(json!(app));
        }
        clear_active();
    }
    if let Some(app) = a["unmuteApp"].as_str() {
        if let Some(muted) = cfg["mutedApps"].as_array_mut() {
            muted.retain(|v| v != app);
        }
    }
    s.set("proactiveSettings", &cfg)?;
    Ok(cfg)
}

pub fn clear_active() {
    if let Ok(mut w) = WATCHER.lock() {
        w.active = None;
    }
}

pub fn follow_front(source: &str, title: &str) {
    if let Ok(mut w) = WATCHER.lock() {
        if w.active
            .as_ref()
            .is_some_and(|v| v["source"] != source || v["originTitle"] != title)
        {
            w.active = None;
        }
    }
}

pub fn dismiss(nudge_id: &str) {
    if let Ok(mut w) = WATCHER.lock() {
        if !nudge_id.is_empty() {
            w.dismissed.insert(nudge_id.to_owned());
            if w.dismissed.len() > 100 {
                w.dismissed.clear();
            }
        }
        if w.active.as_ref().is_some_and(|v| v["id"] == nudge_id) {
            w.active = None;
        }
    }
}

pub fn latest() -> Value {
    let now_ts = Utc::now().timestamp();
    let Ok(mut w) = WATCHER.lock() else {
        return Value::Null;
    };
    if let Some(nudge) = w.active.as_ref() {
        if now_ts - nudge["createdAt"].as_i64().unwrap_or(0) > 240 {
            w.active = None;
            return Value::Null;
        }
        return nudge.clone();
    }
    Value::Null
}

fn app_name(source: &str, title: &str) -> String {
    if let Some((_, app)) = title.rsplit_once(" · ") {
        if let Some(clean) = app.strip_suffix(" active") {
            return clean.to_owned();
        }
    }
    if let Some((prefix, _)) = title.split_once(" · ") {
        if !prefix.starts_with("Screen ") {
            return prefix.to_owned();
        }
    }
    crate::activity::label(source).to_owned()
}

fn find_error_nudge(source: &str, title: &str, body: &str, now_ts: i64) -> Option<Value> {
    let line = body
        .lines()
        .map(str::trim)
        .find(|l| l.len() >= 12 && l.len() <= 240 && ERROR_LINE.is_match(l))?;
    let app = app_name(source, title);
    let key = format!("help:{}", &hash(&format!("{source}:{line}"))[..16]);
    Some(json!({
        "id": key,
        "kind": "help",
        "badge": "Error spotted",
        "title": format!("Issue visible in {app}"),
        "detail": line.chars().take(120).collect::<String>(),
        "app": app,
        "source": source,
        "prompt": format!("Explain this error on my screen and how to resolve it: {}", line.chars().take(240).collect::<String>()),
        "createdAt": now_ts
    }))
}

fn content_words(text: &str) -> HashSet<String> {
    const CHROME: &[&str] = &[
        "screen", "active", "window", "safari", "chrome", "finder", "antigravity",
        "history", "bookmarks", "develop", "personal", "follow", "settings",
        "conversations", "conversation", "projects", "notifications", "explore",
        "messages", "message", "apple", "macos", "today", "tomorrow", "worked", "inbox", "mailbox",
        "compose", "safari", "favorite", "unread", "folder", "archive", "toolbar", "gmail", "google", "search", "october", "november", "december", "january", "february", "march", "april", "august", "september", "saturday", "sunday", "monday", "thursday", "friday", "order",
        "forward", "reply", "account", "thread", "subject", "draft", "chatgpt", "bookmark", "notification",
        "bring", "think", "thing", "suggest", "write", "consider", "review", "include", "detail",
        "there", "above", "below", "first", "other", "share", "during",
    ];
    let chrome: HashSet<&str> = CHROME.iter().copied().collect();
    conversation::words(text)
        .into_iter()
        .filter(|w| w.len() >= 5 && w.chars().any(char::is_alphabetic) && !chrome.contains(w.as_str()))
        .collect()
}

fn clean_nudge_title(source: &str, title: &str) -> String {
    if let Some(rest) = title.strip_prefix("Notes · ").or_else(|| title.strip_prefix("Mail · ")) {
        return rest.to_owned();
    }
    app_name(source, title)
}

/// Keep short names and acronyms from the focused document title. Generic UI
/// words shouldn't turn every document with the same sidebar into a connection.
fn title_terms(title: &str) -> HashSet<String> {
    let title = title.strip_prefix("Screen ").and_then(|t| t.split_once(" · ").map(|(_, t)| t))
        .unwrap_or(title);
    let title = title.rsplit_once(" · ").map_or(title, |(topic, _)| topic);
    conversation::words(title).into_iter().filter(|w| w.len() >= 3 && !matches!(
        w.as_str(), "personal" | "active" | "safari" | "chrome" | "screen" | "chatgpt" | "claude" |
        "mail" | "note" | "message" | "outlook" | "browser" | "idea" | "comparison" | "thing" | "plan" | "window" |
        "explain" | "explanation" | "compare" | "overview" | "summarize" | "summary" | "analyze" |
        "analysis" | "describe" | "introduction" | "understand" | "understanding" | "please" |
        "google" | "new" | "chat" | "untitled" | "conversation" | "temporary"
    )).collect()
}

fn as_read(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn note_passage<'a>(name: &str, body: &'a str, focus: &HashSet<String>, screen: &HashSet<String>, visible: &str) -> Option<(usize, &'a str)> {
    // A short heading can identify the subject of the sentences below it.
    // Reset at the next heading, including headings for unrelated projects.
    let title_hits = conversation::words(name).intersection(focus).count();
    let mut heading_hits = 0;
    let mut best = None;
    for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line.ends_with(':') && line.chars().count() <= 90 {
            heading_hits = conversation::words(line).intersection(focus).count();
            continue;
        }
        for sentence in line.split_inclusive(['.', '!', '?']).map(str::trim) {
            if !(15..=600).contains(&sentence.chars().count()) || visible.contains(&as_read(sentence)) { continue; }
            let words = conversation::words(sentence);
            let shared = content_words(sentence).intersection(screen).count();
            let direct = words.intersection(focus).count();
            let anchor = heading_hits + title_hits;
            if (anchor == 0 && ((!focus.is_empty() && direct == 0) || shared < 3))
                || (anchor != 0 && shared == 0 && direct == 0) { continue; }
            let score = 5 * shared + 7 * direct + 9 * anchor;
            if best.is_none_or(|(prev, _)| score > prev) { best = Some((score, sentence)); }
        }
    }
    best
}

fn profile_records(s: &Store) -> Vec<(&'static str, Value)> {
    let approved = scroll::approved(s).unwrap_or_default().into_iter().map(|v| ("rule", v));
    let current = memory::timeline(s).unwrap_or_default().into_iter().filter(memory::current).map(|v| ("note", v));
    approved.chain(current).collect()
}

pub fn read_profile(s: &Store, request: &Value) -> Res<Value> {
    let nudge = latest();
    let expired = "That memory has changed or this suggestion has expired.";
    if nudge["profileId"].as_str().is_none() || nudge["id"] != request["id"]
        || nudge["eventId"] != request["eventId"] || nudge["epoch"] != s.get("activityEpoch")?
        || !crate::activity::allowed(&crate::activity::config(s)?, nudge["source"].as_str().unwrap_or(""))
    {
        return Err(expired.into());
    }
    profile_records(s).into_iter().find_map(|(kind, record)| {
        let text = record["text"].as_str()?;
        (record["id"] == nudge["profileId"] && nudge["profileKind"] == kind && nudge["digest"] == hash(text))
            .then(|| json!({"text":text,"date":record["learnedAt"].as_str().or(record["date"].as_str())}))
    }).ok_or_else(|| expired.into())
}

fn find_memory_nudge(
    s: &Store,
    event_id: &str,
    source: &str,
    title: &str,
    body: &str,
    now_ts: i64,
) -> Option<Value> {
    let screen_words = content_words(body);
    let visible = as_read(body);
    if screen_words.len() < 4 {
        return None;
    }

    // Compare exact lines from independent Notes and Mail windows. Toolbar words
    // occur across unrelated windows and make poor evidence for a suggestion.
    let scope = crate::activity::config(s).ok()?;
    let mut terms: Vec<_> = screen_words.iter().collect();
    terms.sort_by_key(|w| (std::cmp::Reverse(w.len()), *w));
    let query = terms
        .into_iter()
        .take(48)
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    let saved =
        crate::activity::search_excluding(s, &query, &[], title, "", "", (true, true)).ok()?;
    let mut best: Option<(usize, Value)> = None;
    let mut consider = |name: &str, text: &str, id: &Value, live: bool, mail: bool| {
        if mail
            && ["all inboxes", "all mailboxes", "unread messages"]
                .iter()
                .any(|label| name.to_lowercase().starts_with(label))
        {
            return;
        }
        if content_words(&format!("{name} {text}"))
            .intersection(&screen_words)
            .count()
            < 3
        {
            return;
        }
        for line in text
            .lines()
            .map(str::trim)
            .filter(|l| (18..=420).contains(&l.chars().count()))
        {
            let words = content_words(line);
            let matches = words.intersection(&screen_words).count();
            if matches < 2 || visible.contains(&as_read(line)) {
                continue;
            }
            let score = matches * 4 + usize::from(line.ends_with('.')) + usize::from(live) * 3;
            if best.as_ref().is_some_and(|(b, _)| *b >= score) {
                continue;
            }
            let key = format!(
                "passage:{}",
                &hash(&format!("{source}:{name}:{line}"))[..20]
            );
            best = Some((
                score,
                json!({
                    "id":key,"kind":"memory","badge":if mail {"Related email"} else {"Related note"},
                    "title":name, "detail":line, "sourceId":id, "liveWindow":live,
                    "source":source,"app":app_name(source,title),"createdAt":now_ts,
                    "prompt":"How does this relate to the passage Beside found?"
                }),
            ));
        }
    };
    for h in &saved {
        let other = h["source"].as_str()?;
        let title = h["title"].as_str()?;
        if h["id"] != event_id && crate::activity::allowed(&scope, other) {
            // Older display captures can include other apps behind a Mail window.
            if other == "display" && !title.starts_with("Notes · ") && !title.starts_with("Mail · ")
            {
                continue;
            }
            let mail = other.to_lowercase().contains("mail") || title.starts_with("Mail · ");
            consider(
                &clean_nudge_title(other, title),
                h["body"].as_str()?,
                &h["id"],
                false,
                mail,
            );
        }
    }
    for w in crate::activity::workspace::windows(s) {
        let app = w["app"].as_str().unwrap_or("");
        if w["front"] == true || !matches!(app, "Notes" | "Mail" | "Microsoft Outlook") {
            continue;
        }
        consider(
            w["title"].as_str().unwrap_or(app),
            w["text"].as_str().unwrap_or(""),
            &w["id"],
            true,
            app != "Notes",
        );
    }
    let focus = title_terms(title);
    let mut query_terms: Vec<_> = focus.iter().cloned().collect();
    query_terms.sort();
    let context_terms = query.split_whitespace().take(24).map(str::to_owned).collect::<Vec<_>>();
    // Untitled chats and new browser tabs still expose the current conversation.
    // With no topic in the title, require three terms in the same unseen sentence.
    let lookup = if query_terms.is_empty() { &context_terms } else { &query_terms };
    if !lookup.is_empty() {
        for row in crate::activity::imported::candidates(s, lookup, &context_terms).unwrap_or_default() {
            let name = row["title"].as_str().unwrap_or("");
            let origin = clean_nudge_title(source, title);
            if (matches!(app_name(source, title).as_str(), "Mail" | "Notes" | "Microsoft Outlook")
                && title.to_lowercase().contains(&name.to_lowercase())) || name.eq_ignore_ascii_case(&origin) {
                continue;
            }
            let Some((score, line)) = note_passage(name, row["body"].as_str().unwrap_or(""), &focus, &screen_words, &visible) else { continue };
            if best.as_ref().is_some_and(|(prev, _)| *prev >= score) { continue; }
            best = Some((score, json!({
                "id":format!("passage:{}", &hash(&format!("{}:{title}:{line}",row["id"]))[..20]),
                "kind":"memory", "badge":if row["kind"] == "mail" {"Related email"} else {"Related note"}, "title":name,"detail":line,"sourceId":row["id"],
                "source":source,"app":app_name(source,title),"createdAt":now_ts,
                "prompt":"How does this relate to the passage Beside found?"
            })));
        }
    }
    if let Some((_, passage)) = best {
        return Some(passage);
    }
    // A precise document can answer a question outright. Consider profile
    // reminders only when several distinctive terms tie them to this screen.
    profile_records(s).into_iter().filter_map(|(kind, record)| {
        let text = record["text"].as_str()?;
        let words = content_words(text);
        let shared = words.intersection(&screen_words).count();
        if shared < 3 || record["id"].as_str().is_none() || visible.contains(&as_read(text)) { return None; }
        let digest = hash(text);
        let key = format!("mem:{}", &hash(&format!("{source}:{digest}"))[..16]);
        Some((shared, json!({
            "id":key,"kind":"memory","profileKind":kind,"profileId":record["id"],"digest":digest,
            "badge":"From your memory","title":record["title"].as_str().unwrap_or("Saved memory"),
            "detail":text.chars().take(140).collect::<String>(),"app":app_name(source,title),
            "source":source,"createdAt":now_ts
        })))
    }).max_by_key(|(shared, _)| *shared).map(|(_, v)| v)
}

fn find_reply_nudge(source: &str, title: &str, body: &str, now_ts: i64) -> Option<Value> {
    let lower_title = title.to_lowercase();
    let is_chat = source == "com.apple.MobileSMS"
        || source == "com.apple.mail"
        || lower_title.contains("messages")
        || lower_title.contains("mail")
        || lower_title.contains("slack");
    if !is_chat {
        return None;
    }
    let question = body
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.ends_with('?') && l.len() >= 12 && l.len() <= 180)?;
    let app = app_name(source, title);
    let key = format!("reply:{}", &hash(&format!("{source}:{question}"))[..16]);
    Some(json!({
        "id": key,
        "kind": "reply",
        "badge": "Draft reply",
        "title": format!("Quick reply in {app}"),
        "detail": question.chars().take(110).collect::<String>(),
        "app": app,
        "source": source,
        "prompt": format!("Draft a short, natural reply to this message: {}", question),
        "createdAt": now_ts
    }))
}

pub fn evaluate(
    s: &Store,
    event_id: &str,
    source: &str,
    title: &str,
    body: &str,
    force: bool,
) -> Option<Value> {
    follow_front(source, title);
    let cfg = settings(s).ok()?;
    if !crate::activity::allowed(&crate::activity::config(s).ok()?, source) || cfg["enabled"] != true {
        return None;
    }
    let app = app_name(source, title);
    if cfg["mutedApps"]
            .as_array()
            .is_some_and(|list| list.iter().any(|v| v == source || v == app.as_str()))
    {
        return None;
    }
    let now_ts = Utc::now().timestamp();
    let cooldown = cfg["cooldownSeconds"].as_i64().unwrap_or(90);
    let digest = hash(&format!("{source}\n{title}\n{}", body.chars().take(400).collect::<String>()));

    {
        let w = WATCHER.lock().ok()?;
        if !force {
            if w.last_digest == digest {
                return None;
            }
            if now_ts - w.last_nudge_at < cooldown {
                return None;
            }
        }
    }

    let mut candidate = find_memory_nudge(s, event_id, source, title, body, now_ts)
        .or_else(|| find_error_nudge(source, title, body, now_ts))
        .or_else(|| find_reply_nudge(source, title, body, now_ts))?;

    candidate["epoch"] = s.get("activityEpoch").ok()?;
    candidate["eventId"] = json!(event_id);
    candidate["originTitle"] = json!(title);
    let key = candidate["id"].as_str().unwrap_or("").to_owned();
    let mut w = WATCHER.lock().ok()?;
    if !force && w.dismissed.contains(&key) {
        return None;
    }
    w.last_nudge_at = now_ts;
    w.last_digest = digest;
    w.active = Some(candidate.clone());
    Some(candidate)
}

// Reproduce retrieval on a saved screen without publishing a dot or changing
// its cooldown. Transient windows from the original moment aren't replayable.
pub fn review_saved(s: &Store, topic: &str, through: i64) -> Res<Value> {
    if topic.trim().is_empty() { return Err("Specify part of a saved window title.".into()); }
    let (id, source, title, body, stamp) = s.db.lock().unwrap().query_row(
        "SELECT id,source,title,body,stamp FROM activity WHERE instr(lower(title),lower(?))>0 AND stamp BETWEEN ? AND ? ORDER BY stamp DESC,rowid DESC LIMIT 1",
        rusqlite::params![topic,Utc::now().timestamp()-30*86400,through], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,i64>(4)?))
    ).map_err(|_| "No saved screen with that title.".to_string())?;
    if !crate::activity::allowed(&crate::activity::config(s)?, &source) { return Err("That screen source is disabled.".into()); }
    let found = find_memory_nudge(s,&id,&source,&title,&body,Utc::now().timestamp());
    Ok(json!({"screen":title,"capturedAt":stamp,"importedNotes":crate::activity::imported::count(s,"note")?, "importedMail":crate::activity::imported::count(s,"mail")?,
        "match":found.map(|v|json!({"title":v["title"],"passage":v["detail"]})), "settings":settings(s)?}))
}

pub fn scan_latest(s: &Store) -> Res<Value> {
    let row = {
        let db = s.db.lock().unwrap();
        use rusqlite::OptionalExtension;
        db.query_row(
            "SELECT id, source, title, body FROM activity ORDER BY stamp DESC, rowid DESC LIMIT 1",
            [],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .msg()?
    };
    let Some((eid, source, title, body)) = row else {
        return Ok(Value::Null);
    };
    Ok(evaluate(s, &eid, &source, &title, &body, true).unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_headings_ground_short_names_and_do_not_leak_between_sections() {
        assert!(!title_terms("Screen 1 · Explain transit fares · Safari active").contains("explain"));
        let focus = title_terms("Screen 1 · Personal — Miro options · Safari active");
        let body = "Routing policy affects each regional booking. Clear tokens after validation.";
        let context = content_words(body);
        let note = "Miro:\nRouting policy needs an operator to approve each booking.\nOther:\nRouting policy affects each regional booking.";
        let (_, line) = note_passage("Monday notebook", note, &focus, &context, "").unwrap();
        assert_eq!(line, "Routing policy needs an operator to approve each booking.");
        assert!(note_passage("Monday notebook", "Other:\nRouting policy affects each regional booking.", &focus, &context, "").is_none());
    }

    #[test]
    fn an_untitled_chat_can_find_an_imported_source_without_repeating_whats_already_visible() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::with_key(dir.path().join("book"), &[7; 32]).unwrap();
        crate::activity::init(&s).unwrap();
        crate::activity::configure(&s, &json!({"scope":"displays"})).unwrap();
        crate::activity::configure(&s, &json!({"enabled":true})).unwrap();
        let quote = "For the courtyard screening, bring the projector through the north entrance before 5 pm.";
        s.db.lock().unwrap().execute(
            "INSERT INTO imported_passages VALUES (?,?,?,?)",
            ["apple-note:check", "Site visit", quote, "note"],
        ).unwrap();
        let title = "Screen 1 · ChatGPT active";
        let body = "Plan the courtyard screening and suggest where to set up the projector for the evening.";
        let nudge = find_memory_nudge(&s, "screen", "display", title, body, Utc::now().timestamp()).unwrap();
        assert_eq!(nudge["title"], "Site visit");
        assert_eq!(nudge["detail"], quote);
        assert_eq!(nudge["sourceId"], "apple-note:check");
        let already_answered = format!("{body}\n{}", quote.replace("projector ", "projector\n").to_uppercase());
        assert!(find_memory_nudge(&s, "next", "display", title, &already_answered, Utc::now().timestamp()).is_none());
        let unrelated = "Plan the evening screening and bring a blanket for the projector demonstration.";
        assert!(find_memory_nudge(&s, "other", "display", title, unrelated, Utc::now().timestamp()).is_none());
    }

    #[test]
    fn spots_errors_and_related_notes_and_respects_mute_and_dismiss() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Store::with_key(tmp.path().join("book"), &[4; 32]).unwrap();
        crate::activity::init(&s).unwrap();
        crate::activity::configure(&s, &json!({"scope": "displays"})).unwrap();
        let c = crate::activity::configure(&s, &json!({"enabled": true})).unwrap();
        let now_ts = Utc::now().timestamp();

        crate::activity::ingest(
            &s,
            &c,
            &json!({
                "source": "display",
                "title": "Notes · Gallery installation",
                "body": "Gallery installation\nBring indigo panels to the west entrance loading dock before 4 pm."
            }),
            now_ts - 600,
        )
        .unwrap();

        let memory_id = id();
        s.put("note", &json!({
            "id":memory_id,"topic":"event/loading","validFrom":now(),"learnedAt":now(),
            "text":"Indigo panels need careful loading during delivery."
        })).unwrap();
        let nudge = evaluate(
            &s,
            "live-event",
            "display",
            "Screen 1 · Chrome active",
            "Can you deliver the indigo panels to the loading dock tomorrow?",
            true,
        )
        .expect("should connect current screen to saved gallery note");
        assert_eq!(nudge["kind"], "memory");
        assert_eq!(nudge["title"], "Gallery installation");
        assert!(nudge["sourceId"].is_string());
        let vague = find_memory_nudge(
            &s, "other-event", "display", "Screen 1 · Mail active",
            "Our careful delivery includes flowers, envelopes and postage stamps.", now_ts,
        );
        assert!(vague.is_none());

        let profile = evaluate(
            &s, "reminder", "display", "Screen 1 · Mail active",
            "Our careful delivery needs careful handling during transit, particularly during loading.", true,
        ).expect("a strongly matched saved memory");
        assert_eq!(profile["profileId"], memory_id);
        assert_eq!(read_profile(&s, &profile).unwrap()["text"], "Indigo panels need careful loading during delivery.");
        let mut revised = s.doc("note", &memory_id).unwrap();
        revised["text"] = json!("Ask the delivery team to bring the parcel tomorrow.");
        s.put("note", &revised).unwrap();
        assert!(read_profile(&s, &profile).is_err());

        let err = evaluate(
            &s,
            "err-event",
            "display",
            "Screen 1 · Terminal active",
            "cargo build\nerror[E0425]: cannot find value `foo` in this scope",
            true,
        )
        .expect("should detect compiler error");
        assert_eq!(err["kind"], "help");

        dismiss(err["id"].as_str().unwrap());
        assert!(latest().is_null());
    }
}
