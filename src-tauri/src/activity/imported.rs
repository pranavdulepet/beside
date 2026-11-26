// Explicit Notes and Mail imports live in the encrypted notebook. Labels and
// excerpts are searched locally. A refresh replaces the previous snapshot.
use super::*;

pub fn count(s: &Store, kind: &str) -> Res<i64> {
    s.db.lock()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM imported_passages WHERE kind=?",
            [kind],
            |r| r.get(0),
        )
        .msg()
}

pub fn refresh_notes(s: &Store) -> Res<Value> {
    let epoch = s.get("activityEpoch")?;
    let output = std::process::Command::new("/usr/bin/osascript")
        .args(["-l", "JavaScript", "-e", include_str!("read_apple_notes.js")])
        .output()
        .msg()?;
    if !output.status.success() {
        return Err(format!(
            "Couldn’t finish the local import. {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let notes: Vec<[String; 3]> = serde_json::from_slice(&output.stdout).msg()?;
    replace(s, "note", &notes, &epoch)
}

pub(super) fn replace(s: &Store, kind: &str, notes: &[[String; 3]], epoch: &Value) -> Res<Value> {
    let _guard = s.activity_edit.lock().unwrap();
    if &s.get("activityEpoch")? != epoch {
        return Err("The notebook changed during the import. Try the import again.".into());
    }
    let cfg = config(s)?;
    let mut db = s.db.lock().unwrap();
    let tx = db.transaction().msg()?;
    tx.execute("DELETE FROM imported_passages WHERE kind=?", [kind])
        .msg()?;
    {
        let mut insert = tx
            .prepare("INSERT INTO imported_passages(id,title,body,kind) VALUES (?,?,?,?)")
            .msg()?;
        for [id, title, body] in notes {
            if !body.trim().is_empty() {
                insert
                    .execute(params![
                        format!("apple-{kind}:{}", hash(id)),
                        title,
                        body,
                        kind
                    ])
                    .msg()?;
            }
        }
    }
    if kind=="note" {super::baggage::refresh_recent(&tx,&cfg,Utc::now().timestamp())?;}
    tx.execute(
        "INSERT OR REPLACE INTO kv VALUES ('activityEpoch',?)",
        [json!(id()).to_string()],
    )
    .msg()?;
    tx.commit().msg()?;
    crate::proactive::clear_active();
    Ok(Value::Null)
}

pub fn remove(s: &Store, kind: &str) -> Res<Value> {
    replace(s, kind, &[], &s.get("activityEpoch")?)
}

pub fn candidates(s: &Store, focus: &[String], context: &[String]) -> Res<Vec<Value>> {
    if focus.is_empty() {
        return Ok(vec![]);
    }
    let group = |terms: &[String]| {
        terms
            .iter()
            .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR ")
    };
    let mut query = group(focus);
    if !context.is_empty() {
        // Every passage we can show has a topic term in its title, heading or
        // body. Apply that condition before BM25's limit, even in a large inbox.
        query = format!("({query}) AND ({query} OR {})", group(context));
    }
    let db = s.db.lock().unwrap();
    let mut q = db.prepare("SELECT id,title,body,kind FROM imported_passages WHERE imported_passages MATCH ? ORDER BY bm25(imported_passages,0,4,1,0),id LIMIT 24").msg()?;
    let rows = q.query_map([query], row).msg()?;
    rows.collect::<Result<Vec<_>, _>>().msg()
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"id":r.get::<_,String>(0)?, "title":r.get::<_,String>(1)?, "body":r.get::<_,String>(2)?, "kind":r.get::<_,String>(3)?}),
    )
}

pub fn source(s: &Store, id: &str) -> Res<Value> {
    s.db.lock()
        .unwrap()
        .query_row(
            "SELECT id,title,body,kind FROM imported_passages WHERE id=?",
            [id],
            row,
        )
        .map_err(|_| "That imported passage is no longer available.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_inbox_full_of_context_words_cannot_displace_a_matching_note() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Store::with_key(tmp.path().join("notebook"), &[7; 32]).unwrap();
        super::super::init(&s).unwrap();
        let mail: Vec<_> = (0..40)
            .map(|i| {
                [
                    i.to_string(),
                    "Gallery schedule".to_owned(),
                    "Cotton panels for tomorrow".to_owned(),
                ]
            })
            .collect();
        replace(&s, "mail", &mail, &s.get("activityEpoch").unwrap()).unwrap();
        replace(
            &s,
            "note",
            &[[
                "one".into(),
                "From the studio".into(),
                "Iris:\nHang the panels near the entrance.".into(),
            ]],
            &s.get("activityEpoch").unwrap(),
        )
        .unwrap();
        let hits = candidates(&s, &["iris".into()], &["gallery".into(), "panel".into()]).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["kind"], "note");
        remove(&s, "mail").unwrap();
        assert_eq!(count(&s, "mail").unwrap(), 0);
        assert_eq!(count(&s, "note").unwrap(), 1);
    }

    #[test]
    fn refresh_replaces_deleted_notes_and_a_stale_import_cannot_undo_removal() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Store::with_key(tmp.path().join("notebook"), &[8; 32]).unwrap();
        super::super::init(&s).unwrap();
        replace(
            &s,
            "note",
            &[["42".into(), "Studio".into(), "Quilt exhibit".into()]],
            &s.get("activityEpoch").unwrap(),
        )
        .unwrap();
        let hit = candidates(&s, &["quilt".into()], &[])
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(
            source(&s, hit["id"].as_str().unwrap()).unwrap()["title"],
            "Studio"
        );
        let stale = s.get("activityEpoch").unwrap();
        remove(&s, "note").unwrap();
        assert!(replace(&s, "note", &[], &stale).is_err());
        assert!(candidates(&s, &["quilt".into()], &[]).unwrap().is_empty());
        assert!(source(&s, hit["id"].as_str().unwrap()).is_err());
    }
}
