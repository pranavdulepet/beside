mod fallback;
use crate::{local, plan, sources, store::*};
use chrono::{Datelike, Days, NaiveDate};
use serde_json::{json, Value};
use std::{collections::HashSet, io::Read, time::Duration};

pub fn outing(q: &str, last: &Value) -> bool {
    let q = q.to_lowercase();
    (q.contains("weekend")
        && [
            "what should",
            "what can",
            "what could",
            "things to do",
            "what's on",
            "what’s on",
            "where should",
            "plans for",
            "plan my",
            "what to do",
            "any ideas",
            "what is happening",
            "museum events",
            "free events",
            "weekend events",
        ]
        .iter()
        .any(|cue| q.contains(cue)))
        || (last["web"]["status"] == "place" && q.len() < 100)
}
pub(crate) fn city(text: &str) -> Option<&str> {
    let place = text.trim().trim_end_matches(['?', '.', '!']);
    let text = place
        .strip_suffix(" this weekend")
        .or_else(|| place.strip_suffix(" for the weekend"))
        .unwrap_or(place);
    let city = text.trim().trim_end_matches(['?', '.', '!']).trim();
    (!city.is_empty()
        && city.len() <= 55
        && city.split_whitespace().count() <= 5
        && !city.split_whitespace().any(|part| matches!(
            part.to_ascii_lowercase().as_str(), "my" | "our" | "his" | "her" | "their" | "this" | "that"
        ))
        && city
            .chars()
            .all(|c| c.is_alphabetic() || " ,’-'-".contains(c)))
    .then_some(city)
}
pub(crate) fn weekend(day: NaiveDate) -> (NaiveDate, NaiveDate) {
    let dow = day.weekday().num_days_from_monday();
    let sat = if dow == 6 {
        day.pred_opt().unwrap()
    } else {
        day + Days::new(u64::from(5 - dow))
    };
    (sat, sat.succ_opt().unwrap())
}
pub fn prepare(
    q: &str,
    last: &Value,
    routing: &Value,
    notes: &[Value],
    day: NaiveDate,
    restricted: bool,
) -> Option<Value> {
    let lower = q.to_lowercase();
    if current_screen_question(q) { return None; }
    let is_outing = outing(q, last);
    let standalone = standalone_query(q);
    let general = routing["web"].as_str().unwrap_or("").trim();
    let explicit = [
        "search the web",
        "look up ",
        "search online",
        "latest ",
        "current price",
        "what's happening",
        "what’s happening",
        "in the news",
    ]
    .iter()
    .any(|s| lower.contains(s));
    if standalone.is_none() && !is_outing
        && !explicit
        && (general.len() < 4
            || general.eq_ignore_ascii_case("none")
            || general.eq_ignore_ascii_case("null")
            || !q.contains('?'))
    {
        return None;
    }
    if standalone.is_none() && !is_outing
        && !explicit
        && (lower.contains("tomorrow")
            || lower.starts_with("what did i")
            || lower.contains("split")
            || lower.contains("my "))
    {
        return None;
    }
    if restricted {
        return Some(json!({"status":"private"}));
    }
    let mut dates = Vec::new();
    let mut city_name = None;
    let query = if is_outing {
        let supplied = lower
            .rfind(" in ")
            .and_then(|i| q.get(i + 4..))
            .and_then(city)
            .or_else(|| {
                if last["web"]["status"] == "place" {
                    city(q)
                } else {
                    None
                }
            });
        let home = notes
            .iter()
            .find(|n| n["topic"] == "home" && n["restricted"] != true);
        let Some(place) =
            supplied.or_else(|| home.and_then(|n| n["value"].as_str()).and_then(city))
        else {
            return Some(json!({"status":"place"}));
        };
        city_name = Some(place.to_owned());
        let (start, end) = weekend(day);
        dates.extend([start, end]);
        // Only broad public categories can be drawn from personal notes. Ranking happens afterwards.
        let prefs = notes
            .iter()
            .filter(|n| n["restricted"] != true && n["topic"] != "home")
            .filter_map(|n| n["text"].as_str())
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let category = if prefs.contains("printmaking") || prefs.contains("block printing") {
            "art workshops, printmaking and block printing"
        } else if [
            "art",
            "printmaking",
            "gallery",
            "galleries",
            "museum",
            "pottery",
        ]
        .iter()
        .any(|w| prefs.contains(w))
        {
            "museum exhibitions and art workshops"
        } else if ["hik", "outdoors", "nature", "walk"]
            .iter()
            .any(|w| prefs.contains(w))
        {
            "parks and guided nature walks"
        } else if ["music", "concert", "jazz"]
            .iter()
            .any(|w| prefs.contains(w))
        {
            "live music and concerts"
        } else {
            "weekend events and things to do"
        };
        format!(
            "{place} {category}, {} through {}. Official venue listings",
            start.format("%B %-d, %Y"),
            end.format("%B %-d, %Y")
        )
    } else {
        let query = if let Some(terms) = standalone {
            terms
        } else if general.len() >= 4 && !general.eq_ignore_ascii_case("none") {
            general
        } else {
            q
        };
        query
            .split_whitespace()
            .take(28)
            .collect::<Vec<_>>()
            .join(" ")
    };
    if query.len() > 360 || local::sensitive(&query) || sources::audit(&query).is_none() {
        return Some(json!({"status":"private"}));
    }
    Some(json!({"status":"ready","query":query,"outing":is_outing,"dates":dates,"city":city_name}))
}
pub fn introduction(web: &Value) -> &'static str {
    match web["status"].as_str() {
        Some("place")=>"Which city or town should I look around?",
        Some("private")=>"This conversation stays on your Mac. I can brainstorm here, or you can ask for current listings on a new sheet.",
        _ if web["outing"]==true=>"I’ll check the venue listings for a plan that fits.",
        _=>"I can look that up and bring back the source.",
    }
}
pub fn public_url(raw: &str) -> bool {
    let Ok(u) = url::Url::parse(raw) else { return false };
    if !matches!(u.scheme(), "https" | "http") || !u.username().is_empty() || u.password().is_some() {return false}
    match u.host() {
        Some(url::Host::Domain(d)) => {
            let d=d.trim_end_matches('.'); d.contains('.') && ![".localhost",".local",".lan",".internal",".test",".invalid",".home.arpa",".onion"].iter().any(|t|d.ends_with(t))
        }
        // Public source links are publisher domain names. Numeric IPs are unnecessary here.
        _ => false,
    }
}
fn blocks(raw: &str) -> Res<Vec<String>> {
    let mut blocks = Vec::new();
    let data;
    let payload = if raw.trim_start().starts_with('{') {
        raw
    } else {
        data = raw.lines()
            .filter_map(|line| line.strip_prefix("data:").map(str::trim))
            .filter(|line| !line.is_empty() && *line != "[DONE]")
            .collect::<Vec<_>>().join("\n");
        &data
    };
    for message in serde_json::Deserializer::from_str(payload).into_iter::<Value>() {
        let res = message.map_err(|_| "The search service returned an unreadable response.")?;
        if res["error"].is_object() || res["result"]["isError"] == true {
            return Err("Web search is unavailable. Try again in a moment.".into());
        }
        for item in res["result"]["content"].as_array().into_iter().flatten() {
            let Some(text) = item["text"].as_str() else {
                continue;
            };
            blocks.push(text.to_owned());
        }
    }
    Ok(blocks)
}

