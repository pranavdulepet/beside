// Store evidence pointers alongside the captured booking. All wording and values
// are read back from their originals under the current observation permissions.
mod checkout;
mod vision;
use super::*;
use chrono::{Datelike, NaiveDate};
use regex::Regex;
use rusqlite::{Connection, OptionalExtension};
use std::sync::LazyLock;

static ASK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)\b(?:do (?:i|we) need|should (?:i|we) (?:buy|add|get|pay for|check)|do (?:i|we) have to (?:buy|add))\b.{0,24}\b(?:extra|another|second|checked|additional) (?:[1-9][0-9]?\s*kg )?(?:checked |check[- ]in |carry[- ]on )?(?:bags?|baggage|luggage)\b").unwrap()
});
static BAG_AT_CHECKOUT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?ix)\b(?:(?:(?:should|can)\s+(?:i|we)\s+(?:add|buy|get|check|skip|pay\s+for)|do\s+(?:i|we)\s+(?:need(?:\s+to\s+(?:add|buy|check))?|have\s+to\s+(?:add|buy|pay\s+for))|is\s+it\s+worth\s+(?:adding|buying|checking))\s+(?:(?:a|an|one|the|this|my|our|another|extra|second|additional|checked|carry[-\s]on|check[-\s]in)\s+)*(?:bags?|baggage|suitcases?|luggage)|(?:is|will)\s+one\s+(?:checked\s+)?bag\s+(?:be\s+)?enough)\b"
).unwrap()
});
static LIMIT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)^\s*(?:(?:your|included|one|1) )?(?:carry[- ]on|cabin bag)(?: baggage| bag| allowance| weight limit)?\s*(?:[:·—-]\s*|up to\s+|maximum\s+|limit\s+)([0-9]{1,2}(?:\.[0-9])?)\s*kg(?:\s*(?:max(?:imum)?|per bag))?[.!]?\s*$").unwrap()
});
static WEIGHED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)^\s*(?:my|our) (?:packed |reweighed )?(?:carry[- ]on|bag|suitcase) (?:weighs|weighed|is now) ([0-9]{1,2}(?:\.[0-9])?)\s*kg[.!]?\s*$").unwrap()
});
static REPLY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?ix)^\s*(?:actually[, ]+)?(?:i\s+(?:just\s+)?(?:re)?weighed\s+(?:it|(?:my|the)\s+(?:bag|suitcase))[.,: —-]*\s*)?(?:(?:it|my\s+(?:packed\s+)?(?:bag|suitcase))(?:\s+weighs|\s+is|['’]s)\s+)?([0-9]{1,2}(?:\.[0-9])?)\s*(?:kg|kilograms?)[.!]?\s*$").unwrap()
});

fn grams(text: &str) -> Option<i32> {
    let (kg, tenth) = text.split_once('.').unwrap_or((text, "0"));
    let g = kg.parse::<i32>().ok()? * 1000 + tenth.parse::<i32>().ok()? * 100;
    (g > 0).then_some(g)
}
fn kg(g: i32) -> String {
    if g % 1000 == 0 {
        (g / 1000).to_string()
    } else {
        format!("{}.{}", g / 1000, (g % 1000) / 100)
    }
}
fn one(body: &str, pattern: &Regex) -> Option<(i32, String)> {
    let mut hits = body.lines().filter_map(|l| {
        let c = pattern.captures(l)?;
        Some((grams(&c[1])?, l.trim().to_owned()))
    });
    let first = hits.next()?;
    hits.all(|h| h.0 == first.0).then_some(first)
}
// Native OCR preserves cells on the same visual row. Accept a fare matrix
// only if every headed column explicitly gives the same primary bag allowance.
static CELL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)^([0-9]{1,2}(?:\.[0-9])?)\s*kg(?:\s*\+\s*[0-9]{1,2}(?:\.[0-9])?\s*kg\s+personal item)?$"
).unwrap()
});

fn matrix(body: &str) -> Option<(i32, String, &str)> {
    let lines: Vec<_> = body.lines().collect();
    let mut found = None;
    for (i, line) in lines.iter().enumerate() {
        let mut cells = line.split('\t').map(str::trim);
        if !matches!(
            cells.next()?.to_lowercase().as_str(),
            "hand baggage" | "carry-on" | "cabin baggage" | "cabin bag"
        ) {
            continue;
        }
        let header = lines.get(i.checked_sub(1)?)?;
        let columns = header.split('\t').count().checked_sub(1)?;
        let cells: Vec<_> = cells.collect();
        if columns == 0 || cells.len() != columns || found.is_some() {
            return None;
        }
        let mut weights = cells
            .iter()
            .map(|c| CELL.captures(c).and_then(|v| grams(&v[1])));
        let first = weights.next()??;
        if !weights.all(|w| w == Some(first)) {
            return None;
        }
        let section = lines[..i - 1]
            .iter()
            .rev()
            .find(|s| !s.trim().is_empty())
            .copied()
            .unwrap_or("");
        found = Some((first, cells[0].to_string(), section));
    }
    found
}

// The header identifies what each value buys. Read the selected itinerary's
// row as a unit, including its destination. A second itinerary is ambiguous.
struct CheckedFees<'a> {
    destination: &'a str,
    first: &'a str,
    second: String,
    limit: i32,
    line: String,
    included: bool,
    included_count: u32,
    selected: Option<u32>,
}
static PRICE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:[$€£][0-9]{1,4}(?:[.,][0-9]{2})?|[0-9]{1,4} (?:USD|EUR|GBP))$").unwrap()
});
static CHECKED_LIMIT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:[0-9]{1,3} [li]bs?\s*\()?([1-9][0-9]?(?:\.[0-9])?)\s*kgs?\)?$").unwrap()
});
fn checked_fees(body: &str) -> Option<CheckedFees<'_>> {
    if body.starts_with(checkout::MARKER) {
        let mut lines = body.lines();
        if lines.next()? != checkout::MARKER {
            return None;
        }
        let destination = lines.next()?.strip_prefix("Destination: ")?;
        let outbound = lines.next()?.strip_prefix("Outbound: ")?;
        let (code, date) = outbound.split_once(' ')?;
        if !destination.ends_with(&format!("({code})"))
            || code.len() != 3
            || !code.bytes().all(|b| b.is_ascii_uppercase())
            || trip_date(date).is_none()
        {
            return None;
        }
        let limit = grams(
            lines
                .next()?
                .strip_prefix("Per bag: ")?
                .strip_suffix(" kg")?,
        )?;
        let included_count: u32 = lines.next()?.strip_prefix("Included: ")?.parse().ok()?;
        let selected: u32 = lines.next()?.strip_prefix("Selected: ")?.parse().ok()?;
        let price = lines.next()?.strip_prefix("Price: ")?;
        let host = lines.next()?.strip_prefix("Source: ")?;
        let detail = lines.next()?.strip_prefix("On screen: ")?;
        let itinerary = lines.next()?.strip_prefix("Itinerary on screen: ")?;
        if lines.next().is_some()
            || itinerary.is_empty()
            || !host.contains('.')
            || included_count > 9
            || selected > 9
            || !checkout::allowed_price(price)
            || detail.is_empty()
        {
            return None;
        }
        return Some(CheckedFees {
            destination,
            first: price,
            second: price.to_owned(),
            limit,
            line: detail.to_owned(),
            included: true,
            included_count,
            selected: Some(selected),
        });
    }
    if body.starts_with("Check-in bag selection\n") {
        return checked_offer(body);
    }
    let mut lines = body.lines();
    lines.find(|l| {
        l.trim()
            .eq_ignore_ascii_case("flight\tfirst bag\tsecond bag\tweight per bag")
    })?;
    let row = lines.next()?;
    let parse = |line: &str| {
        line.split('\t').count() == 4
            && line
                .split('\t')
                .nth(1)
                .is_some_and(|v| PRICE.is_match(v.trim()))
    };
    if !parse(row) || lines.any(|l| parse(l) || l.to_ascii_lowercase().contains("weight per bag")) {
        return None;
    }
    let mut cells = row.split('\t').map(str::trim);
    let route = cells.next()?;
    let (_, destination) = route
        .split_once('→')
        .or_else(|| route.split_once(" -> "))
        .or_else(|| route.split_once(" to "))?;
    let first = cells.next()?;
    let second = cells.next()?;
    if destination.trim().is_empty() || !PRICE.is_match(second) {
        return None;
    }
    let limit = grams(&CHECKED_LIMIT.captures(cells.next()?)?[1])?;
    Some(CheckedFees {
        destination: destination.trim(),
        first,
        second: second.to_owned(),
        limit,
        line: row.to_owned(),
        included: false,
        included_count: 0,
        selected: None,
    })
}

