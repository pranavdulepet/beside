use crate::local;
use regex::Regex;
use serde_json::Value;
use std::{collections::HashMap, sync::LazyLock};

static SECRET_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?x)
        (?:sk-(?:ant-|proj-|or-v1-)?[A-Za-z0-9_\-]{16,}) |
        (?:AIza[0-9A-Za-z_\-]{28,}) |
        (?:gh[pousr]_[A-Za-z0-9]{24,}) |
        (?:AKIA[0-9A-Z]{16}) |
        (?:-----BEGIN\s+(?:[A-Z]+\s+)*PRIVATE\s+KEY-----[\s\S]*?-----END\s+(?:[A-Z]+\s+)*PRIVATE\s+KEY-----)
    ")
    .unwrap()
});

const REDACTED: &str = "[REDACTED_SECRET]";
const PRESERVE_ALIASES: &str =
    "Preserve bracketed placeholders like [B...] verbatim if you refer to them.";
static DIGIT_GROUP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b\d+\b").unwrap());
static DECIMAL_DIGIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d$").unwrap());

// Unicode decimal digits occur in consecutive sets of ten starting at zero.
// This also handles adjacent sets, such as the mathematical typefaces.
fn decimal_digit(ch: char) -> u8 {
    if ch.is_ascii_digit() {
        return ch as u8 - b'0';
    }
    let mut first = ch as u32;
    let mut buf = [0; 4];
    while let Some(prev) = char::from_u32(first - 1) {
        if !DECIMAL_DIGIT.is_match(prev.encode_utf8(&mut buf)) {
            break;
        }
        first -= 1;
    }
    ((ch as u32 - first) % 10) as u8
}

fn valid_card(digits: &[u8]) -> bool {
    if !(13..=19).contains(&digits.len()) {
        return false;
    }
    let mut sum = 0;
    for (i, &digit) in digits.iter().rev().enumerate() {
        let n = if i % 2 == 1 { 2 * digit } else { digit };
        sum += if n > 9 { n - 9 } else { n };
    }
    sum % 10 == 0
}

fn redact_cards(text: &str) -> String {
    let groups: Vec<_> = DIGIT_GROUP.find_iter(text).collect();
    let mut output = String::with_capacity(text.len());
    let mut copied = 0;
    let mut digits = Vec::with_capacity(19);
    // Try whole groups so a nearby quantity cannot hide a card or truncate a longer ID.
    for (i, first) in groups.iter().enumerate() {
        digits.clear();
        let mut end = first.start();
        for group in &groups[i..] {
            if !text[end..group.start()]
                .bytes()
                .all(|b| matches!(b, b' ' | b'-'))
            {
                break;
            }
            digits.extend(
                group
                    .as_str()
                    .chars()
                    .take(20 - digits.len())
                    .map(decimal_digit),
            );
            if digits.len() > 19 {
                break;
            }
            end = group.end();
            if valid_card(&digits) {
                if first.start() >= copied {
                    output.push_str(&text[copied..first.start()]);
                    output.push_str(REDACTED);
                }
                copied = copied.max(end);
            }
        }
    }
    output.push_str(&text[copied..]);
    output
}

static EMAIL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}\b").unwrap());

static PHONE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        (?:\+\d{1,3}[\s.\-]?)?
        (?:\(\d{3}\)|\b\d{3})[\s.\-]\d{3}[\s.\-]\d{4}\b
    ",
    )
    .unwrap()
});

static HOME_PATH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"/(?:Users|home)/[A-Za-z0-9._\-]+(?:/[^\s,;'\x22\)\]]+)*").unwrap()
});

static PLACEHOLDER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[B[a-f0-9]{32}_(?:EMAIL|PHONE|PATH|PERSON)_\d+\]").unwrap());