fn parse(raw: &str) -> Res<Vec<Value>> {
    let mut sources = Vec::new();
    let mut seen = HashSet::new();
    for text in blocks(raw)? {
        let normalized = text.replace("\r\n", "\n");
        for chunk in normalized.split("\n---\n") {
            let mut lines = chunk.lines();
            let mut title = None;
            let mut url = None;
            for line in lines.by_ref().map(str::trim) {
                if matches!(line, "Highlights:" | "Text:" | "Content:") {
                    break;
                }
                if let Some(value) = line.strip_prefix("Title:") {
                    title.get_or_insert(value.trim());
                } else if let Some(value) = line.strip_prefix("URL:") {
                    url.get_or_insert(value.trim());
                }
            }
            let (Some(title), Some(url)) = (title, url) else { continue };
            if !public_url(url) || !seen.insert(url.to_owned()) || title.trim().is_empty() {
                continue;
            }
            let content = lines
                .filter(|l| !l.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            sources.push(json!({"title":title.trim().chars().take(180).collect::<String>(),"url":url,"excerpt":content.chars().take(2600).collect::<String>()}));
            if sources.len() == 8 {
                return Ok(sources);
            }
        }
    }
    Ok(sources)
}
// Both MCP operations read public information. A retry stays at the same endpoint
// and has to fit inside the original request deadline.
fn retry_delay(error: &ureq::Error) -> Option<Duration> {
    match error {
        ureq::Error::Status(429 | 502 | 503 | 504, response) => match response.header("Retry-After") {
            Some(value) => value.trim().parse::<u64>().ok().filter(|s| *s <= 1).map(Duration::from_secs),
            None if response.status() == 429 => None,
            None => Some(Duration::from_millis(180)),
        },
        ureq::Error::Transport(failure) if failure.kind() == ureq::ErrorKind::ConnectionFailed => {
            Some(Duration::from_millis(180))
        }
        _ => None,
    }
}

fn exa_agent() -> &'static ureq::Agent {
    static POOL: std::sync::LazyLock<ureq::Agent> = std::sync::LazyLock::new(|| {
        ureq::AgentBuilder::new()
            .redirects(0)
            .build()
    });
    &POOL
}

fn request(tool: &str, arguments: Value) -> Res<String> {
    let budget = Duration::from_secs(if tool == "web_search_exa" { 9 } else { 12 });
    let agent = exa_agent();
    let payload = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":tool,"arguments":arguments}});
    let deadline = std::time::Instant::now() + budget;
    let send = |timeout| {
        agent.post("https://mcp.exa.ai/mcp")
            .set("Accept", "application/json, text/event-stream")
            .timeout(timeout)
            .send_json(&payload)
    };
    let response = send(budget).or_else(|error| {
        let Some(delay) = retry_delay(&error) else { return Err(error) };
        if deadline.saturating_duration_since(std::time::Instant::now()) <= delay { return Err(error); }
        std::thread::sleep(delay);
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() { return Err(error); }
        send(remaining)
    }).map_err(|_| "I couldn’t reach web search. Check your connection and try again.".to_owned())?;
    let mut body = String::new();
    response
        .into_reader()
        .take(512_001)
        .read_to_string(&mut body)
        .msg()?;
    if body.len() > 512_000 {
        return Err("The search response was too large. Try a narrower question.".into());
    }
    Ok(body)
}

pub(crate) fn search(query: &str, outing: bool) -> Res<Vec<Value>> {
    search_using(query,outing,request,fallback::search)
}

/// Fetch readable text for one validated public URL via Exa's page fetcher.
pub(crate) fn fetch_page(url: &str) -> Res<String> {
    let url = url.trim();
    if !public_url(url) {
        return Err("Only public HTTPS/HTTP pages can be fetched.".into());
    }
    let raw = request("web_fetch_exa", json!({"urls": [url], "maxCharacters": 5000}))?;
    let mut collected = String::new();
    for block in blocks(&raw)? {
        if !collected.is_empty() {
            collected.push('\n');
        }
        collected.push_str(block.trim());
    }
    let trimmed = collected.trim();
    if trimmed.is_empty() {
        Err("No readable content returned for that page.".into())
    } else {
        Ok(trimmed.chars().take(4200).collect())
    }
}

fn search_using(query:&str, outing:bool, exa:impl FnOnce(&str,Value)->Res<String>, alternative:impl FnOnce(&str,usize)->Res<Vec<Value>>)->Res<Vec<Value>>{
    let primary = exa(
        "web_search_exa",
        json!({"query":query,"numResults":if outing{8}else{5},"objective":format!(
            "Today is {}. {}",chrono::Local::now().date_naive(),
            if outing {"Find official venue event pages within the requested place and dates. Include the title, date, time and admission. Exclude sold-out and cancelled listings."}
            else {"Find primary sources directly answering the question. Return specific passage evidence, dates and details from official documentation or publishers."})}),
    )
    .and_then(|raw| parse(&raw));
    match primary {
        Ok(pages) if !pages.is_empty() => Ok(pages),
        primary_err => match alternative(query, if outing { 8 } else { 5 }) {
            Ok(pages) if !pages.is_empty() => Ok(pages),
            _ => primary_err,
        },
    }
}

