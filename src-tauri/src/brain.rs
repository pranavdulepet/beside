use crate::{activity, bundled, models, plan, providers, store::*};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read};

pub const CLOUD_PROVIDERS: [&str; 6] = [
    "ChatGPT",
    "Anthropic",
    "Gemini",
    "OpenAI",
    "OpenRouter",
    "Compatible",
];

pub struct BrainOpts<'a> {
    pub max_tokens: u32,
    pub direct_reply: bool,
    pub image_base64: Option<&'a str>,
    pub job: &'a str,
    pub allow_cloud: bool,
}

impl Default for BrainOpts<'_> {
    fn default() -> Self {
        Self {
            max_tokens: 320,
            direct_reply: false,
            image_base64: None,
            job: "assistant",
            allow_cloud: true,
        }
    }
}

fn raw_settings(s: &Store) -> Res<Value> {
    let v = s.get("brainSettings")?;
    if v.is_object() {
        Ok(v)
    } else {
        Ok(json!({
            "mode": "local",
            "provider": "",
            "cloudConsent": false
        }))
    }
}

fn available_cloud(s: &Store) -> Res<Vec<Value>> {
    let mut out = Vec::new();
    for p in s.list("provider")? {
        let Some(id) = p["id"].as_str() else { continue };
        if !CLOUD_PROVIDERS.contains(&id) {
            continue;
        }
        let connected = if id == "ChatGPT" {
            p["primary"] == true || p["standalone"] == true || providers::credential(s, id).is_ok()
        } else {
            providers::credential(s, id).is_ok()
        };
        if !connected {
            continue;
        }
        out.push(json!({
            "id": id,
            "model": p["model"].as_str().unwrap_or(""),
            "label": provider_label(id),
            "enabled": p["enabled"] == true || id == "ChatGPT"
        }));
    }
    Ok(out)
}

pub fn provider_label(id: &str) -> &'static str {
    match id {
        "ChatGPT" => "ChatGPT",
        "Anthropic" => "Claude",
        "Gemini" => "Gemini",
        "OpenAI" => "OpenAI",
        "OpenRouter" => "OpenRouter",
        "Compatible" => "Custom Endpoint",
        _ => "On-device",
    }
}

pub fn status(s: &Store) -> Res<Value> {
    let cfg = raw_settings(s)?;
    let local = activity::private_model(s);
    let cloud_list = available_cloud(s)?;
    let consent = cfg["cloudConsent"] == true;
    let wanted_mode = cfg["mode"].as_str().unwrap_or("local");
    let wanted_provider = cfg["provider"].as_str().unwrap_or("");
    let active_cloud = if wanted_mode == "cloud" && consent {
        cloud_list
            .iter()
            .find(|p| p["id"] == wanted_provider)
    } else {
        None
    };

    if let Some(cloud) = active_cloud {
        let pid = cloud["id"].as_str().unwrap_or("Cloud");
        let model = cloud["model"].as_str().unwrap_or(pid);
        Ok(json!({
            "mode": "cloud",
            "provider": pid,
            "providerLabel": provider_label(pid),
            "model": model,
            "runtime": pid,
            "cloudConsent": true,
            "contextShared": true,
            "localModel": local,
            "availableCloud": cloud_list
        }))
    } else {
        let model_name = local
            .as_ref()
            .and_then(|m| m["name"].as_str())
            .unwrap_or("");
        let runtime = local
            .as_ref()
            .and_then(|m| m["runtime"].as_str())
            .unwrap_or("Local");
        Ok(json!({
            "mode": "local",
            "provider": "local",
            "providerLabel": "On-device",
            "model": model_name,
            "runtime": runtime,
            "cloudConsent": consent,
            "contextShared": false,
            "localModel": local,
            "availableCloud": cloud_list
        }))
    }
}

