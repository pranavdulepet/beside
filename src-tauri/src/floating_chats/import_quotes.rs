//! Keep verbatim excerpts attached to their import, including in older floating chats.
use super::KIND;
use crate::store::*;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

pub(crate) type Passages<'a> = HashMap<String, (&'a str, &'a str)>;
type Snapshot<'a> = (&'a str, &'a Passages<'a>);
const INLINE_REMOVED: &str = "[quote removed]";
const QUOTE_FIELDS: [&str; 2] = ["quote", "supportingLine"];
static INLINE_QUOTE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r#"“([^”]+)”|"([^"\n]+)"|‘([^’]+)’"#).unwrap());

fn source_id(source: &Value) -> Option<&str> {
    source["id"].as_str().or_else(|| source.as_str())
}

fn import_kind(id: &str) -> Option<&str> {
    if id.starts_with("apple-note:") {
        Some("note")
    } else if id.starts_with("apple-mail:") {
        Some("mail")
    } else {
        None
    }
}

// Early menu and proactive replies kept the title and exact passage but no ID.
// Bind only when both match. If two imports independently support it, retain both.
fn bind_legacy(db: &Connection, turn: &mut Value) -> Res<()> {
    if turn["sources"].as_array().is_some_and(|v| !v.is_empty())
        || turn["files"].as_array().is_some_and(|v| !v.is_empty())
    {
        return Ok(());
    }
    let (Some(title), Some(quote)) = (turn["quoteTitle"].as_str(), turn["quote"].as_str()) else {
        return Ok(());
    };
    if title.is_empty() || quote.trim().is_empty() {
        return Ok(());
    }
    let mut q = db.prepare_cached("SELECT DISTINCT id,kind FROM imported_passages WHERE title=? COLLATE NOCASE AND instr(body,?)>0 ORDER BY id").msg()?;
    let matches = q.query_map(params![title,quote], |r| {
        let kind: String = r.get(1)?;
        Ok(json!({"id":r.get::<_,String>(0)?,"title":title,"label":if kind=="mail" {"Mail"} else {"Notes"},"quote":quote}))
    }).msg()?.collect::<Result<Vec<_>,_>>().msg()?;
    if !matches.is_empty() {
        turn["sources"] = json!(matches);
    }
    Ok(())
}