fn available_pages(raw: &str, results: &[Value], today: NaiveDate) -> Res<Vec<Value>> {
    static DEADLINE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)(?:registration (?:closes|ends|deadline)|register by)[: ]+([a-z]+) (\d{1,2})(?:, (20\d{2}))?").unwrap()
    });
    let mut verified = Vec::new();
    for block in blocks(raw)? {
        let lines: Vec<_> = block.lines().collect();
        let starts: Vec<_> = (1..lines.len())
            .filter(|i| lines[i - 1].starts_with("# ") && lines[*i].starts_with("URL: "))
            .collect();
        for (j, &start) in starts.iter().enumerate() {
            let url = lines[start].trim_start_matches("URL: ").trim();
            let Some(page) = results.iter().find(|p| p["url"] == url) else {
                continue;
            };
            let end = starts.get(j + 1).map_or(lines.len(), |v| v - 1);
            let body = lines[start + 1..end].join("\n");
            let lower = format!("{} {}", page["title"], body).to_lowercase();
            let unavailable = [
                "sold out",
                "sold-out",
                "cancelled",
                "canceled",
                "currently full",
                "registration closed",
                "registration is closed",
                "fully booked",
                "waitlist only",
            ]
            .iter()
            .any(|v| lower.contains(v));
            let too_late = DEADLINE.captures_iter(&lower).any(|c| {
                let year = c
                    .get(3)
                    .map(|v| v.as_str().to_owned())
                    .unwrap_or_else(|| today.format("%Y").to_string());
                let raw = format!("{} {} {}", &c[1], &c[2], year);
                ["%B %d %Y", "%b %d %Y"]
                    .iter()
                    .filter_map(|f| NaiveDate::parse_from_str(&raw, f).ok())
                    .any(|d| d < today)
            });
            if unavailable || too_late || body.trim().len() < 35 {
                continue;
            }
            let mut p = page.clone();
            p["excerpt"] = json!(body
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
                .chars()
                .take(5900)
                .collect::<String>());
            verified.push(p);
        }
    }
    Ok(verified)
}

fn short_quote(line: &str) -> String {
    let words: Vec<_> = line.split_whitespace().collect();
    if words.len() > 22 {
        format!("{}…", words[..22].join(" "))
    } else {
        words.join(" ")
    }
}
fn display_lines(page: &Value) -> Vec<String> {
    page["excerpt"]
        .as_str()
        .unwrap_or("")
        .lines()
        .map(|s| s.trim_matches(|c: char| c.is_whitespace() || matches!(c, '#' | '*' | '|')))
        .filter(|s| s.len() > 4 && !s.contains("://") && !s.starts_with("Photo by"))
        .take(36)
        .map(|s| s.chars().take(240).collect())
        .collect()
}
fn mentions_day(line: &str, d: NaiveDate) -> bool {
    let s = line.to_lowercase();
    let tokens: Vec<_> = s
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.iter().any(|w| {
        w.len() == 4 && w.starts_with("20") && w.parse::<i32>().is_ok_and(|y| y != d.year())
    }) {
        return false;
    }
    ["%B %-d", "%b %-d", "%Y-%m-%d", "%-m/%-d/%Y", "%m/%d/%Y"]
        .iter()
        .any(|f| {
            let date = d.format(f).to_string().to_lowercase();
            s.match_indices(&date).any(|(i, _)| {
                s[i + date.len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| !c.is_ascii_digit())
                    && (i == 0 || !s[..i].chars().last().is_some_and(|c| c.is_alphanumeric()))
            })
        })
}

// When a calendar page contains several events, keep each activity under its
// own date heading. Separate matches elsewhere on a page cannot verify a date.
fn dated_choice(
    pages: &[Value],
    dates: &[NaiveDate],
    prefs: &[&Value],
) -> Option<(usize, String, String, Option<usize>)> {
    use std::sync::LazyLock;
    static DATE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?i)\b(?:jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|jun(?:e)?|jul(?:y)?|aug(?:ust)?|sep(?:tember)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?)\.? \d{1,2}\b|\b\d{1,2}/\d{1,2}(?:/\d{2,4})?\b|\b20\d{2}-\d{2}-\d{2}\b").unwrap()
    });
    let mut best = None;
    let mut best_score = 0;
    for (i, p) in pages.iter().enumerate() {
        if p["availabilityUnverified"]==true {continue;}
        let lines = display_lines(p);
        let mut active: Option<(usize, &str)> = None;
        for (j, line) in lines.iter().enumerate() {
            let dated = dates.iter().any(|d| mentions_day(line, *d));
            if dated {
                active = Some((j, line));
            } else if DATE.is_match(line) {
                active = None;
            }
            let Some((start, heading)) = active else {
                continue;
            };
            if j - start > 4 {
                continue;
            }
            let lower = line.to_lowercase();
            let topic = [
                "workshop",
                "printing",
                "sculpt",
                "concert",
                "guided",
                "walk",
                "hike",
                "drop-in",
                "exhibition",
                "gallery",
                "jazz",
                "festival",
                "tour",
                "printmaking",
                "art making",
            ]
            .iter()
            .any(|term| lower.contains(term));
            if !topic {
                continue;
            }
            let mut matched = None;
            let mut best_overlap = 0;
            for (k, n) in prefs.iter().enumerate() {
                let cue = n["text"].as_str().unwrap_or("").to_lowercase();
                let terms = crate::conversation::words(&cue);
                let print = cue.contains("printmaking")
                    && (lower.contains("printmaking") || lower.contains("printing"));
                let overlap = crate::conversation::words(line)
                    .intersection(&terms)
                    .count()
                    + if print { 3 } else { 0 };
                if overlap > best_overlap {
                    matched = Some(k);
                    best_overlap = overlap;
                }
            }
            let score = 6 * best_overlap + if dated { 4 } else { 1 };
            if score > best_score {
                best_score = score;
                best = Some((i, line.to_owned(), heading.to_owned(), matched));
            }
        }
    }
    best
}

