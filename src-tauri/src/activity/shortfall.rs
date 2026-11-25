// Calculate a shopping shortfall only when the user's counts and a saved recipe
// use the same units. Pack sizes and numbers in cooking instructions aren't totals.
use super::{allowed, config, named_notes, search_excluding, Res, Store};
use regex::Regex;
use serde_json::{json, Value};
use std::sync::LazyLock;

struct Stock {
    count: u32,
    name: String,
}

fn count(s: &str) -> Option<u32> {
    const WORDS: [&str; 13] = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
        "eleven", "twelve",
    ];
    s.parse()
        .ok()
        .or_else(|| WORDS.iter().position(|w| *w == s).map(|i| i as u32))
}

fn inventory(question: &str) -> Option<Vec<Stock>> {
    static INVENTORY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\b(?:i (?:already )?have|i['’]ve got|we have|with)\s+([^.!?;]+)").unwrap()
    });
    static END: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\s+(?:at home|on hand|already|in (?:my|the) (?:fridge|pantry|kitchen))\b|,?\s+(?:what|how many|can you|please|so |draft|write)\b").unwrap()
    });
    static SEP: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r",\s*(?:and\s+)?|\s+and\s+").unwrap());
    let q = question.to_lowercase();
    if ![
        "missing",
        "remaining",
        "still need",
        "need to buy",
        "left to buy",
        "short of",
    ]
    .iter()
    .any(|s| q.contains(s))
        || ![
            "buy",
            "store",
            "grocery",
            "groceries",
            "shopping",
            "recipe",
            "batch",
        ]
        .iter()
        .any(|s| q.contains(s))
    {
        return None;
    }
    // Scaling a recipe is a different calculation.
    static SCALED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\b(double|triple|half|twice|batches|scale|servings)\b").unwrap()
    });
    if SCALED.is_match(&q) {
        return None;
    }
    let owned = INVENTORY.captures(&q)?.get(1)?.as_str();
    let list = END.split(owned).next()?.trim().trim_end_matches(',');
    SEP.split(list)
        .map(|part| {
            let (qty, name) = part.trim().split_once(char::is_whitespace)?;
            let name = name.trim();
            if name.is_empty()
                || name.split_whitespace().count() > 7
                || !name
                    .chars()
                    .all(|c| c.is_alphabetic() || c == ' ' || c == '-')
            {
                return None;
            }
            Some(Stock {
                count: count(qty)?,
                name: name.to_owned(),
            })
        })
        .collect()
}

fn stems(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphabetic())
        .filter(|s| !s.is_empty() && *s != "of")
        .map(|s| s.trim_end_matches('s').to_string())
        .collect()
}

impl Stock {
    fn required(&self, body: &str) -> Option<u32> {
        let words = stems(&self.name);
        let mut found = None;
        for raw in body.split(['\n', '\r', '\u{2028}', '\u{2029}']) {
            let lower = raw
                .trim()
                .trim_start_matches(['•', '-', ' '])
                .to_lowercase();
            let primary = lower.split(['(', ';']).next().unwrap_or("");
            let (label, qty) = primary.split_once(':').unwrap_or(("", primary));
            let Some((n, units)) = qty.trim().split_once(char::is_whitespace) else {
                continue;
            };
            let Some(n) = count(n) else { continue };
            let units = units.trim();
            // Ignore ranges and composite amounts like "2 to 3" or "2 1/2".
            if ["to ", "or ", "and "]
                .iter()
                .any(|prefix| units.starts_with(prefix))
                || !units.starts_with(char::is_alphabetic)
            {
                continue;
            }
            let source_words = stems(&format!("{label} {units}"));
            let unit_words = stems(units);
            if words.first() != unit_words.first()
                || !words.iter().all(|w| source_words.contains(w))
            {
                continue;
            }
            if found.is_some_and(|other| other != n) {
                return None;
            }
            found = Some(n);
        }
        found
    }

    fn missing(&self, needed: u32) -> Option<String> {
        let n = needed.saturating_sub(self.count);
        if n == 0 {
            return None;
        }
        let (head, tail) = self.name.split_once(' ').unwrap_or((&self.name, ""));
        let unit = if n == 1 {
            head.trim_end_matches('s').to_owned()
        } else if head.ends_with('s') {
            head.to_owned()
        } else {
            format!("{head}s")
        };
        Some(format!("{n} {unit} {tail}").trim_end().to_owned())
    }
}