const EXCLUDED_NAMES: &[&str] = &[
    "beside",
    "sumi",
    "inkling",
    "apple",
    "safari",
    "chrome",
    "claude",
    "openai",
    "chatgpt",
    "gemini",
    "mac",
    "macos",
    "xcode",
    "cursor",
    "codex",
    "slack",
    "gmail",
    "google",
    "github",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// Each request gets a new set of aliases. Names, contact details, and paths can
/// be restored in the local reply. Recognized secrets have no reverse mapping.
pub struct Pseudonymizer {
    nonce: String,
    forward: HashMap<String, String>,
    reverse: HashMap<String, String>,
    counts: HashMap<&'static str, usize>,
}

impl Pseudonymizer {
    pub fn new() -> Self {
        Self {
            nonce: crate::store::id(),
            forward: HashMap::new(),
            reverse: HashMap::new(),
            counts: HashMap::new(),
        }
    }

    fn token_for(&mut self, prefix: &'static str, raw: &str) -> String {
        if let Some(existing) = self.forward.get(raw) {
            return existing.clone();
        }
        let next = self.counts.entry(prefix).or_insert(0);
        *next += 1;
        let token = format!("[B{}_{prefix}_{next}]", self.nonce);
        self.forward.insert(raw.to_owned(), token.clone());
        self.reverse.insert(token.clone(), raw.to_owned());
        token
    }

    pub fn scrub_str(&mut self, input: &str) -> String {
        if input.is_empty() {
            return String::new();
        }
        // 1. Hard-redact irreversible secrets first.
        let step1 = redact_cards(&SECRET_RE.replace_all(input, REDACTED));

        // 2. Reversible replacement for emails.
        let mut out = String::with_capacity(step1.len());
        let mut last = 0;
        for m in EMAIL_RE.find_iter(&step1) {
            out.push_str(&step1[last..m.start()]);
            out.push_str(&self.token_for("EMAIL", m.as_str()));
            last = m.end();
        }
        out.push_str(&step1[last..]);

        // 3. Reversible replacement for local filesystem home paths (preserve trailing sentence punctuation).
        let step3 = out;
        let mut out = String::with_capacity(step3.len());
        let mut last = 0;
        for m in HOME_PATH_RE.find_iter(&step3) {
            let raw = m.as_str();
            let trimmed = raw.trim_end_matches(['.', ',', ';', ':']);
            let end = m.start() + trimmed.len();
            out.push_str(&step3[last..m.start()]);
            out.push_str(&self.token_for("PATH", trimmed));
            last = end;
        }
        out.push_str(&step3[last..]);

        // 4. Reversible replacement for phone numbers.
        let step4 = out;
        let mut out = String::with_capacity(step4.len());
        let mut last = 0;
        for m in PHONE_RE.find_iter(&step4) {
            out.push_str(&step4[last..m.start()]);
            out.push_str(&self.token_for("PHONE", m.as_str()));
            last = m.end();
        }
        out.push_str(&step4[last..]);

        // 5. Reversible replacement for personal names via native macOS NLTagger.
        let mut names = local::names(&out);
        names.sort_by_key(|n| std::cmp::Reverse(n.len()));
        names.dedup();
        for name in names {
            let trimmed = name.trim();
            if trimmed.len() < 3
                || trimmed.starts_with('[')
                || EXCLUDED_NAMES.contains(&trimmed.to_lowercase().as_str())
            {
                continue;
            }
            if out.contains(trimmed) {
                let placeholder = self.token_for("PERSON", trimmed);
                out = out.replace(trimmed, &placeholder);
            }
        }
        out
    }

    // Call after scrubbing the payload, so aliases from either part are covered.
    pub fn scrub_instructions(&mut self, system: &str) -> String {
        let mut instructions = self.scrub_str(system);
        if self.count() > 0 {
            instructions.push('\n');
            instructions.push_str(PRESERVE_ALIASES);
        }
        instructions
    }

    pub fn scrub_value(&mut self, value: &Value) -> Value {
        match value {
            Value::String(s) => Value::String(self.scrub_str(s)),
            Value::Number(n) if n.is_u64() => {
                let digits: Vec<_> = n.to_string().bytes().map(|b| b - b'0').collect();
                if valid_card(&digits) {
                    Value::String(REDACTED.into())
                } else {
                    value.clone()
                }
            }
            Value::Array(arr) => Value::Array(arr.iter().map(|v| self.scrub_value(v)).collect()),
            Value::Object(map) => {
                let mut next = serde_json::Map::with_capacity(map.len());
                for (k, v) in map {
                    next.insert(self.scrub_str(k), self.scrub_value(v));
                }
                Value::Object(next)
            }
            other => other.clone(),
        }
    }

    pub fn restore(&self, text: &str) -> String {
        if self.reverse.is_empty() || !text.contains('[') {
            return text.to_owned();
        }
        PLACEHOLDER_RE
            .replace_all(text, |caps: &regex::Captures| {
                let tok = &caps[0];
                self.reverse
                    .get(tok)
                    .cloned()
                    .unwrap_or_else(|| tok.to_owned())
            })
            .into_owned()
    }

    pub fn count(&self) -> usize {
        self.reverse.len()
    }
}

/// Restores this request's aliases even when SSE splits one across several deltas.
pub struct StreamRestorer<'a> {
    bridge: &'a Pseudonymizer,
    buffer: String,
}

