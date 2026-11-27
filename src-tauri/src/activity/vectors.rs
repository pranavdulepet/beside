use crate::store::*;
use rusqlite::{params, OptionalExtension};
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use std::{
    collections::HashMap,
    ffi::{c_char, CString},
    path::PathBuf,
};

extern "C" {
    fn beside_passage_vector(text: *const c_char, output: *mut f32, size: i32) -> i32;
}

pub(super) struct Vector {
    pub space: String,
    pub feature: Vec<u8>,
}

pub(super) fn init(s: &Store) -> Res<()> {
    let mut db = s.db.lock().unwrap();
    let tx = db.transaction().msg()?;
    let schema: Option<String> = tx
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='activity_vectors'",
            [],
            |row| row.get(0),
        )
        .optional()
        .msg()?;
    if schema.is_some_and(|sql| !sql.to_ascii_lowercase().contains("partition key")) {
        // FTS stays available while the idle worker rebuilds this derived index.
        tx.execute_batch("DROP TRIGGER IF EXISTS activity_unembed; DROP TABLE activity_vectors;")
            .msg()?;
    }
    tx.execute_batch(include_str!("../activity.sql")).msg()?;
    tx.commit().msg()
}

fn normalized(values: [f32; 192]) -> Vec<u8> {
    let norm = values
        .iter()
        .map(|v| v * v)
        .sum::<f32>()
        .sqrt()
        .max(f32::EPSILON);
    values
        .iter()
        .flat_map(|v| (v / norm).to_le_bytes())
        .collect()
}

pub(super) fn native(text: &str) -> Vector {
    let mut v = [0.0; 192];
    let revision = CString::new(text).ok().map_or(0, |t| unsafe {
        beside_passage_vector(t.as_ptr(), v.as_mut_ptr(), 192)
    });
    if revision > 0 {
        return Vector {
            space: format!("apple-jl1-{revision}"),
            feature: normalized(v),
        };
    }
    for word in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        let digest = hash(&word.to_lowercase());
        let n = u32::from_str_radix(&digest[..8], 16).unwrap();
        v[n as usize % 192] += if n & 0x80000000 == 0 { 1.0 } else { -1.0 };
    }
    Vector {
        space: "lexical-1".into(),
        feature: normalized(v),
    }
}

// A tag locates a local Ollama GGUF. Passages go directly to the bundled offline
// runtime using the content-addressed blob, never to a mutable daemon endpoint.
fn dense_model() -> Option<(PathBuf, String)> {
    let name = std::env::var("BESIDE_OLLAMA_EMBED_MODEL").ok()?;
    let (model, tag) = name.split_once(':').unwrap_or((&name, "latest"));
    if [model, tag].iter().any(|v| {
        v.is_empty()
            || v.len() > 128
            || *v == "."
            || *v == ".."
            || !v
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
    }) {
        return None;
    }
    let root = std::env::var_os("OLLAMA_MODELS")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::sources::home().join(".ollama/models"));
    let file = std::fs::File::open(
        root.join("manifests/registry.ollama.ai/library")
            .join(model)
            .join(tag),
    )
    .ok()?;
    let manifest: Value = serde_json::from_reader(std::io::Read::take(file, 128 * 1024)).ok()?;
    let layer = manifest["layers"]
        .as_array()?
        .iter()
        .find(|v| v["mediaType"] == "application/vnd.ollama.image.model")?;
    let digest = layer["digest"].as_str()?.strip_prefix("sha256:")?;
    if digest.len() != 64 || !digest.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let path = root.join("blobs").join(format!("sha256-{digest}"));
    if !std::fs::symlink_metadata(&path).ok()?.is_file() {
        return None;
    }
    Some((path, format!("ollama-jl1-{digest}")))
}

