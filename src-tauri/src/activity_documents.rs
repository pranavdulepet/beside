use crate::{attachments, conversation, store::*};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::{
    collections::{HashSet, VecDeque},
    fs,
    io::{BufRead, BufReader, Read},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

pub fn roots(s: &Store) -> Res<Vec<PathBuf>> {
    Ok(s.get("activityFolders")?
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(PathBuf::from)
        .collect())
}
pub fn configure(s: &Store, remove: Option<&str>) -> Res<Value> {
    let picked = if remove.is_none() {
        rfd::FileDialog::new()
            .set_title("Choose a folder for local answers")
            .pick_folder()
    } else {
        None
    };
    let _guard = s.activity_edit.lock().unwrap();
    let mut folders = roots(s)?;
    if let Some(path) = remove {
        folders.retain(|p| p != Path::new(path));
    } else if let Some(path) = picked {
        let p = path.canonicalize().msg()?;
        if !folders.contains(&p) {
            if folders.len() >= 12 {
                return Err("Choose up to twelve folders.".into());
            }
            folders.push(p);
        }
    } else {
        return Ok(Value::Null);
    }
    s.set("activityFolders", &json!(folders))?;
    s.set("activityEpoch", &json!(id()))?;
    Ok(Value::Null)
}
fn usable(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|v| v.to_str())
            .map(str::to_lowercase)
            .as_deref(),
        Some("md" | "txt" | "csv" | "tsv" | "pdf" | "docx" | "xlsx" | "rtf" | "doc")
    )
}
fn checked(root: &Path, path: &Path) -> Option<PathBuf> {
    let path = path.canonicalize().ok()?;
    let rel = path.strip_prefix(root).ok()?;
    if rel
        .components()
        .any(|c| !matches!(c,Component::Normal(n) if !n.to_string_lossy().starts_with('.')))
        || !usable(&path)
    {
        return None;
    }
    Some(path)
}
fn matches<'a>(text: &str, words: impl Iterator<Item = &'a String>) -> usize {
    let text = text.to_lowercase();
    words.filter(|t| text.contains(t.as_str())).count()
}
fn candidates(root: &Path, words: &[String]) -> Vec<PathBuf> {
    let mut result = Vec::new();
    if !words.is_empty() {
        let query = words
            .iter()
            .map(|w| format!("(kMDItemFSName == '*{w}*'cd || kMDItemTextContent == '*{w}*'cd)"))
            .collect::<Vec<_>>()
            .join(" || ");
        if let Ok(mut child) = Command::new("/usr/bin/mdfind")
            .arg("-0")
            .arg("-onlyin")
            .arg(root)
            .arg(query)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(out) = child.stdout.take() {
                for path in BufReader::new(out).split(0).take(64).flatten() {
                    if let Ok(p) = String::from_utf8(path) {
                        let p = PathBuf::from(p);
                        if usable(&p) {
                            result.push(p);
                        }
                    }
                }
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    // Fresh files and volumes with Spotlight disabled still work.
    let mut queue = VecDeque::from([root.to_owned()]);
    let mut seen = 0;
    while let Some(dir) = queue.pop_front() {
        let Ok(items) = fs::read_dir(dir) else {
            continue;
        };
        for item in items.flatten() {
            seen += 1;
            if seen > 512 {
                return result;
            }
            let path = item.path();
            let name = item.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let Ok(kind) = item.file_type() else { continue };
            if kind.is_dir()
                && !matches!(name.to_str(), Some("node_modules" | "target" | "Library"))
                && path.extension().is_none()
            {
                queue.push_back(path);
            } else if kind.is_file() && usable(&path) {
                result.push(path);
            }
        }
    }
    result
}
pub fn search(s: &Store, question: &str, prior: &Value) -> Res<Vec<Value>> {
    let folders = roots(s)?;
    if folders.is_empty() {
        return Ok(Vec::new());
    }
    let mut words: Vec<_> = conversation::words(question)
        .into_iter()
        .filter(|w| w.len() > 2 && w.chars().all(char::is_alphanumeric))
        .collect();
    words.sort_by_key(|w| std::cmp::Reverse(w.len()));
    words.truncate(8);
    let mut ranked = Vec::new();
    let mut unique = HashSet::new();
    for root in folders {
        let earlier: Vec<PathBuf> = prior
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(PathBuf::from)
            .filter(|p| p.starts_with(&root))
            .collect();
        let mut list = earlier.clone();
        list.extend(candidates(&root, &words));
        for (order, p) in list.iter().enumerate() {
            if let Some(p) = checked(&root, p) {
                if !unique.insert(p.clone()) {
                    continue;
                }
                let name = p.file_name().unwrap().to_string_lossy();
                let name_hits = matches(&name, words.iter());
                let pinned = earlier.contains(&p);
                ranked.push((pinned, name_hits, order, p));
            }
        }
    }
    ranked.sort_by_key(|(p, n, i, _)| (std::cmp::Reverse(*p), std::cmp::Reverse(*n), *i));
    let mut hits = Vec::new();
    let mut total = 0;
    for (pinned, n, _, p) in ranked.into_iter().take(36) {
        let Ok(file) = fs::File::open(&p) else {
            continue;
        };
        let Ok(meta) = file.metadata() else { continue };
        if !meta.is_file() || meta.len() > 8 * 1024 * 1024 {
            continue;
        }
        let name = p.file_name().unwrap().to_string_lossy();
        let plain = matches!(
            p.extension()
                .and_then(|e| e.to_str())
                .map(str::to_lowercase)
                .as_deref(),
            Some("md" | "txt" | "csv" | "tsv")
        );
        let limit = if plain { 256 * 1024 } else { 8 * 1024 * 1024 };
        let mut bytes = Vec::new();
        if file.take(limit).read_to_end(&mut bytes).is_err() {
            continue;
        }
        total += bytes.len();
        if total > 24 * 1024 * 1024 {
            break;
        }
        let complete_text = plain && meta.len() == bytes.len() as u64;
        let body = if plain {
            String::from_utf8_lossy(&bytes).into_owned()
        } else {
            let Ok(v) = attachments::read(&json!({"name":name,"data":STANDARD.encode(bytes)}))
            else {
                continue;
            };
            v["text"].as_str().unwrap_or("").to_owned()
        };
        let count = matches(&body, words.iter());
        if !pinned && n + count == 0 {
            continue;
        }
        let excerpt =
            crate::activity::excerpt(&body.chars().take(65000).collect::<String>(), question);
        hits.push((
            pinned,
            4 * n + count,
            json!({"name":name,"path":p,"excerpt":excerpt,"complete":complete_text && excerpt == body}),
        ));
    }
    hits.sort_by_key(|(p, n, _)| (std::cmp::Reverse(*p), std::cmp::Reverse(*n)));
    Ok(hits.into_iter().take(3).map(|v| v.2).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn read_fresh_authorized_documents_and_skip_external_links() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Documents");
        fs::create_dir(&root).unwrap();
        let s = Store::with_key(temp.path().join("notebook"), &[5; 32]).unwrap();
        let private = temp.path().join("private.md");
        fs::write(&private, "Zephyr secret").unwrap();
        std::os::unix::fs::symlink(&private, root.join("leak.md")).unwrap();
        let plan = root.join("schedule.md");
        fs::write(
            &plan,
            format!("{}\nZephyr studio opens at 7.", "Other notes. ".repeat(110)),
        )
        .unwrap();
        s.set("activityFolders", &json!([root.canonicalize().unwrap()]))
            .unwrap();
        let found = search(&s, "When does Zephyr studio open?", &Value::Null).unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0]["excerpt"].as_str().unwrap().contains("opens at 7"));
        assert_eq!(found[0]["complete"], false);
        fs::write(&plan, "Zephyr now opens at 9.").unwrap();
        let updated = search(&s, "Zephyr", &Value::Null).unwrap();
        assert_eq!(updated[0]["excerpt"], "Zephyr now opens at 9.");
        assert_eq!(updated[0]["complete"], true);
        configure(&s, Some(root.canonicalize().unwrap().to_str().unwrap())).unwrap();
        assert!(search(&s, "Zephyr", &json!([plan])).unwrap().is_empty());
    }
}
