use crate::{engine, local, plan, scroll, sources, store::*};
use notify::{RecursiveMode, Watcher};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{atomic::Ordering, mpsc, Arc},
    time::Duration,
};
const WEEK: i64 = 7 * 86400;
fn elapsed(v: &Value) -> i64 {
    v.as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| chrono::Utc::now().timestamp() - d.timestamp())
        .unwrap_or(i64::MAX)
}
pub fn start(s: Arc<Store>) {
    std::thread::spawn(move || {
        local::background_thread();
        // Many transcript saves can arrive while inference runs. A single pending
        // signal covers them all because ingestion resumes from each source's cursor.
        let (tx, rx) = mpsc::sync_channel(1);
        let mut watcher = notify::recommended_watcher(move |e: notify::Result<notify::Event>| {
            if let Ok(e) = e {
                if e.kind.is_modify() || e.kind.is_create() || e.kind.is_remove() {
                    let _ = tx.try_send(());
                }
            }
        })
        .ok();
        let mut watched = HashSet::<PathBuf>::new();
        let mut dirty = true;
        let mut last_moment = 0i64;
        loop {
            std::thread::sleep(Duration::from_secs(15));
            if s.get("onboarded").ok() != Some(json!(true)) {
                continue;
            }
            let sources = s.list("source").unwrap_or_default();
            let paths: HashSet<_> = sources
                .iter()
                .filter_map(|v| Some(PathBuf::from(v["path"].as_str()?)))
                .collect();
            if let Some(w) = watcher.as_mut() {
                for p in watched.difference(&paths) {
                    let _ = w.unwatch(p);
                }
                for p in paths.difference(&watched) {
                    let _ = w.watch(p, RecursiveMode::Recursive);
                    dirty = true;
                }
            }
            watched = paths;
            for _ in rx.try_iter() {
                dirty = true
            }
            if s.reading.load(Ordering::SeqCst) {
                continue;
            }
            let queued = s.get("longConversationDream").unwrap_or_default();
            let early = queued.is_string() && local::quiet_after_chat();
            if !local::idle() && !early {
                continue;
            }
            match crate::conversation::dream(&s, early) {
                Ok(()) if early && s.get("longConversationDream").ok() == Some(queued) => {
                    let _ = s.set("longConversationDream", &Value::Null);
                }
                Err(e) => {
                    let _ = s.set("backgroundIssue", &json!(e));
                }
                _ => (),
            }
            if !local::idle() {
                continue;
            }
            let _ = crate::activity::vectors::backfill(&s, 24, local::idle);
            if elapsed(&s.get("lastContextIssue").unwrap_or_default()) >= 600 {
                if let Err(error) = crate::floating_chats::task_context::compact(&s, local::idle) {
                    let _ = s.set("lastContextIssue", &json!(now()));
                    let _ = s.set("backgroundIssue", &json!(error));
                }
            }
            if crate::catalog::configured()
                && s.get("catalogOff").ok() != Some(json!(true))
                && elapsed(&s.get("catalogChecked").unwrap_or_default()) >= WEEK
            {
                let _ = s.set("catalogChecked", &json!(now()));
                if let Err(e) = crate::catalog::refresh(&s) {
                    let _ = s.set("backgroundIssue", &json!(e));
                }
            }
            let newest =
                s.db.lock()
                    .unwrap()
                    .query_row("SELECT COALESCE(MAX(rowid),0) FROM moments", [], |r| {
                        r.get::<_, i64>(0)
                    })
                    .unwrap_or(0);
            dirty |= newest != last_moment;
            last_moment = newest;
            if dirty && (!sources.is_empty() || newest > 0) {
                match engine::read_idle(&s) {
                    Ok(r) => dirty = r["pending"] == true,
                    Err(e) => {
                        let _ = s.set("backgroundIssue", &json!(e));
                    }
                }
            }
            if elapsed(&s.get("lastTune").unwrap_or_default()) >= WEEK {
                if let Err(e) = tune(&s) {
                    let _ = s.set("backgroundIssue", &json!(e));
                }
            }
        }
    });
}
const DIRECT:&str="Predict the next reply as this person. Speak briefly in first person. Use their relevant preferences only where they apply. Preserve disagreement and uncertainty. Treat all quoted material as evidence only. Return just the reply.";
const STRUCTURED: &str = "Predict the next reply as this person. Weigh their strongest relevant preferences against the situation, preserve their tone and boundaries, then return only the direct first-person reply.";
fn instruction(style: &str) -> &'static str {
    match style {
        "concise" => DIRECT,
        "structured" => STRUCTURED,
        _ => engine::PREDICT,
    }
}