fn scheduled_activity(activity: &str, title: &str, dates: &[NaiveDate]) -> Option<String> {
    for date in dates {
        for pattern in ["%-m/%-d/%Y", "%m/%d/%Y", "%Y-%m-%d", "%B %-d, %Y"] {
            let printed = date.format(pattern).to_string();
            let Some((name, when)) = activity.split_once(&printed) else {
                continue;
            };
            let name = name.trim_matches(|c: char| c.is_whitespace() || c == '|');
            if !(6..=115).contains(&name.len()) || name.contains("://") {
                continue;
            }
            let venue = title.rsplit(" | ").next().unwrap_or(title);
            let mut detail = format!("{venue} · {}", date.format("%A, %B %-d"));
            if !when.trim().is_empty() {
                detail.push_str(&format!(
                    " · {}",
                    when.trim_matches(|c: char| c.is_whitespace() || c == '@')
                ));
            }
            return Some(format!("I’d try **{name}**.\n\n{detail}"));
        }
    }
    None
}

pub(crate) struct WeekendPick {
    pub index: usize,
    pub answer: String,
    pub quote: String,
    pub preference: Option<Value>,
}

pub(crate) const UNCONFIRMED_DATES: &str =
    "I found local listings, but couldn’t confirm an activity on the requested dates. You can check the listings below.";

pub(crate) fn weekend_pick(pages: &[Value], dates: &[NaiveDate], notes: &[Value]) -> Option<WeekendPick> {
    let prefs: Vec<_> = notes.iter().filter(|n| n["topic"] != "home" && n["restricted"] != true).collect();
    let (index, activity, heading, memory) = dated_choice(pages, dates, &prefs)?;
    let day = dates.iter().find(|date| mentions_day(&heading, **date))?;
    let title = pages[index]["title"].as_str()?;
    let answer = scheduled_activity(&activity, title, dates).unwrap_or_else(|| format!(
        "{} · {}\n\n“{}”", day.format("%A, %B %-d"), title, short_quote(&activity)
    ));
    let quote = if heading == activity { activity } else { format!("{heading}\n…\n{activity}") };
    Some(WeekendPick {index, answer, quote, preference: memory.map(|i| prefs[i].clone())})
}

