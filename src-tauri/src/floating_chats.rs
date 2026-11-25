// Floating conversations live in the notebook's SQLCipher database.
use crate::store::*;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

const KIND: &str = "floatingChat";

pub fn list(s: &Store) -> Res<Value> {
    let mut rows = s.list(KIND)?;
    rows.sort_by_key(|row| std::cmp::Reverse(row["updatedAt"].as_i64().unwrap_or_default()));
    Ok(json!(rows))
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
    let epoch: Option<String> = tx
        .query_row("SELECT v FROM kv WHERE k='activityEpoch'", [], |r| r.get(0))
        .optional()
        .msg()?;
    let epoch: Value = serde_json::from_str(epoch.as_deref().unwrap_or("null")).msg()?;
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
        if row["epoch"] != epoch {
            if migrating {
                return Err("These older chats belong to a different notebook or include forgotten context. The earlier copy is still on this Mac.".into());
            }
            return Err("The notebook changed. Reopen the conversation.".into());
        }
        let sql = if migrating {
            "INSERT OR IGNORE INTO docs VALUES(?,?,?)"
        } else {
            "INSERT OR REPLACE INTO docs VALUES(?,?,?)"
        };
        tx.execute(sql, params![KIND, id, row.to_string()]).msg()?;
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
    let at = now();
    let row = json!({"id":&key[..32],"topic":format!("reply/{}", &key[..32]),"text":text,"value":text,"scale":"trait","learnedAt":at,"validFrom":at,"importance":1.0,"private":true,"restricted":true,"dates":[]});
    s.put("note", &row)?;
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
        crate::activity::forget(&s, &json!({"scope":"event","id":event})).unwrap();
        assert_eq!(list(&s).unwrap(), json!([separate]));
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
