// Resolve a description such as "my gear checklist" against observations already
// in the local index. The label people use in conversation needn't be the title.
use super::{allowed, conversation, event, Store, DAY};
use crate::store::*;
use chrono::Utc;
use regex::Regex;
use rusqlite::params;
use serde_json::Value;
use std::{collections::HashSet, sync::LazyLock};

pub(super) fn lookup(s: &Store, question: &str, settings: &Value) -> Res<Vec<Value>> {
    static REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\b(?:my|our)\s+((?:[\p{L}-]+\s+){1,4})(?:checklist|list)\b").unwrap()
    });
    let Some(found) = REFERENCE.captures(question) else {
        return Ok(vec![]);
    };
    let description = conversation::words(&found[1]);
    if description.is_empty() {
        return Ok(vec![]);
    }

    // Expand vocabulary for the kind of list, never for a particular person,
    // destination, or answer. Literal descriptions work for other list types.
    let mut topic = description.clone();
    for family in [
        &[
            "pack", "packing", "travel", "trip", "vacation", "luggage", "suitcase",
        ][..],
        &["shopping", "grocery", "groceries", "ingredient"][..],
        &["reading", "book", "books", "read"][..],
    ] {
        if family.iter().any(|t| description.contains(*t)) {
            topic.extend(family.iter().map(|t| t.to_string()));
        }
    }
    let query = topic
        .iter()
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(" OR ");
    let db = s.db.lock().unwrap();
    let mut stmt = db
        .prepare(
            "SELECT a.id,a.source,a.title,a.body,a.stamp
        FROM activity_search JOIN activity a ON a.rowid=activity_search.rowid
        WHERE activity_search MATCH ? AND a.title LIKE 'Notes · %' AND a.stamp >= ?
        ORDER BY bm25(activity_search,4,1,0),a.stamp DESC LIMIT 96",
        )
        .msg()?;
    let rows = stmt
        .query_map(params![query, Utc::now().timestamp() - 30 * DAY], event)
        .msg()?;
    let question_words = conversation::words(question);
    let mut ranked = Vec::new();
    for row in rows {
        let row = row.msg()?;
        if !allowed(settings, row["source"].as_str().unwrap_or("")) {
            continue;
        }
        let title = row["title"]
            .as_str()
            .unwrap_or("")
            .trim_start_matches("Notes · ");
        let title_words = conversation::words(title);
        let body = row["body"].as_str().unwrap_or("");
        let items = body
            .split(['\n', '\r', '\u{2028}', '\u{2029}'])
            .map(str::trim)
            .filter(|l| !l.is_empty() && *l != title && l.split_whitespace().count() <= 8)
            .count();
        let literal = title_words.intersection(&description).count();
        let related = title_words.intersection(&topic).count();
        if literal == 0 && items < 2 {
            continue;
        }
        let overlap = conversation::words(body)
            .intersection(&question_words)
            .count();
        let score = (
            literal,
            related,
            items >= 2,
            title_words.intersection(&question_words).count(),
            overlap.min(8),
        );
        ranked.push((score, row));
    }
    ranked.sort_by(|a, b| b.0.cmp(&a.0));
    let Some((best, _)) = ranked.first() else {
        return Ok(Vec::new());
    };
    let best_title_match = (best.0, best.1);
    let mut titles = HashSet::new();
    Ok(ranked
        .into_iter()
        // A passing mention in another note should not dilute a matching title.
        .take_while(|(score, _)| (score.0, score.1) == best_title_match)
        .map(|(_, row)| row)
        .filter(|row| titles.insert(row["title"].as_str().unwrap_or("").to_lowercase()))
        .take(2)
        .collect())
}