// On a bag selector, the included allowance and the marginal price must
// belong to the same passenger, control and checked-bag cell.
fn checked_offer(body: &str) -> Option<CheckedFees<'_>> {
    static ADD: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^Add ([1-9][0-9]?)\s*kg check-in bag$").unwrap());
    static OFFER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r"(?i)^([1-3])\s+([$£€])\s*([0-9]{1,4})(?:\s*[.,]\s*([0-9]{2}))?\s+1 bag included (?:with|in) .+$"
    ).unwrap()
    });
    let mut lines = body.lines();
    if lines.next()? != "Check-in bag selection" {
        return None;
    }
    let destination = lines.next()?.strip_prefix("Destination: ")?;
    let flight = lines.next()?.strip_prefix("Outbound: ")?;
    if destination.is_empty() {
        return None;
    }
    if flight != "unverified" {
        let (code, date) = flight.split_once(' ')?;
        if code.len() != 3
            || !code.bytes().all(|b| b.is_ascii_uppercase())
            || chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err()
        {
            return None;
        }
    }
    let control = lines.next()?;
    let limit = grams(&ADD.captures(control)?[1])?;
    let raw = lines.next()?;
    let cell = OFFER.captures(raw)?;
    if lines.any(|l| !l.trim().is_empty()) {
        return None;
    }
    let selected = cell[1].parse::<u32>().ok()?;
    let mut price = format!("{}{}", &cell[2], &cell[3]);
    if let Some(cents) = cell.get(4) {
        price.push('.');
        price.push_str(cents.as_str());
    }
    Some(CheckedFees {
        destination,
        first: "included",
        second: price,
        limit,
        line: format!("{control} · {raw}"),
        included: true,
        included_count: 1,
        selected: Some(selected),
    })
}

fn weather_trip(body: &str, fees: &CheckedFees<'_>) -> Option<Value> {
    let (airport, day) = if fees.included {
        let (code, date) = body
            .lines()
            .nth(2)?
            .strip_prefix("Outbound: ")?
            .split_once(' ')?;
        (code, NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?)
    } else {
        let (name, suffix) = fees.destination.rsplit_once('(')?;
        let code = suffix.strip_suffix(')')?;
        if name.trim().is_empty()
            || code.len() != 3
            || !code.bytes().all(|c| c.is_ascii_uppercase())
        {
            return None;
        }
        let mut lines = body.lines().skip_while(|l| *l != fees.line);
        lines.next()?;
        let date = lines.find(|l| !l.trim().is_empty())?.trim();
        let day = ["%B %d, %Y", "%b %d, %Y", "%Y-%m-%d"]
            .iter()
            .find_map(|p| NaiveDate::parse_from_str(date, p).ok())?;
        (code, day)
    };
    Some(json!({"airport":airport,"date":day.to_string()}))
}

// Mixed fare classes, passengers or units need the user to open a single rule.
fn allowance(body: &str) -> Option<(i32, String)> {
    if body.lines().any(|l| {
        l.split_once('\t').is_some_and(|(label, _)| {
            matches!(
                label.trim().to_lowercase().as_str(),
                "hand baggage" | "cabin bag" | "cabin baggage" | "carry-on"
            )
        })
    }) {
        return matrix(body).map(|(g, line, _)| (g, line));
    }
    for line in body.lines() {
        let text = line.to_lowercase();
        if ["carry-on", "carry on", "cabin bag"]
            .iter()
            .any(|v| text.contains(v))
            && ["kg", "kilogram", " lb", "pound"]
                .iter()
                .any(|v| text.contains(v))
            && !LIMIT.is_match(line)
        {
            return None;
        }
    }
    one(body, &LIMIT)
}

pub(super) fn booking(title: &str, body: &str) -> bool {
    if body.starts_with(checkout::MARKER) {
        return body
            .lines()
            .nth(7)
            .and_then(|s| s.strip_prefix("Source: "))
            .is_some_and(|host| {
                title.strip_prefix("Checked bags · ") == Some(host) && checked_fees(body).is_some()
            });
    }
    if title == "Check-in bags · Ryanair" && checked_offer(body).is_some() {
        return true;
    }
    let title = title.to_lowercase();
    let body = body.to_lowercase();
    [
        "booking",
        "reservation",
        "flight confirmation",
        "flight itinerary",
        "manage trip",
        "check-in",
        "fare types",
        "fare bundles",
        "fare options",
    ]
    .iter()
    .any(|t| title.contains(t))
        && [
            "carry-on",
            "carry on",
            "cabin bag",
            "hand baggage",
            "baggage allowance",
            "checked baggage charges",
        ]
        .iter()
        .any(|t| body.contains(t))
        && !["notes ·", "chatgpt", "claude", "beside ·"]
            .iter()
            .any(|t| title.contains(t))
}

fn note_name(title: &str) -> &str {
    let name = title.trim_start_matches("Notes · ");
    let name = name
        .strip_suffix(" · TextEdit active")
        .and_then(|value| value.split_once(" · ").map(|(_, name)| name))
        .unwrap_or(name);
    name.trim_end_matches(".txt")
        .trim_end_matches(".md")
        .trim_end_matches(".rtf")
}

fn outbound_label(line: &str) -> Option<&str> {
    let (label, value) = line.split_once(':')?;
    [
        "outbound",
        "depart",
        "departs",
        "departure",
        "departure date",
        "travel date",
        "leaving",
    ]
    .contains(&label.trim().to_lowercase().as_str())
    .then_some(value.trim())
}

fn trip_date(raw: &str) -> Option<NaiveDate> {
    ["%Y-%m-%d", "%B %e, %Y", "%b %e, %Y", "%e %B %Y", "%e %b %Y"]
        .iter()
        .find_map(|format| NaiveDate::parse_from_str(raw, format).ok())
}

static NOTE_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?ix)\b(?:[0-9]{4}-[0-9]{2}-[0-9]{2}|(?:jan\w*|feb\w*|mar\w*|apr\w*|may|jun\w*|jul\w*|aug\w*|sep\w*|oct\w*|nov\w*|dec\w*)\s+[0-9]{1,2}(?:,?\s+[0-9]{4})?|[0-9]{1,2}\s+(?:jan\w*|feb\w*|mar\w*|apr\w*|may|jun\w*|jul\w*|aug\w*|sep\w*|oct\w*|nov\w*|dec\w*)(?:\s+[0-9]{4})?)\b").unwrap()
});

// Only discard a travel sentence when the complete sentence describes where
// and when the trip starts. Packing details on the same line stay unestimated.
fn travel_sentence(line: &str) -> Option<(&str, &str)> {
    static PREFIX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r"(?i)^(?:(?:i(?:['’]m| am)|we(?:['’]re| are)) (?:flying|going|travelling|traveling) to |(?:flying to|leaving for|heading to|trip to|weekend in) |(?:my|our) (?:trip to|weekend in) )([\p{L}][\p{L}\p{M} '’.-]{0,55}?)(?:[,]? (?:on|from|leaves(?: on)?|starts(?: on)?|leaving(?: on)?)|,)?\s*$").unwrap()
    });
    let sentence = line.trim().trim_end_matches(['.', '!']).trim();
    let date = NOTE_DATE.find(sentence)?;
    if date.end() != sentence.len() || trip_date(date.as_str()).is_none() {
        return None;
    }
    let place = PREFIX
        .captures(&sentence[..date.start()])?
        .get(1)?
        .as_str()
        .trim();
    Some((place, date.as_str()))
}

