use crate::store::*;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::{collections::HashMap, ffi::{c_char, CString}, sync::Mutex, time::{Duration, Instant}};

extern "C" { fn beside_passage_vector(text: *const c_char, output: *mut f32, size: i32) -> i32; }

pub(super) struct Vector {
    pub space: String,
    pub feature: Vec<u8>,
}

pub(super) fn init(s: &Store) -> Res<()> {
    let mut db = s.db.lock().unwrap();
    let tx = db.transaction().msg()?;
    let schema: Option<String> = tx.query_row(
        "SELECT sql FROM sqlite_master WHERE name='activity_vectors'", [], |row| row.get(0)
    ).optional().msg()?;
    if schema.is_some_and(|sql| !sql.to_ascii_lowercase().contains("partition key")) {
        // FTS stays available while the idle worker rebuilds this derived index.
        tx.execute_batch("DROP TRIGGER IF EXISTS activity_unembed; DROP TABLE activity_vectors;").msg()?;
    }
    tx.execute_batch(include_str!("../activity.sql")).msg()?;
    tx.commit().msg()
}

fn normalized(values: [f32; 192]) -> Vec<u8> {
    let norm = values.iter().map(|v| v * v).sum::<f32>().sqrt().max(f32::EPSILON);
    values.iter().flat_map(|v| (v / norm).to_le_bytes()).collect()
}

fn native(text: &str) -> Vector {
    let mut v = [0.0; 192];
    let revision = CString::new(text).ok().map_or(0, |t| unsafe {
        beside_passage_vector(t.as_ptr(), v.as_mut_ptr(), 192)
    });
    if revision > 0 {
        return Vector { space: format!("apple-jl1-{revision}"), feature: normalized(v) };
    }
    for word in text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        let digest = hash(&word.to_lowercase());
        let n = u32::from_str_radix(&digest[..8], 16).unwrap();
        v[n as usize % 192] += if n & 0x80000000 == 0 { 1.0 } else { -1.0 };
    }
    Vector { space: "lexical-1".into(), feature: normalized(v) }
}

const OLLAMA: &str = "http://127.0.0.1:11434";
type Model = (String, String);
static PROBE: Mutex<Option<(String, Instant, Option<Model>)>> = Mutex::new(None);
fn ollama() -> ureq::Agent {
    ureq::AgentBuilder::new().redirects(0).timeout(Duration::from_millis(1400)).build()
}

fn dense_model() -> Option<Model> {
    let name = std::env::var("BESIDE_OLLAMA_EMBED_MODEL").ok()?;
    if name.is_empty() || name.len() > 128 || name.contains("cloud") ||
        !name.bytes().all(|c| c.is_ascii_alphanumeric() || b".-_:/".contains(&c)) { return None; }
    let mut cache = PROBE.lock().ok()?;
    if let Some((key, at, result)) = cache.as_ref() {
        if key == &name && at.elapsed() < Duration::from_secs(30) { return result.clone(); }
    }
    let probe = || {
        let client = ollama();
        let tags: Value = client.get(&format!("{OLLAMA}/api/tags")).call().ok()?.into_json().ok()?;
        let tag = tags["models"].as_array()?.iter().find(|row| {
            row["name"] == name || row["name"].as_str().and_then(|v| v.strip_suffix(":latest")) == Some(&name)
        })?;
        let digest = tag["digest"].as_str()?.trim_start_matches("sha256:");
        if digest.len() != 64 || !digest.bytes().all(|c| c.is_ascii_hexdigit()) { return None; }
        let model = tag["name"].as_str()?;
        let info: Value = client.post(&format!("{OLLAMA}/api/show")).send_json(json!({"model":model})).ok()?.into_json().ok()?;
        if info["remote_model"].is_string() || info["remote_host"].is_string()
            || !info["model_info"].as_object().is_some_and(|o| !o.is_empty())
            || !info["capabilities"].as_array()?.iter().any(|c| c == "embedding") { return None; }
        Some((model.to_owned(), format!("ollama-jl1-{digest}")))
    };
    let result = probe();
    *cache = Some((name, Instant::now(), result.clone()));
    result
}