pub fn complete(s: &Store, turn: &mut Value, model: &Value, notes: &[Value]) -> Res<()> {
    let query = turn["web"]["query"]
        .as_str()
        .ok_or("Missing search query.")?
        .to_owned();
    let outing = turn["web"]["outing"] == true;
    let pages = research(&query, outing, turn["web"]["city"].as_str())?;
    if pages.is_empty() {
        turn["reply"] = json!(if outing {
            "I couldn’t verify a weekend option in town with current venue details. I’d broaden the activity or try a nearby town."
        } else {
            "I couldn’t find a source for that search. Try a broader topic."
        });
        turn["web"]["status"] = json!("empty");
        turn["web"]["checkedAt"] = json!(now());
        return Ok(());
    }
    if outing {
        let dates: Vec<NaiveDate> = serde_json::from_value(turn["web"]["dates"].clone()).unwrap_or_default();
        let pick = weekend_pick(&pages, &dates, notes);
        turn["reply"] = json!(pick.as_ref().map_or(UNCONFIRMED_DATES, |p| p.answer.as_str()));
        turn["web"]["pick"] = json!(pick.as_ref().map(|p| p.index));
        if let Some(pick) = pick {
            turn["web"]["quote"] = json!(pick.quote);
            if let Some(note) = pick.preference { turn["usedMemory"] = note; }
        }
        turn["web"]["sources"] = json!(pages);
        turn["web"]["checkedAt"] = json!(now());
        turn["web"]["status"] = json!("done");
        return Ok(());
    }
    let indexed: Vec<_> = pages.iter().enumerate().map(|(i,p)| {
        let lines=display_lines(p);
        json!({"number":i+1,"title":p["title"],"lines":lines.iter().enumerate().map(|(j,l)|json!([j+1,l])).collect::<Vec<_>>()})
    }).collect();
    let prefs: Vec<_> = notes.iter().filter(|n| n["topic"] != "home").collect();
    let memories: Vec<_> = prefs
        .iter()
        .enumerate()
        .map(|(i, n)| json!({"number":i+1,"fact":n["text"]}))
        .collect();
    let schema = json!({"type":"object","properties":{"source":{"type":"integer"},"line":{"type":"integer"},"answer":{"type":"string"},"memory":{"type":"integer"}},"required":["source","line","answer","memory"],"additionalProperties":false});
    let answer=plan::think(s,model,"web synthesis",include_str!("../../prompts/web.txt"),&json!({"question":turn["question"],"search":query,"sources":indexed,"preferences":memories}),220,Some(&schema)).ok().and_then(|r|serde_json::from_str::<Value>(&r).ok()).unwrap_or(Value::Null);
    fn index(value: &Value, count: usize) -> Option<usize> {
        let i = value.as_u64()?.checked_sub(1)? as usize;
        (i < count).then_some(i)
    }
    let pick = index(&answer["source"], pages.len());
    turn["reply"] = json!("Here are the pages I found. Open a source to check the details.");
    if pick.is_some() {
        if let Some(text) = answer["answer"].as_str().map(str::trim)
            .filter(|text| !text.is_empty() && !text.contains("://"))
        {
            turn["reply"] = json!(text);
        }
        if let Some(i) = index(&answer["memory"], prefs.len()) {
            turn["usedMemory"] = json!(prefs[i]);
        }
    }
    turn["web"]["sources"] = json!(pages);
    turn["web"]["pick"] = json!(pick);
    turn["web"]["checkedAt"] = json!(now());
    turn["web"]["status"] = json!("done");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_overrides_accept_protocols_and_unavailable_listing_pages_stay_unverified() {
        for q in ["HTTP/3 status","C# 13 release date"] {assert!(public_typed_lookup(q),"{q}");}
        for q in ["HTTP/3 status in my email","HTTP/3 /Users/someone/private","C# 13 private meeting"] {assert!(!public_typed_lookup(q),"{q}");}
        let page=json!({"url":"https://example.org/shows","title":"Town hall","excerpt":"December 12, 2026 — pottery workshop at 10 am","searchProvider":"DuckDuckGo"});
        let found=research_with("ceramics weekend",true,None,|_,_|Ok(vec![page]),|_,_|Err("offline".into())).unwrap();
        assert_eq!(found[0]["availabilityUnverified"],true);
        assert!(weekend_pick(&found,&["2026-12-12".parse().unwrap()],&[]).is_none());
    }

    #[test]
    fn weekends_include_today_and_cross_year() {
        for (today, start) in [("2026-11-22", "2026-11-21"), ("2027-01-01", "2027-01-02")] {
            let (sat, sun) = weekend(today.parse().unwrap());
            assert_eq!(sat.to_string(), start);
            assert_eq!(sun, sat.succ_opt().unwrap());
        }
    }
    #[test]
    fn source_metadata_ends_when_the_excerpt_starts() {
        let text = "Rank: 1\r\nTitle: Museum\r\nPublished: today\r\nURL: https://museum.example\r\nText:\r\nOpen at ten\r\n---\r\nTitle: Other\r\nContent:\r\nURL: https://wrong.example\r\nUnknown";
        let pages = parse(&json!({"result":{"content":[{"text":text}]}}).to_string()).unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0]["url"], "https://museum.example");
        assert_eq!(pages[0]["excerpt"], "Open at ten");
        let frames = ": keepalive\r\nevent: message\r\ndata: {\"result\":\r\ndata: {\"content\":[{\"text\":\"one\"}]}}\r\n\r\ndata: {\"result\":{\"content\":[{\"text\":\"two\"}]}}\r\n\r\ndata: [DONE]\r\n";
        assert_eq!(blocks(frames).unwrap(), ["one", "two"]);
        assert!(blocks("data: {broken}").is_err());
    }

    #[test]
    fn retries_only_transient_errors_with_a_short_wait() {
        let error = |code, header: &str| {
            let response = format!("HTTP/1.1 {code} status\r\n{header}\r\n").parse().unwrap();
            ureq::Error::Status(code, response)
        };
        assert!(retry_delay(&error(503, "")).is_some());
        assert_eq!(retry_delay(&error(429, "Retry-After: 1\r\n")), Some(Duration::from_secs(1)));
        for code in [301, 400, 401, 403, 404, 429] { assert!(retry_delay(&error(code, "")).is_none()); }
        assert!(retry_delay(&error(503, "Retry-After: 60\r\n")).is_none());
    }

    #[test]
    fn typed_search_clarification_carries_only_public_terms_and_the_requested_detail() {
        assert_eq!(clarified_topic("Look up its opening hours", "Kew Gardens").as_deref(), Some("Kew Gardens opening hours"));
        assert_eq!(clarified_topic("Look this up", "apple checklist shortcuts").as_deref(), Some("apple checklist shortcuts"));
        for topic in ["LAX", "the \"silver birch\""] {
            assert_eq!(clarified_topic("look it up", topic).as_deref(), Some(topic));
            assert_eq!(confirmed_topic(topic, Some(topic)), Some(topic));
            assert_eq!(confirmed_topic(topic, Some("a different search")), None);
        }
        assert_eq!(confirmed_topic("my itinerary", Some("my itinerary")), None);
        for reply in ["her home address", "my notes", "/Volumes/work", "no thanks", "new@example.org", "write a poem", "what is a sonnet?"] {
            assert_eq!(clarified_topic("look it up", reply), None, "{reply}");
        }
        assert_eq!(clarified_topic("draft a reply", "City Hall"), None);
    }

    #[test]
    fn reads_only_public_source_links_and_surfaces_service_errors() {
        let list="Title: A\nURL: https://museum.example/events\nHighlights:\nSaturday\n---\nTitle: Bad\nURL: file:///private/data\nHighlights:\nNo";
        let raw = json!({"result":{"content":[{"text": list}]}});
        assert_eq!(
            parse(&format!("event: message\ndata: {raw}\n"))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(parse(&raw.to_string()).unwrap()[0]["excerpt"], "Saturday");
        assert!(parse(r#"{"result":{"isError":true}}"#).is_err());
    }
}

#[cfg(test)]
mod privacy_tests {
    use super::*;
    #[test]
    fn asks_for_place_before_network_and_excludes_private_notes() {
        let day = "2026-11-19".parse().unwrap();
        let notes = [json!({"topic":"home","value":"Secret Town","restricted":true})];
        let q = "What should I do this weekend?";
        assert_eq!(
            prepare(q, &Value::Null, &Value::Null, &notes, day, false).unwrap()["status"],
            "place"
        );
        assert_eq!(
            prepare(q, &Value::Null, &Value::Null, &notes, day, true).unwrap()["status"],
            "private"
        );
        let last = json!({"web":{"status":"place"}});
        let public = prepare("San Francisco", &last, &Value::Null, &notes, day, false).unwrap();
        let query = public["query"].as_str().unwrap();
        assert!(query.starts_with("San Francisco"));
        assert!(!query.contains("Secret"));
        assert!(query.contains("November 21, 2026"));
        assert!(prepare(
            "What is 2 + 2?",
            &Value::Null,
            &Value::Null,
            &[],
            day,
            false
        )
        .is_none());
        assert!(!mentions_day(
            "November 21, 2024 brunch",
            day + Days::new(2)
        ));
    }
}

#[cfg(test)]
mod listing_tests {
    use super::*;
    #[test]
    fn does_not_attach_another_weekends_workshop_to_this_saturday() {
        let pages = [
            json!({"excerpt":"Saturday, November 21, 2026\nDrop-In Art Making: Expressive Block Printing\nSaturday, December 5, 2026\nSculpted Pendants workshop"}),
        ];
        let n = json!({"text":"I enjoy sculpted pendants workshops"});
        let picked = dated_choice(&pages, &["2026-11-21".parse().unwrap()], &[&n]).unwrap();
        assert!(picked.1.contains("Block Printing"));
        assert!(picked.2.contains("November 21"));
    }

    #[test]
    fn a_weekend_pick_needs_a_dated_activity_even_when_the_interest_matches_exactly() {
        let day = "2026-11-28".parse().unwrap();
        let preference = json!({"text":"I enjoy printmaking workshops"});
        let undated = json!({"title":"Open studio","excerpt":"A printmaking workshop for beginners.\nSaturday, December 5, 2026\nWoodblock printing workshop"});
        assert!(weekend_pick(&[undated], &[day], std::slice::from_ref(&preference)).is_none());
        let dated = json!({"title":"Open studio","excerpt":"November 28, 2026\nWoodblock printing workshop at noon"});
        let pick = weekend_pick(&[dated], &[day], std::slice::from_ref(&preference)).unwrap();
        assert!(pick.answer.contains("November 28"));
        assert!(pick.quote.contains("Woodblock printing workshop at noon"));
        assert_eq!(pick.preference, Some(preference));
    }
}

#[cfg(test)]
mod availability_tests {
    use super::*;
    #[test]
    fn a_real_date_does_not_make_a_sold_out_or_closed_event_a_recommendation() {
        let mut pages = Vec::new();
        let mut text = String::new();
        for (i, availability) in [
            "SOLD OUT: Printing workshop",
            "Registration closes November 17.",
            "Drop-in art making. Open Saturday November 21.",
        ]
        .iter()
        .enumerate()
        {
            let url = format!("https://museum.example/{i}");
            pages.push(json!({"title":"Art workshop","url":url}));
            text.push_str(&format!(
                "# Workshop\nURL: {url}\n\nNovember 21, 10 am to 4 pm.\n{availability}\n\n"
            ));
        }
        let response = json!({"result":{"content":[{"text":text}]}});
        let good =
            available_pages(&response.to_string(), &pages, "2026-11-19".parse().unwrap()).unwrap();
        assert_eq!(good.len(), 1);
        assert_eq!(good[0]["url"], pages[2]["url"]);
    }
}

/// A request to identify the visible screen is answered from the capture at summon time.
pub(crate) fn current_screen_question(q: &str) -> bool {
    static SCREEN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)^\s*(?:(?:hey beside|beside|please)[, ]+)?(?:what (?:am i|are we) (?:looking at|reading|viewing|working on)|what(?:['’]s| is) (?:currently )?(?:on (?:my|the) screen|in (?:this|the current) window))(?: (?:right now|now|currently|at the moment|on my screen))*[?.! ]*$").unwrap()
    });
    SCREEN.is_match(q)
}

pub fn floating_lookup(question: &str) -> bool {
    let q = question.to_lowercase();
    let explicit = [
        "search the web",
        "search for ",
        "look up ",
        "look it up",
        "look this up",
        "look that up",
        "search online",
        "web search",
        "check online",
        "google ",
    ]
    .iter()
    .any(|cue| q.contains(cue));
    if current_screen_question(question) { return false; }
    if explicit || outing(question, &Value::Null) {
        return true;
    }
    let personal = ["my ", "our ", "did i ", "this note", "my notes", "on my screen"]
        .iter()
        .any(|cue| q.contains(cue));
    if personal {
        return false;
    }
    [
        "current price",
        "price of ",
        "latest ",
        "opening hours",
        "open now",
        "open today",
        "open this weekend",
        "official docs",
        "official source",
        "official guidance",
        "docs for ",
        "documentation for ",
        "current version",
        "latest version",
        "current hours",
        "still supported",
        "verify online",
        "weather",
        "temperature in",
        "is it raining",
        "forecast for",
        "time in ",
        "latest news",
        "today's news",
        "today’s news",
        "news about",
        "stock price",
        "exchange rate",
        "who won ",
        "score of ",
        "who is the current",
        "who is the ceo",
        "who is the president",
        "who plays ",
        "who directed ",
        "release date",
        "fact check",
        "fact-check",
        "is it open",
        "are they open",
        "open on saturday",
        "open on sunday",
        "holiday hours",
        "how much does it cost",
        "how much is ",
        "how many calories",
        "ticket price",
        "current fare",
        "flights from ",
        "flights to ",
        "near me",
        "best ",
        "compare ",
        " vs ",
        "reviews of ",
        "reviews for ",
        "2025",
        "2026",
        "official documentation",
        "release notes",
        "still available",
        "still accurate",
        "on sale now",
        "when does registration",
        "what time does",
        "does apple recommend",
        "what does apple recommend",
        "can you verify",
        "is this up to date",
    ]
    .iter()
    .any(|cue| q.contains(cue))
}

/// Read the underlying pages while retaining search snippets if a publisher is unavailable.
pub(crate) fn research(query: &str, outing: bool, city: Option<&str>) -> Res<Vec<Value>> {
    research_with(query,outing,city,search,request)
}

fn research_with(query:&str,outing:bool,city:Option<&str>,search:impl FnOnce(&str,bool)->Res<Vec<Value>>, fetch:impl Fn(&str,Value)->Res<String>)->Res<Vec<Value>>{
    let mut pages = search(query, outing)?;
    let has_snippets = pages
        .iter()
        .filter(|p| p["excerpt"].as_str().is_some_and(|s| s.trim().len() >= 80))
        .count()
        >= 2;
    if !outing && has_snippets {
        return Ok(pages);
    }
    let urls: Vec<_> = pages
        .iter()
        .filter_map(|p| p["url"].as_str())
        .take(if outing { 8 } else { 4 })
        .collect();
    if urls.is_empty() {
        return Ok(pages);
    }
    let raw =
        match fetch("web_fetch_exa", json!({"urls":urls,"maxCharacters":9000})) {
            Ok(raw) => raw,
            Err(_) if outing => {
                // Offer the search links without inferring current availability from a snippet.
                for page in &mut pages { page["availabilityUnverified"] = json!(true); }
                return Ok(pages);
            }
            Err(_) => return Ok(pages),
        };
    if outing {
        let mut verified = available_pages(&raw, &pages, chrono::Local::now().date_naive())?;
        if let Some(city) = city {
            let city = city.split(',').next().unwrap_or(city).trim().to_lowercase();
            let compact = city.replace(' ', "");
            verified.retain(|p| {
                let page = format!("{} {}", p["title"].as_str().unwrap_or(""), p["excerpt"].as_str().unwrap_or(""));
                let host = url::Url::parse(p["url"].as_str().unwrap_or("")).ok()
                    .and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default();
                page.to_lowercase().contains(&city) || host.contains(&compact)
            });
        }
        return Ok(verified);
    }
    for block in blocks(&raw)? {
        let lines: Vec<_> = block.lines().collect();
        let starts: Vec<_> = (1..lines.len())
            .filter(|i| lines[i - 1].starts_with("# ") && lines[*i].starts_with("URL: "))
            .collect();
        for (j, &start) in starts.iter().enumerate() {
            let url = lines[start].trim_start_matches("URL: ").trim();
            if let Some(page) = pages.iter_mut().find(|p| p["url"] == url) {
                let end = starts.get(j + 1).map_or(lines.len(), |i| i - 1);
                let body = lines[start + 1..end].join("\n");
                if body.trim().len() > 80 {
                    page["excerpt"] = json!(format!(
                        "{}\n{}",
                        page["excerpt"].as_str().unwrap_or(""),
                        body.chars().take(7200).collect::<String>()
                    ));
                }
            }
        }
    }
    Ok(pages)
}

pub(crate) fn evidence(pages: &[Value], query: &str) -> Vec<Value> {
    let terms = crate::conversation::words(query);
    pages
        .iter()
        .take(5)
        .enumerate()
        .map(|(i, page)| {
            let lines: Vec<_> = page["excerpt"]
                .as_str()
                .unwrap_or("")
                .lines()
                .map(str::trim)
                .filter(|line| {
                    !line.is_empty()
                        && !line.starts_with("http")
                        && line
                            .chars()
                            .any(|c| c.is_alphanumeric() || matches!(c, '⌘' | '⇧' | '⌥' | '⌃'))
                })
                .collect();
            // Search already matched this page's title. Rank its passages on the
            // part of the question that the title itself does not answer.
            let title_terms = crate::conversation::words(page["title"].as_str().unwrap_or(""));
            let subject: HashSet<_> = terms.difference(&title_terms).cloned().collect();
            let mut seen = HashSet::new();
            let mut ranked: Vec<_> = lines
                .iter()
                .enumerate()
                .filter(|(_, line)| seen.insert(**line))
                .map(|(index, line)| {
                    let words = crate::conversation::words(line);
                    let specific = words.intersection(&subject).count();
                    let overlap = words.intersection(&terms).count();
                    let heading = line.starts_with('#') || page["title"].as_str() == Some(*line);
                    let table_row = line
                        .trim_matches('|')
                        .split('|')
                        .filter(|cell| !cell.trim().is_empty())
                        .count()
                        >= 2;
                    (
                        (specific, table_row && specific > 0, overlap, !heading),
                        index,
                    )
                })
                .collect();
            ranked.sort_by_key(|(score, index)| (std::cmp::Reverse(*score), *index));
            let mut indices: Vec<_> = ranked
                .iter()
                .take(2)
                .flat_map(|(_, j)| [j.saturating_sub(1), *j, j + 1])
                .filter(|j| *j < lines.len())
                .collect();
            indices.sort_unstable();
            indices.dedup();
            let quote = ranked
                .first()
                .map(|(_, j)| {
                    let heading = short_quote(lines[*j]);
                    if let Some(value) = lines.get(j + 1).filter(|v| v.chars().count() <= 14) {
                        format!("{heading}\n{value}")
                    } else {
                        heading
                    }
                })
                .unwrap_or_default();
            let excerpts: Vec<_> = indices
                .iter()
                .map(|j| lines[*j].chars().take(330).collect::<String>())
                .collect();
            json!({"number":i+1,"title":page["title"],"quote":quote,"lines":excerpts})
        })
        .collect()
}

/// A direct "search the web for X" supplies its own outbound terms.
pub(crate) fn dictated_query(q: &str) -> Option<&str> {
    let raw = q.trim().trim_end_matches(['.', '?', '!']);
    let lower = raw.to_ascii_lowercase();
    let prefix = [
        "search the web for ",
        "search online for ",
        "search for ",
        "look up ",
        "google ",
    ]
        .into_iter()
        .find(|p| lower.starts_with(p))?;
    let terms = raw.get(prefix.len()..)?.trim();
    let words = terms.to_lowercase();
    if terms.len() < 4
        || terms.len() > 220
        || terms.contains(['@', '/', '\\', '\n', '"'])
        || words.split(|c: char| !c.is_alphanumeric()).any(|w| {
            matches!(
                w,
                "this"
                    | "that"
                    | "these"
                    | "those"
                    | "it"
                    | "my"
                    | "our"
                    | "me"
                    | "his"
                    | "her"
                    | "their"
                    | "mine"
                    | "we"
                    | "i"
            )
        })
        || local::sensitive(terms)
        || sources::audit(terms).is_none()
    {
        return None;
    }
    Some(terms)
}
#[cfg(test)]
mod floating_tests {
    use super::*;
    #[test]
    fn short_values_stay_with_their_labels_in_search_evidence() {
        let pages = [
            json!({"title":"Keyboard shortcuts","excerpt":"Mark done\n⇧⌘U\nApply checklist format\n⇧⌘L"}),
        ];
        let result = evidence(&pages, "apply checklist format");
        assert_eq!(result[0]["quote"], "Apply checklist format\n⇧⌘L");
        assert!(result[0]["lines"]
            .as_array()
            .unwrap()
            .contains(&json!("⇧⌘L")));

        let pages = [
            json!({"title":"Admission","excerpt":"Students\nFree\nResident admission Saturday\nFree\nGeneral admission Sunday\n$20"}),
        ];
        let result = evidence(&pages, "resident admission Saturday");
        assert_eq!(result[0]["quote"], "Resident admission Saturday\nFree");
    }

    #[test]
    fn a_specific_table_entry_outranks_the_title_that_matched_the_search() {
        let pages = [json!({
            "title":"Editor keyboard shortcuts on Mac",
            "excerpt":"# Editor keyboard shortcuts on Mac\nKeyboard shortcuts for the editor on Mac\n| Apply checklist format | Shift-Command-L |\n| Mark a checklist item complete | Shift-Command-U |"
        })];
        let result = evidence(&pages, "editor checklist keyboard shortcut for Mac");
        assert_eq!(
            result[0]["quote"],
            "| Apply checklist format | Shift-Command-L |"
        );
    }

    #[test]
    fn only_explicit_outbound_terms_bypass_review() {
        assert_eq!(
            dictated_query("Look up Apple Notes checklist shortcuts."),
            Some("Apple Notes checklist shortcuts")
        );
        for q in [
            "Look this up",
            "Look up this label",
            "Look up my notes",
            "Look up /Users/person/private",
            "Do you know the latest price?",
        ] {
            assert!(dictated_query(q).is_none());
        }
    }
}

/// A confirmed public topic stays on the sources-only path even when the
/// term is an airport code or contains a quoted phrase.
pub(crate) fn confirmed_topic<'a>(question: &'a str, approved: Option<&str>) -> Option<&'a str> {
    (approved == Some(question) && !outing(question, &Value::Null)
        && public_typed_lookup(question)).then_some(question)
}