fn packing_rank(
    title: &str,
    body: &str,
    context: &str,
    destination: Option<&str>,
    day: Option<NaiveDate>,
) -> Option<u8> {
    let name = words(note_name(title));
    let named_list = name.contains("packing list") || name.contains("packing checklist");
    if !named_list && !super::packing::looks_like_inventory(body) && one(body, &WEIGHED).is_none() {
        return None;
    }
    let has_place = if let Some(destination) = destination {
        let city = words(destination.split('(').next()?.trim());
        let code = destination
            .rsplit_once('(')
            .and_then(|(_, part)| part.strip_suffix(')'));
        let mut heading_matches = format!(" {name} ").contains(&format!(" {city} "));
        for place in body.lines().take(12).filter_map(|line| {
            if let Some((place, _)) = travel_sentence(line) {
                return Some(place);
            }
            let (label, value) = line.split_once(':')?;
            label
                .trim()
                .eq_ignore_ascii_case("destination")
                .then_some(value.trim())
        }) {
            if words(place) != city
                && !code.is_some_and(|code| place.trim().eq_ignore_ascii_case(code))
                && words(place) != words(destination)
            {
                return None;
            }
            heading_matches = true;
        }
        !city.is_empty() && heading_matches
    } else {
        let scope = name
            .replace("packing checklist", "")
            .replace("packing list", "");
        let scope = scope.trim();
        scope.len() >= 3
            && !["my", "our", "travel", "the", "new"].contains(&scope)
            && context.contains(&format!(" {scope} "))
    };
    if !has_place {
        return None;
    }
    // An explicit outbound in the list must agree with this trip. Two trips to
    // the same place can need very different luggage. An undated list ranks
    // below a dated one and ambiguous lists do not supply a weight.
    let mut score = 1;
    for raw in NOTE_DATE
        .find_iter(title)
        .map(|m| m.as_str())
        .chain(body.lines().filter_map(outbound_label))
        .chain(
            body.lines()
                .filter_map(|line| travel_sentence(line).map(|(_, date)| date)),
        )
    {
        let booked = day?;
        let stated = trip_date(raw).or_else(|| {
            ["%B %e %Y", "%b %e %Y", "%e %B %Y", "%e %b %Y"]
                .iter()
                .find_map(|f| {
                    NaiveDate::parse_from_str(&format!("{raw} {}", booked.year()), f).ok()
                })
        })?;
        if stated != booked {
            return None;
        }
        score = 2;
    }
    Some(score)
}