fn clean_turn(
    db: &Connection,
    turn: &mut Value,
    snapshot: Option<Snapshot<'_>>,
) -> Res<HashSet<String>> {
    bind_legacy(db, turn)?;
    let mut lost = HashSet::new();
    let Some(sources) = turn["sources"].as_array() else {
        return Ok(lost);
    };
    if !sources
        .iter()
        .filter_map(source_id)
        .filter_map(import_kind)
        .any(|kind| snapshot.is_none_or(|(target, _)| target == kind))
    {
        return Ok(lost);
    }
    let mut keep = Vec::new();
    let mut protected = HashSet::new();
    let answer = turn["answer"].as_str().unwrap_or("").trim();
    let quoted: Vec<_> = INLINE_QUOTE
        .captures_iter(answer)
        .filter_map(|c| c.iter().skip(1).flatten().next().map(|m| m.as_str()))
        .chain(
            answer
                .lines()
                .filter_map(|s| s.trim_start().strip_prefix('>').map(str::trim)),
        )
        .filter(|s| !s.is_empty())
        .collect();
    for source in sources {
        let id = source_id(source).unwrap_or("");
        let Some(kind) =
            import_kind(id).filter(|k| snapshot.is_none_or(|(target, _)| target == *k))
        else {
            if let Some(q) = source["quote"].as_str() {
                protected.insert(q.to_owned());
            }
            keep.push(source.clone());
            continue;
        };
        let old: Option<String> = db
            .prepare_cached("SELECT body FROM imported_passages WHERE id=? AND kind=? LIMIT 1")
            .msg()?
            .query_row(params![id, kind], |r| r.get(0))
            .optional()
            .msg()?;
        let current = match snapshot {
            Some((_, next)) => next.get(id).map(|(_, body)| *body),
            None => old.as_deref(),
        };
        let mut quotes = Vec::new();
        if let Some(q) = source["quote"].as_str() {
            quotes.push(q);
        }
        for field in QUOTE_FIELDS {
            if let Some(q) = turn[field].as_str() {
                if (sources.len() == 1 && (source["quote"].as_str().is_none() || old.is_none()))
                    || quotes.contains(&q)
                    || (field == "quote"
                        && source["title"].is_string()
                        && source["title"] == turn["quoteTitle"])
                    || old.as_ref().is_some_and(|body| body.contains(q))
                {
                    quotes.push(q);
                }
            }
        }
        // A saved answer may print the quoted line directly without a citation field.
        if let Some(body) = old.as_deref().filter(|old| Some(*old) != current) {
            quotes.extend(body.lines().map(str::trim).filter(|line| {
                !line.is_empty()
                    && (answer.lines().any(|s| s.trim() == *line)
                        || (line.split_whitespace().count() >= 3 && answer.contains(line)))
            }));
        }
        for &fragment in &quoted {
            if old.as_ref().is_some_and(|b| b.contains(fragment))
                || quotes.iter().any(|q| q.contains(fragment))
            {
                quotes.push(fragment);
            }
        }
        let mut citation_stale = false;
        for q in quotes.into_iter().filter(|q| !q.trim().is_empty()) {
            if current.is_some_and(|body| {
                if Some(body) == old.as_deref() {
                    body.contains(q)
                } else {
                    literal_spans(body, q).next().is_some()
                }
            }) {
                protected.insert(q.to_owned());
            } else {
                lost.insert(q.to_owned());
                if !answer.is_empty() && literal_spans(q, answer).next().is_some() {
                    lost.insert(answer.to_owned());
                }
                citation_stale |= source["quote"].as_str() == Some(q);
            }
        }
        if current.is_some() && !citation_stale {
            keep.push(source.clone());
        }
    }
    lost.retain(|q| {
        !protected
            .iter()
            .any(|kept| literal_spans(kept, q).next().is_some())
    });
    let lost_source = keep.len() != sources.len();
    if lost_source {
        turn["sources"] = Value::Array(keep);
        if let Some(obj) = turn.as_object_mut() {
            obj.remove("mealTask");
            if obj["sources"].as_array().is_some_and(Vec::is_empty)
                || matches!(
                    obj.get("workKind").and_then(Value::as_str),
                    Some("meal-plan" | "shopping" | "meal-choice")
                )
            {
                obj.remove("workKind");
            }
        }
    }
    if let Some(obj) = turn.as_object_mut() {
        for key in QUOTE_FIELDS {
            if obj
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|q| lost.contains(q))
            {
                obj.remove(key);
                if key == "quote" {
                    obj.remove("quoteTitle");
                }
            }
        }
    }
    Ok(lost)
}

fn literal_spans<'a>(text: &'a str, quote: &'a str) -> impl Iterator<Item = (usize, usize)> + 'a {
    text.match_indices(quote)
        .filter_map(move |(start, matched)| {
            let end = start + matched.len();
            let inside_word = (matched.starts_with(char::is_alphanumeric)
                && text[..start].ends_with(char::is_alphanumeric))
                || (matched.ends_with(char::is_alphanumeric)
                    && text[end..].starts_with(char::is_alphanumeric));
            (!inside_word).then_some((start, end))
        })
}

fn redact_answer(turn: &mut Value, quotes: &HashSet<String>) {
    let Some(original) = turn["answer"].as_str() else {
        return;
    };
    let supported: Vec<_> = QUOTE_FIELDS
        .iter()
        .filter_map(|key| turn[*key].as_str())
        .chain(
            ["sources", "webSources"]
                .iter()
                .flat_map(|key| turn[*key].as_array().into_iter().flatten())
                .filter_map(|s| s["quote"].as_str()),
        )
        .collect();
    let mut spans = Vec::new();
    for q in quotes {
        if supported
            .iter()
            .any(|line| literal_spans(line, q).next().is_some())
        {
            continue;
        }
        spans.extend(literal_spans(original, q));
    }
    if spans.is_empty() {
        return;
    }
    spans.sort_unstable();
    let mut answer = String::new();
    let mut end = 0;
    for (a, b) in spans {
        if a >= end {
            answer.push_str(&original[end..a]);
            answer.push_str(INLINE_REMOVED);
        }
        end = end.max(b);
    }
    answer.push_str(&original[end..]);
    if answer.trim().trim_matches(['“', '”', '"', '‘', '’']) == INLINE_REMOVED {
        answer = "Quoted passage removed.".into();
    }
    turn["answer"] = json!(answer);
}