/// Complete an immediately preceding request for public terms using only
/// the typed reply and, when known, the kind of lookup the user asked for.
pub(crate) fn clarified_topic(previous: &str, reply: &str) -> Option<String> {
    let topic = reply.trim().trim_end_matches('.');
    if !(3..=180).contains(&topic.len()) || !topic.chars().any(char::is_alphabetic)
        || topic.split_whitespace().count() > 12 || !public_typed_lookup(topic)
        || topic.ends_with('?') || !floating_lookup(previous)
    {
        return None;
    }
    let lower = topic.to_lowercase();
    if matches!(lower.as_str(), "no" | "no thanks" | "thanks" | "thank you" | "stop" | "cancel" | "never mind" | "nevermind" | "forget it" | "yes" | "yeah" | "okay" | "sure")
        || matches!(lower.split_whitespace().next(), Some("what" | "how" | "why" | "can" | "write" | "draft" | "remember" | "summarize" | "instead" | "actually"))
    {
        return None;
    }
    let request = previous.to_lowercase();
    let detail = ["opening hours", "current price", "latest version"]
        .into_iter().find(|kind| request.contains(kind) && !lower.contains(kind));
    Some(match detail {
        Some(kind) => format!("{topic} {kind}"),
        None => topic.to_owned(),
    })
}

