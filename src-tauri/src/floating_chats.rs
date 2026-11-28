// Floating conversations live in the notebook's SQLCipher database.
use crate::store::*;
use rusqlite::params;
use serde_json::{json, Value};
pub(crate) mod import_quotes;
pub mod task_context;

const KIND: &str = "floatingChat";

/// Read the most recent version of a task from the same local conversation.
/// An old window, a cleared notebook, or a different thread cannot supply it.
pub fn last_task(s: &Store, a: &Value, epoch: &Value, kind: &str) -> Res<Option<Value>> {
    let (Some(chat), Some(turn)) = (a["chatId"].as_str(), a["previousTurn"].as_str()) else {
        return Ok(None);
    };
    if &a["epoch"] != epoch {
        return Ok(None);
    }
    let Some(record) = s.optional_doc(KIND, chat)? else {
        return Ok(None);
    };
    Ok(record["turns"]
        .as_array()
        .and_then(|turns| turns.last())
        .filter(|t| {
            &record["epoch"] == epoch
                && &t["epoch"] == epoch
                && t["id"] == turn
                && t["workKind"] == kind
        })
        .cloned())
}

fn records(db: &rusqlite::Connection) -> Res<Value> {
    let mut stmt = db.prepare("SELECT v FROM docs WHERE kind=? ORDER BY coalesce(json_extract(v,'$.updatedAt'),0) DESC,rowid DESC").msg()?;
    let mut rows = Vec::new();
    for raw in stmt.query_map([KIND], |r| r.get::<_, String>(0)).msg()? {
        rows.push(serde_json::from_str(&raw.msg()?).msg()?);
    }
    Ok(Value::Array(rows))
}

pub fn list(s: &Store) -> Res<Value> {
    records(&s.db.lock().unwrap())
}

pub fn snapshot(s: &Store, args: &Value) -> Res<Value> {
    let mut db = s.db.lock().unwrap();
    let tx = db.transaction().msg()?;
    let epoch = Store::get_from(&tx, "activityEpoch")?;
    let import_changed = epoch.is_string() && epoch == Store::get_from(&tx, "lastImportEpoch")?;
    let mut result = json!({"chats":records(&tx)?,"epoch":epoch,"importChanged":import_changed});
    // The notebook's temporary activity sheet has no saved record. Clean that
    // view in memory only, and never revive a turn across an earlier privacy change.
    if let Some(turns) = args["transient"].as_array().filter(|_| import_changed) {
        let old = Store::get_from(&tx, "previousImportEpoch")?;
        if turns
            .iter()
            .all(|t| t["epoch"] == old || t["epoch"] == epoch)
        {
            let mut chat = json!({"turns":turns});
            import_quotes::reconcile(&tx, &mut chat, None)?;
            result["transient"] = chat["turns"].take();
        }
    }
    tx.commit().msg()?;
    Ok(result)
}

fn chat_id(row: &Value) -> Res<&str> {
    let id = row["id"].as_str().unwrap_or("");
    if id.len() != 36
        || !id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
    {
        return Err("Invalid conversation ID.".into());
    }
    Ok(id)
}