pub fn configure(s: &Store, a: &Value) -> Res<Value> {
    let mut cfg = raw_settings(s)?;
    if let Some(consent) = a["cloudConsent"].as_bool() {
        cfg["cloudConsent"] = json!(consent);
        if !consent {
            cfg["mode"] = json!("local");
        }
    }
    if let Some(provider) = a["provider"].as_str() {
        if !provider.is_empty() && !CLOUD_PROVIDERS.contains(&provider) {
            return Err("Unknown cloud provider.".into());
        }
        cfg["provider"] = json!(provider);
    }
    if let Some(mode) = a["mode"].as_str() {
        if !["local", "cloud"].contains(&mode) {
            return Err("Choose either local or cloud intelligence.".into());
        }
        if mode == "cloud" {
            if cfg["cloudConsent"] != true {
                return Err("Confirm that screen and memory context can be sent to your chosen cloud provider.".into());
            }
            let list = available_cloud(s)?;
            let target = cfg["provider"].as_str().unwrap_or("");
            let chosen = list
                .iter()
                .find(|p| p["id"] == target)
                .ok_or("Connect ChatGPT, Claude, or Gemini first.")?;
            cfg["provider"] = chosen["id"].clone();
        }
        cfg["mode"] = json!(mode);
    }
    s.set("brainSettings", &cfg)?;
    status(s)
}

// Apply sharing policy at the last boundary, even if the provider was changed
// while retrieval was running. Local conversation history can contain a private reply.
fn cloud_input(s: &Store, input: &Value, provider: &str) -> Res<Value> {
    let mut shared = input.clone();
    if let Some(fields) = shared.as_object_mut() {
        if fields.contains_key("personal_profile") || fields.contains_key("remembered_notes") {
            let (profile, notes) = activity::personal_memory_bundle(s, true)?;
            fields.insert("personal_profile".into(), json!(profile));
            fields.insert("remembered_notes".into(), json!(notes));
        }
        for field in ["history", "recent_turns"] {
            if let Some(turns) = fields.get_mut(field).and_then(Value::as_array_mut) {
                turns.retain(|turn| turn["sharedWith"] == provider);
            }
        }
        fields.remove("open_windows_on_this_mac");
        fields.remove("personal_notes");
        fields.remove("previous_question");
        if let Some(notes) = fields.get_mut("notes").and_then(Value::as_array_mut) {
            notes.retain(|note| note["user_corrected"] != true);
        }
    }
    Ok(shared)
}

pub fn infer(
    s: &Store,
    system: &str,
    input: &Value,
    opts: &BrainOpts<'_>,
    sink: &mut dyn FnMut(&str),
) -> Res<(String, Value)> {
    if let Some(active) = opts.allow_cloud.then(|| status(s)).transpose()?.filter(|b| b["mode"] == "cloud") {
        let provider_id = active["provider"].as_str().unwrap_or("");
        let answer = infer_cloud(s, provider_id, system, &cloud_input(s, input, provider_id)?.to_string(), opts, sink)?;
        return Ok((answer, active));
    }
    let mut model = activity::private_model(s)
        .ok_or("Set up a local model (like Inkling 2 Mini) or connect a cloud account in Settings.")?;
    if opts.direct_reply {
        model["directReply"] = json!(true);
    }
    let answer = infer_local(s, &model, system, input, opts, sink)?;
    let meta = json!({
        "mode": "local",
        "provider": "local",
        "providerLabel": "On-device",
        "model": model["name"].as_str().unwrap_or("Local model"),
        "runtime": model["runtime"].as_str().unwrap_or("Local"),
        "contextShared": false
    });
    Ok((answer, meta))
}

fn infer_local(
    s: &Store,
    model: &Value,
    system: &str,
    input: &Value,
    opts: &BrainOpts<'_>,
    sink: &mut dyn FnMut(&str),
) -> Res<String> {
    if let Some(image) = opts.image_base64 {
        if model["runtime"] == "Sumi" {
            if let Some(projector) = crate::weights::projector(s, model) {
                let mut cfg = model.clone();
                cfg["projector"] = json!(projector);
                let messages = json!([
                    {"role": "system", "content": system},
                    {"role": "user", "content": [
                        {"type": "text", "text": input.to_string()},
                        {"type": "image_url", "image_url": {"url": format!("data:image/jpeg;base64,{image}")}}
                    ]}
                ]);
                return bundled::infer(&cfg, messages, opts.max_tokens.max(240), None, Some(sink));
            }
        } else if model["runtime"] == "Ollama" && models::vision(model) {
            return models::photo_reply(
                model,
                system,
                input,
                opts.max_tokens.max(240),
                &[json!(image)],
                Some(sink),
            );
        }
    }
    if bundled::is_inkling(model) {
        return bundled::inkling_text(
            model,
            system,
            input,
            opts.max_tokens,
            false,
            None,
            sink,
        );
    }
    models::infer_reply(
        model,
        system,
        &input.to_string(),
        opts.max_tokens,
        None,
        Some(sink),
    )
}