// Automatic search can use the user's own public question. A private reference
// needs a narrower query before dispatch.
pub fn public_typed_lookup(q: &str) -> bool {
    static PROTOCOL: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)\b(?:http/[123](?:\.[01])?|c#)(?:\s|$)").unwrap());
    let normalized = PROTOCOL.replace_all(q, "technology ");
    let q = normalized.as_ref();
    let lower = q.to_lowercase();
    let words = lower.replace("this weekend", "weekend").replace("is it raining", "rain").replace("what time is it", "time");
    !words.split(|c: char| !c.is_alphabetic()).any(|w| {
        matches!(
            w,
            "i" | "me"
                | "my"
                | "mine"
                | "his"
                | "her"
                | "their"
                | "our"
                | "ours"
                | "we"
                | "this"
                | "that"
                | "these"
                | "those"
                | "it"
                | "email"
                | "client"
                | "internal"
                | "private"
        )
    }) && !q.contains(['/', '@', '#', ':', '\\', '\n'])
        && !crate::local::sensitive(q)
        && crate::sources::audit(q).is_some()
        && crate::sources::audit(&lower).as_deref() == Some(lower.as_str())
}

/// A public question that can be answered from the user's own terms alone.
/// Outings and references to the user's work keep their contextual routes.
pub(crate) fn standalone_query(question: &str) -> Option<&str> {
    let q = question.trim();
    if current_screen_question(q) || outing(q, &Value::Null) {
        return None;
    }
    dictated_query(q).or_else(|| (floating_lookup(q) && public_typed_lookup(q)).then_some(q))
}

