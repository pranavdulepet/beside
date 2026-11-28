// Optional readable copies of the encrypted index. A day stays dirty until its
// replacement is durable, so an interrupted refresh is retried on startup.
use crate::store::*;
use chrono::{DateTime, Utc};
use rusqlite::params;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

pub fn directory(s: &Store) -> PathBuf {
    s.root.join(".beside/activity-journal")
}
fn day_name(name: &str) -> bool {
    name.strip_suffix(".md")
        .is_some_and(|v| chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d").is_ok() && v.len() == 10)
}
pub fn rebuild(s: &Store) -> Res<()> {
    s.db.lock().unwrap().execute("INSERT OR IGNORE INTO activity_dirty SELECT DISTINCT date(stamp,'unixepoch') FROM activity",[]).msg()?;
    Ok(())
}
pub fn sync(s: &Store) -> Res<()> {
    let _edit = s.activity_edit.lock().unwrap();
    sync_locked(s)
}
pub fn sync_locked(s: &Store) -> Res<()> {
    let dir = directory(s);
    let enabled = s.get("activitySettings")?["journalFiles"] == true;
    if let Ok(meta) = fs::symlink_metadata(&dir) {
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err("The activity journal must be a local directory.".into());
        }
    }
    // An interrupted atomic write can leave a dated temporary file. Clean those
    // along with managed copies, without touching unrelated files.
    let mut db = s.db.lock().unwrap();
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .msg()?;
    if dir.is_dir() {
        for entry in fs::read_dir(&dir).msg()? {
            let p = entry.msg()?.path();
            let name = p.file_name().and_then(|v| v.to_str()).unwrap_or("");
            let temp = name
                .strip_suffix(".tmp")
                .and_then(|v| v.rsplit_once('.'))
                .is_some_and(|(date, token)| {
                    day_name(&format!("{date}.md")) && ident(token).is_ok()
                });
            if temp || (!enabled && day_name(name)) {
                fs::remove_file(p).msg()?;
            }
        }
    }
    if !enabled {
        tx.execute("DELETE FROM activity_dirty", []).msg()?;
        return tx.commit().msg();
    }
    fs::create_dir_all(&dir).msg()?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).msg()?;
    let days = tx
        .prepare("SELECT day FROM activity_dirty ORDER BY day")
        .msg()?
        .query_map([], |row| row.get::<_, String>(0))
        .msg()?
        .collect::<Result<Vec<_>, _>>()
        .msg()?;
    for day in days {
        let name = format!("{day}.md");
        if !day_name(&name) {
            return Err("Invalid journal date.".into());
        }
        let mut out = format!("# Screen journal · {day} (UTC)\n");
        let mut q=tx.prepare("SELECT stamp,title,body,source FROM activity WHERE stamp>=unixepoch(?) AND stamp<unixepoch(?,'+1 day') ORDER BY stamp,rowid").msg()?;
        let mut rows = q.query(params![day, day]).msg()?;
        let mut count = 0;
        while let Some(row) = rows.next().msg()? {
            let stamp: i64 = row.get(0).msg()?;
            let time =
                DateTime::<Utc>::from_timestamp(stamp, 0).ok_or("Invalid observation time.")?;
            let title: String = row.get(1).msg()?;
            let body: String = row.get(2).msg()?;
            let source: String = row.get(3).msg()?;
            out.push_str(&format!(
                "\n## {} · {}\n\n{}\n\n",
                time.format("%H:%M:%S"),
                title.replace(['\n', '\r'], " "),
                crate::activity::label(&source)
            ));
            for line in body.lines() {
                out.push_str("> ");
                out.push_str(line);
                out.push('\n');
            }
            count += 1;
        }
        let path = dir.join(name);
        if count > 0 {
            atomic(&path, out.as_bytes())?;
        } else if path.exists() {
            fs::remove_file(&path).msg()?;
        }
        tx.execute("DELETE FROM activity_dirty WHERE day=?", [day])
            .msg()?;
    }
    tx.commit().msg()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity;
    use serde_json::json;
    #[test]
    fn display_capture_and_readable_files_follow_scope_and_forgetting() {
        let temp = tempfile::tempdir().unwrap();
        let s = Store::with_key(temp.path().join("notebook"), &[1; 32]).unwrap();
        activity::init(&s).unwrap();
        let row = json!({"source":"display","title":"Screen 1 · Notes active","body":"Festival at North Pier"});
        let t = Utc::now().timestamp();
        let c = activity::configure(&s, &json!({"scope":"displays"})).unwrap();
        activity::ingest(&s, &c, &row, t).unwrap();
        assert!(activity::journal(&s).unwrap()["events"]
            .as_array()
            .unwrap()
            .is_empty());
        let live = activity::configure(&s, &json!({"enabled":true})).unwrap();
        activity::ingest(&s, &live, &row, t).unwrap();
        assert!(!directory(&s).exists());
        activity::configure(&s, &json!({"journalFiles":true})).unwrap();
        let file = directory(&s).join(format!("{}.md", Utc::now().date_naive()));
        assert!(fs::read_to_string(&file)
            .unwrap()
            .contains("Festival at North Pier"));
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        activity::forget(&s, &json!({"scope":"all"})).unwrap();
        assert!(!file.exists());
        activity::ingest(&s, &live, &row, t).unwrap();
        assert!(activity::journal(&s).unwrap()["events"]
            .as_array()
            .unwrap()
            .is_empty());
        let restored = activity::configure(&s, &json!({"enabled":true})).unwrap();
        activity::ingest(&s, &restored, &row, t).unwrap();
        sync(&s).unwrap();
        assert!(file.exists());
        activity::configure(&s, &json!({"journalFiles":false})).unwrap();
        assert!(!file.exists());
        let narrow = activity::configure(&s, &json!({"scope":"selected"})).unwrap();
        assert_eq!(narrow["enabled"], false);
    }
}
