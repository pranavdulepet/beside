// Store evidence pointers alongside the captured booking. All wording and values
// are read back from their originals under the current observation permissions.
use super::*;
use regex::Regex;
use rusqlite::{Connection, OptionalExtension};
use std::sync::LazyLock;

static ASK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)\b(?:do (?:i|we) need|should (?:i|we) (?:buy|add|check)|do (?:i|we) have to (?:buy|add))\b.{0,24}\b(?:extra|another|checked|additional) (?:bag|baggage|luggage)\b").unwrap()
});
static LIMIT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)^\s*(?:(?:your|included|one|1) )?(?:carry[- ]on|cabin bag)(?: baggage| bag| allowance| weight limit)?\s*(?:[:·—-]\s*|up to\s+|maximum\s+|limit\s+)([0-9]{1,2}(?:\.[0-9])?)\s*kg(?:\s*(?:max(?:imum)?|per bag))?[.!]?\s*$").unwrap()
});
static WEIGHED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)^\s*(?:my|our) (?:packed |reweighed )?(?:carry[- ]on|bag|suitcase) (?:weighs|weighed|is now) ([0-9]{1,2}(?:\.[0-9])?)\s*kg[.!]?\s*$").unwrap()
});
static REPLY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)^\s*(?:(?:it weighs|it is|it's|it’s|i weighed it[,:]?)\s+)?([0-9]{1,2}(?:\.[0-9])?)\s*(?:kg|kilograms?)[.!]?\s*$").unwrap()
});

fn grams(text: &str) -> Option<i32> {
    let (kg, tenth) = text.split_once('.').unwrap_or((text, "0"));
    let g = kg.parse::<i32>().ok()? * 1000 + tenth.parse::<i32>().ok()? * 100;
    (g > 0).then_some(g)
}
fn kg(g: i32) -> String {
    if g % 1000 == 0 {
        (g / 1000).to_string()
    } else {
        format!("{}.{}", g / 1000, (g % 1000) / 100)
    }
}
fn one(body: &str, pattern: &Regex) -> Option<(i32, String)> {
    let mut hits = body.lines().filter_map(|l| {
        let c = pattern.captures(l)?;
        Some((grams(&c[1])?, l.trim().to_owned()))
    });
    let first = hits.next()?;
    hits.all(|h| h.0 == first.0).then_some(first)
}
// Mixed fare classes, passengers or units need the user to open a single rule.
fn allowance(body: &str) -> Option<(i32, String)> {
    for line in body.lines() {
        let text = line.to_lowercase();
        if ["carry-on", "carry on", "cabin bag"]
            .iter()
            .any(|v| text.contains(v))
            && ["kg", "kilogram", " lb", "pound"]
                .iter()
                .any(|v| text.contains(v))
            && !LIMIT.is_match(line)
        {
            return None;
        }
    }
    one(body, &LIMIT)
}

pub(super) fn booking(title: &str, body: &str) -> bool {
    let title = title.to_lowercase();
    let body = body.to_lowercase();
    [
        "booking",
        "reservation",
        "flight confirmation",
        "flight itinerary",
        "manage trip",
        "check-in",
    ]
    .iter()
    .any(|t| title.contains(t))
        && ["carry-on", "carry on", "cabin bag", "baggage allowance"]
            .iter()
            .any(|t| body.contains(t))
        && !["notes ·", "chatgpt", "claude", "beside ·"]
            .iter()
            .any(|t| title.contains(t))
}