fn packing_items(title: &str, body: &str) -> String {
    body.lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let matches_heading = i == 0 && words(line) == words(note_name(title));
            let is_date = outbound_label(line).is_some_and(|raw| trip_date(raw).is_some());
            let is_destination = line.split_once(':').is_some_and(|(label, value)| {
                label.trim().eq_ignore_ascii_case("destination")
                    && !value.trim().is_empty()
                    && value
                        .chars()
                        .all(|c| c.is_alphabetic() || " .'’()-".contains(c))
            });
            (!matches_heading && !is_date && !is_destination && travel_sentence(line).is_none())
                .then_some(line)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn prepare(
    db: &Connection,
    key: &str,
    title: &str,
    body: &str,
    stamp: i64,
    cfg: &Value,
) -> Res<()> {
    if !booking(title, body) {
        return Ok(());
    }
    let fees = checked_fees(body);
    let trip = fees.as_ref().and_then(|fees| weather_trip(body, fees));
    let day = trip
        .as_ref()
        .and_then(|trip| trip_date(trip["date"].as_str()?));
    let destination = fees.as_ref().map(|fees| {
        let name = fees.destination;
        match trip.as_ref().and_then(|trip| trip["airport"].as_str()) {
            Some(code) if !name.ends_with(&format!("({code})")) => format!("{name} ({code})"),
            _ => name.to_owned(),
        }
    });
    let dest = destination.as_deref();
    let place = dest
        .or_else(|| matrix(body).map(|(_, _, section)| section))
        .unwrap_or(body);
    let context = format!(" {} ", words(&format!("{title} {place}")));
    let mut matches = std::collections::BTreeMap::new();
    let mut consider = |id: String, title: String, body: String| {
        if let Some(rank) = packing_rank(&title, &body, &context, dest, day) {
            // A viewed and an imported copy of the same note can have different
            // ids and headings. Conflicting contents remain distinct choices.
            let key = (
                words(note_name(&title)),
                packing_items(&title, &body).trim().to_owned(),
                rank,
            );
            matches.entry(key).or_insert(id);
        }
    };
    let city = dest
        .map(|place| place.split('(').next().unwrap_or(place).trim())
        .unwrap_or("");
    let code = dest
        .and_then(|place| place.rsplit_once('(')?.1.strip_suffix(')'))
        .unwrap_or("");
    let mut q = db
        .prepare(
            "SELECT a.id,a.title,a.body,a.source FROM activity a
         WHERE (a.title LIKE 'Notes · %' OR a.title LIKE '% · TextEdit active')
         AND ((?2='' AND (a.title LIKE '%packing list%' OR a.title LIKE '%packing checklist%'))
              OR (?2<>'' AND (instr(lower(a.title),lower(?2))>0 OR instr(lower(a.body),lower(?2))>0))
              OR (?3<>'' AND instr(a.body,?3)>0))
         AND a.stamp>=?1 AND NOT EXISTS(SELECT 1 FROM activity n
             WHERE n.source=a.source AND n.title=a.title AND n.rowid>a.rowid)
         ORDER BY a.stamp DESC,a.rowid DESC",
        )
        .msg()?;
    let mut rows = q.query(params![stamp - 30 * DAY, city, code]).msg()?;
    while let Some(r) = rows.next().msg()? {
        if allowed(cfg, &r.get::<_, String>(3).msg()?) {
            consider(r.get(0).msg()?, r.get(1).msg()?, r.get(2).msg()?);
        }
    }
    let mut q = db
        .prepare(
            "SELECT id,title,body FROM imported_passages WHERE kind='note' AND
         ((?1='' AND (title LIKE '%packing list%' OR title LIKE '%packing checklist%'))
          OR (?1<>'' AND (instr(lower(title),lower(?1))>0 OR instr(lower(body),lower(?1))>0))
          OR (?2<>'' AND instr(body,?2)>0)) ORDER BY id",
        )
        .msg()?;
    let mut rows = q.query(params![city, code]).msg()?;
    while let Some(r) = rows.next().msg()? {
        consider(r.get(0).msg()?, r.get(1).msg()?, r.get(2).msg()?);
    }
    let top = matches.keys().map(|(_, _, rank)| *rank).max();
    let mut best = matches
        .into_iter()
        .filter(|((_, _, rank), _)| Some(*rank) == top);
    let note = best
        .next()
        .and_then(|(_, id)| best.next().is_none().then_some(id));
    db.execute(
        "INSERT OR REPLACE INTO prepared_baggage(booking,note) VALUES(?,?)",
        params![key, note],
    )
    .msg()?;
    Ok(())
}
// A list can arrive after the booking. Refresh the handful of recent booking
// references in the same transaction as the source change.
pub(super) fn refresh_recent(db: &Connection, cfg: &Value, now: i64) -> Res<()> {
    let mut q=db.prepare("SELECT a.id,a.title,a.body FROM prepared_baggage p JOIN activity a ON a.id=p.booking WHERE a.stamp>=? AND NOT EXISTS(SELECT 1 FROM activity b WHERE b.source=a.source AND b.title=a.title AND b.rowid>a.rowid) ORDER BY a.stamp DESC,a.rowid DESC LIMIT 12").msg()?;
    let rows = q
        .query_map([now - 1800], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .msg()?
        .collect::<Result<Vec<_>, _>>()
        .msg()?;
    drop(q);
    for (id, title, body) in rows {
        prepare(db, &id, &title, &body, now, cfg)?;
    }
    Ok(())
}

fn words(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn diagnostics() -> bool {
    std::env::var_os("BESIDE_BAG_DIAGNOSTICS").as_deref() == Some(std::ffi::OsStr::new("1"))
}

// Resolve a tap against the captured window before the panel takes focus. A
// changed fare needs a fresh captured passage, even when its tab title is stable.
pub(crate) fn pin(s: &Store, point: &Value, generation: &Value) -> Res<Option<String>> {
    let cfg = config(s)?;
    let src = point["source"].as_str().unwrap_or("");
    let authorized = cfg["generation"] == *generation && allowed(&cfg, src);
    let trace = diagnostics() && point["bookingTitle"].is_string();
    if trace {
        eprintln!(
            "[Beside baggage] pin authorized={authorized} visual={} text={}",
            point["bagLayout"].is_object(),
            point["bookingBody"].is_string()
        );
    }
    if !authorized {
        return Ok(None);
    }
    let checkout = checkout::receipt(&point["checkoutEvidence"]);
    if let (Some(host), Some((title, _))) = (src.strip_prefix("site:"), &checkout) {
        if title.strip_prefix("Checked bags · ") != Some(host) {
            return Ok(None);
        }
    }
    let title = checkout
        .as_ref()
        .map(|(title, _)| title.as_str())
        .or_else(|| {
            point["bookingTitle"]
                .as_str()
                .or(point["pageTitle"].as_str())
        })
        .unwrap_or("");
    if title.is_empty() || (title.starts_with("Checked bags · ") && checkout.is_none()) {
        return Ok(None);
    }
    let visual = if point["bagLayout"].is_object() {
        let Some(body) = vision::receipt(&point["bagLayout"]) else {
            if trace {
                eprintln!("[Beside baggage] receipt verified=false");
            }
            return Ok(None);
        };
        if trace {
            eprintln!("[Beside baggage] receipt verified=true");
        }
        Some(body)
    } else {
        None
    };
    let live = checkout
        .as_ref()
        .map(|(_, body)| body.as_str())
        .or(visual.as_deref())
        .or_else(|| {
            point["bookingBody"]
                .as_str()
                .filter(|b| !b.starts_with(checkout::MARKER))
        });
    let now = Utc::now().timestamp();
    let screen_suffix = format!(
        " · {} · {} active",
        title,
        point["app"].as_str().unwrap_or("")
    );
    let hit=s.db.lock().unwrap().query_row("SELECT a.id,a.body,coalesce(n.title,i.title) FROM activity a JOIN prepared_baggage p ON p.booking=a.id LEFT JOIN activity n ON n.id=p.note LEFT JOIN imported_passages i ON i.id=p.note WHERE a.source=? AND (a.title=? OR substr(a.title,-length(?))=?) AND a.stamp>=? AND NOT EXISTS(SELECT 1 FROM activity newer WHERE newer.source=a.source AND newer.title=a.title AND newer.rowid>a.rowid) ORDER BY a.stamp DESC,a.rowid DESC LIMIT 1",
        params![src,title,screen_suffix,screen_suffix,now-if live.is_some(){1800}else{15}],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?))).optional().msg()?;
    if let Some((id, body, note)) = hit {
        let trip_matches = note.as_deref().is_some_and(|n| {
            let name = n
                .trim_start_matches("Notes · ")
                .strip_suffix(" · TextEdit active")
                .and_then(|v| v.split_once(" · ").map(|(_, t)| t))
                .unwrap_or(n.trim_start_matches("Notes · "));
            let scope = words(name)
                .split("packing")
                .next()
                .unwrap_or("")
                .trim()
                .to_owned();
            !scope.is_empty()
                && live.is_some_and(|b| {
                    format!(" {} ", words(matrix(b).map(|(_, _, s)| s).unwrap_or(b)))
                        .contains(&format!(" {scope} "))
                })
        });
        if live.is_none()
            || live.is_some_and(|v| {
                body == v
                    || (trip_matches && allowance(v).is_some() && allowance(v) == allowance(&body))
            })
        {
            if trace {
                eprintln!("[Beside baggage] pin reused=true");
            }
            return Ok(Some(id));
        }
    }
    let Some(body) = live.filter(|b| booking(title, b)) else {
        return Ok(None);
    };
    ingest(
        s,
        &cfg,
        &json!({"source":src,"title":title,"body":body}),
        now,
    )?;
    let id = s.db.lock().unwrap().query_row("SELECT p.booking FROM prepared_baggage p JOIN activity a ON a.id=p.booking WHERE a.source=? AND a.title=? AND a.body=? ORDER BY a.stamp DESC,a.rowid DESC LIMIT 1",params![src,title,body.chars().take(5600).collect::<String>()],|r|r.get(0)).optional().msg()?;
    if trace {
        eprintln!("[Beside baggage] pin stored={}", id.is_some());
    }
    Ok(id)
}
pub(super) fn answer(s: &Store, a: &Value, q: &str, epoch: &Value) -> Res<Option<Value>> {
    let weight_reply = REPLY.captures(q).and_then(|c| grams(&c[1]));
    let task = if weight_reply.is_some() {
        crate::floating_chats::last_task(s, a, epoch, "baggage")?
    } else {
        None
    };
    let follow = task.is_some()
        || (a["baggageAwaitingWeight"] == true
            && weight_reply.is_some()
            && a["previous"]
                .as_str()
                .is_some_and(|p| ASK.is_match(p) || BAG_AT_CHECKOUT.is_match(p))
            && a["epoch"] == *epoch);
    let generic = !ASK.is_match(q) && BAG_AT_CHECKOUT.is_match(q);
    if !ASK.is_match(q) && !generic && !follow {
        return Ok(None);
    }
    let _guard = s.activity_edit.lock().unwrap();
    if s.get("activityEpoch")? != *epoch {
        return Err("Your sources changed. Ask again.".into());
    }
    let cfg = config(s)?;
    let pointed = a["gesture"]
        .as_str()
        .filter(|_| !follow)
        .map(|id| crate::point::target(s, id))
        .transpose()?;
    let db = s.db.lock().unwrap();
    let fallback = || {
        if pointed
            .as_ref()
            .is_some_and(|p| p["bookingControlsUnavailable"] == true)
        {
            return json!({"answer":"Allow Accessibility so I can match each bag to its price. Then press ⌥⇧Space on the offer again.","workKind":"baggage-access","epoch":epoch,"sources":[]});
        }
        if pointed
            .as_ref()
            .is_some_and(|p| p["checkoutEvidence"].is_object())
        {
            return json!({"answer":"I can see the bag control. I need its checked-bag limit, included quantity and price alongside the itinerary before I can recommend paying for it.","epoch":epoch,"sources":[]});
        }
        json!({"answer":"Open your booking’s baggage allowance so I can check the limit for this fare.","epoch":epoch,"sources":[]})
    };
    if cfg["enabled"] != true {
        if generic {
            return Ok(None);
        }
        return Ok(Some(fallback()));
    }
    let prior = task
        .as_ref()
        .and_then(|t| t["sources"].get(0))
        .and_then(|row| row["id"].as_str())
        .or_else(|| {
            a["sources"]
                .as_array()
                .and_then(|v| v.first())
                .and_then(Value::as_str)
        });
    let key = if follow {
        prior
    } else {
        pointed
            .as_ref()
            .map(|p| p["bookingId"].as_str().unwrap_or(""))
            .or(a["anchor"].as_str())
    }
    .unwrap_or("");
    let row=db.query_row("SELECT a.id,a.source,a.title,a.body,a.stamp,p.note FROM prepared_baggage p JOIN activity a ON a.id=p.booking WHERE p.booking=? AND a.stamp>=? AND NOT EXISTS(SELECT 1 FROM activity b WHERE b.source=a.source AND b.title=a.title AND b.rowid>a.rowid)",params![key,Utc::now().timestamp()-1800],|r|Ok((event(r)?,r.get::<_,Option<String>>(5)?))).optional().msg()?;
    if diagnostics() {
        eprintln!(
            "[Beside baggage] answer follow={follow} gesture={} key={} booking={} note={}",
            pointed.is_some(),
            !key.is_empty(),
            row.is_some(),
            row.as_ref().is_some_and(|(_, note)| note.is_some())
        );
    }
    let Some((b, note)) = row else {
        if generic
            && !pointed
                .as_ref()
                .is_some_and(|p| p["bookingTitle"].is_string() || p["checkoutEvidence"].is_object())
        {
            return Ok(None);
        }
        return Ok(Some(fallback()));
    };
    if !allowed(&cfg, b["source"].as_str().unwrap_or("")) {
        return Ok(Some(fallback()));
    }
    let mut sources =
        vec![json!({"id":key,"title":b["title"],"label":"Booking","stamp":b["stamp"]})];
    let note = if let Some(id) = note {
        if id.starts_with("apple-note:") {
            db.query_row(
                "SELECT title,body FROM imported_passages WHERE kind='note' AND id=?",
                [&id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()
            .msg()?
            .map(|(t, b)| (id, t, b, None))
        } else {
            db.query_row("SELECT a.title,a.body,a.stamp,a.source FROM activity a WHERE a.id=? AND NOT EXISTS(SELECT 1 FROM activity newer WHERE newer.title=a.title AND newer.source=a.source AND newer.rowid>a.rowid)",[&id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?))).optional().msg()?
              .filter(|(_,_,_,src)|allowed(&cfg,src)).map(|(t,b,ts,_)|(id,t,b,Some(ts)))
        }
    } else {
        None
    };
    if let Some((id, title, _, stamp)) = &note {
        sources.push(json!({"id":id,"title":title,"label":"Notes","role":"packing","stamp":stamp}));
    }
    if let Some(fees) = checked_fees(b["body"].as_str().unwrap_or("")) {
        let wording = q.to_lowercase();
        if ["carry-on", "carry on", "cabin bag"]
            .iter()
            .any(|word| wording.contains(word))
        {
            return Ok(Some(
                json!({"answer":"This offer is for a checked bag. Open the cabin allowance for your fare so I can check your carry-on.", "supportingLine":fees.line, "sources":[sources[0]],"epoch":epoch}),
            ));
        }
        let visible_list = note.as_ref().filter(|(id, title, _, _)| {
            id.starts_with("apple-note:")
                || title.ends_with(" · TextEdit active")
                || ["TextEdit · ", "Notes · ", "Visible text · "]
                    .iter()
                    .any(|p| title.starts_with(p))
        });
        let packed = visible_list
            .and_then(|(_, title, body, _)| super::packing::estimate(&packing_items(title, body)));
        let measured = weight_reply
            .filter(|_| follow && prior == Some(key))
            .or_else(|| visible_list.and_then(|(_, _, body, _)| one(body, &WEIGHED).map(|v| v.0)));
        let high = measured.or_else(|| packed.map(|(_, high)| (high + 499) / 500 * 500));
        let suitcase_to_check = visible_list.is_some_and(|(_, _, body, _)| {
            body.lines().any(|line| {
                let item = line
                    .trim()
                    .trim_start_matches(['-', '*', '•', '☐', '☑'])
                    .trim()
                    .to_lowercase();
                matches!(item.as_str(), "checked suitcase" | "1 checked suitcase")
            })
        });
        let compares_checked = fees.included
            || q.to_lowercase().contains("second")
            || follow
            || suitcase_to_check
            || visible_list.is_some_and(|(_, _, body, _)| {
                body.lines()
                    .any(|line| line.trim().eq_ignore_ascii_case("large suitcase"))
            });
        let (mut answer,awaiting)=match high {
            Some(w) if compares_checked && (measured.is_some() || w+500<=fees.limit) && w<=fees.limit=>{
                let weight=if measured.is_some(){format!("At {} kg, your bag has {} kg to spare against the {} kg limit",kg(w),kg(fees.limit-w),kg(fees.limit))}else{format!("I’d budget about {} kg for your suitcase and everything listed, under the {} kg limit",kg(w),kg(fees.limit))};
                let cost = if fees.included && fees.included_count == 0 && fees.selected == Some(0) {
                    if suitcase_to_check {
                        format!("Your fare includes no checked bag. Add one for {} for the checked suitcase on your list.",fees.second)
                    } else {
                        format!("Your fare includes no checked bag. One checked bag is {} if you plan to check it. To see whether you can use a carry-on instead, I need the cabin allowance.",fees.second)
                    }
                } else if fees.included && fees.included_count == 0 {
                    format!("You already selected a checked bag. Skip the {} extra bag.",fees.second)
                } else if fees.included && fees.included_count > 1 {
                    format!("Your fare includes {} checked bags. Skip the {} extra bag.",fees.included_count,fees.second)
                } else if fees.included {
                    format!("One {} kg checked bag is included. Skip the {} extra bag.",kg(fees.limit),fees.second)
                } else { format!("Budget {} for the first bag and skip the {} second bag.",fees.first,fees.second) };
                (format!("One checked bag should do it. {weight}. {cost}"),false)
            }
            Some(w) if measured.is_some() && compares_checked=>{
                let advice = if fees.included_count.max(fees.selected.unwrap_or(0)) > 1 {
                    "Divide the contents between the bags already included or selected before paying for another.".to_string()
                } else {
                    format!("Pack lighter or divide the contents between permitted bags. The checkout lists {} for a checked bag.",fees.second)
                };
                (format!("Your bag is {} kg over the {} kg per-bag limit. {advice}",kg(w-fees.limit),kg(fees.limit)),false)
            }
            _ if compares_checked && fees.included && fees.included_count == 0 && fees.selected == Some(0)=>(format!("This fare includes no checked bag. The checkout offers a {} kg bag for {}. What does your packed suitcase weigh, and do you plan to check it?",kg(fees.limit),fees.second),true),
            _ if compares_checked=>(format!("Hold off on the {} extra bag. You get {} kg per checked bag. What does your packed suitcase weigh?",fees.second,kg(fees.limit)),true),
            _=>(format!("The first checked bag costs {}. To see whether you can skip it, open the cabin allowance for your fare.",fees.first),false),
        };
        if fees
            .selected
            .is_some_and(|count| count > fees.included_count.max(1))
            && high.is_some_and(|w| w + 500 <= fees.limit)
        {
            answer.push_str(" The selector already includes an extra bag. Review the quantity before continuing.");
        } else if fees.included_count > 0 && fees.selected == Some(0) {
            answer.push_str(" The selector currently shows zero. Use your included allowance if you check this suitcase.");
        }
        if let Some((_, title, _, _)) = visible_list {
            let title = note_name(title);
            answer.push_str(&format!(" I used “{title}” for this trip."));
        }
        let mut result = json!({"answer":answer,"sources":sources,"supportingLine":fees.line.replace('\t'," · "),"baggageAwaitingWeight":awaiting,"workKind":"baggage","epoch":epoch});
        if cfg["tripWeather"] == false {
            result["answer"] = json!(format!(
                "{} Destination forecasts are off.",
                result["answer"].as_str().unwrap_or("")
            ));
        } else if let Some(mut trip) = weather_trip(b["body"].as_str().unwrap_or(""), &fees) {
            let listed = |names: &[&str]| {
                packed.is_some()
                    && visible_list.is_some_and(|(_, _, body, _)| {
                        body.lines().any(|line| {
                            names
                                .iter()
                                .any(|name| line.trim().to_lowercase().ends_with(name))
                        })
                    })
            };
            trip["rainLayer"] = json!(listed(&["rain shell", "rain jacket"]));
            trip["warmLayer"] = json!(listed(&["hoodie", "hoodies", "sweatshirt", "sweatshirts"]));
            trip["generation"] = cfg["generation"].clone();
            result["forecastTrip"] = trip;
        } else {
            result["answer"] = json!(format!(
                "{} I couldn’t verify the travel date and airport for a forecast.",
                result["answer"].as_str().unwrap_or("")
            ));
        }
        return Ok(Some(result));
    }
    let Some((limit, line)) = allowance(b["body"].as_str().unwrap_or("")) else {
        return Ok(Some(
            json!({"answer":"I couldn’t verify one carry-on weight limit for this fare. Open the baggage allowance for your ticket.","epoch":epoch,"sources":sources}),
        ));
    };
    if ["checked bag", "check-in bag", "hold bag"]
        .iter()
        .any(|term| q.to_lowercase().contains(term))
    {
        return Ok(Some(
            json!({"answer":"I found the cabin allowance. Open the checked-bag offer to see what this fare includes and what another bag costs.", "supportingLine":line, "sources":[sources[0]],"epoch":epoch}),
        ));
    }
    let weight = weight_reply
        .filter(|_| follow && prior == Some(key))
        .or_else(|| {
            note.as_ref()
                .and_then(|(_, _, body, _)| one(body, &WEIGHED).map(|h| h.0))
        });
    let estimate = note.as_ref().and_then(|(id, title, body, _)| {
        let document = id.starts_with("apple-note:") || title.starts_with("TextEdit · ");
        let passage = title.starts_with("Notes · ") || title.starts_with("Visible text · ");
        if document || passage {
            super::packing::estimate(body).map(|bounds| (bounds, document))
        } else {
            None
        }
    });
    let awaiting_weight =
        weight.is_none() && !estimate.is_some_and(|((_, hi), _)| hi + 500 <= limit);
    let answer = match (weight, estimate) {
        (Some(w), _) if w>limit => format!("Pack lighter or use a checked bag. Your bag is {} kg over the {} kg carry-on limit. Remove at least {} kg to meet the weight limit.",kg(w-limit),kg(limit),kg(w-limit)),
        (Some(w), _) => format!("Skip the extra bag on weight grounds. Your bag weighs {} kg and the cabin limit is {} kg.",kg(w),kg(limit)),
        (_,Some(((low,high),complete))) => {
            let upper = (high+499)/500*500;
            let list = if complete {"your packing list"} else {"the visible items on your list"};
            if high + 500 <= limit {
                format!("Skip the extra bag. I’d budget about {} kg for {list}, including the bag itself. That leaves about {} kg under the {} kg cabin limit. Just check the bag fits the airline’s size limit.",kg(upper),kg(limit-upper),kg(limit))
            } else if low > limit {
                format!("Plan to check a bag or pack lighter. Even my low estimate of {} kg for {list} exceeds the {} kg cabin limit. Use any checked allowance already included before paying for more. What does your packed carry-on weigh?",kg(low/100*100),kg(limit))
            } else {
                format!("Hold off on buying extra baggage. I’d estimate {}–{} kg for {list}, including the bag. That’s close enough to the {} kg cabin limit that weighing it could change the decision. What does your packed carry-on weigh?",kg(low/500*500),kg(upper),kg(limit))
            }
        }
        _ if note.is_some()=>format!("Your fare allows {} kg. I found your packing list, but couldn’t verify a packed weight. What does your packed carry-on weigh?",kg(limit)),
        _=>format!("Your fare allows {} kg. What does your packed carry-on weigh?",kg(limit)),
    };
    Ok(Some(
        json!({"answer":answer,"sources":sources,"supportingLine":line,"baggageAwaitingWeight":awaiting_weight,"workKind":"baggage","epoch":epoch}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_selector_joins_only_the_active_cells_price_allowance_and_day() {
        let body="Check-in bag selection\nDestination: Athens\nOutbound: ATH 2026-12-02\nAdd 20kg Check-in Bag\n1 £ 41 . 49 1 bag included with Plus";
        let offer = checked_fees(body).unwrap();
        assert!(offer.included);
        assert_eq!(
            (
                offer.destination,
                offer.limit,
                offer.second.as_str(),
                offer.selected
            ),
            ("Athens", 20_000, "£41.49", Some(1))
        );
        assert_eq!(
            weather_trip(body, &offer),
            Some(json!({"airport":"ATH","date":"2026-12-02"}))
        );
        for ambiguous in [
            body.replace("1 bag included", "2 bags included"),
            body.replace("2026-12-02", "2026-02-31"),
            body.replace("1 £", "0 £"),
            format!("{body}\n£24.99"),
            body.replace(" £ ", " price unknown "),
        ] {
            assert!(checked_fees(&ambiguous).is_none());
        }
        assert!(ASK.is_match("Should I add another 20kg checked bag?"));
        assert!(ASK.is_match("Should I pay for a second checked bag?"));
    }

    #[test]
    fn booking_resolves_destination_and_departure_before_using_a_list() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::with_key(tmp.path().join("book"), &[5; 32]).unwrap();
        super::super::init(&store).unwrap();
        let cfg =
            json!({"enabled":true,"scope":"displays","generation":"bag-date","tripWeather":false});
        store.set("activitySettings", &cfg).unwrap();
        let now = Utc::now().timestamp();
        let record = |title: &str, body: &str| {
            ingest(
                &store,
                &cfg,
                &json!({"source":"display","title":title,"body":body}),
                now,
            )
            .unwrap()
        };
        let fare = "Check-in bag selection\nDestination: Athens\nOutbound: ATH 2026-12-02\nAdd 20kg Check-in Bag\n1 £ 41 . 49 1 bag included with Plus";
        let name = "Check-in bags · Ryanair";
        record("Notes · Athens packing list", "Large suitcase\n5 laptops");
        record(
            "Notes · Rome packing list",
            "Departure: 2026-12-02\nLarge suitcase\n1 T-shirt",
        );
        record(
            "Notes · Athens packing list – January 5, 2027",
            "Large suitcase\n10 jeans",
        );
        record("Notes · My packing list","My packing list\nDestination: Athens\nOutbound: 2 December 2026\nLarge suitcase\n4 T-shirts");
        record(name, fare);
        let key = pin(
            &store,
            &json!({"source":"display","bookingTitle":name,"bookingBody":fare}),
            &cfg["generation"],
        )
        .unwrap()
        .unwrap();
        let epoch = store.get("activityEpoch").unwrap();
        let ask = || {
            answer(
                &store,
                &json!({"anchor":key}),
                "Should I add another checked bag?",
                &epoch,
            )
            .unwrap()
            .unwrap()
        };
        let first = ask();
        assert_eq!(first["sources"][1]["title"], "Notes · My packing list");
        assert!(first["answer"].as_str().unwrap().contains("£41.49"));
        assert_eq!(first["baggageAwaitingWeight"], false);
        assert!(first["forecastTrip"].is_null());
        record(
            "Notes · Shared Athens packing checklist – Dec 2",
            "Large suitcase\n8 laptops",
        );
        let ambiguous = ask();
        assert_eq!(ambiguous["sources"].as_array().unwrap().len(), 1);
        assert_eq!(ambiguous["baggageAwaitingWeight"], true);
        assert_eq!(
            packing_rank(
                "Athens packing list",
                "Destination: Rome",
                "",
                Some("Athens"),
                Some(trip_date("2026-12-02").unwrap())
            ),
            None
        );
    }

    #[test]
    fn ordinary_note_and_visible_checkout_drive_the_first_bag_decision() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::with_key(tmp.path().join("book"), &[11; 32]).unwrap();
        super::super::init(&store).unwrap();
        let cfg = json!({"enabled":true,"scope":"displays","generation":"guest-checkout","tripWeather":true});
        store.set("activitySettings", &cfg).unwrap();
        let epoch = store.get("activityEpoch").unwrap();
        let record = |title: &str, body: &str| {
            ingest(
                &store,
                &cfg,
                &json!({"source":"display","title":title,"body":body}),
                Utc::now().timestamp(),
            )
            .unwrap()
        };
        record(
            "Notes · Weekend gear",
            "We’re flying to Porto on 3 January 2027.\n• Large suitcase\n• 5 laptops",
        );
        record(
            "Notes · Somebody else's plans",
            "I’m flying to Rome on 6 December 2026.\n• Large suitcase\n• 3 laptops",
        );
        record(
            "Notes · Cargo research",
            "Porto has an airport. Freight can include suitcases and clothing.",
        );
        let list = "Things to bring\nI’m flying to Porto on 6 December 2026.\n• Checked suitcase\n• 2 T-shirts\n• 1 pair of jeans\n• 3 pairs of socks";
        record("Notes · Things to bring", list);
        let mut checkout = json!({"source":"display","pageTitle":"Manage journey","checkoutEvidence":{
            "origin":"https://checkout.example.org","control":"Add a checked bag","quantity":0,
            "itinerary":["One way","1 adult","Dublin (DUB) → Porto (OPO)", "Departure date: 6 December 2026"],
            "groups":[["Checked bag up to 15 kg","No checked bags included","€29.50"]] }});
        let read = |key: &str, request: &str| {
            answer(&store, &json!({"anchor": key}), request, &epoch).unwrap()
        };
        let pin_checkout =
            |checkout: &Value| pin(&store, checkout, &cfg["generation"]).unwrap().unwrap();
        assert!(read("", "Should I add a bag?").is_none());
        assert!(pin(&store, &checkout, &json!("superseded"))
            .unwrap()
            .is_none());
        let first = pin_checkout(&checkout);
        for phrasing in [
            "Do I need to add a bag?",
            "Can we skip the second bag?",
            "Is one checked bag enough?",
        ] {
            assert_eq!(
                read(&first, phrasing).unwrap()["workKind"],
                "baggage",
                "{phrasing}"
            );
        }
        let result = read(&first, "Should I add a bag?").unwrap();
        assert_eq!(
            result["sources"][0]["title"],
            "Checked bags · checkout.example.org"
        );
        assert_eq!(result["sources"][1]["title"], "Notes · Things to bring");
        assert_eq!(result["sources"].as_array().unwrap().len(), 2);
        assert_eq!(result["forecastTrip"]["airport"], "OPO");
        assert_eq!(result["forecastTrip"]["date"], "2026-12-06");
        assert_eq!(result["baggageAwaitingWeight"], false);
        let text = result["answer"].as_str().unwrap();
        assert!(
            text.contains("budget about 8 kg")
                && text.contains("includes no checked bag")
                && text.contains("Add one for €29.50"),
            "{text}"
        );
        assert!(result["supportingLine"]
            .as_str()
            .unwrap()
            .contains("No checked bags included · €29.50"));
        let cabin = read(&first, "Should I add a carry-on bag?").unwrap();
        assert!(cabin["answer"]
            .as_str()
            .unwrap()
            .contains("Open the cabin allowance"));
        assert!(cabin["forecastTrip"].is_null());
        checkout["checkoutEvidence"]["quantity"] = json!(1);
        checkout["checkoutEvidence"]["groups"][0][2] = json!("€42.25");
        let selected = pin_checkout(&checkout);
        let response = read(&selected, "Is it worth adding another bag?").unwrap();
        let explanation = response["answer"].as_str().unwrap();
        assert!(
            explanation.contains("already selected")
                && explanation.contains("€42.25")
                && !explanation.contains("€29.50"),
            "{explanation}"
        );
        assert!(read(&first, "Should I add a bag?").is_none());
        checkout["checkoutEvidence"]["quantity"] = json!(0);
        checkout["checkoutEvidence"]["groups"][0][1] = json!("2 checked bags included");
        checkout["checkoutEvidence"]["groups"][0][2] = json!("Additional checked bag: €42.25");
        let included = pin_checkout(&checkout);
        let response = read(&included, "Do we need an extra bag?").unwrap();
        assert!(response["answer"]
            .as_str()
            .unwrap()
            .contains("includes 2 checked bags"));
        assert!(response["answer"]
            .as_str()
            .unwrap()
            .contains("currently shows zero"));
        record(
            "Notes · My other list",
            "Our trip to Porto leaves on December 6, 2026.\n• Daypack\n• 1 T-shirt",
        );
        let ambiguous = read(&included, "Should I add a bag?").unwrap();
        assert_eq!(ambiguous["sources"].as_array().unwrap().len(), 1);
        assert_eq!(ambiguous["baggageAwaitingWeight"], true);
        assert!(!ambiguous["answer"]
            .as_str()
            .unwrap()
            .contains("budget about"));
        let follow=answer(&store, &json!({"baggageAwaitingWeight":true,"previous":"Should I add a bag?","sources":[included],"epoch":epoch}), "I just weighed it. 15.8 kg", &epoch).unwrap().unwrap();
        assert!(follow["answer"]
            .as_str()
            .unwrap()
            .contains("0.8 kg over the 15 kg per-bag limit"));
        checkout["checkoutEvidence"]["groups"][0][2] = json!("Overweight fee €9 per kg");
        assert!(pin(&store, &checkout, &cfg["generation"])
            .unwrap()
            .is_none());
    }

    #[test]
    fn fare_table_keeps_the_personal_item_and_other_routes_separate() {
        let table = "Flights to Egypt\nFlights to Morocco\nItem\tBasic\tPlus\nHand baggage\t7kg + 3kg personal item\t7kg + 3kg personal item\nChecked baggage\tChargeable\t20 kg";
        assert_eq!(
            matrix(table),
            Some((7000, "7kg + 3kg personal item".into(), "Flights to Morocco"))
        );
        for bad in [
            table.replacen("7kg", "9kg", 1),
            table.replace("Basic\tPlus", "Basic\tPlus\tMax"),
            table.replace("7kg + 3kg", "7 lb + 3kg"),
        ] {
            assert!(allowance(&bad).is_none());
        }
        assert_eq!(allowance("Other\tText\nCarry-on: 7 kg").unwrap().0, 7000);
    }

    #[test]
    fn prepared_fare_reevaluates_a_changed_list_and_prefers_a_measurement() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::with_key(temp.path().join("book"), &[9; 32]).unwrap();
        super::super::init(&store).unwrap();
        let cfg = json!({"enabled":true,"scope":"displays","generation":"test"});
        store.set("activitySettings", &cfg).unwrap();
        let now = Utc::now().timestamp();
        let record = |title: &str, body: &str| {
            ingest(
                &store,
                &cfg,
                &json!({"source":"display","title":title,"body":body}),
                now,
            )
            .unwrap();
        };
        let table="Flights to Egypt\nFlights to Morocco\nItem\tBasic\tPlus\nHand baggage\t7kg + 3kg personal item\t7kg + 3kg personal item";
        record(
            "Visible text · Egypt packing list.txt · TextEdit active",
            "Egypt packing list\nMy packed bag weighs 15 kg",
        );
        record("Fare Types", table);
        let title = "Visible text · Morocco packing list.txt · TextEdit active";
        record(title, "Morocco packing list\nDaypack\n3 T-shirts");
        let point = json!({"source":"display","bookingTitle":"Fare Types","bookingBody":table,"app":"Browser"});
        let key = pin(&store, &point, &cfg["generation"]).unwrap().unwrap();
        let ask = || {
            answer(
                &store,
                &json!({"anchor":key}),
                "Do I need an extra bag?",
                &store.get("activityEpoch").unwrap(),
            )
            .unwrap()
            .unwrap()
        };
        let first = ask();
        assert!(first["answer"]
            .as_str()
            .unwrap()
            .starts_with("Skip the extra bag."));
        assert_eq!(first["baggageAwaitingWeight"], false);
        assert_eq!(first["sources"][1]["title"], title);
        record(title, "Morocco packing list\nDaypack\n2 laptops");
        assert!(ask()["answer"].as_str().unwrap().starts_with("Hold off"));
        record(title, "Morocco packing list\nDaypack\n9 laptops");
        assert!(ask()["answer"]
            .as_str()
            .unwrap()
            .starts_with("Plan to check"));
        record(title, "Morocco packing list\nDaypack\nCamera rig");
        assert_eq!(ask()["baggageAwaitingWeight"], true);
        record(
            title,
            "Morocco packing list\nDaypack\n9 laptops\nMy packed bag weighs 5 kg",
        );
        assert!(ask()["answer"].as_str().unwrap().contains("weighs 5 kg"));
    }

    #[test]
    fn reads_trip_specific_checked_fees_and_never_uses_them_as_cabin_limits() {
        let row = "Boston (BOS) → Rome (FCO)\t$75\t$110\t50 lbs (23 kgs)";
        let table=format!("Calculate checked baggage charges\nFLIGHT\tFIRST BAG\tSECOND BAG\tWEIGHT PER BAG\n{row}\nJanuary 22");
        let f = checked_fees(&table).unwrap();
        assert_eq!(
            (f.destination, f.first, f.second.as_str(), f.limit),
            ("Rome (FCO)", "$75", "$110", 23000)
        );
        assert!(checked_fees(&format!("{table}\n{row}")).is_none());
        assert!(checked_fees(&table.replace("50 lbs (23 kgs)", "50 lbs")).is_none());
        assert!(allowance(&table).is_none());
        assert!(ASK.is_match("Do I need a second checked bag?"));
        let temp = tempfile::tempdir().unwrap();
        let store = Store::with_key(temp.path().join("book"), &[8; 32]).unwrap();
        super::super::init(&store).unwrap();
        let cfg = json!({"enabled":true,"scope":"displays","generation":"fees"});
        store.set("activitySettings", &cfg).unwrap();
        let capture = |title: &str, body: &str| {
            ingest(
                &store,
                &cfg,
                &json!({"source":"display","title":title,"body":body}),
                Utc::now().timestamp(),
            )
            .unwrap()
        };
        capture("Notes · Boston packing list", "Large suitcase\n12 jeans");
        capture(
            "Notes · Rome packing list",
            "Large suitcase\n2 hoodies\n4 T-shirts",
        );
        capture("Flight reservation", &table);
        let point =
            json!({"source":"display","bookingTitle":"Flight reservation","bookingBody":table});
        let key = pin(&store, &point, &cfg["generation"]).unwrap().unwrap();
        let result = answer(
            &store,
            &json!({"anchor":key}),
            "Do I need a second checked bag?",
            &store.get("activityEpoch").unwrap(),
        )
        .unwrap()
        .unwrap();
        assert!(result["answer"]
            .as_str()
            .unwrap()
            .contains("skip the $110 second bag"));
        assert_eq!(result["sources"][1]["title"], "Notes · Rome packing list");
        let epoch = store.get("activityEpoch").unwrap();
        let mut previous = result;
        previous["id"] = json!("first");
        store
            .put(
                "floatingChat",
                &json!({"id":"trip","epoch":epoch,"turns":[previous]}),
            )
            .unwrap();
        let follow = json!({"chatId":"trip","previousTurn":"first","epoch":epoch,"anchor":"a different window"});
        let ask_weight = |text: &str, args: &Value| answer(&store, args, text, &epoch).unwrap();
        let weighed = ask_weight("I just weighed my suitcase. It’s 21.5 kg.", &follow).unwrap();
        assert!(weighed["answer"]
            .as_str()
            .unwrap()
            .contains("1.5 kg to spare"));
        assert!(ask_weight("24.5 kg", &follow).unwrap()["answer"]
            .as_str()
            .unwrap()
            .contains("1.5 kg over"));
        assert!(ask_weight("My laptop weighs 2 kg.", &follow).is_none());
        assert!(ask_weight(
            "21 kg",
            &json!({"chatId":"other","previousTurn":"first","epoch":epoch})
        )
        .is_none());
        assert!(ask_weight(
            "21 kg",
            &json!({"chatId":"trip","previousTurn":"other","epoch":epoch})
        )
        .is_none());
    }

    #[test]
    fn booking_prepares_only_the_matching_list_and_checks_changes() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::with_key(temp.path().join("book"), &[9; 32]).unwrap();
        super::super::init(&store).unwrap();
        let cfg = json!({"enabled":true,"scope":"displays","generation":"test"});
        store.set("activitySettings", &cfg).unwrap();
        let now = Utc::now().timestamp();
        let record = |title: &str, body: &str| {
            ingest(
                &store,
                &cfg,
                &json!({"source":"display","title":title,"body":body}),
                now,
            )
            .unwrap();
        };
        record("Notes · Alps packing list", "My packed bag weighs 14 kg");
        record(
            "Screen 1 · Japan packing list.txt · TextEdit active",
            "Japan packing list\nRain shell\nCharger",
        );
        let title = "Your booking";
        let screen = "Japan\nCarry-on: 7 kg";
        record(title, screen);
        let id: String = store
            .db
            .lock()
            .unwrap()
            .query_row("SELECT booking FROM prepared_baggage", [], |r| r.get(0))
            .unwrap();
        let point =
            json!({"source":"display","bookingTitle":title,"bookingBody":screen,"app":"Browser"});
        assert_eq!(
            pin(&store, &point, &cfg["generation"]).unwrap().as_ref(),
            Some(&id)
        );
        let epoch = store.get("activityEpoch").unwrap();
        let q = "Do I need an extra bag?";
        let args = json!({"anchor":id});
        let first = answer(&store, &args, q, &epoch).unwrap().unwrap();
        assert!(first["answer"]
            .as_str()
            .unwrap()
            .contains("found your packing list"));
        assert!(first["answer"].as_str().unwrap().contains("What does"));
        assert!(first["sources"][1]["title"]
            .as_str()
            .unwrap()
            .contains("Japan"));
        let follow = json!({"anchor":id,"previous":q,"baggageAwaitingWeight":true,"epoch":epoch,"sources":[id]});
        assert!(
            answer(&store, &follow, "8.4 kg", &epoch).unwrap().unwrap()["answer"]
                .as_str()
                .unwrap()
                .contains("1.4 kg over")
        );
        record(
            "Screen 1 · Japan packing list.txt · TextEdit active",
            "Japan packing list\nMy packed bag weighs 6.6 kg",
        );
        let updated = answer(&store, &args, q, &epoch).unwrap().unwrap();
        assert!(updated["answer"].as_str().unwrap().contains("6.6 kg"));
        record(title, "Japan\nNew booking; baggage allowance pending");
        assert!(
            !answer(&store, &args, q, &epoch).unwrap().unwrap()["answer"]
                .as_str()
                .unwrap()
                .contains("7 kg")
        );
        assert!(one("Carry-on: 7 kg\nCarry-on: 10 kg", &LIMIT).is_none());
        assert!(one("My sister’s bag weighs 9 kg", &WEIGHED).is_none());
        assert!(allowance("Carry-on: 7 kg\nPremium carry-on: 10 kg").is_none());
    }
}