// A local manifest can change independently of Ollama's model tag. Check the
// content-addressed bytes before admitting a new file into an embedding space.
fn verified_blob(path: &std::path::Path, space: &str, may_hash: bool) -> bool {
    use sha2::{Digest, Sha256};
    use std::{
        io::Read,
        os::unix::fs::MetadataExt,
        sync::{LazyLock, Mutex},
    };
    type Stamp = (u64, u64, u64, i64, i64, String);
    static VERIFIED: LazyLock<Mutex<Option<Stamp>>> = LazyLock::new(|| Mutex::new(None));
    let Some(expected) = space.strip_prefix("ollama-jl1-") else {
        return false;
    };
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let Ok(meta) = file.metadata() else {
        return false;
    };
    let stamp = (
        meta.dev(),
        meta.ino(),
        meta.size(),
        meta.ctime(),
        meta.ctime_nsec(),
        expected.to_owned(),
    );
    if VERIFIED.lock().unwrap().as_ref() == Some(&stamp) {
        return true;
    }
    if !may_hash {
        return false;
    }
    let mut hash = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hash.update(&buf[..n]),
            Err(_) => return false,
        }
    }
    if hex::encode(hash.finalize()) != expected {
        return false;
    }
    *VERIFIED.lock().unwrap() = Some(stamp);
    true
}

fn dense(s: &Store, text: &str, path: PathBuf, space: String, warm_only: bool) -> Option<Vector> {
    if !verified_blob(&path, &space, !warm_only) {
        return None;
    }
    let values = crate::bundled::passage_embedding(s, &path, text, warm_only).ok()?;
    if values.is_empty() || values.len() > 65536 {
        return None;
    }
    let mut v = [0.0; 192];
    for (i, n) in values.iter().enumerate() {
        let x = *n;
        if !x.is_finite() {
            return None;
        }
        let mut h = (i as u64).wrapping_mul(0x9E3779B97F4A7C15) ^ 0xBF58476D1CE4E5B9;
        for _ in 0..3 {
            h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            h = (h ^ (h >> 27)).wrapping_mul(0x94D049BB133111EB);
            h ^= h >> 31;
            v[(h % 192) as usize] += if h >> 63 == 0 { x } else { -x };
        }
    }
    Some(Vector {
        space,
        feature: normalized(v),
    })
}

pub(super) fn embed(s: &Store, text: &str) -> Vector {
    let dense_index = dense_model().filter(|(_, space)| {
        s.db.lock()
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM activity_vectors WHERE space=?1)",
                [space],
                |r| r.get::<_, bool>(0),
            )
            .unwrap_or(false)
    });
    let Some((path, space)) = dense_index else {
        return native(text);
    };
    if !verified_blob(&path, &space, false) || !crate::bundled::embedding_ready(&path) {
        use std::sync::atomic::{AtomicBool, Ordering};
        static WARMING: AtomicBool = AtomicBool::new(false);
        if !WARMING.swap(true, Ordering::AcqRel) {
            let base = s.root.join(".sumi");
            std::thread::spawn(move || {
                if verified_blob(&path, &space, true) {
                    let _ = crate::bundled::warm(
                        &serde_json::json!({"path":path,"private":base,"embedding":true}),
                    );
                }
                WARMING.store(false, Ordering::Release);
            });
        }
        return native(text);
    }
    dense(s, text, path, space, true).unwrap_or_else(|| native(text))
}

pub(super) fn passage_text(title: &str, body: &str) -> String {
    format!("{title}. {}", body.chars().take(500).collect::<String>())
}