#[cfg(test)]
mod floating_policy_tests {
    use super::*;

    #[test]
    fn both_views_can_look_up_a_public_question_without_a_planner_or_private_context() {
        let day = "2026-11-26".parse().unwrap();
        let q = "What do the official docs say about keyboard shortcuts in Apple Notes?";
        assert_eq!(standalone_query(q), Some(q));
        let plan = prepare(q, &Value::Null, &json!({"web":"unrelated terms"}), &[], day, false).unwrap();
        assert_eq!(plan["query"], q);
        assert_eq!(prepare(q, &Value::Null, &Value::Null, &[], day, true).unwrap()["status"], "private");
        assert_eq!(standalone_query("Look up Apple Notes shortcuts."), Some("Apple Notes shortcuts"));
        assert_eq!(prepare("What should I do this weekend in my area?", &Value::Null, &Value::Null, &[], day, false).unwrap()["status"], "place");
        assert_eq!(prepare("What should I do this weekend in New York?", &Value::Null, &Value::Null, &[], day, false).unwrap()["city"], "New York");
        for dependent in ["Look up her current price", "What am I looking at right now?",
            "What should I do this weekend?", "Check the latest on this document"] {
            assert_eq!(standalone_query(dependent), None, "{dependent}");
        }
    }

    #[test]
    fn screen_identity_precedes_freshness_and_web_planning() {
        for question in ["What am I looking at right now", "What's on my screen?", "What are we looking at now?"] {
            assert!(super::current_screen_question(question));
            assert!(!super::floating_lookup(question));
            assert!(super::prepare(question, &serde_json::Value::Null, &serde_json::json!({"web":"today's news"}), &[], chrono::Local::now().date_naive(), false).is_none());
        }
        assert!(super::floating_lookup("weather in oslo right now?"));
        assert!(!super::floating_lookup("What should I write right now?"));
    }

    #[test]
    fn automatic_lookup_reviews_private_references() {
        assert!(super::public_typed_lookup("weather in oslo tomorrow?"));
        for q in [
            "weather at my office",
            "latest on this email",
            "price for a@b.org",
            "forecast for /Users/me",
        ] {
            assert!(!super::public_typed_lookup(q));
        }
    }
}
