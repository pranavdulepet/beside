use crate::{conversation, engine, local, memory, scroll, sources, store::*};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{BufRead, Write},
    path::Path,
};
pub const AGENTS: [&str; 4] = ["Claude Code", "Codex", "Claude Desktop", "Cursor"];
pub(crate) fn token_hash(token: &str) -> String {
    hash(&format!("sumi-seal/{token}"))
}
fn locate(s: &Store, token: &str) -> Res<Value> {
    s.list("agent")?
        .into_iter()
        .find(|a| a["linkState"].is_null() && a["tokenHash"] == token_hash(token))
        .ok_or("This agent is not connected.".into())
}
pub fn authorized(s: &Store, token: &str) -> bool {
    locate(s, token).is_ok()
}
fn shareable(row: &Value) -> bool {
    let local_source = |item: &Value| matches!(item.as_str(), Some("private-desk" | "messages"));
    row["localOnly"] != true
        && row["private"] != true
        && row["restricted"] != true
        && !local_source(&row["source"])
        && !row["sources"]
            .as_array()
            .is_some_and(|list| list.iter().any(local_source))
}
fn scoped(s: &Store, a: &Value) -> Res<Vec<Value>> {
    let mut items = scroll::approved(s)?;
    items.retain(|v| {
        shareable(v)
            && a["sections"]
                .as_array()
                .is_some_and(|ss| ss.contains(&v["section"]))
    });
    Ok(items)
}
fn shared_rules(items: Vec<Value>) -> Vec<Value> {
    items
        .into_iter()
        .map(|v| json!({"id":v["id"],"text":v["text"],"section":v["section"]}))
        .collect()
}
fn public_memories(s: &Store) -> Vec<Value> {
    memory::timeline(s)
        .unwrap_or_default()
        .into_iter()
        .filter(|n| memory::current(n) && shareable(n))
        .collect()
}
fn cosine_bytes(a: &[u8], b: &[u8]) -> f32 {
    if a.len() != b.len() || a.len() < 4 || a.len() % 4 != 0 {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (ca, cb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let va = f32::from_le_bytes([ca[0], ca[1], ca[2], ca[3]]);
        let vb = f32::from_le_bytes([cb[0], cb[1], cb[2], cb[3]]);
        dot += va * vb;
        na += va * va;
        nb += vb * vb;
    }
    let denom = (na * nb).sqrt();
    if denom > 1e-6 {
        dot / denom
    } else {
        0.0
    }
}

/// Rank only entries the agent can read, including current notes when granted.
fn hybrid_recall(s: &Store, a: &Value, query: &str, limit: usize) -> Res<Vec<Value>> {
    let q_words = conversation::words(query);
    let q_vec = local::embedding(query);
    let mut scored: Vec<(f64, Value)> = Vec::new();

    for entry in scoped(s, a)? {
        let text = entry["text"].as_str().unwrap_or("");
        if text.trim().is_empty() {
            continue;
        }
        let lex = q_words.intersection(&conversation::words(text)).count() as f64;
        let cos = cosine_bytes(&q_vec, &local::embedding(text)).max(0.0) as f64;
        let score = lex * 1.4 + cos * 2.2;
        if query.trim().is_empty() || lex > 0.0 || cos >= 0.35 {
            scored.push((
                score,
                json!({
                    "id": entry["id"],
                    "kind": "scroll",
                    "section": entry["section"],
                    "text": text,
                    "score": (score * 100.0).round() / 100.0
                }),
            ));
        }
    }

    if a["shareMemories"] == true {
        for note in public_memories(s) {
            let text = note["text"].as_str().unwrap_or("");
            if text.trim().is_empty() {
                continue;
            }
            let lex = q_words.intersection(&conversation::words(text)).count() as f64;
            let cos = cosine_bytes(&q_vec, &local::embedding(text)).max(0.0) as f64;
            let score = lex * 1.3 + cos * 2.0;
            if query.trim().is_empty() || lex > 0.0 || cos >= 0.42 {
                scored.push((
                    score,
                    json!({
                        "id": note["id"],
                        "kind": "memory",
                        "topic": note["topic"],
                        "text": text,
                        "learnedAt": note["learnedAt"],
                        "score": (score * 100.0).round() / 100.0
                    }),
                ));
            }
        }
    }

    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    Ok(scored
        .into_iter()
        .take(limit.clamp(1, 24))
        .map(|(_, v)| v)
        .collect())
}

fn build_profile(s: &Store, a: &Value) -> Res<Value> {
    let mut sections: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for entry in scoped(s, a)? {
        let sec = entry["section"].as_str().unwrap_or("general").to_owned();
        sections
            .entry(sec)
            .or_default()
            .push(json!({"id": entry["id"], "text": entry["text"]}));
    }
    let notes: Vec<Value> = if a["shareMemories"] == true {
        public_memories(s)
            .into_iter()
            .take(20)
            .map(|n| json!({"id": n["id"], "topic": n["topic"], "text": n["text"], "learnedAt": n["learnedAt"]}))
            .collect()
    } else {
        Vec::new()
    };
    Ok(json!({
        "sections": sections,
        "memories": notes,
        "permittedSections": a["sections"]
    }))
}

fn is_low_stakes_generalized_preference(question: &str, options: &[Value]) -> bool {
    if !(2..=8).contains(&options.len()) {
        return false;
    }
    // Autonomous answers describe a presentation preference. An unknown option
    // can encode a real-world action even if the question asks about formatting.
    static PRESENTATION: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)^(?:which|what) (?:(?:commit message|message|response|writing|report|document|heading|list|email|reply|summary|measurement) )?(?:style|format|tone|length|layout|spelling|headings|units) (?:do|would) you prefer\??$").unwrap()
    });
    if !PRESENTATION.is_match(question.trim()) {
        return false;
    }
    let labels = [
        "brief",
        "detailed",
        "short",
        "long",
        "bullets",
        "prose",
        "table",
        "paragraphs",
        "sentences",
        "numbered list",
        "formal",
        "casual",
        "conversational",
        "plain",
        "conventional",
        "direct",
        "friendly",
        "sentence case",
        "title case",
        "american",
        "british",
        "metric",
        "imperial",
    ];
    options.iter().all(|o| {
        o.as_str()
            .is_some_and(|v| labels.contains(&v.trim().to_lowercase().as_str()))
    })
}