fn infer_cloud(
    s: &Store,
    provider_id: &str,
    system: &str,
    input: &str,
    opts: &BrainOpts<'_>,
    sink: &mut dyn FnMut(&str),
) -> Res<String> {
    let p = s.doc("provider", provider_id)?;
    let model = p["model"].as_str().unwrap_or("");
    let limit = opts.max_tokens.max(256);
    let photos: Vec<Value> = opts
        .image_base64
        .map(|b| vec![json!(b)])
        .unwrap_or_default();

    let answer = match provider_id {
        "ChatGPT" => {
            let token = providers::access_token(s)?;
            let name = if model.is_empty() { "gpt-4.1-mini" } else { model };
            plan::complete_with(
                plan::API,
                &token,
                name,
                system,
                input,
                limit,
                None,
                Some(sink),
                &photos,
            )?
        }
        "OpenAI" => {
            let cred = providers::credential(s, provider_id)?;
            let key = cred["key"].as_str().ok_or("Reconnect OpenAI.")?;
            plan::complete_with(
                plan::API,
                key,
                model,
                system,
                input,
                limit,
                None,
                Some(sink),
                &photos,
            )?
        }
        "Anthropic" => {
            let cred = providers::credential(s, provider_id)?;
            let key = cred["key"].as_str().ok_or("Reconnect Claude.")?;
            let mut content = vec![json!({"type": "text", "text": input})];
            if let Some(img) = opts.image_base64 {
                content.push(json!({
                    "type": "image",
                    "source": {"type": "base64", "media_type": "image/jpeg", "data": img}
                }));
            }
            let resp = providers::http()
                .post("https://api.anthropic.com/v1/messages")
                .set("x-api-key", key)
                .set("anthropic-version", "2023-06-01")
                .send_json(json!({
                    "model": model,
                    "max_tokens": limit,
                    "stream": true,
                    "system": system,
                    "messages": [{"role": "user", "content": content}]
                }))
                .map_err(|e| format!("Claude request failed: {e}"))?;
            consume_anthropic_sse(resp.into_reader(), sink)?
        }
        "Gemini" => {
            let cred = providers::credential(s, provider_id)?;
            let key = cred["key"].as_str().ok_or("Reconnect Gemini.")?;
            if !model
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c))
            {
                return Err("Invalid Gemini model identifier.".into());
            }
            let mut parts = vec![json!({"text": input})];
            if let Some(img) = opts.image_base64 {
                parts.push(json!({"inlineData": {"mimeType": "image/jpeg", "data": img}}));
            }
            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{model}:streamGenerateContent?alt=sse"
            );
            let resp = providers::http()
                .post(&url)
                .set("x-goog-api-key", key)
                .send_json(json!({
                    "systemInstruction": {"parts": [{"text": system}]},
                    "contents": [{"parts": parts}],
                    "generationConfig": {"maxOutputTokens": limit}
                }))
                .map_err(|e| format!("Gemini request failed: {e}"))?;
            consume_gemini_sse(resp.into_reader(), sink)?
        }
        "OpenRouter" | "Compatible" => {
            let cred = providers::credential(s, provider_id)?;
            let key = cred["key"].as_str().ok_or("Reconnect provider.")?;
            let base = if provider_id == "OpenRouter" {
                "https://openrouter.ai/api/v1"
            } else {
                p["endpoint"].as_str().ok_or("Missing endpoint.")?
            };
            let user_content = if let Some(img) = opts.image_base64 {
                json!([
                    {"type": "text", "text": input},
                    {"type": "image_url", "image_url": {"url": format!("data:image/jpeg;base64,{img}")}}
                ])
            } else {
                json!(input)
            };
            let resp = providers::http()
                .post(&format!("{}/chat/completions", base.trim_end_matches('/')))
                .set("Authorization", &format!("Bearer {key}"))
                .send_json(json!({
                    "model": model,
                    "stream": true,
                    "max_tokens": limit,
                    "messages": [
                        {"role": "system", "content": system},
                        {"role": "user", "content": user_content}
                    ]
                }))
                .map_err(|e| format!("Provider request failed: {e}"))?;
            models::consume_chat(resp.into_reader(), false, limit, sink)?
        }
        _ => return Err("Unsupported cloud provider.".into()),
    };

    let _ = s.put(
        "route",
        &json!({
            "id": id(),
            "date": now(),
            "job": opts.job,
            "provider": provider_id,
            "model": model,
            "items": 1
        }),
    );
    Ok(answer)
}