pub fn save(s: &Store, args: &Value, migrating: bool) -> Res<Value> {
    let _guard = s.activity_edit.lock().unwrap();
    let rows = args["chats"].as_array().ok_or("Missing conversations.")?;
    if rows.len() > 30 {
        return Err("Too many conversations in one request.".into());
    }
    let mut db = s.db.lock().unwrap();
    let tx = db.transaction().msg()?;
    let epoch = Store::get_from(&tx, "activityEpoch")?;
    for row in rows {
        let id = chat_id(row)?;
        let turns = row["turns"]
            .as_array()
            .ok_or("Missing conversation turns.")?;
        if turns.is_empty()
            || turns
                .iter()
                .any(|t| !t["question"].is_string() || !t["answer"].is_string())
        {
            return Err("Unreadable conversation.".into());
        }
        if row["epoch"] != epoch
            || turns
                .iter()
                .any(|turn| turn.get("epoch").is_some_and(|e| e != &epoch))
        {
            if migrating {
                return Err("These older chats belong to a different notebook or include forgotten context. The earlier copy is still on this Mac.".into());
            }
            return Err("The notebook changed. Reopen the conversation.".into());
        }
        let sql = if migrating {
            "INSERT OR IGNORE INTO docs VALUES(?,?,?)"
        } else {
            "INSERT INTO docs VALUES(?,?,?) ON CONFLICT(kind,id) DO UPDATE SET v=excluded.v"
        };
        let mut saved = row.clone();
        import_quotes::reconcile(&tx, &mut saved, None)?;
        tx.execute(sql, params![KIND, id, saved.to_string()])
            .msg()?;
    }
    tx.commit().msg()?;
    Ok(Value::Null)
}

pub fn delete(s: &Store, args: &Value) -> Res<Value> {
    let id = chat_id(args)?;
    s.db.lock()
        .unwrap()
        .execute("DELETE FROM docs WHERE kind=? AND id=?", params![KIND, id])
        .msg()?;
    Ok(Value::Null)
}