pub(super) fn prepare(
    db: &Connection,
    key: &str,
    title: &str,
    body: &str,
    stamp: i64,
    cfg: &Value,
) -> Res<()> {
    if !booking(title, body) {
        return Ok(());
    }
    let context = format!(" {} ", words(&format!("{title} {body}")));
    let mut q=db.prepare("SELECT id,title,source FROM activity WHERE (title LIKE 'Notes · %' OR title LIKE '% · TextEdit active') AND (title LIKE '%packing list%' OR title LIKE '%packing checklist%') AND stamp>=?1 ORDER BY stamp DESC,rowid DESC LIMIT 120").msg()?;
    let mut rows = q.query([stamp - 30 * DAY]).msg()?;
    let mut found = std::collections::BTreeMap::new();
    let mut consider = |id: String, title: String| {
        let name = title.trim_start_matches("Notes · ");
        let name = name
            .strip_suffix(" · TextEdit active")
            .and_then(|s| s.split_once(" · ").map(|(_, t)| t))
            .unwrap_or(name);
        let n = words(
            name.trim_end_matches(".txt")
                .trim_end_matches(".md")
                .trim_end_matches(".rtf"),
        );
        let scope = n
            .replace("packing checklist", "")
            .replace("packing list", "")
            .trim()
            .to_owned();
        if scope.len() >= 3
            && !["my", "our", "travel", "the", "new"].contains(&scope.as_str())
            && context.contains(&format!(" {scope} "))
        {
            found.entry(n).or_insert(id);
        }
    };
    while let Some(r) = rows.next().msg()? {
        if allowed(cfg, &r.get::<_, String>(2).msg()?) {
            consider(r.get(0).msg()?, r.get(1).msg()?);
        }
    }
    let mut q=db.prepare("SELECT id,title FROM imported_passages WHERE kind='note' AND (title LIKE '%packing list%' OR title LIKE '%packing checklist%') LIMIT 250").msg()?;
    let mut rows = q.query([]).msg()?;
    while let Some(r) = rows.next().msg()? {
        consider(r.get(0).msg()?, r.get(1).msg()?);
    }
    let note = (found.len() == 1).then(|| found.into_values().next().unwrap());
    db.execute(
        "INSERT OR REPLACE INTO prepared_baggage(booking,note) VALUES(?,?)",
        params![key, note],
    )
    .msg()?;
    Ok(())
}
// A list can arrive after the booking. Refresh the handful of recent booking
// references in the same transaction as the source change.
pub(super) fn refresh_recent(db: &Connection, cfg: &Value, now: i64) -> Res<()> {
    let mut q=db.prepare("SELECT a.id,a.title,a.body FROM prepared_baggage p JOIN activity a ON a.id=p.booking WHERE a.stamp>=? AND NOT EXISTS(SELECT 1 FROM activity b WHERE b.source=a.source AND b.title=a.title AND b.rowid>a.rowid) ORDER BY a.stamp DESC,a.rowid DESC LIMIT 12").msg()?;
    let rows = q
        .query_map([now - 1800], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .msg()?
        .collect::<Result<Vec<_>, _>>()
        .msg()?;
    drop(q);
    for (id, title, body) in rows {
        prepare(db, &id, &title, &body, now, cfg)?;
    }
    Ok(())
}

fn words(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

// Resolve a tap against the captured window before the panel takes focus. A
// changed fare needs a fresh captured passage, even when its tab title is stable.
pub(crate) fn pin(s: &Store, point: &Value, generation: &Value) -> Res<Option<String>> {
    let cfg = config(s)?;
    let src = point["source"].as_str().unwrap_or("");
    if cfg["generation"] != *generation || !allowed(&cfg, src) {
        return Ok(None);
    }
    let title = point["bookingTitle"]
        .as_str()
        .or(point["pageTitle"].as_str())
        .unwrap_or("");
    if title.is_empty() {
        return Ok(None);
    }
    let live = point["bookingBody"].as_str();
    let now = Utc::now().timestamp();
    let screen_suffix = format!(
        " · {} · {} active",
        title,
        point["app"].as_str().unwrap_or("")
    );
    let hit=s.db.lock().unwrap().query_row("SELECT a.id,a.body,coalesce(n.title,i.title) FROM activity a JOIN prepared_baggage p ON p.booking=a.id LEFT JOIN activity n ON n.id=p.note LEFT JOIN imported_passages i ON i.id=p.note WHERE a.source=? AND (a.title=? OR substr(a.title,-length(?))=?) AND a.stamp>=? AND NOT EXISTS(SELECT 1 FROM activity newer WHERE newer.source=a.source AND newer.title=a.title AND newer.rowid>a.rowid) ORDER BY a.stamp DESC,a.rowid DESC LIMIT 1",
        params![src,title,screen_suffix,screen_suffix,now-if live.is_some(){1800}else{15}],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?))).optional().msg()?;
    if let Some((id, body, note)) = hit {
        let trip_matches = note.as_deref().is_some_and(|n| {
            let name = n
                .trim_start_matches("Notes · ")
                .strip_suffix(" · TextEdit active")
                .and_then(|v| v.split_once(" · ").map(|(_, t)| t))
                .unwrap_or(n.trim_start_matches("Notes · "));
            let scope = words(name)
                .split("packing")
                .next()
                .unwrap_or("")
                .trim()
                .to_owned();
            !scope.is_empty()
                && live.is_some_and(|b| format!(" {} ", words(b)).contains(&format!(" {scope} ")))
        });
        if live.is_none()
            || live.is_some_and(|v| {
                body == v
                    || (trip_matches && allowance(v).is_some() && allowance(v) == allowance(&body))
            })
        {
            return Ok(Some(id));
        }
    }
    let Some(body) = live.filter(|b| booking(title, b)) else {
        return Ok(None);
    };
    ingest(
        s,
        &cfg,
        &json!({"source":src,"title":title,"body":body}),
        now,
    )?;
    s.db.lock().unwrap().query_row("SELECT p.booking FROM prepared_baggage p JOIN activity a ON a.id=p.booking WHERE a.source=? AND a.title=? AND a.body=? ORDER BY a.stamp DESC,a.rowid DESC LIMIT 1",params![src,title,body.chars().take(5600).collect::<String>()],|r|r.get(0)).optional().msg()
}
pub(super) fn answer(s: &Store, a: &Value, q: &str, epoch: &Value) -> Res<Option<Value>> {
    let weight_reply = REPLY.captures(q).and_then(|c| grams(&c[1]));
    let follow = a["baggageAwaitingWeight"] == true
        && weight_reply.is_some()
        && a["previous"].as_str().is_some_and(|p| ASK.is_match(p))
        && a["epoch"] == *epoch;
    if !ASK.is_match(q) && !follow {
        return Ok(None);
    }
    let _guard = s.activity_edit.lock().unwrap();
    let cfg = config(s)?;
    let pointed = a["gesture"]
        .as_str()
        .map(|id| crate::point::target(s, id))
        .transpose()?;
    let db = s.db.lock().unwrap();
    let fallback = || json!({"answer":"Open your booking’s baggage allowance so I can check the limit for this fare.","epoch":epoch,"sources":[]});
    if cfg["enabled"] != true {
        return Ok(Some(fallback()));
    }
    let prior = a["sources"]
        .as_array()
        .and_then(|v| v.first())
        .and_then(Value::as_str);
    let key = pointed
        .as_ref()
        .map(|p| p["bookingId"].as_str().unwrap_or(""))
        .or(a["anchor"].as_str())
        .or(if follow { prior } else { None })
        .unwrap_or("");
    let row=db.query_row("SELECT a.id,a.source,a.title,a.body,a.stamp,p.note FROM prepared_baggage p JOIN activity a ON a.id=p.booking WHERE p.booking=? AND a.stamp>=? AND NOT EXISTS(SELECT 1 FROM activity b WHERE b.source=a.source AND b.title=a.title AND b.rowid>a.rowid)",params![key,Utc::now().timestamp()-1800],|r|Ok((event(r)?,r.get::<_,Option<String>>(5)?))).optional().msg()?;
    let Some((b, note)) = row else {
        return Ok(Some(fallback()));
    };
    if !allowed(&cfg, b["source"].as_str().unwrap_or("")) {
        return Ok(Some(fallback()));
    }
    let mut sources =
        vec![json!({"id":key,"title":b["title"],"label":"Booking","stamp":b["stamp"]})];
    let note = if let Some(id) = note {
        if id.starts_with("apple-note:") {
            db.query_row(
                "SELECT title,body FROM imported_passages WHERE kind='note' AND id=?",
                [&id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()
            .msg()?
            .map(|(t, b)| (id, t, b, None))
        } else {
            db.query_row("SELECT a.title,a.body,a.stamp,a.source FROM activity a WHERE a.id=? AND NOT EXISTS(SELECT 1 FROM activity newer WHERE newer.title=a.title AND newer.source=a.source AND newer.rowid>a.rowid)",[&id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?))).optional().msg()?
              .filter(|(_,_,_,src)|allowed(&cfg,src)).map(|(t,b,ts,_)|(id,t,b,Some(ts)))
        }
    } else {
        None
    };
    if let Some((id, title, _, stamp)) = &note {
        sources.push(json!({"id":id,"title":title,"label":"Notes","stamp":stamp}));
    }
    let Some((limit, line)) = allowance(b["body"].as_str().unwrap_or("")) else {
        return Ok(Some(
            json!({"answer":"I couldn’t verify one carry-on weight limit for this fare. Open the baggage allowance for your ticket.","epoch":epoch,"sources":sources}),
        ));
    };
    let weight = weight_reply
        .filter(|_| follow && prior == Some(key))
        .or_else(|| {
            note.as_ref()
                .and_then(|(_, _, body, _)| one(body, &WEIGHED).map(|h| h.0))
        });
    let answer=match weight {
        Some(w) if w>limit=>format!("Your bag is {} kg over the {} kg carry-on limit. Remove at least {} kg to meet the weight limit.",kg(w-limit),kg(limit),kg(w-limit)),
        Some(w)=>format!("At {} kg, your bag is within the {} kg carry-on limit. You don’t need extra baggage on weight grounds.",kg(w),kg(limit)),
        None if note.is_some()=>format!("Your fare allows {} kg. I found your packing list, but couldn’t verify a packed weight. What does your packed carry-on weigh?",kg(limit)),
        None=>format!("Your fare allows {} kg. What does your packed carry-on weigh?",kg(limit)),
    };
    Ok(Some(
        json!({"answer":answer,"sources":sources,"supportingLine":line,"baggageAwaitingWeight":weight.is_none(),"epoch":epoch}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn booking_prepares_only_the_matching_list_and_checks_changes() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::with_key(temp.path().join("book"), &[9; 32]).unwrap();
        super::super::init(&store).unwrap();
        let cfg = json!({"enabled":true,"scope":"displays","generation":"test"});
        store.set("activitySettings", &cfg).unwrap();
        let now = Utc::now().timestamp();
        let record = |title: &str, body: &str| {
            ingest(
                &store,
                &cfg,
                &json!({"source":"display","title":title,"body":body}),
                now,
            )
            .unwrap();
        };
        record("Notes · Alps packing list", "My packed bag weighs 14 kg");
        record(
            "Screen 1 · Japan packing list.txt · TextEdit active",
            "Japan packing list\nRain shell\nCharger",
        );
        let title = "Your booking";
        let screen = "Japan\nCarry-on: 7 kg";
        record(title, screen);
        let id: String = store
            .db
            .lock()
            .unwrap()
            .query_row("SELECT booking FROM prepared_baggage", [], |r| r.get(0))
            .unwrap();
        let point =
            json!({"source":"display","bookingTitle":title,"bookingBody":screen,"app":"Browser"});
        assert_eq!(
            pin(&store, &point, &cfg["generation"]).unwrap().as_ref(),
            Some(&id)
        );
        let epoch = store.get("activityEpoch").unwrap();
        let q = "Do I need an extra bag?";
        let args = json!({"anchor":id});
        let first = answer(&store, &args, q, &epoch).unwrap().unwrap();
        assert!(first["answer"]
            .as_str()
            .unwrap()
            .contains("found your packing list"));
        assert!(first["answer"].as_str().unwrap().contains("What does"));
        assert!(first["sources"][1]["title"]
            .as_str()
            .unwrap()
            .contains("Japan"));
        let follow = json!({"anchor":id,"previous":q,"baggageAwaitingWeight":true,"epoch":epoch,"sources":[id]});
        assert!(
            answer(&store, &follow, "8.4 kg", &epoch).unwrap().unwrap()["answer"]
                .as_str()
                .unwrap()
                .contains("1.4 kg over")
        );
        record(
            "Screen 1 · Japan packing list.txt · TextEdit active",
            "Japan packing list\nMy packed bag weighs 6.6 kg",
        );
        let updated = answer(&store, &args, q, &epoch).unwrap().unwrap();
        assert!(updated["answer"].as_str().unwrap().contains("6.6 kg"));
        record(title, "Japan\nNew booking; baggage allowance pending");
        assert!(
            !answer(&store, &args, q, &epoch).unwrap().unwrap()["answer"]
                .as_str()
                .unwrap()
                .contains("7 kg")
        );
        assert!(one("Carry-on: 7 kg\nCarry-on: 10 kg", &LIMIT).is_none());
        assert!(one("My sister’s bag weighs 9 kg", &WEIGHED).is_none());
        assert!(allowance("Carry-on: 7 kg\nPremium carry-on: 10 kg").is_none());
    }
}