pub(crate) fn backfill(s: &Store, limit: usize, idle: impl Fn() -> bool) -> Res<()> {
    let model = dense_model();
    let target = model
        .as_ref()
        .map(|m| m.1.clone())
        .unwrap_or_else(|| native("index revision").space);
    let rows = {
        let db = s.db.lock().unwrap();
        let mut stmt = db.prepare("SELECT a.id,a.title,a.body FROM activity a
            LEFT JOIN activity_vectors v ON v.event_id=a.id
            WHERE v.event_id IS NULL OR (v.space<>? AND (? NOT LIKE 'apple-%' OR v.space<>'lexical-1'))
            ORDER BY a.stamp DESC LIMIT ?").msg()?;
        let rows = stmt
            .query_map(params![target, target, limit as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .msg()?;
        rows.collect::<Result<Vec<_>, _>>().msg()?
    };
    for (id, title, body) in rows {
        if !idle() {
            break;
        }
        let text = passage_text(&title, &body);
        let vector = match model.as_ref() {
            Some((path, space)) => match dense(s, &text, path.clone(), space.clone(), false) {
                Some(v) => v,
                None => break,
            },
            None => native(&text),
        };
        if vector.space != target && target.starts_with("ollama-") {
            break;
        }
        let mut db = s.db.lock().unwrap();
        let tx = db.transaction().msg()?;
        tx.execute("DELETE FROM activity_vectors WHERE event_id=?", [&id])
            .msg()?;
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
    results.sort_by(|(a, left), (b, right)| {
        b.total_cmp(a)
            .then_with(|| right["stamp"].as_i64().cmp(&left["stamp"].as_i64()))
            .then_with(|| left["id"].as_str().cmp(&right["id"].as_str()))
    });
    results.into_iter().map(|(_, v)| v).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Requires explicitly selected local Ollama weights and an offline llama-server"]
    fn installed_embedding_weights_are_shared_only_with_the_bundled_loopback_runtime() {
        let Some((path, space)) = dense_model() else {
            panic!("Select installed embedding weights")
        };
        let dir = tempfile::tempdir().unwrap();
        let s = Store::with_key(dir.path().join("book"), &[19; 32]).unwrap();
        let v = dense(
            &s,
            "quiet library opening hours",
            path,
            space.clone(),
            false,
        )
        .expect("local embedding");
        assert_eq!(v.space, space);
        assert_eq!(v.feature.len(), 768);
        assert!(v.feature.iter().any(|b| *b != 0));
        crate::bundled::reap(true);
    }

    #[test]
    fn filtered_semantic_search_finds_a_note_beyond_the_unfiltered_neighbors() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Store::with_key(tmp.path().into(), &[16; 32]).unwrap();
        init(&s).unwrap();
        let v = native("trilobite");
        let mut db = s.db.lock().unwrap();
        let tx = db.transaction().unwrap();
        for i in 0..75 {
            let id = format!("row-{i:02}");
            let source = if i == 74 {
                "com.apple.Notes"
            } else {
                "display"
            };
            tx.execute(
                "INSERT INTO activity VALUES (?,?,?, ?,unixepoch(),?)",
                params![id, source, "Field record", "Mineral observation", &id],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO activity_vectors(event_id,space,feature) VALUES(?,?,?)",
                params![id, v.space, v.feature],
            )
            .unwrap();
        }
        tx.commit().unwrap();
        drop(db);
        assert!(
            !super::super::search_excluding(&s, "trilobite", &[], "", "", "", (false, false))
                .unwrap()
                .is_empty()
        );
        let found = super::super::search_excluding(&s, "trilobite", &[], "", "", "", (true, false))
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0]["id"], "row-74");
    }

    #[test]
    fn both_rankings_vote_and_forgotten_rows_cannot_return_during_migration() {
        let row = |id| json!({"id":id});
        assert_eq!(
            fuse(vec![row("a"), row("b")], vec![row("c"), row("b")])[0]["id"],
            "b"
        );
        let dir = tempfile::tempdir().unwrap();
        let s = Store::with_key(dir.path().into(), &[8; 32]).unwrap();
        s.db.lock().unwrap().execute_batch("CREATE VIRTUAL TABLE activity_vectors USING vec0(event_id TEXT PRIMARY KEY,feature float[192]);").unwrap();
        init(&s).unwrap();
        s.db.lock().unwrap().execute("INSERT INTO activity VALUES('a','display','Planning','Library visit tomorrow',unixepoch(),'test')",[]).unwrap();
        backfill(&s, 10, || true).unwrap();
        let db = s.db.lock().unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM activity_vectors", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        db.execute("DELETE FROM activity WHERE id='a'", []).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM activity_vectors", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
