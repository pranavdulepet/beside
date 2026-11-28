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
        .args([
            "-l",
            "JavaScript",
            "-e",
            include_str!("read_apple_notes.js"),
        ])
        .output()
        .msg()?;
    if !output.status.success() {
        return Err(format!(
            "Couldn’t finish the local import. {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    import_notes(s, output.stdout.as_slice(), &epoch)
}

pub(crate) fn import_notes(s: &Store, reader: impl std::io::Read, epoch: &Value) -> Res<Value> {
    let notes: Vec<[String; 3]> = serde_json::from_reader(reader).msg()?;
    replace(s, "note", &notes, epoch)
}

pub(super) fn replace(s: &Store, kind: &str, notes: &[[String; 3]], epoch: &Value) -> Res<Value> {
    let _guard = s.activity_edit.lock().unwrap();
    let cfg = config(s)?;
    let next: crate::floating_chats::import_quotes::Passages<'_> = notes
        .iter()
        .filter(|[_, _, body]| !body.trim().is_empty())
        .map(|[key, title, body]| {
            (
                format!("apple-{kind}:{}", hash(key)),
                (title.as_str(), body.as_str()),
            )
        })
        .collect();
    let mut db = s.db.lock().unwrap();
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .msg()?;
    if &Store::get_from(&tx, "activityEpoch")? != epoch {
        return Err("The notebook changed during the import. Try the import again.".into());
    }
    // The old snapshot can still identify unlabelled quotes from earlier versions.
    let next_epoch = json!(id());
    crate::floating_chats::import_quotes::replace_snapshot(&tx, kind, &next, epoch, &next_epoch)?;
    tx.execute("DELETE FROM imported_passages WHERE kind=?", [kind])
        .msg()?;
    {
        let mut insert = tx
            .prepare("INSERT INTO imported_passages(id,title,body,kind) VALUES (?,?,?,?)")
            .msg()?;
        for (key, (title, body)) in next {
            insert.execute(params![key, title, body, kind]).msg()?;
        }
    }
    if kind == "note" {
        super::baggage::refresh_recent(&tx, &cfg, Utc::now().timestamp())?;
    }
    tx.execute(
        "INSERT OR REPLACE INTO kv VALUES ('activityEpoch',?1),('lastImportEpoch',?1),('previousImportEpoch',?2)",
        params![next_epoch.to_string(),epoch.to_string()],
    )
    .msg()?;
    tx.commit().msg()?;
    crate::proactive::clear_active(s);
    Ok(Value::Null)
}

pub fn remove(s: &Store, kind: &str) -> Res<Value> {
    replace(s, kind, &[], &s.get("activityEpoch")?)
}

pub fn candidates(s: &Store, focus: &[String], context: &[String]) -> Res<Vec<Value>> {
    candidates_of_kind(s, focus, context, None)
}

pub(super) fn candidates_of_kind(
    s: &Store,
    focus: &[String],
    context: &[String],
    kind: Option<&str>,
) -> Res<Vec<Value>> {
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
    let mut q = db.prepare("SELECT id,title,body,kind FROM imported_passages WHERE imported_passages MATCH ? AND (?2 IS NULL OR kind=?2) ORDER BY bm25(imported_passages,0,4,1,0),id LIMIT 24").msg()?;
    let rows = q.query_map(params![query, kind], row).msg()?;
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

    #[test]
    fn forget_edits_existing_chats_and_exports_without_losing_the_other_source() {
        use crate::{app, floating_chats, notebook_data};
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("book");
        let key = [21; 32];
        let s = Store::with_key(root.clone(), &key).unwrap();
        app::setup(&s).unwrap();
        let notes = [[
            "n".to_owned(),
            "Picnic".into(),
            "Menu: Peas and mint.".into(),
        ]];
        replace(&s, "note", &notes, &s.get("activityEpoch").unwrap()).unwrap();
        let letter = "Use the orchard entrance.";
        let mbox = temp.path().join("picnic.mbox");
        std::fs::write(&mbox,format!("From robin@example.org Sat Jan  1 00:00:00 2025\nSubject: Arrival\nMessage-ID: <r@example.org>\n\n{letter}\n")).unwrap();
        super::super::mail_archive::import(&s, &mbox).unwrap();
        let epoch = s.get("activityEpoch").unwrap();
        let menu = app::action(
            &s,
            "activityAsk",
            &json!({"text":"What was on the menu in my note “Picnic”?"}),
        )
        .unwrap();
        assert_eq!(
            menu["sources"][0]["id"],
            format!("apple-note:{}", hash("n"))
        );
        let note = &menu["sources"][0];
        let mail_id = format!("apple-mail:{}", hash("<r@example.org>"));
        let mail = json!({"id":mail_id,"title":"Arrival","label":"Mail","quote":letter});
        let id = uuid::Uuid::new_v4().to_string();
        let question = "Keep my original question and the rest of this reply.";
        let mixed = json!({"id":"two","question":question,"epoch":epoch,"workKind":"comparison",
            "answer":format!("My plan. “Menu: Peas and mint.” Then “{letter}” Done."),"sources":[note,mail]});
        // This is the exact pre-upgrade format of an imported menu in an existing chat.
        let old_menu = json!({"question":"What was on the menu?","answer":"Peas and mint.","quote":"Menu: Peas and mint.","quoteTitle":"Picnic","sources":[],"epoch":epoch});
        let separate =
            json!({"question":"What did I type?","answer":"Keep the blue pencil.","epoch":epoch});
        let chat = json!({"id":id,"title":"Picnic planning","updatedAt":123,"epoch":epoch,"turns":[
            old_menu,mixed,{"question":"Repeat the wording.","answer":"Menu: Peas and mint.","epoch":epoch},
            separate,{"id":"last","question":"Which entrance?","answer":letter,"supportingLine":letter,"sources":[mail],"workKind":"passage","epoch":epoch}
        ]});
        s.put("floatingChat", &chat).unwrap();
        // A failure at the final epoch write must roll back both chat edits and the import.
        s.db.lock().unwrap().execute_batch("CREATE TEMP TRIGGER fail_forget BEFORE INSERT ON kv WHEN new.k='activityEpoch' BEGIN SELECT raise(ABORT,'interrupted'); END;").unwrap();
        assert!(app::action(&s, "removeAppleNotes", &json!({})).is_err());
        assert_eq!(count(&s, "note").unwrap(), 1);
        assert_eq!(s.doc("floatingChat", &id).unwrap(), chat);
        s.db.lock()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_forget")
            .unwrap();
        app::action(&s, "removeAppleNotes", &json!({})).unwrap();
        let transient = app::action(
            &s,
            "floatingChatSnapshot",
            &json!({"transient":[menu,separate]}),
        )
        .unwrap();
        assert_eq!(
            transient["transient"][0]["answer"],
            "Quoted passage removed."
        );
        assert_eq!(transient["transient"][1], separate);
        let clean = &floating_chats::list(&s).unwrap()[0];
        assert_eq!(clean["id"], id);
        assert_eq!(clean["updatedAt"], 123);
        assert_eq!(clean["turns"].as_array().unwrap().len(), 5);
        assert!(!clean.to_string().contains("Peas and mint"));
        assert_eq!(clean["turns"][0]["answer"], "Quoted passage removed.");
        assert!(clean["turns"][0].get("quoteTitle").is_none());
        assert_eq!(clean["turns"][1]["question"], question);
        assert_eq!(clean["turns"][1]["sources"], json!([mail]));
        assert_eq!(
            clean["turns"][1]["answer"],
            format!("My plan. “[quote removed]” Then “{letter}” Done.")
        );
        assert_eq!(clean["turns"][3]["answer"], separate["answer"]);
        let next = s.get("activityEpoch").unwrap();
        let follow = json!({"epoch":next,"chatId":id,"previousTurn":"last","text":"Give me the exact line from that email"});
        assert_eq!(
            app::action(&s, "activityAsk", &follow).unwrap()["answer"],
            letter
        );
        assert!(app::action(&s, "saveFloatingChat", &json!({"chats":[chat]})).is_err());
        assert!(app::action(&s,"activityAsk",&json!({"epoch":epoch,"previousTurn":"last","text":"Repeat that","history":[{"answer":"Menu: Peas and mint."}]})).is_err());
        // Simulate a restart, then drop Mail through its separate public command.
        drop(s);
        let s = Store::with_key(root, &key).unwrap();
        app::setup(&s).unwrap();
        app::action(&s, "removeMailInbox", &json!({})).unwrap();
        app::action(&s, "removeMailInbox", &json!({})).unwrap();
        replace(&s, "note", &notes, &s.get("activityEpoch").unwrap()).unwrap();
        super::super::mail_archive::import(&s, &mbox).unwrap();
        let file = temp.path().join("export.zip");
        notebook_data::export(&s, &file).unwrap();
        let mut zip = zip::ZipArchive::new(std::fs::File::open(file).unwrap()).unwrap();
        let exported: Value =
            serde_json::from_reader(zip.by_name("records/floatingChat.json").unwrap()).unwrap();
        assert_eq!(exported, floating_chats::list(&s).unwrap());
        assert!(!exported.to_string().contains("Peas and mint"));
        assert!(!exported.to_string().contains(letter));
        assert_eq!(
            exported[0]["turns"][1]["answer"],
            "My plan. “[quote removed]” Then “[quote removed]” Done."
        );
        assert_eq!(exported[0]["turns"][3]["answer"], "Keep the blue pencil.");
        assert!(std::fs::read_to_string(mbox).unwrap().contains(letter));
    }

    #[test]
    fn refresh_and_upgrade_keep_independently_supported_identical_quotes() {
        let temp = tempfile::tempdir().unwrap();
        let s = Store::with_key(temp.path().into(), &[38; 32]).unwrap();
        super::super::init(&s).unwrap();
        let quote = "Deliver the violet folders tomorrow.";
        let row = ["1".into(), "Delivery".into(), quote.into()];
        for kind in ["note", "mail"] {
            replace(
                &s,
                kind,
                std::slice::from_ref(&row),
                &s.get("activityEpoch").unwrap(),
            )
            .unwrap();
        }
        let chat = json!({"id":uuid::Uuid::new_v4().to_string(),"turns":[
            {"question":"Earlier unrelated turn","answer":quote},
            {"question":"What did they say?","answer":format!("Save “violet folders” and “{quote}” here."),"sources":[],"quoteTitle":"Delivery","quote":quote},
            {"question":quote,"answer":"A question in my own words."}
        ]});
        s.put("floatingChat", &chat).unwrap();
        s.db.lock()
            .unwrap()
            .execute("DELETE FROM kv WHERE k='importQuoteProvenance'", [])
            .unwrap();
        super::super::init(&s).unwrap();
        let read = || s.doc("floatingChat", chat["id"].as_str().unwrap()).unwrap();
        assert_eq!(read()["turns"][1]["sources"].as_array().unwrap().len(), 2);
        replace(
            &s,
            "note",
            std::slice::from_ref(&row),
            &s.get("activityEpoch").unwrap(),
        )
        .unwrap();
        assert_eq!(read()["turns"][1]["answer"], chat["turns"][1]["answer"]);
        replace(
            &s,
            "note",
            &[["1".into(), "Delivery".into(), "A different edition.".into()]],
            &s.get("activityEpoch").unwrap(),
        )
        .unwrap();
        assert_eq!(read()["turns"][1]["answer"], chat["turns"][1]["answer"]);
        assert_eq!(read()["turns"][1]["sources"].as_array().unwrap().len(), 1);
        remove(&s, "mail").unwrap();
        assert_eq!(
            read()["turns"][1]["answer"],
            "Save “[quote removed]” and “[quote removed]” here."
        );
        assert_eq!(read()["turns"][0], chat["turns"][0]);
        assert_eq!(read()["turns"][2], chat["turns"][2]);
    }

    #[test]
    fn forgetting_a_short_quotation_leaves_similar_words_intact() {
        let temp = tempfile::tempdir().unwrap();
        let s = Store::with_key(temp.path().into(), &[27; 32]).unwrap();
        super::super::init(&s).unwrap();
        replace(
            &s,
            "note",
            &[["n".into(), "Gate".into(), "A2".into()]],
            &s.get("activityEpoch").unwrap(),
        )
        .unwrap();
        let chat_id = uuid::Uuid::new_v4().to_string();
        let other = uuid::Uuid::new_v4().to_string();
        let epoch = s.get("activityEpoch").unwrap();
        let other_turn = json!({"id":"own","question":"Which bus?","answer":"Take the A20.","epoch":epoch,"workKind":"passage"});
        s.put(
            "floatingChat",
            &json!({"id":other,"turns":[other_turn],"epoch":epoch}),
        )
        .unwrap();
        let citation = json!({"id":format!("apple-note:{}",hash("n")),"quote":"A2"});
        let chat = json!({"id":chat_id,"turns":[
            {"question":"Which gate?","answer":"A2","sources":[citation]},
            {"question":"Again?","answer":"A2"},
            {"question":"Which connection?","answer":"It says “A2”. Keep the A20 timetable."},
            {"question":"Where is A2?","answer":"The A20 bus is outside."}
        ]});
        s.put("floatingChat", &chat).unwrap();
        replace(
            &s,
            "note",
            &[["n".into(), "Gate".into(), "A20".into()]],
            &epoch,
        )
        .unwrap();
        let edited = s.doc("floatingChat", &chat_id).unwrap();
        assert_eq!(edited["turns"][1]["answer"], "Quoted passage removed.");
        assert_eq!(
            edited["turns"][2]["answer"],
            "It says “[quote removed]”. Keep the A20 timetable."
        );
        assert_eq!(edited["turns"][3], chat["turns"][3]);
        let now = s.get("activityEpoch").unwrap();
        let active = crate::floating_chats::last_task(
            &s,
            &json!({"chatId":other,"previousTurn":"own","epoch":now}),
            &now,
            "passage",
        )
        .unwrap()
        .unwrap();
        assert_eq!(active["answer"], other_turn["answer"]);
    }
}