pub(super) fn from_saved_recipe(
    s: &Store,
    a: &Value,
    question: &str,
    epoch: &Value,
) -> Res<Option<Value>> {
    let Some(stock) = inventory(question) else {
        return Ok(None);
    };
    let lower = question.to_lowercase();
    let prior: Vec<_> = a["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|id| id.as_str().map(str::to_owned))
        .collect();
    if !lower.contains("note") && prior.is_empty() {
        return Ok(None);
    }
    let settings = config(s)?;
    if !a["epoch"].is_null() && &a["epoch"] != epoch {
        return Err("Your sources changed. Ask again.".into());
    }
    let target = match a["gesture"].as_str() {
        Some(id) => crate::point::target(s, id)?,
        None => Value::Null,
    };
    let query = format!("{question} {}", target["line"].as_str().unwrap_or(""));
    let mut notes = named_notes(s, question, &settings)?;
    if notes.is_empty() {
        notes = search_excluding(s, &query, &prior, "", "", question, (true, false))?;
        notes.retain(|v| {
            v["title"]
                .as_str()
                .is_some_and(|t| t.starts_with("Notes · "))
                && allowed(&settings, v["source"].as_str().unwrap_or(""))
        });
    }
    if notes.is_empty() && !lower.contains("note") {
        return Ok(None);
    }
    let (answer, index) = answer(&lower, &stock, &notes);
    let sources: Vec<_> = index
        .map(|i| &notes[i])
        .into_iter()
        .map(|v| json!({"id":v["id"],"title":v["title"],"label":v["label"],"stamp":v["stamp"]}))
        .collect();
    if &s.get("activityEpoch")? != epoch || config(s)?["generation"] != settings["generation"] {
        return Err("Your sources changed. Ask again.".into());
    }
    Ok(Some(
        json!({"answer":answer,"sources":sources,"notes":[],"epoch":epoch,
        "pointLine":target["line"],"pointApp":target["app"]}),
    ))
}

// One note must support every requested amount. Conflicting recipes need clarification.
fn answer(q: &str, stock: &[Stock], notes: &[Value]) -> (String, Option<usize>) {
    let mut match_found = None;
    for (index, note) in notes.iter().enumerate() {
        let Some(body) = note["body"].as_str() else {
            continue;
        };
        let Some(totals) = stock
            .iter()
            .map(|item| item.required(body))
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        if let Some((_, ref old)) = match_found {
            if old != &totals {
                return (
                    "I found different batch quantities. Which recipe should I use?".into(),
                    None,
                );
            }
        } else {
            match_found = Some((index, totals));
        }
    }
    let Some((index, totals)) = match_found else {
        return ("What amounts does the recipe call for? I couldn’t verify a total for each item you listed.".into(),None);
    };
    let mut missing: Vec<_> = stock
        .iter()
        .zip(totals)
        .filter_map(|(item, n)| item.missing(n))
        .collect();
    let text = match missing.pop() {
        None => "You already have enough of everything you listed.".into(),
        Some(last) => {
            let items = if missing.is_empty() {
                last
            } else {
                format!("{} and {last}", missing.join(", "))
            };
            if ["text", "message", "draft"]
                .iter()
                .any(|cue| q.contains(cue))
            {
                format!("Hey, could you grab {items} for me?")
            } else {
                format!("You still need {items}.")
            }
        }
    };
    (text, Some(index))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn check(q: &str, notes: &[Value]) -> Option<(String, Option<usize>)> {
        Some(answer(q, &inventory(q)?, notes))
    }
    #[test]
    fn subtracts_owned_stock_without_adding_other_items_or_package_sizes() {
        let notes = [
            json!({"body":"Soft rolls: 8 rolls (buy a 12-pack)\nFirm tempeh: 3 blocks (12 oz each)\nFresh cabbage: 2 heads"}),
        ];
        let q="I have five rolls and 1 block of tempeh at home. Draft a text for what I still need to buy.";
        assert_eq!(
            check(q, &notes),
            Some((
                "Hey, could you grab 3 rolls and 2 blocks of tempeh for me?".into(),
                Some(0)
            ))
        );
        assert!(check(&q.replace("1 block", "1 bag"), &notes)
            .unwrap()
            .1
            .is_none());
        assert!(check(
            &q.replace("what I still need", "a double batch: what I still need"),
            &notes
        )
        .is_none());
        assert!(check(
            q,
            &[
                notes[0].clone(),
                json!({"body":"10 rolls\n3 blocks of tempeh"})
            ]
        )
        .unwrap()
        .1
        .is_none());
    }
}