pub(super) fn reconcile(
    db: &Connection,
    chat: &mut Value,
    snapshot: Option<Snapshot<'_>>,
) -> Res<()> {
    let Some(turns) = chat["turns"].as_array_mut() else {
        return Ok(());
    };
    let mut earlier: HashSet<String> = HashSet::new();
    for turn in turns {
        let mut lost = clean_turn(db, turn, snapshot)?;
        // Follow-ups can repeat a quotation without citing the source again.
        // Leave independently sourced quotes and the user's own words intact.
        let independent = ["sources", "files", "webSources"]
            .iter()
            .any(|k| turn[*k].as_array().is_some_and(|v| !v.is_empty()))
            || turn["pointApp"].is_string();
        let next = lost.clone();
        if !independent {
            if let Some(obj) = turn.as_object_mut() {
                for field in QUOTE_FIELDS {
                    if let Some(q) = obj
                        .get(field)
                        .and_then(Value::as_str)
                        .filter(|q| !q.is_empty() && earlier.iter().any(|line| line.contains(*q)))
                    {
                        lost.insert(q.to_owned());
                        obj.remove(field);
                        if field == "quote" {
                            obj.remove("quoteTitle");
                        }
                    }
                }
            }
        }
        let answer = turn["answer"].as_str().unwrap_or("").trim();
        // Short wording like a gate number can be a full quoted reply.
        // Only carry it forward when the answer marks it as a quotation or repeats it alone.
        if !independent && !earlier.is_empty() {
            for fragment in INLINE_QUOTE
                .captures_iter(answer)
                .filter_map(|c| c.iter().skip(1).flatten().next().map(|m| m.as_str()))
            {
                if earlier.contains(fragment)
                    || (fragment.split_whitespace().count() >= 3
                        && earlier
                            .iter()
                            .any(|line| literal_spans(line, fragment).next().is_some()))
                {
                    lost.insert(fragment.to_owned());
                }
            }
        }
        lost.extend(
            earlier
                .iter()
                .filter(|q| {
                    q.split_whitespace().count() >= 3 || (!independent && q.as_str() == answer)
                })
                .cloned(),
        );
        if !lost.is_empty() {
            redact_answer(turn, &lost);
        }
        earlier.extend(next);
    }
    Ok(())
}

fn update_chats(
    tx: &Transaction<'_>,
    snapshot: Option<Snapshot<'_>>,
    epochs: Option<(&Value, &Value)>,
) -> Res<()> {
    // Other current chats also need the new epoch to keep their follow-ups usable.
    let ids = tx.prepare("SELECT id FROM docs WHERE kind=?1 AND (?2 OR instr(v,'apple-note:') OR instr(v,'apple-mail:') OR instr(v,'\"quoteTitle\"'))")
        .msg()?.query_map(params![KIND, epochs.is_some()], |r| r.get::<_,String>(0)).msg()?.collect::<Result<Vec<_>,_>>().msg()?;
    for id in ids {
        let raw: String = tx
            .query_row(
                "SELECT v FROM docs WHERE kind=? AND id=?",
                params![KIND, id],
                |r| r.get(0),
            )
            .msg()?;
        let mut chat: Value = serde_json::from_str(&raw).msg()?;
        reconcile(tx, &mut chat, snapshot)?;
        if let Some((old, next)) = epochs.filter(|(old, _)| &chat["epoch"] == *old) {
            chat["epoch"] = next.clone();
            for turn in chat["turns"].as_array_mut().into_iter().flatten() {
                if turn.get("epoch") == Some(old) {
                    turn["epoch"] = next.clone();
                }
            }
        }
        let updated = chat.to_string();
        if updated != raw {
            tx.execute(
                "UPDATE docs SET v=? WHERE kind=? AND id=?",
                params![updated, KIND, id],
            )
            .msg()?;
        }
    }
    Ok(())
}

pub(crate) fn replace_snapshot(
    tx: &Transaction<'_>,
    kind: &str,
    next: &Passages<'_>,
    old_epoch: &Value,
    new_epoch: &Value,
) -> Res<()> {
    update_chats(tx, Some((kind, next)), Some((old_epoch, new_epoch)))
}

pub(crate) fn upgrade(s: &Store) -> Res<()> {
    let mut db = s.db.lock().unwrap();
    let tx = db.transaction().msg()?;
    if tx
        .execute(
            "INSERT OR IGNORE INTO kv VALUES ('importQuoteProvenance', '1')",
            [],
        )
        .msg()?
        != 0
    {
        update_chats(&tx, None, None)?;
    }
    tx.commit().msg()
}