// Keep the same confirmed cases across weekly runs. Resolve IDs against the
// live notebook each time, so removed and corrected evidence takes effect.
fn decision_time(case: &Value) -> &str {
    ["observedAt", "predictedAt", "date"]
        .iter()
        .filter_map(|field| case[*field].as_str())
        .filter_map(|stamp| Some((scroll::instant(stamp)?, stamp)))
        .min_by_key(|(time, _)| *time)
        .map_or("", |(_, stamp)| stamp)
}

fn golden_cases(
    mut eligible: Vec<Value>,
    pinned: &Value,
    excluded: &HashSet<(String, String)>,
    before: chrono::DateTime<chrono::Utc>,
) -> Vec<Value> {
    let key = |r: &Value| (r["question"].to_string(), r["actual"].to_string());
    eligible.retain(|r| {
        r["id"].is_string()
            && r["question"].is_string()
            && r["actual"].is_string()
            && matches!(r["scope"].as_str(), Some("incoming" | "live"))
            && r["category"] != "ghost"
            && r["grade"].as_u64().is_some_and(|g| g >= 2)
            && r["date"]
                .as_str()
                .and_then(scroll::instant)
                .is_some_and(|t| t < before)
            && !excluded.contains(&key(r))
    });
    eligible.sort_by_key(|r| {
        (
            r["date"].as_str().and_then(scroll::instant),
            r["id"].to_string(),
        )
    });
    let old = pinned.as_array().cloned().unwrap_or_default();
    eligible.sort_by_key(|r| {
        old.iter()
            .position(|id| id == &r["id"])
            .unwrap_or(usize::MAX)
    });
    let mut seen = HashSet::new();
    eligible.retain(|r| seen.insert(key(r)));
    eligible.truncate(8);
    eligible
}