pub fn correct(s: &Store, args: &Value) -> Res<Value> {
    let _guard = s.activity_edit.lock().unwrap();
    let id = chat_id(args)?;
    let chat = s.doc(KIND, id)?;
    let turn = chat["turns"]
        .as_array()
        .and_then(|ts| {
            ts.iter()
                .find(|t| t["question"] == args["question"] && t["answer"] == args["answer"])
        })
        .ok_or("Reopen the reply you want to correct.")?;
    let text = args["text"].as_str().unwrap_or("").trim();
    if text.is_empty() || text.chars().count() > 320 {
        return Err("Write a correction of up to 320 characters.".into());
    }
    let key = hash(&json!([id, turn["question"], turn["answer"]]).to_string());
    let _edit = s.edit.lock().unwrap();
    let at = chrono::Local::now();
    let timeline = crate::memory::timeline(s)?;
    let linked = turn["preferenceId"]
        .as_str()
        .and_then(|id| timeline.iter().find(|note| note["id"] == id));
    let derived = crate::memory::observe(text, &key, at, true, true)
        .into_iter()
        .next()
        .and_then(|v| v["topic"].as_str().map(str::to_owned));
    let generic = derived.as_deref().is_none_or(|topic| {
        topic
            .strip_prefix("preference/")
            .is_some_and(|v| v.len() == 16 && v.bytes().all(|b| b.is_ascii_hexdigit()))
    });
    let topic = if generic {
        linked
            .and_then(|v| v["topic"].as_str())
            .map(str::to_owned)
            .or(derived)
    } else {
        derived
    }
    .unwrap_or_else(|| format!("reply/{}", &key[..32]));
    let previous = timeline.into_iter().find(|note| {
        (note["correctionOf"] == key || note["id"] == key[..32]) && crate::memory::current(note)
    });
    if previous.as_ref().is_some_and(|note| note["text"] == text) {
        return Ok(json!(text));
    }
    let stamp = at.to_rfc3339();
    let row = json!({"id":crate::store::id(),"topic":topic,"sourceChat":id,"sourceTurn":turn["id"],"correctionOf":key,"text":text,"value":text,"scale":"trait","learnedAt":stamp,"validFrom":stamp,"importance":1.0,"private":true,"restricted":true,"dates":[]});
    // An edit to the same reply can change its subject. Close the old version
    // explicitly so the old and new phrases cannot both be treated as current.
    {
        let mut db = s.db.lock().unwrap();
        let tx = db.transaction().msg()?;
        if let Some(mut old) = previous.filter(|v| v["topic"] != topic) {
            old["validUntil"] = json!(stamp);
            tx.execute(
                "UPDATE docs SET v=? WHERE kind='note' AND id=?",
                params![old.to_string(), old["id"].as_str()],
            )
            .msg()?;
        }
        tx.execute(
            "INSERT INTO docs VALUES('note',?,?)",
            params![row["id"].as_str(), row.to_string()],
        )
        .msg()?;
        tx.commit().msg()?;
    }
    crate::memory::project(s)?;
    Ok(json!(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forgetting_a_passage_only_removes_chats_that_cite_it() {
        let temp = tempfile::tempdir().unwrap();
        let s = Store::with_key(temp.path().to_path_buf(), &[7; 32]).unwrap();
        crate::activity::init(&s).unwrap();
        let event = id();
        s.db.lock().unwrap().execute(
            "INSERT INTO activity VALUES(?, 'display','Gallery','Open at noon',unixepoch(),'abc')", [&event]
        ).unwrap();
        let cited = json!({"id":"af8681d6-69c4-4d78-927c-177fcca1a2c9","turns":[{"question":"When?","answer":"Noon","sources":[{"id":event}]}]});
        let separate = json!({"id":"34d9ac7a-90e3-44ec-bdbb-f0e24a757570","turns":[{"question":"Which tea?","answer":"Mint."}]});
        save(&s, &json!({"chats":[cited, separate]}), false).unwrap();
        s.db.lock().unwrap().execute("INSERT INTO floating_context(chat,covered,through_turn,refs) VALUES(?,1,'one','[]')",[cited["id"].as_str().unwrap()]).unwrap();
        crate::activity::forget(&s, &json!({"scope":"event","id":event})).unwrap();
        assert_eq!(list(&s).unwrap(), json!([separate]));
        assert_eq!(
            s.db.lock()
                .unwrap()
                .query_row("SELECT count(*) FROM floating_context", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn longer_conversations_keep_the_first_reply() {
        let temp = tempfile::tempdir().unwrap();
        let s = Store::with_key(temp.path().to_path_buf(), &[7; 32]).unwrap();
        let turns: Vec<Value> = (0..25)
            .map(|i| json!({"question":i.to_string(),"answer":"Saved."}))
            .collect();
        let chat = json!({"id":"de844655-355f-4071-9cd6-c54ad95a8ea0","turns":turns});
        save(&s, &json!({"chats":[chat]}), false).unwrap();
        assert_eq!(list(&s).unwrap()[0]["turns"], json!(turns));
    }

    #[test]
    fn chats_survive_restart_export_and_forgetting_without_resurrection() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("book");
        let s = Store::with_key(root.clone(), &[7; 32]).unwrap();
        crate::activity::init(&s).unwrap();
        let chat_id = uuid::Uuid::new_v4().to_string();
        let row = json!({"id":chat_id,"title":"Lunch","updatedAt":1234,"epoch":null,"turns":[{"question":"What time?","answer":"Noon."}]});
        let args = json!({"chats":[row]});
        save(&s, &args, true).unwrap();
        save(&s, &args, true).unwrap();
        correct(&s,&json!({"id":chat_id,"question":"What time?","answer":"Noon.","text":"I prefer lunch after one."})).unwrap();
        drop(s);
        let s = Store::with_key(root, &[7; 32]).unwrap();
        assert_eq!(list(&s).unwrap(), args["chats"]);
        assert_eq!(s.list("note").unwrap()[0]["private"], true);
        let path = temp.path().join("book.zip");
        crate::notebook_data::export(&s, &path).unwrap();
        let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        let records: Value =
            serde_json::from_reader(archive.by_name("records/floatingChat.json").unwrap()).unwrap();
        assert_eq!(records, args["chats"]);
        crate::activity::forget(&s, &json!({"scope":"all"})).unwrap();
        assert_eq!(list(&s).unwrap(), json!([]));
        assert!(save(&s, &args, false).is_err());
        assert!(save(&s, &args, true).is_err());
    }
}