pub(crate) fn config_path(home: &Path, name: &str) -> Res<std::path::PathBuf> {
    Ok(home.join(match name {
        "Claude Code" => ".claude.json",
        "Codex" => ".codex/config.toml",
        "Claude Desktop" => "Library/Application Support/Claude/claude_desktop_config.json",
        "Cursor" => ".cursor/mcp.json",
        _ => return Err("Unknown agent.".into()),
    }))
}
pub fn detected() -> Vec<&'static str> {
    let home = sources::home();
    AGENTS
        .into_iter()
        .filter(|n| {
            config_path(&home, n).is_ok_and(|p| p.exists())
                || match *n {
                    "Codex" => home.join(".codex").is_dir(),
                    "Claude Code" => home.join(".claude").is_dir(),
                    "Claude Desktop" => Path::new("/Applications/Claude.app").exists(),
                    "Cursor" => Path::new("/Applications/Cursor.app").exists(),
                    _ => false,
                }
        })
        .collect()
}
pub use crate::agent_link::{connect, disconnect};
pub fn export(s: &Store) -> Res<()> {
    let mut agents = s.list("agent")?;
    agents.retain(|a| a["linkState"].is_null());
    for a in &mut agents {
        a.as_object_mut().unwrap().remove("tokenHash");
        a.as_object_mut().unwrap().remove("link");
    }
    let path = s.root.join("seal.yaml");
    let data = serde_json::to_vec_pretty(&json!({"agents":agents})).msg()?;
    if std::fs::read(&path).ok().as_deref() != Some(data.as_slice()) {
        atomic(&path, &data)?;
    }
    if s.get("sealExportPending")? == true {
        s.set("sealExportPending", &json!(false))?;
    }
    Ok(())
}
pub fn policy(s: &Store, a: &Value) -> Res<()> {
    let _lock = crate::agent_link::notebook_lock(s)?;
    let mut record = s.doc("agent", a["id"].as_str().unwrap_or(""))?;
    if !record["linkState"].is_null() {
        return Err("Finish changing this connection first.".into());
    }
    if !["Read", "Ask", "Answer for me"].contains(&a["permission"].as_str().unwrap_or("")) {
        return Err("Unknown permission.".into());
    }
    if !a["sections"].as_array().is_some_and(|v| {
        v.iter()
            .all(|x| scroll::SECTIONS.contains(&x.as_str().unwrap_or("")))
    }) {
        return Err("Invalid section.".into());
    }
    record["shareMemories"] = json!(a["shareMemories"]
        .as_bool()
        .unwrap_or(record["shareMemories"] == true));
    record["sections"] = a["sections"].clone();
    record["permission"] = a["permission"].clone();
    record["threshold"] = json!(a["threshold"].as_f64().unwrap_or(0.96).clamp(0.8, 1.0));
    s.set("sealExportPending", &json!(true))?;
    s.put("agent", &record)?;
    export(s)
}
pub fn call(s: &Store, token: &str, name: &str, args: &Value) -> Res<Value> {
    let is_answer_tool = matches!(name, "sumi_answer" | "beside_answer");
    let is_read = matches!(
        name,
        "sumi_predict" | "beside_predict" | "sumi_context" | "beside_recall" | "beside_profile"
    );
    // A queued request and revocation must agree on whether the ticket can still be answered.
    let _ticket_lock = if is_answer_tool || matches!(name, "sumi_observe" | "beside_observe") {
        Some(crate::agent_link::notebook_lock(s)?)
    } else {
        None
    };
    let a = locate(s, token)?;
    let p = a["permission"].as_str().unwrap_or("Read");
    let who = a["id"].as_str().unwrap();
    let q = args["question"]
        .as_str()
        .or(args["query"].as_str())
        .or(args["task"].as_str())
        .or(args["event"].as_str())
        .unwrap_or("");
    let q = if is_answer_tool && args["ticket"].is_string() {
        "Check a queued question".into()
    } else if name == "beside_profile" && q.trim().is_empty() {
        "Read permitted profile".into()
    } else {
        sources::audit(q).ok_or("This request cannot be retained.")?
    };
    let mut log = json!({"id":id(),"agent":who,"name":a["name"],"date":now(),"tool":name,"question":q,"status":"served"});
    let result = match name {
        "sumi_context" => json!({"entries":shared_rules(engine::rank(scoped(s,&a)?,&q))}),
        "beside_recall" => {
            let limit = args["limit"].as_u64().unwrap_or(8) as usize;
            let items = hybrid_recall(s, &a, &q, limit)?;
            json!({"query": q, "items": items})
        }
        "beside_profile" => build_profile(s, &a)?,
        "sumi_predict" | "beside_predict" if p != "Read" => {
            let model = s.get("model")?;
            let entries = shared_rules(engine::select_rules(s, &model, &q, scoped(s, &a)?));
            let guess = if args["options"].is_null() {
                engine::predict_ranked(s, &model, &q, &entries, crate::jobs::prompt(s))?
            } else {
                let choices = args["options"]
                    .as_array()
                    .filter(|o| (2..=16).contains(&o.len()))
                    .ok_or("Provide two to sixteen choices.")?;
                for option in choices {
                    let label = option
                        .as_str()
                        .filter(|v| !v.trim().is_empty() && v.len() <= 240)
                        .ok_or("Use short text options.")?;
                    if sources::audit(label).as_deref() != Some(label) {
                        return Err("These option labels include private identifiers.".into());
                    }
                }
                let schema = json!({"type":"object","properties":{"choice":{"type":"string","enum":choices}},"required":["choice"],"additionalProperties":false});
                let raw = crate::plan::think(
                    s,
                    &model,
                    "agent prediction",
                    crate::jobs::prompt(s),
                    &json!({"situation":q,"rules":entries,"options":choices}),
                    120,
                    Some(&schema),
                )?;
                let answer: Value = serde_json::from_str(&raw).msg()?;
                if !choices.contains(&answer["choice"]) {
                    return Err("The local model abstained.".into());
                }
                answer["choice"].as_str().unwrap().to_owned()
            };
            json!({"guess":guess,"confidence":null,"rules":entries,"advisory":true})
        }
        "sumi_answer" | "beside_answer" if p == "Answer for me" => {
            if let Some(ticket) = args["ticket"].as_str() {
                let old = s.doc("log", ticket)?;
                if old["agent"] != who
                    || !matches!(old["tool"].as_str(), Some("sumi_answer" | "beside_answer"))
                    || !matches!(
                        old["status"].as_str(),
                        Some("waiting" | "answered" | "reviewed")
                    )
                {
                    return Err("Unknown question for this agent.".into());
                }
                if old["policyScope"].is_string()
                    && old["policyScope"]
                        != hash(
                            &json!([a["sections"], a["shareMemories"] == true, a["permission"]])
                                .to_string(),
                        )
                {
                    return Err("Permissions changed since this question was queued.".into());
                }
                let result = if old["human"].is_string() {
                    json!({"answer":old["human"],"decidedBy":"user"})
                } else if old["status"] == "answered" {
                    old["shared"].clone()
                } else {
                    json!({"queued":true,"id":ticket})
                };
                log["ticket"] = json!(ticket);
                log["status"] = json!("checked");
                log["shared"] = match name {
                    "beside_recall" => {
                        json!({"items":result["items"].as_array().map_or(0,Vec::len)})
                    }
                    "beside_profile" => {
                        json!({"permittedSections":a["sections"], "memories":result["memories"].as_array().map_or(0,Vec::len)})
                    }
                    _ => result.clone(),
                };
                s.put("log", &log)?;
                return Ok(result);
            }

            let options = args["options"].as_array().cloned().unwrap_or_default();
            // `sumi_answer` keeps its original strict 2-question policy; `beside_answer`
            // generalizes shadow-calibrated autonomous replies to any repeated, closed,
            // non-sensitive low-stakes preference question once >= 20 shadow trials match.
            let safe = if name == "sumi_answer" {
                match q.as_str() {
                    "Which response length do you prefer?" => {
                        options == vec![json!("Brief"), json!("Detailed")]
                    }
                    "Which format do you prefer?" => {
                        options == vec![json!("Bullets"), json!("Prose")]
                    }
                    _ => false,
                }
            } else {
                is_low_stakes_generalized_preference(&q, &options)
            };
            let scope = hash(
                &json!([a["sections"], a["shareMemories"] == true, a["permission"]]).to_string(),
            );
            log["policyScope"] = json!(scope);
            let hist: Vec<Value> = s
                .list("log")?
                .into_iter()
                .filter(|l| {
                    l["agent"] == who
                        && (name == "sumi_answer" || l["policyScope"] == scope)
                        && matches!(l["tool"].as_str(), Some("sumi_answer" | "beside_answer"))
                        && l["status"] == "reviewed"
                        && l["question"] == q
                        && l["options"] == json!(options)
                        && l["human"].is_string()
                })
                .collect();
            let predicted = hist.first().and_then(|l| l["human"].as_str());
            let checked: Vec<_> = hist.iter().filter(|l| l["shadow"].is_string()).collect();
            let confidence = (1.
                + checked.iter().filter(|l| l["shadow"] == l["human"]).count() as f64)
                / (2. + checked.len() as f64);
            log["shadow"] = json!(predicted);
            log["confidence"] = json!(confidence);
            log["options"] = json!(options);
            if safe
                && checked.len() >= 20
                && confidence >= a["threshold"].as_f64().unwrap_or(1.)
                && options.iter().any(|v| v.as_str() == predicted)
            {
                log["status"] = json!("answered");
                json!({"id":log["id"],"answer":predicted,"confidence":confidence,"trials":checked.len()})
            } else {
                log["status"] = json!("waiting");
                json!({"queued":true,"id":log["id"],"trials":checked.len(),"needed":20})
            }
        }
        "sumi_observe" | "beside_observe" if p != "Read" => {
            log["status"] = json!("observed");
            sources::add(
                s,
                &format!("agent-{who}"),
                &id(),
                &now(),
                "Agent-reported observation",
                &q,
            )?;
            json!({"recorded":true})
        }
        _ => return Err("This permission does not allow that tool.".into()),
    };
    if is_read {
        let current = locate(s, token)?;
        if current["permission"] != a["permission"]
            || current["sections"] != a["sections"]
            || current["shareMemories"] != a["shareMemories"]
        {
            return Err("The agent permissions changed during this prediction.".into());
        }
    }
    log["shared"] = match name {
        "beside_recall" => json!({"items":result["items"].as_array().map_or(0,Vec::len)}),
        "beside_profile" => {
            json!({"permittedSections":a["sections"], "memories":result["memories"].as_array().map_or(0,Vec::len)})
        }
        _ => result.clone(),
    };
    s.put("log", &log)?;
    Ok(result)
}
fn tools_list(permission: &str) -> Value {
    let mut out = vec![];
    for (name, arg, detail) in [
        (
            "beside_recall",
            "query",
            "Hybrid semantic and lexical recall across permitted personal memory and Scroll rules.",
        ),
        (
            "beside_profile",
            "query",
            "Read a structured snapshot of permitted Scroll sections and non-private personal memories.",
        ),
        (
            "beside_predict",
            "question",
            "Ask Beside for an advisory prediction grounded in permitted Scroll rules.",
        ),
        (
            "beside_answer",
            "question",
            "Ask a closed low-stakes preference question; answers autonomously only after 20+ calibrated shadow trials, otherwise queues for the person.",
        ),
        (
            "beside_observe",
            "event",
            "Record an observed correction or preference signal into Beside's encrypted notebook.",
        ),
        // Legacy sumi_* aliases retained for full backwards compatibility
        (
            "sumi_context",
            "task",
            "Read relevant permitted Scroll entries.",
        ),
        ("sumi_predict", "question", "Ask for an advisory guess."),
        (
            "sumi_answer",
            "question",
            "Ask for a low-stakes preference or queue it for the person.",
        ),
        ("sumi_observe", "event", "Record an observed correction."),
    ] {
        if (matches!(name, "sumi_answer" | "beside_answer") && permission != "Answer for me")
            || (matches!(name, "sumi_predict" | "beside_predict" | "sumi_observe" | "beside_observe") && permission == "Read")
        {
            continue;
        }
        let mut tool = json!({"name":name,"description":detail,"inputSchema":{"type":"object","properties":{(arg):{"type":"string"},"limit":{"type":"integer"},"options":{"type":"array","items":{"type":"string"}},"ticket":{"type":"string","description":"ID of a previously queued question."}},"required":[arg]}});
        if matches!(name, "sumi_answer" | "beside_answer") {
            tool["inputSchema"]
                .as_object_mut()
                .unwrap()
                .remove("required");
            tool["inputSchema"]["anyOf"] =
                json!([{"required":["question"]},{"required":["ticket"]}]);
        } else if name == "beside_profile" {
            tool["inputSchema"]
                .as_object_mut()
                .unwrap()
                .remove("required");
        }
        out.push(tool)
    }
    json!({"tools":out})
}
pub fn rpc(s: &Store, token: &str, req: &Value) -> Option<Value> {
    if req.get("id").is_none() {
        return None;
    }
    let result = (|| -> Res<Value> {
        let a = locate(s, token)?;
        match req["method"].as_str().unwrap_or("") {
            "initialize" => Ok(
                json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{},"resources":{}},"serverInfo":{"name":"Beside","version":env!("CARGO_PKG_VERSION")}}),
            ),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(tools_list(a["permission"].as_str().unwrap_or("Read"))),
            "resources/list" => {
                let mut resources: Vec<Value> = a["sections"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|v| {
                        let sec = v.as_str().unwrap_or("");
                        vec![
                            json!({"uri":format!("sumi://scroll/{sec}"),"name":sec,"mimeType":"text/plain"}),
                            json!({"uri":format!("beside://memory/{sec}"),"name":format!("Beside memory · {sec}"),"mimeType":"text/plain"}),
                        ]
                    })
                    .collect();
                if a["shareMemories"] == true
                    || a["sections"].as_array().is_some_and(|ss| !ss.is_empty())
                {
                    resources.push(json!({"uri":"beside://memory/profile","name":"Beside portable profile","mimeType":"application/json"}));
                }
                Ok(json!({"resources": resources}))
            }
            "resources/read" => {
                let _scope_lock = crate::agent_link::notebook_lock(s)?;
                let a = locate(s, token)?;
                let uri = req["params"]["uri"].as_str().unwrap_or("");
                if uri == "beside://memory/profile" {
                    if a["shareMemories"] != true
                        && a["sections"].as_array().is_none_or(|ss| ss.is_empty())
                    {
                        return Err("No sections are permitted for this agent.".into());
                    }
                    let profile = build_profile(s, &a)?;
                    s.put("log",&json!({"id":id(),"date":now(),"agent":a["id"],"name":a["name"],"tool":"resources/read","question":uri,"status":"served","shared":{"permittedSections":a["sections"],"memories":profile["memories"].as_array().map_or(0,Vec::len)}}))?;
                    return Ok(
                        json!({"contents":[{"uri":uri,"mimeType":"application/json","text":profile.to_string()}]}),
                    );
                }
                let sec = uri
                    .strip_prefix("sumi://scroll/")
                    .or_else(|| uri.strip_prefix("beside://memory/"))
                    .ok_or("Unknown resource")?;
                if !a["sections"].as_array().unwrap().iter().any(|v| v == sec) {
                    return Err("Section is outside this agent's scope.".into());
                }
                let scoped_rows: Vec<_> = scoped(s, &a)?
                    .into_iter()
                    .filter(|v| v["section"] == sec)
                    .collect();
                let rule_ids = scoped_rows
                    .iter()
                    .map(|v| v["id"].clone())
                    .collect::<Vec<_>>();
                let entries = scoped_rows
                    .iter()
                    .map(|v| v["text"].as_str().unwrap_or("").to_owned())
                    .collect::<Vec<_>>();
                s.put("log",&json!({"id":id(),"date":now(),"agent":a["id"],"name":a["name"],"tool":"resources/read","question":uri,"ruleIds":rule_ids,"status":"served","shared":entries}))?;
                Ok(
                    json!({"contents":[{"uri":uri,"mimeType":"text/plain","text":entries.join("\n")}]}),
                )
            }
            "tools/call" => {
                let r = call(
                    s,
                    token,
                    req["params"]["name"].as_str().unwrap_or(""),
                    &req["params"]["arguments"],
                );
                Ok(match r {
                    Ok(v) => json!({"content":[{"type":"text","text":v.to_string()}]}),
                    Err(e) => json!({"isError":true,"content":[{"type":"text","text":e}]}),
                })
            }
            _ => Err("Unsupported method.".into()),
        }
    })();
    Some(match result {
        Ok(v) => json!({"jsonrpc":"2.0","id":req["id"],"result":v}),
        Err(e) => json!({"jsonrpc":"2.0","id":req["id"],"error":{"code":-32000,"message":e}}),
    })
}
pub fn serve(s: &Store, token: &str) -> Res<()> {
    for line in std::io::stdin().lock().lines() {
        let line = line.msg()?;
        let Ok(v) = serde_json::from_str(&line) else {
            continue;
        };
        if let Some(out) = rpc(s, token, &v) {
            writeln!(std::io::stdout().lock(), "{out}").msg()?
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_agent(s: &Store, permission: &str, sections: Vec<&str>) -> String {
        let token = "test-agent-token-xyz";
        s.put(
            "agent",
            &json!({
                "id": "agent-1",
                "name": "Claude Code",
                "tokenHash": token_hash(token),
                "permission": permission,
                "sections": sections,
                "threshold": 0.90
            }),
        )
        .unwrap();
        token.to_owned()
    }

    #[test]
    fn beside_recall_and_profile_respect_section_and_privacy_boundaries() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Store::with_key(tmp.path().join("book"), &[41; 32]).unwrap();
        crate::app::setup(&s).unwrap();
        let token = setup_agent(&s, "Read", vec!["writing"]);

        for alias in ["sumi_observe","beside_observe"]{
            assert!(call(&s,&token,alias,&json!({"event":"A read-only client offered a correction"})).is_err());
        }
        assert_eq!(s.db.lock().unwrap().query_row("SELECT count(*) FROM moments",[],|r|r.get::<_,i64>(0)).unwrap(),0);

        // Permitted Scroll entry in `writing`, unpermitted in `goals`
        scroll::change(
            &s,
            "create",
            &json!({"section": "writing", "text": "Write terse Rust code reviews with concrete diffs."}),
        )
        .unwrap();
        scroll::change(
            &s,
            "create",
            &json!({"section": "goals", "text": "Secret career plan for next year."}),
        )
        .unwrap();

        scroll::change(&s,"createFromReview",&json!({"id":"private-writing","section":"writing","localOnly":true,"text":"Private manuscript schedule."})).unwrap();
        let hidden=call(&s,&token,"beside_profile",&json!({})).unwrap();
        assert!(!hidden.to_string().contains("manuscript"));
        for uri in ["sumi://scroll/writing","beside://memory/writing"]{
            let read=rpc(&s,&token,&json!({"id":1,"method":"resources/read","params":{"uri":uri}})).unwrap();
            assert!(read.to_string().contains("terse Rust"));assert!(!read.to_string().contains("manuscript"));
        }
        // Public memory note vs private memory note
        s.put(
            "note",
            &json!({"id": "pub1", "topic": "code", "text": "Uses cargo clippy before pushing PRs.", "validFrom": now(), "private": false}),
        )
        .unwrap();
        s.put(
            "note",
            &json!({"id": "priv1", "topic": "health", "text": "Private medical note.", "validFrom": now(), "private": true}),
        )
        .unwrap();

        let recalled = call(
            &s,
            &token,
            "beside_recall",
            &json!({"query": "Rust code reviews cargo clippy"}),
        )
        .unwrap();
        let dump = recalled["items"].to_string();
        assert!(dump.contains("terse Rust code reviews"));
        assert!(!dump.contains("cargo clippy"));
        let mut permitted=s.doc("agent","agent-1").unwrap(); permitted["shareMemories"]=json!(true); policy(&s,&permitted).unwrap();
        let permitted=call(&s,&token,"beside_recall",&json!({"query":"cargo clippy"})).unwrap().to_string();
        assert!(permitted.contains("cargo clippy")); assert!(!permitted.contains("Private medical note"));
        assert!(!dump.contains("Secret career plan"));
        assert!(!dump.contains("Private medical note"));

        let profile = call(&s, &token, "beside_profile", &json!({})).unwrap();
        assert!(profile["sections"]["writing"].is_array());
        assert!(profile["sections"].get("goals").is_none());
        assert!(!profile.to_string().contains("Private medical note"));
        let audit=s.list("log").unwrap();
        assert!(audit.iter().all(|r|!r["shared"].to_string().contains("Uses cargo clippy")));
        let before=call(&s,&token,"beside_profile",&json!({})).unwrap();
        assert!(before["memories"].as_array().unwrap().iter().any(|m|m["id"]=="pub1"));
        let mut policy_record=s.doc("agent","agent-1").unwrap(); policy_record["shareMemories"]=json!(false);
        policy(&s,&policy_record).unwrap();
        let read=rpc(&s,&token,&json!({"id":1,"method":"resources/read","params":{"uri":"beside://memory/profile"}})).unwrap();
        assert!(!read.to_string().contains("Uses cargo clippy"));
    }

    #[test]
    fn generalized_beside_answer_requires_shadow_calibration_before_autonomous_reply() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Store::with_key(tmp.path().join("book"), &[42; 32]).unwrap();
        crate::app::setup(&s).unwrap();
        let token = setup_agent(&s, "Answer for me", vec!["writing"]);

        let q = "Which commit message style do you prefer?";
        let opts = json!(["Conventional", "Plain"]);

        // Initially queues because 0 verified shadow trials exist.
        let first = call(&s, &token, "beside_answer", &json!({"question": q, "options": opts})).unwrap();
        assert_eq!(first["queued"], true);

        // Seed 20 verified agreeing shadow trials for this exact closed question.
        for i in 0..20 {
            s.put(
                "log",
                &json!({
                    "id": format!("trial-{i}"),
                    "policyScope": hash(&json!([["writing"],false,"Answer for me"]).to_string()),
                    "agent": "agent-1",
                    "name": "Claude Code",
                    "date": now(),
                    "tool": "beside_answer",
                    "question": q,
                    "options": opts,
                    "shadow": "Conventional",
                    "human": "Conventional",
                    "status": "reviewed"
                }),
            )
            .unwrap();
        }

        let auto = call(&s, &token, "beside_answer", &json!({"question": q, "options": opts})).unwrap();
        assert_eq!(auto["answer"], "Conventional");
        assert!(auto["confidence"].as_f64().unwrap() >= 0.90);

        // High-stakes / destructive questions ALWAYS escalate even with trials.
        let dangerous = call(
            &s,
            &token,
            "beside_answer",
            &json!({"question": "Approve wire transfer?", "options": ["Yes", "No"]}),
        )
        .unwrap();
        assert_eq!(dangerous["queued"], true);
        assert!(!is_low_stakes_generalized_preference(q, &[json!("Plain"),json!("Transfer the money")]));
    }
}