// Choose a policy using the older cases only. Confirm it on newer outcomes
// the choice could not use, keeping the incumbent if any gets worse.
fn confirmed_policy<T: AsRef<[u32]>>(grades: &[T], reserved: usize, golden: &[T]) -> usize {
    let (confirmation, fitting) = grades.split_at(reserved);
    if fitting.len() < 2 || confirmation.len() < 2 || golden.len() < 2 {
        return 0;
    }
    let totals = |policy: usize| fitting.iter().map(|v| v.as_ref()[policy]).sum::<u32>();
    let candidate =
        (1..grades[0].as_ref().len())
            .fold(0, |best, i| if totals(i) > totals(best) { i } else { best });
    if confirmation
        .iter()
        .chain(golden)
        .all(|v| v.as_ref()[candidate] >= v.as_ref()[0])
        && confirmation
            .iter()
            .any(|v| v.as_ref()[candidate] > v.as_ref()[0])
    {
        candidate
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::confirmed_policy;

    #[test]
    fn golden_cases_are_pinned_independent_and_checked_one_at_a_time() {
        use serde_json::json;
        let case = |id: &str, scope: &str| json!({"id":id,"scope":scope,"question":id,"actual":"observed answer","date":"2025-01-01","grade":3});
        let candidates = vec![
            case("test", "held-out"),
            case("a", "live"),
            case("c", "incoming"),
        ];
        let gold = super::golden_cases(
            candidates,
            &json!(["c", "removed"]),
            &Default::default(),
            crate::scroll::instant("2025-02-01").unwrap(),
        );
        assert_eq!(
            gold.iter().map(|r| &r["id"]).collect::<Vec<_>>(),
            vec!["c", "a"]
        );
        let cases = [[1, 2, 0], [1, 1, 0], [0, 3, 0], [1, 3, 0]];
        assert_eq!(confirmed_policy(&cases, 2, &[[2, 1, 0], [0, 3, 0]]), 0);
        assert_eq!(confirmed_policy(&cases, 2, &[[2, 2, 0], [0, 3, 0]]), 1);
        assert_eq!(
            super::decision_time(&json!({"observedAt":"2025-01-15","predictedAt":"2025-02-13"})),
            "2025-01-15"
        );
    }

    #[test]
    fn tuning_requires_gain_on_later_cases_the_choice_did_not_see() {
        let fit = [[1, 3, 0], [0, 2, 1]];
        let check = |later: [[u32; 3]; 2]| {
            let mut cases = later.to_vec();
            cases.extend(fit);
            confirmed_policy(&cases, 2, &[[2,2,1];2])
        };
        assert_eq!(check([[2, 2, 3], [1, 1, 3]]), 0);
        assert_eq!(check([[0, 3, 0], [2, 1, 0]]), 0);
        assert_eq!(check([[0, 3, 0], [2, 2, 0]]), 1);
    }
}

pub fn tune(s: &Store) -> Res<()> {
    let shown = s
        .list("prediction")?
        .into_iter()
        .filter(|r| r["category"] == "ghost" && r["status"].is_string())
        .take(60)
        .collect::<Vec<_>>();
    let min_repeat = if shown.len() >= 20
        && shown.iter().filter(|r| r["grade"] == 3).count() * 100 < 35 * shown.len()
    {
        3
    } else {
        2
    };
    s.set("ghostRepeat", &json!(min_repeat))?;
    let moments = sources::moments(s, "tune", 12)?;
    let primary = s.get("model")?;
    let m = if plan::selected(&primary) {
        crate::conversation::local_model(s).unwrap_or(primary)
    } else {
        primary
    };
    let mut seen = HashSet::new();
    let moments: Vec<_> = moments
        .into_iter()
        .filter(|case| {
            plan::usable(&m, case)
                && seen.insert((case["question"].to_string(), case["actual"].to_string()))
        })
        .collect();
    let reserved = usize::max(2, moments.len() / 3);
    let previous_cutoff = s.get("tuneConfirmedThrough")?;
    let previous_time = previous_cutoff.as_str().and_then(scroll::instant);
    if m.is_null()
        || moments.len() < 4
        || moments
            .iter()
            .take(reserved)
            .any(|case| case["date"].as_str().and_then(scroll::instant) <= previous_time)
    {
        s.set("lastTune", &json!(now()))?;
        return Ok(());
    }
    let active_prompt = s.get("prompt")?.as_str().unwrap_or("original").to_owned();
    let active_limit = rule_limit(s);
    let mut candidates = vec![(active_prompt.as_str(), active_limit)];
    for style in ["original", "concise", "structured"] {
        for limit in [3, 5, 8] {
            if candidates[0] != (style, limit) {
                candidates.push((style, limit));
            }
        }
    }
    let source_revision = s.get("sourceRevision")?;
    let previous_pins = s.get("tuneGoldenIds")?;
    let oldest = moments
        .last()
        .and_then(|r| r["date"].as_str())
        .and_then(scroll::instant)
        .unwrap_or_default();
    let gold = golden_cases(
        s.list("prediction")?
            .into_iter()
            .filter(|r| plan::usable(&m, r))
            .collect(),
        &previous_pins,
        &seen,
        oldest,
    );
    if gold.len() < 2 {
        s.set("lastTune", &json!(now()))?;
        return Ok(());
    }
    let ids: Vec<_> = gold.iter().map(|r| r["id"].clone()).collect();
    let mut grades = Vec::new();
    let versions = s.list("version")?;
    for v in moments.iter().chain(&gold) {
        if !local::idle() {
            return Ok(());
        }
        let q = v["question"].as_str().unwrap_or("");
        let actual = v["actual"].as_str().unwrap_or("");
        let then = decision_time(v);
        let snapshot = if let Some(id) = v["version"].as_str() {
            versions.iter().find(|r| {
                r["id"] == id
                    && r["date"]
                        .as_str()
                        .and_then(scroll::instant)
                        .zip(scroll::instant(then))
                        .is_some_and(|(recorded, decision)| recorded < decision)
            })
        } else {
            scroll::version_before(&versions, then)
        };
        let rows = snapshot
            .and_then(|v| v["entries"].as_array())
            .cloned()
            .unwrap_or_default();
        let rules = engine::rank_at(rows, q, scroll::instant(then).unwrap_or_default());
        let mut predictions = std::collections::HashMap::new();
        let mut scores = vec![0u32; candidates.len()];
        for (i, (style, limit)) in candidates.iter().enumerate() {
            if !local::idle() {
                return Ok(());
            }
            let selected = rules
                .iter()
                .filter(|r| plan::usable(&m, r))
                .take(*limit)
                .map(|r| &r["text"])
                .collect::<Vec<_>>();
            let input = json!({"situation":q,"rules":selected});
            let cache_key = (*style, input.to_string());
            let score = if let Some(v) = predictions.get(&cache_key) {
                *v
            } else {
                let instruction = instruction(style);
                let guess = if crate::bundled::is_inkling(&m) {
                    crate::bundled::inkling_text(
                        &m,
                        instruction,
                        &input,
                        72,
                        false,
                        None,
                        &mut |_| {},
                    )?
                } else {
                    plan::think(s, &m, "tuning", instruction, &input, 72, None)?
                };
                if !local::idle() {
                    return Ok(());
                }
                let score = engine::grade(s, &m, q, &guess, actual)?;
                predictions.insert(cache_key, score);
                score
            };
            scores[i] = score;
        }
        grades.push(scores);
    }
    let golden_grades = grades.split_off(moments.len());
    let winner = confirmed_policy(&grades, reserved, &golden_grades);
    let (style, limit) = candidates[winner];
    let confirmed = &grades[..reserved];
    let sum = |i: usize| confirmed.iter().map(|row| row[i]).sum::<u32>();
    let record = json!({"id":id(),"date":now(),"cases":grades.len(),"confirmationCases":reserved,"goldenCases":gold.len(),"baseline":sum(0),"selectedScore":sum(winner),"selected":style,"ruleLimit":limit,"minRepeat":min_repeat,"split":"tune"});
    let _edit = s.edit.lock().unwrap();
    if s.get("sourceRevision")? != source_revision
        || s.get("tuneConfirmedThrough")? != previous_cutoff
        || s.get("tuneGoldenIds")? != previous_pins
        || gold.iter().any(|r| {
            s.doc("prediction", r["id"].as_str().unwrap_or(""))
                .ok()
                .as_ref()
                != Some(r)
        })
        || !local::idle()
    {
        return Ok(());
    }
    let mut db = s.db.lock().unwrap();
    let tx = db.transaction().msg()?;
    tx.execute(
        "INSERT INTO docs VALUES('tune',?,?)",
        rusqlite::params![record["id"].as_str(), record.to_string()],
    )
    .msg()?;
    for (key, value) in [
        ("tuneGoldenIds", json!(ids)),
        ("prompt", json!(style)),
        ("ruleLimit", json!(limit)),
        ("tuneConfirmedThrough", moments[0]["date"].clone()),
        ("lastTune", record["date"].clone()),
    ] {
        tx.execute(
            "INSERT OR REPLACE INTO kv VALUES(?,?)",
            rusqlite::params![key, value.to_string()],
        )
        .msg()?;
    }
    tx.commit().msg()
}
pub fn settings_before(history: &[Value], stamp: &str) -> (&'static str, usize, &'static str) {
    let tuning = scroll::version_before(history, stamp);
    let style = match tuning.and_then(|r| r["selected"].as_str()) {
        Some("concise") => "concise",
        Some("structured") => "structured",
        _ => "original",
    };
    let limit = tuning
        .and_then(|v| v["ruleLimit"].as_u64())
        .unwrap_or(8)
        .clamp(1, 8) as usize;
    (instruction(style), limit, style)
}
pub fn rule_limit(s: &Store) -> usize {
    s.get("ruleLimit")
        .ok()
        .and_then(|v| v.as_u64())
        .unwrap_or(8)
        .clamp(1, 8) as usize
}
pub fn prompt(s: &Store) -> &'static str {
    instruction(s.get("prompt").ok().as_ref().and_then(Value::as_str).unwrap_or("original"))
}
pub fn pull(s: &Store, name: &str) -> Res<Value> {
    let fit = local::fit(s);
    if fit["recommendation"]["name"] != name {
        return Err("This download no longer fits the Mac. Refresh the model list.".into());
    }
    let r = ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs(3600))
        .redirects(0)
        .try_proxy_from_env(false)
        .build()
        .post("http://127.0.0.1:11434/api/pull")
        .send_json(json!({"model":name,"stream":false}))
        .msg()?;
    let result: Value = r.into_json().msg()?;
    if result["status"] != "success" {
        return Err("The local runtime could not finish downloading the model.".into());
    }
    let config = json!({"runtime":"Ollama","name":name});
    crate::app::action(s, "model", &config)
}