impl<'a> StreamRestorer<'a> {
    pub fn new(bridge: &'a Pseudonymizer) -> Self {
        Self {
            bridge,
            buffer: String::new(),
        }
    }

    pub fn push(&mut self, delta: &str) -> String {
        if self.bridge.count() == 0 {
            return delta.to_owned();
        }
        self.buffer.push_str(delta);
        // Buffer a suffix only while it can still become one of this request's aliases.
        if let Some(open) = self.buffer.rfind('[') {
            let tail = &self.buffer[open..];
            if tail.len() <= 80 && self.bridge.reverse.keys().any(|key| key.starts_with(tail)) {
                let safe_prefix = self.buffer[..open].to_owned();
                self.buffer = tail.to_owned();
                return self.bridge.restore(&safe_prefix);
            }
        }
        let flushed = std::mem::take(&mut self.buffer);
        self.bridge.restore(&flushed)
    }

    pub fn finish(&mut self) -> String {
        if self.buffer.is_empty() {
            return String::new();
        }
        let tail = std::mem::take(&mut self.buffer);
        self.bridge.restore(&tail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn card_redaction_keeps_non_card_identifiers_and_their_json_types() {
        let mut bridge = Pseudonymizer::new();
        for card in [
            "4222222222222",
            "3782-822463-10005",
            "4242 4242 4242 4242",
            "4000000000000000006",
            "４２４２ ４２４２ ４２４２ ４２４２",
        ] {
            let masked = bridge.scrub_str(card);
            assert_eq!(bridge.restore(&masked), REDACTED);
        }
        assert_eq!(bridge.count(), 0);
        let identifiers = json!({"isbn":"978-0-306-40615-7", "order":"1234567890123456",
                                 "stamp":1700000000123_u64, "tracking":"123456789012345678901"});
        assert_eq!(bridge.scrub_value(&identifiers), identifiers);
        assert_eq!(bridge.scrub_value(&json!(4242424242424242_u64)), REDACTED);
        assert_eq!(
            redact_cards("2 4242 4242 4242 4242."),
            format!("2 {REDACTED}.")
        );
        assert_eq!(
            redact_cards("4111111111111111 6011111111111117"),
            format!("{REDACTED} {REDACTED}")
        );
    }

    #[test]
    fn cloud_instructions_preserve_aliases_created_in_either_part_of_the_request() {
        let mut bridge = Pseudonymizer::new();
        let text = "Answer directly.";
        bridge.scrub_value(&json!({"secret":"4242424242424242"}));
        assert_eq!(bridge.scrub_instructions(text), text);
        let value = bridge.scrub_value(&json!({"recipients":["test@example.com"]}));
        assert_eq!(
            bridge.scrub_instructions(text),
            format!("{text}\n{PRESERVE_ALIASES}")
        );
        assert_eq!(
            bridge.restore(value["recipients"][0].as_str().unwrap()),
            "test@example.com"
        );
        let mut bridge = Pseudonymizer::new();
        let system = bridge.scrub_instructions("Reply to test@example.com.");
        assert!(system.ends_with(PRESERVE_ALIASES));
        assert!(!system.contains("test@example.com"));
        assert!(bridge
            .restore(&system)
            .starts_with("Reply to test@example.com."));
    }

    #[test]
    fn secrets_cannot_be_restored_and_literal_tokens_are_not_aliases() {
        let mut bridge = Pseudonymizer::new();
        let masked=bridge.scrub_str("Card 4242 4242 4242 4242. -----BEGIN PRIVATE KEY-----\nSYNTHETIC\n-----END PRIVATE KEY-----");
        let restored = bridge.restore(&masked);
        assert!(!restored.contains("4242"));
        assert!(!restored.contains("SYNTHETIC"));
        let other = Pseudonymizer::new();
        let token = bridge.scrub_str("test@example.com");
        assert_eq!(other.restore(&token), token);
    }

    #[test]
    fn round_trips_emails_phones_paths_and_names_while_hard_redacting_secrets() {
        let mut p = Pseudonymizer::new();
        let raw = "Email maya.lin@example.com or call 415-555-0199 about /Users/maya/Documents/plan.md. Secret key: sk-ant-1234567890abcdef123456.";
        let scrubbed = p.scrub_str(raw);
        assert!(!scrubbed.contains("maya.lin@example.com"));
        assert!(!scrubbed.contains("415-555-0199"));
        assert!(!scrubbed.contains("/Users/maya/Documents/plan.md"));
        assert!(!scrubbed.contains("sk-ant-1234567890abcdef123456"));
        let tok = |label: &str| format!("[B{}_{label}_1]", p.nonce);
        assert!(scrubbed.contains(&tok("EMAIL")));
        assert!(scrubbed.contains(&tok("PHONE")));
        assert!(scrubbed.contains(&tok("PATH")));
        assert!(scrubbed.contains("[REDACTED_SECRET]"));

        // Rehydration restores safe personal identifiers, NEVER secrets!
        let reply = "Sure, I drafted a note to [EMAIL_1] referencing [PATH_1] and [PHONE_1].";
        let restored = p.restore(
            &reply
                .replace("[EMAIL_1]", &tok("EMAIL"))
                .replace("[PATH_1]", &tok("PATH"))
                .replace("[PHONE_1]", &tok("PHONE")),
        );
        assert_eq!(
            restored,
            "Sure, I drafted a note to maya.lin@example.com referencing /Users/maya/Documents/plan.md and 415-555-0199."
        );
        assert!(!p.restore("Key was [REDACTED_SECRET]").contains("sk-ant-"));

        // Recursive JSON scrubbing works on nested objects & arrays.
        let payload = json!({"notes": ["Contact maya.lin@example.com"]});
        let clean = p.scrub_value(&payload);
        assert_eq!(
            p.restore(clean["notes"][0].as_str().unwrap()),
            "Contact maya.lin@example.com"
        );
        assert_eq!(p.restore("[EMAIL_1]"), "[EMAIL_1]");
    }

    #[test]
    fn stream_restorer_rehydrates_placeholders_split_across_sse_chunks() {
        let mut p = Pseudonymizer::new();
        let token = p.scrub_str("alex@beside.app");
        let mut stream = StreamRestorer::new(&p);
        let mut out = String::new();
        for ch in format!("Drafting to {token} right now!").chars() {
            out.push_str(&stream.push(&ch.to_string()));
        }
        out.push_str(&stream.finish());
        assert_eq!(out, "Drafting to alex@beside.app right now!");
    }
}
