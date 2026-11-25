// Apple Mail exports a mailbox as a .mbox directory containing an "mbox" file.
// Read it as a stream. Only decoded message text enters the encrypted index.
use super::*;
use mailparse::{DispositionType, MailHeaderMap, ParsedMail};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

const MAX_MESSAGE: usize = 24 * 1024 * 1024;

fn body_part<'a, 'b>(message: &'a ParsedMail<'b>, mime: &str) -> Option<&'a ParsedMail<'b>> {
    let disposition = message.get_content_disposition();
    if disposition.disposition == DispositionType::Attachment
        || disposition.params.contains_key("filename")
        || message.ctype.params.contains_key("name")
    {
        return None;
    }
    if message.ctype.mimetype.eq_ignore_ascii_case(mime) {
        return Some(message);
    }
    message
        .subparts
        .iter()
        .find_map(|part| body_part(part, mime))
}

fn visible_text(message: &ParsedMail<'_>) -> Option<String> {
    if let Some(text) = body_part(message, "text/plain")
        .and_then(|part| part.get_body().ok())
        .filter(|s| !s.trim().is_empty())
    {
        return Some(text);
    }
    let html = body_part(message, "text/html")?.get_body().ok()?;
    // Parse as data only. No browser, network fetches or script execution.
    let doc = dom_query::Document::from(html);
    doc.select("script,style,template,noscript,head,svg,[hidden],img")
        .remove();
    let text = doc.select("body").formatted_text().to_string();
    (!text.trim().is_empty()).then_some(text)
}

fn message_row(bytes: &[u8]) -> Option<[String; 3]> {
    let parsed = mailparse::parse_mail(bytes).ok()?;
    let title = parsed.headers.get_first_value("Subject")?;
    let body = visible_text(&parsed)?
        .replace("\r\n", "\n")
        .chars()
        .take(12000)
        .collect::<String>();
    let key = parsed
        .headers
        .get_first_value("Message-ID")
        .unwrap_or_else(|| hash(&format!("{title}\n{body}")));
    Some([key, title, body])
}

fn read_archive(reader: impl BufRead, rows: &mut BTreeMap<String, [String; 3]>) -> Res<usize> {
    let mut skipped = 0;
    let mut current = Vec::new();
    let mut oversized = false;
    let mut started = false;
    let mut finish = |raw: &[u8], oversized: bool| {
        if !oversized {
            if let Some(row) = message_row(raw) {
                rows.insert(row[0].clone(), row);
                return;
            }
        }
        skipped += 1;
    };
    for line in reader.split(b'\n') {
        let line = line.msg()?;
        if line.starts_with(b"From ") {
            if started {
                finish(&current, oversized);
            }
            started = true;
            current.clear();
            oversized = false;
        } else if started && !oversized {
            // mboxrd escapes body lines beginning with "From " (also ">From ", etc).
            let from = line.iter().take_while(|c| **c == b'>').count();
            let unescaped = if from > 0 && line[from..].starts_with(b"From ") {
                &line[1..]
            } else {
                &line[..]
            };
            if current.len() + unescaped.len() + 1 > MAX_MESSAGE {
                oversized = true;
                current.clear();
            } else {
                current.extend_from_slice(unescaped);
                current.push(b'\n');
            }
        }
    }
    if started {
        finish(&current, oversized);
    } else {
        return Err("This file has no mbox messages.".into());
    }
    Ok(skipped)
}

fn files(path: &Path) -> Res<Vec<PathBuf>> {
    let meta = path.symlink_metadata().msg()?;
    if meta.file_type().is_symlink() {
        return Err("Choose the original mailbox export.".into());
    }
    if meta.is_file() {
        return Ok(vec![path.to_owned()]);
    }
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(path).msg()? {
        let entry = entry.msg()?;
        let p = entry.path();
        let typ = entry.file_type().msg()?;
        if typ.is_file()
            && (entry.file_name() == "mbox" || p.extension().is_some_and(|v| v == "mbox"))
        {
            paths.push(p);
        } else if typ.is_dir() && p.extension().is_some_and(|v| v == "mbox") {
            let mbox = p.join("mbox");
            if mbox.symlink_metadata().is_ok_and(|m| m.is_file()) {
                paths.push(mbox);
            }
        }
    }
    if paths.is_empty() {
        return Err(
            "Choose an exported .mbox folder or the folder containing your mailbox exports.".into(),
        );
    }
    paths.sort();
    Ok(paths)
}

pub fn import(s: &Store, path: &Path) -> Res<Value> {
    let epoch = s.get("activityEpoch")?;
    let mut rows = BTreeMap::new();
    let mut skipped = 0;
    for file in files(path)? {
        skipped += read_archive(BufReader::new(std::fs::File::open(file).msg()?), &mut rows)?;
    }
    if rows.is_empty() {
        return Err(
            "No readable message bodies were found. Your existing import is intact.".into(),
        );
    }
    let rows: Vec<_> = rows.into_values().collect();
    imported::replace(s, "mail", &rows, &epoch)?;
    Ok(json!({"count": rows.len(), "skipped": skipped}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn html_only_mail_uses_decoded_visible_text() {
        let raw = b"Subject: Appointment\nContent-Type: text/html; charset=utf-8\n\n<style>.internal{}</style><p>Use door A &amp; B.</p><script>hidden()</script><img src='https://invalid.example/pixel'>";
        let row = message_row(raw).unwrap();
        assert_eq!(row[2].trim(), "Use door A & B.");
    }

    #[test]
    fn imports_decoded_plain_text_and_skips_attachments_and_duplicate_messages() {
        let mailbox = b"From a@example.test Wed Mar 11 12:00:00 2026\nSubject: Studio\nMessage-ID: <a>\nContent-Type: multipart/mixed; boundary=x\n\n--x\nContent-Disposition: attachment\nContent-Type: text/plain\n\nSecret attachment\n--x\nContent-Type: text/plain; charset=utf-8\nContent-Transfer-Encoding: quoted-printable\n\nMeet at =34.\n>From the studio.\n--x--\n";
        let mut rows = BTreeMap::new();
        for _ in 0..2 {
            assert_eq!(read_archive(&mailbox[..], &mut rows).unwrap(), 0);
        }
        assert_eq!(rows.len(), 1);
        assert!(
            rows["<a>"][2].contains("Meet at 4.\nFrom the studio."),
            "{:?}",
            rows["<a>"][2]
        );
        assert!(!rows["<a>"][2].contains("Secret"));
    }
}