fn consume_anthropic_sse(reader: impl Read, sink: &mut dyn FnMut(&str)) -> Res<String> {
    let mut answer = String::new();
    for line in BufReader::new(reader.take(2 * 1024 * 1024)).lines() {
        let line = line.msg()?;
        let Some(raw) = line.strip_prefix("data:") else {
            continue;
        };
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(raw) else {
            continue;
        };
        if event["type"] == "content_block_delta" {
            if let Some(delta) = event.pointer("/delta/text").and_then(Value::as_str) {
                if !delta.is_empty() {
                    answer.push_str(delta);
                    sink(delta);
                }
            }
        } else if event["type"] == "error" {
            let msg = event
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("Claude stopped before finishing.");
            return Err(msg.into());
        }
    }
    if answer.trim().is_empty() {
        Err("Claude returned an empty reply.".into())
    } else {
        Ok(answer.trim().to_owned())
    }
}

fn consume_gemini_sse(reader: impl Read, sink: &mut dyn FnMut(&str)) -> Res<String> {
    let mut answer = String::new();
    for line in BufReader::new(reader.take(2 * 1024 * 1024)).lines() {
        let line = line.msg()?;
        let Some(raw) = line.strip_prefix("data:") else {
            continue;
        };
        let raw = raw.trim();
        if raw.is_empty() || raw == "[DONE]" {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(raw) else {
            continue;
        };
        if let Some(parts) = event
            .pointer("/candidates/0/content/parts")
            .and_then(Value::as_array)
        {
            for part in parts {
                if let Some(delta) = part["text"].as_str() {
                    if !delta.is_empty() {
                        answer.push_str(delta);
                        sink(delta);
                    }
                }
            }
        }
    }
    if answer.trim().is_empty() {
        Err("Gemini returned an empty reply.".into())
    } else {
        Ok(answer.trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_payload_excludes_private_memories_and_cross_provider_history() {
        let temp = tempfile::tempdir().unwrap();
        let s = Store::with_key(temp.path().join("book"), &[6;32]).unwrap();
        crate::app::setup(&s).unwrap();
        for (id, private) in [("public",false),("secret",true)] {
            s.put("note",&json!({"id":id,"topic":id,"text":id,"validFrom": now(),"private":private})).unwrap();
        }
        let input=json!({"question":"What time?","personal_profile":[],"remembered_notes":["secret"],
            "history":[{"sharedWith":"ChatGPT","answer":"prior shared answer"},{"answer":"private answer"}],
            "personal_notes":["secret"]});
        let mut input = input;
        input["open_windows_on_this_mac"] = json!([{"title":"private window","excerpt":"private draft"}]);
        let clean=cloud_input(&s,&input,"ChatGPT").unwrap();
        assert!(clean.get("open_windows_on_this_mac").is_none());
        assert_eq!(clean["remembered_notes"],json!(["public"]));
        assert_eq!(clean["history"].as_array().unwrap().len(),1);
        assert!(!clean.to_string().contains("secret"));
        assert!(!clean.to_string().contains("private answer"));
    }

    #[test]
    fn cloud_mode_requires_explicit_consent_and_defaults_local() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Store::with_key(tmp.path().join("book"), &[3; 32]).unwrap();
        assert_eq!(status(&s).unwrap()["mode"], "local");
        assert_eq!(status(&s).unwrap()["contextShared"], false);
        assert!(configure(&s, &json!({"mode": "cloud"})).is_err());
        assert!(configure(&s, &json!({"cloudConsent": true, "mode": "cloud"})).is_err());
        assert_eq!(status(&s).unwrap()["mode"], "local");
    }

    #[test]
    fn parses_anthropic_and_gemini_sse_streams() {
        let anthropic = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello \"}}\n\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"world\"}}\n\n";
        let mut chunks = Vec::new();
        let out = consume_anthropic_sse(anthropic.as_bytes(), &mut |d| chunks.push(d.to_owned())).unwrap();
        assert_eq!(out, "Hello world");
        assert_eq!(chunks, ["Hello ", "world"]);

        let gemini = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Click \"}]}}]}\n\ndata: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Export\"}]}}]}\n\n";
        chunks.clear();
        let out = consume_gemini_sse(gemini.as_bytes(), &mut |d| chunks.push(d.to_owned())).unwrap();
        assert_eq!(out, "Click Export");
        assert_eq!(chunks, ["Click ", "Export"]);
    }
}