fn dense(text: &str, name: &str, space: String) -> Option<Vector> {
    let res: Value = ollama().post(&format!("{OLLAMA}/api/embed")).send_json(
        json!({"model":name,"input":text,"truncate":true})
    ).ok()?.into_json().ok()?;
    let values = res["embeddings"].get(0)?.as_array()?;
    if values.is_empty() || values.len() > 65536 { return None; }
    let mut v = [0.0; 192];
    for (i, n) in values.iter().enumerate() {
        let x = n.as_f64()? as f32;
        if !x.is_finite() { return None; }
        let mut h = (i as u64).wrapping_mul(0x9E3779B97F4A7C15) ^ 0xBF58476D1CE4E5B9;
        for _ in 0..3 {
            h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            h = (h ^ (h >> 27)).wrapping_mul(0x94D049BB133111EB);
            h ^= h >> 31;
            v[(h % 192) as usize] += if h >> 63 == 0 { x } else { -x };
        }
    }
    Some(Vector { space, feature: normalized(v) })
}

pub(super) fn embed(text: &str) -> Vector {
    dense_model().and_then(|(model, space)| dense(text, &model, space)).unwrap_or_else(|| native(text))
}

pub(super) fn passage(title: &str, body: &str) -> Vector {
    embed(&format!("{title}. {}", body.chars().take(500).collect::<String>()))
}

pub(crate) fn backfill(s: &Store, limit: usize) -> Res<()> {
    let target = dense_model().map(|m| m.1).unwrap_or_else(|| native("index revision").space);
    let rows = {
        let db = s.db.lock().unwrap();
        let mut stmt = db.prepare("SELECT a.id,a.title,a.body FROM activity a
            LEFT JOIN activity_vectors v ON v.event_id=a.id
            WHERE v.event_id IS NULL OR (v.space<>? AND (? NOT LIKE 'apple-%' OR v.space<>'lexical-1'))
            ORDER BY a.stamp DESC LIMIT ?").msg()?;
        let rows = stmt.query_map(params![target,target,limit], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).msg()?;
        rows.collect::<Result<Vec<_>, _>>().msg()?
    };
    for (id,title,body) in rows {
        let vector = passage(&title,&body);
        if vector.space != target && target.starts_with("ollama-") { break; }
        let mut db = s.db.lock().unwrap();
        let tx = db.transaction().msg()?;
        tx.execute("DELETE FROM activity_vectors WHERE event_id=?", [&id]).msg()?;
        tx.execute("INSERT INTO activity_vectors(event_id,space,feature) SELECT id,?,? FROM activity WHERE id=?",
            params![vector.space,vector.feature,id]).msg()?;
        tx.commit().msg()?;
    }
    Ok(())
}

pub(super) fn fuse(lexical: Vec<Value>, semantic: Vec<Value>) -> Vec<Value> {
    let mut by_id = HashMap::new();
    for list in [lexical, semantic] {
        for (rank, row) in list.into_iter().enumerate() {
            let id = row["id"].as_str().unwrap_or_default().to_owned();
            let entry = by_id.entry(id).or_insert((0.0, row));
            entry.0 += 1.0 / (61 + rank) as f64;
        }
    }
    let mut results: Vec<_> = by_id.into_values().collect();
    results.sort_by(|(a, left),(b,right)| b.total_cmp(a)
        .then_with(|| right["stamp"].as_i64().cmp(&left["stamp"].as_i64()))
        .then_with(|| left["id"].as_str().cmp(&right["id"].as_str())));
    results.into_iter().map(|(_, v)| v).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_rankings_vote_and_forgotten_rows_cannot_return_during_migration() {
        let row = |id| json!({"id":id});
        assert_eq!(fuse(vec![row("a"),row("b")],vec![row("c"),row("b")])[0]["id"],"b");
        let dir = tempfile::tempdir().unwrap();
        let s = Store::with_key(dir.path().into(), &[8;32]).unwrap();
        s.db.lock().unwrap().execute_batch("CREATE VIRTUAL TABLE activity_vectors USING vec0(event_id TEXT PRIMARY KEY,feature float[192]);").unwrap();
        init(&s).unwrap();
        s.db.lock().unwrap().execute("INSERT INTO activity VALUES('a','display','Planning','Library visit tomorrow',unixepoch(),'test')",[]).unwrap();
        backfill(&s,10).unwrap();
        let db=s.db.lock().unwrap();
        assert_eq!(db.query_row("SELECT count(*) FROM activity_vectors",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        db.execute("DELETE FROM activity WHERE id='a'",[]).unwrap();
        assert_eq!(db.query_row("SELECT count(*) FROM activity_vectors",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    }
}
