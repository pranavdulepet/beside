use crate::{activity, bundled, models, plan, providers, store::*};
use serde_json::{json, Value};
use std::cell::Cell;
use std::io::{BufRead, BufReader, Read};
use std::sync::atomic::{AtomicU64, Ordering};

mod confidential;
pub fn verify_confidential() -> Res<Value> {
    confidential::exchange("", None, || Ok(()), &mut |_| {})?;
    Ok(json!({"verified":true}))
}

static TURN_GEN: AtomicU64 = AtomicU64::new(1);
thread_local! {
    static ACTIVE_TURN: Cell<u64> = const { Cell::new(0) };
}

pub struct TurnGuard {
    previous: u64,
}
impl TurnGuard {
    pub fn start() -> Self {
        let next = TURN_GEN.fetch_add(1, Ordering::SeqCst) + 1;
        let previous = ACTIVE_TURN.with(|cell| {
            let prev = cell.get();
            cell.set(next);
            prev
        });
        Self { previous }
    }
}
impl Drop for TurnGuard {
    fn drop(&mut self) {
        ACTIVE_TURN.with(|cell| cell.set(self.previous));
    }
}

pub fn cancel_turns() {
    TURN_GEN.fetch_add(1, Ordering::SeqCst);
}

pub fn turn_cancelled() -> bool {
    ACTIVE_TURN.with(|cell| {
        let gen = cell.get();
        gen != 0 && TURN_GEN.load(Ordering::SeqCst) != gen
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatTurn {
    pub user: String,
    pub assistant: String,
}

#[derive(Clone, Debug)]
pub struct NativeToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub param_name: &'static str,
    pub param_description: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Clone, Debug)]
pub struct NativeToolTurn {
    pub assistant_text: String,
    pub calls: Vec<NativeToolCall>,
    pub results: Vec<(String, String, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InferOutcome {
    Text(String),
    ToolCalls {
        text: String,
        calls: Vec<NativeToolCall>,
    },
}

/// True when the active brain is a frontier cloud provider that supports native structured
/// tool calling (Anthropic, OpenAI, ChatGPT, Gemini, OpenCode, OpenRouter, Compatible).
pub(crate) fn supports_native_tools(s: &Store) -> bool {
    status(s).is_ok_and(|st| {
        st["mode"] == "cloud"
            && st["maskCloudText"] != true
            && matches!(
                st["provider"].as_str(),
                Some("ChatGPT" | "OpenAI" | "Anthropic" | "Gemini" | "OpenCode" | "OpenRouter" | "Compatible")
            )
    })
}

/// Silent, zero-latency tiering inside the user's single active cloud provider.
/// Never switches mid-thread so multi-turn KV prompt caching stays 100% hot.
pub(crate) fn resolve_cloud_model(
    s: &Store,
    provider_id: &str,
    configured: &str,
    turns: &[ChatTurn],
    effort: ThinkingEffort,
) -> String {
    let trimmed = configured.trim();
    let is_auto = trimmed.is_empty() || trimmed.eq_ignore_ascii_case("auto");
    let standalone = turns.is_empty();
    match provider_id {
        "ChatGPT" => {
            let plan_model = s.get("model").unwrap_or_default();
            if standalone && effort == ThinkingEffort::Fast {
                if let Some(fast) = plan_model["fast"].as_str().filter(|v| !v.is_empty()) {
                    return fast.to_owned();
                }
            }
            if is_auto {
                plan_model["name"]
                    .as_str()
                    .filter(|v| !v.is_empty())
                    .unwrap_or("gpt-4.1-mini")
                    .to_owned()
            } else {
                trimmed.to_owned()
            }
        }
        "Anthropic" => {
            if is_auto || trimmed == "claude-sonnet-4-5" {
                if standalone && effort == ThinkingEffort::Fast {
                    "claude-haiku-4-5".into()
                } else {
                    "claude-sonnet-4-5".into()
                }
            } else {
                trimmed.to_owned()
            }
        }
        "Gemini" => {
            if is_auto || trimmed == "gemini-2.5-flash" {
                if standalone && effort == ThinkingEffort::Deep {
                    "gemini-2.5-pro".into()
                } else {
                    "gemini-2.5-flash".into()
                }
            } else {
                trimmed.to_owned()
            }
        }
        "OpenAI" => {
            if is_auto || trimmed == "gpt-4.1-mini" {
                if standalone && effort == ThinkingEffort::Fast {
                    "gpt-4.1-mini".into()
                } else {
                    "gpt-4.1".into()
                }
            } else {
                trimmed.to_owned()
            }
        }
        "OpenCode" => {
            if is_auto {
                "claude-sonnet-4-5".into()
            } else {
                trimmed.to_owned()
            }
        }
        "OpenRouter" => {
            if is_auto {
                "anthropic/claude-sonnet-4.5".into()
            } else {
                trimmed.to_owned()
            }
        }
        _ => trimmed.to_owned(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ThinkingEffort {
    #[default]
    Fast,
    Balanced,
    Deep,
}

pub fn classify_effort(question: &str) -> ThinkingEffort {
    let lower = question.to_lowercase();
    if [
        "think deeply",
        "deep dive",
        "carefully analyze",
        "thoroughly",
        "prove ",
        "formal proof",
        "architecture review",
    ]
    .iter()
    .any(|cue| lower.contains(cue))
    {
        return ThinkingEffort::Deep;
    }
    let complex = question.chars().count() >= 180
        || [
            "debug",
            "refactor",
            "architecture",
            "compare",
            "tradeoff",
            "trade-off",
            "optimize",
            "investigate",
            "root cause",
            "step by step",
            "step-by-step",
            "design ",
            "why does",
            "why is",
            "research",
            "synthesiz",
            "plan ",
            "stack trace",
            "exception",
        ]
        .iter()
        .any(|cue| lower.contains(cue));
    if complex {
        ThinkingEffort::Balanced
    } else {
        ThinkingEffort::Fast
    }
}

/// Extract prior conversation turns from `input` so local and cloud runtimes can send
/// native multi-turn message arrays and preserve KV prefix caching across turns.
pub(crate) fn extract_turns(input: &Value) -> (Vec<ChatTurn>, Value) {
    let mut cleaned = input.clone();
    let mut turns = Vec::new();
    let Some(fields) = cleaned.as_object_mut() else {
        return (turns, cleaned);
    };
    for key in ["history", "recent_turns", "this_conversation"] {
        if let Some(items) = fields.remove(key).and_then(|v| match v {
            Value::Array(arr) => Some(arr),
            _ => None,
        }) {
            for row in items {
                let user = row["user"]
                    .as_str()
                    .or_else(|| row["question"].as_str())
                    .map(str::trim)
                    .unwrap_or("");
                let assistant = row["assistant"]
                    .as_str()
                    .or_else(|| row["answer"].as_str())
                    .map(str::trim)
                    .unwrap_or("");
                if !user.is_empty() && !assistant.is_empty() {
                    turns.push(ChatTurn {
                        user: user.to_owned(),
                        assistant: assistant.to_owned(),
                    });
                }
            }
        }
    }
    (turns, cleaned)
}

pub const CLOUD_PROVIDERS: [&str; 8] = [
    "ChatGPT",
    "Anthropic",
    "Gemini",
    "OpenAI",
    "OpenCode",
    "OpenRouter",
    "Compatible",
    "Tinfoil",
];

#[derive(Clone)]
pub struct BrainOpts<'a> {
    pub max_tokens: u32,
    pub direct_reply: bool,
    pub effort: ThinkingEffort,
    pub image_base64: Option<&'a str>,
    pub requires_image: bool,
    pub job: &'a str,
    pub allow_cloud: bool,
}

impl Default for BrainOpts<'_> {
    fn default() -> Self {
        Self {
            max_tokens: 320,
            direct_reply: false,
            effort: ThinkingEffort::Fast,
            image_base64: None,
            requires_image: false,
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
            "endpoint": p["endpoint"].as_str().unwrap_or(""),
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
        "OpenCode" => "OpenCode Zen",
        "OpenRouter" => "OpenRouter",
        "Compatible" => "Custom Endpoint",
        "Tinfoil" => "Tinfoil",
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
            "maskCloudText":cfg["maskCloudText"] == true,
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
            "maskCloudText":cfg["maskCloudText"] == true,
            "availableCloud": cloud_list
        }))
    }
}

pub fn configure(s: &Store, a: &Value) -> Res<Value> {
    let mut cfg = raw_settings(s)?;
    if let Some(mask) = a["maskCloudText"].as_bool() {
        cfg["maskCloudText"] = json!(mask);
    }
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
            let question = ["request", "question", "message", "user_request"]
                .iter()
                .find_map(|key| fields.get(*key).and_then(Value::as_str))
                .unwrap_or("");
            let (profile, notes) = activity::personal_memory_bundle(s, question, true)?;
            fields.insert("personal_profile".into(), json!(profile));
            fields.insert("remembered_notes".into(), json!(notes));
        }
        for field in ["history", "recent_turns"] {
            if let Some(turns) = fields.get_mut(field).and_then(Value::as_array_mut) {
                turns.retain(|turn| turn["sharedWith"] == provider);
            }
        }
        fields.remove("open_windows_on_this_mac");
        fields.remove("working_context");
        fields.remove("procedural_lessons");
        fields.remove("personal_notes");
        fields.remove("previous_question");
        if let Some(notes) = fields.get_mut("notes").and_then(Value::as_array_mut) {
            notes.retain(|note| note["user_corrected"] != true);
        }
    }
    Ok(shared)
}

struct CloudGrant {
    settings: Value,
    epoch: Value,
    generation: Value,
    source_revision: Value,
}
impl CloudGrant {
    fn capture(s: &Store) -> Res<Self> {
        Ok(Self {
            settings: raw_settings(s)?,
            epoch: s.get("activityEpoch")?,
            generation: activity::config(s)?["generation"].clone(),
            source_revision: s.get("sourceRevision")?,
        })
    }
    fn check(&self, s: &Store, provider: &str) -> Res<()> {
        if self.settings["mode"] != "cloud"
            || self.settings["provider"] != provider
            || self.settings["cloudConsent"] != true
            || raw_settings(s)? != self.settings
            || s.get("activityEpoch")? != self.epoch
            || activity::config(s)?["generation"] != self.generation
            || s.get("sourceRevision")? != self.source_revision
        {
            return Err("Sharing changed while preparing this reply. Ask again.".into());
        }
        Ok(())
    }
}

pub fn infer(
    s: &Store,
    system: &str,
    input: &Value,
    opts: &BrainOpts<'_>,
    sink: &mut dyn FnMut(&str),
) -> Res<(String, Value)> {
    infer_for_window(s, system, input, opts, None, sink)
}

pub(crate) fn infer_for_window(
    s: &Store,
    system: &str,
    input: &Value,
    opts: &BrainOpts<'_>,
    window: Option<&Value>,
    sink: &mut dyn FnMut(&str),
) -> Res<(String, Value)> {
    infer_with_turns(s, system, &[], input, opts, window, sink)
}

pub(crate) fn infer_with_turns(
    s: &Store,
    system: &str,
    extra_turns: &[ChatTurn],
    input: &Value,
    opts: &BrainOpts<'_>,
    window: Option<&Value>,
    sink: &mut dyn FnMut(&str),
) -> Res<(String, Value)> {
    let (outcome, meta) = infer_step_for_window(
        s,
        system,
        extra_turns,
        &[],
        input,
        opts,
        None,
        window,
        sink,
    )?;
    match outcome {
        InferOutcome::Text(text) => Ok((text, meta)),
        InferOutcome::ToolCalls { text, .. } => Ok((text, meta)),
    }
}

pub(crate) fn infer_step_for_window(
    s: &Store,
    system: &str,
    extra_turns: &[ChatTurn],
    tool_turns: &[NativeToolTurn],
    input: &Value,
    opts: &BrainOpts<'_>,
    tools: Option<&[NativeToolSpec]>,
    window: Option<&Value>,
    sink: &mut dyn FnMut(&str),
) -> Res<(InferOutcome, Value)> {
    if turn_cancelled() {
        return Err("Cancelled.".into());
    }
    let verify_window = || {
        if window.is_none_or(|snap| crate::overlay::usable(s, snap)) {
            Ok(())
        } else {
            Err("The window changed. Summon Beside again.".to_string())
        }
    };
    verify_window()?;
    if opts.requires_image && opts.image_base64.is_none() {
        let message = "I don’t have an image of that moment to check.";
        sink(message);
        return Ok((InferOutcome::Text(message.into()), json!({"usedImage": false})));
    }
    let grant = opts
        .allow_cloud
        .then(|| CloudGrant::capture(s))
        .transpose()?;
    let cloud = grant
        .as_ref()
        .filter(|g| g.settings["mode"] == "cloud")
        .map(|_| status(s))
        .transpose()?
        .filter(|b| b["mode"] == "cloud");
    let local_image = opts.requires_image
        && cloud
            .as_ref()
            .is_some_and(|b| b["maskCloudText"] == true || b["provider"] == "Tinfoil");
    if local_image && activity::private_model(s).is_none() {
        let answer = "This image stays on your Mac. Set up a local vision model to read it.";
        sink(answer);
        return Ok((
            InferOutcome::Text(answer.into()),
            json!({"usedImage":false,"contextShared":false}),
        ));
    }
    if let Some(mut active) = cloud.filter(|_| !local_image) {
        let provider_id = active["provider"].as_str().unwrap_or("");
        let grant = grant.as_ref().ok_or("Cloud sharing is off.")?;
        grant.check(s, provider_id)?;
        let shared = cloud_input(s, input, provider_id)?;
        let (mut turns, current) = extract_turns(&shared);
        turns.extend_from_slice(extra_turns);
        let mut selected = opts.clone();
        if provider_id == "Tinfoil" {
            selected.image_base64 = None;
        }
        let mut emitted_any = false;
        let cloud_result = if active["maskCloudText"] == true {
            selected.image_base64 = None;
            let mut bridge = crate::privacy_bridge::Pseudonymizer::new();
            let scrubbed_turns: Vec<ChatTurn> = turns
                .iter()
                .map(|t| ChatTurn {
                    user: bridge.scrub_str(&t.user),
                    assistant: bridge.scrub_str(&t.assistant),
                })
                .collect();
            let payload = bridge.scrub_value(&current).to_string();
            let instructions = bridge.scrub_instructions(system);
            let mut stream = crate::privacy_bridge::StreamRestorer::new(&bridge);
            let res = infer_cloud(
                s,
                grant,
                &verify_window,
                provider_id,
                &instructions,
                &scrubbed_turns,
                &[],
                &payload,
                &selected,
                None,
                &mut |part| {
                    let text = stream.push(part);
                    if !text.is_empty() {
                        emitted_any = true;
                        sink(&text)
                    }
                },
            );
            res.map(|outcome| match outcome {
                InferOutcome::Text(raw) => {
                    let tail = stream.finish();
                    if !tail.is_empty() {
                        emitted_any = true;
                        sink(&tail);
                    }
                    InferOutcome::Text(bridge.restore(&raw))
                }
                InferOutcome::ToolCalls { text, calls } => InferOutcome::ToolCalls {
                    text: bridge.restore(&text),
                    calls,
                },
            })
        } else {
            let payload = current.to_string();
            let first = infer_cloud(
                s,
                grant,
                &verify_window,
                provider_id,
                system,
                &turns,
                tool_turns,
                &payload,
                &selected,
                tools,
                &mut |part| {
                    if !part.is_empty() {
                        emitted_any = true;
                        sink(part);
                    }
                },
            );
            match first {
                Ok(v) => Ok(v),
                Err(err)
                    if !emitted_any
                        && !turn_cancelled()
                        && is_transient_cloud_error(&err) =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    if turn_cancelled() {
                        Err("Cancelled.".into())
                    } else {
                        infer_cloud(
                            s,
                            grant,
                            &verify_window,
                            provider_id,
                            system,
                            &turns,
                            tool_turns,
                            &payload,
                            &selected,
                            tools,
                            &mut |part| {
                                if !part.is_empty() {
                                    emitted_any = true;
                                    sink(part);
                                }
                            },
                        )
                    }
                }
                Err(err) => Err(err),
            }
        };

        match cloud_result {
            Ok(outcome) => {
                active["usedImage"] = json!(selected.image_base64.is_some());
                return Ok((outcome, active));
            }
            Err(err)
                if !emitted_any
                    && !turn_cancelled()
                    && tool_turns.is_empty()
                    && is_transient_cloud_error(&err)
                    && activity::private_model(s).is_some() =>
            {
                // Cloud is offline or temporarily unavailable; gracefully fall back to on-device
                // inference (which is strictly more private) instead of failing the turn.
                let mut model = activity::private_model(s).unwrap();
                if opts.direct_reply {
                    model["directReply"] = json!(true);
                }
                let (mut local_turns, local_current) = extract_turns(input);
                local_turns.extend_from_slice(extra_turns);
                let (answer, used_image) =
                    infer_local(s, &model, system, &local_turns, &local_current, opts, sink)?;
                let meta = json!({
                    "mode": "local",
                    "provider": "local",
                    "providerLabel": "On-device",
                    "model": model["name"].as_str().unwrap_or("Local model"),
                    "runtime": model["runtime"].as_str().unwrap_or("Local"),
                    "usedImage": used_image,
                    "contextShared": false,
                    "fallbackFrom": provider_id
                });
                return Ok((InferOutcome::Text(answer), meta));
            }
            Err(err) => return Err(err),
        }
    }
    let mut model = activity::private_model(s).ok_or(
        "Set up a local model (like Inkling 2 Mini) or connect a cloud account in Settings.",
    )?;
    if opts.direct_reply {
        model["directReply"] = json!(true);
    }
    let (mut turns, current) = extract_turns(input);
    turns.extend_from_slice(extra_turns);
    let (answer, used_image) = infer_local(s, &model, system, &turns, &current, opts, sink)?;
    let meta = json!({
        "mode": "local",
        "provider": "local",
        "providerLabel": "On-device",
        "model": model["name"].as_str().unwrap_or("Local model"),
        "runtime": model["runtime"].as_str().unwrap_or("Local"),
        "usedImage": used_image,
        "contextShared": false
    });
    Ok((InferOutcome::Text(answer), meta))
}

fn is_transient_cloud_error(err: &str) -> bool {
    let lower = err.to_lowercase();
    if lower.contains("cancelled")
        || lower.contains("sign in")
        || lower.contains("401")
        || lower.contains("403")
        || lower.contains("sharing changed")
        || lower.contains("window changed")
        || lower.contains("consent")
    {
        return false;
    }
    lower.contains("502")
        || lower.contains("503")
        || lower.contains("504")
        || lower.contains("529")
        || lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("couldn't reach")
        || lower.contains("connection")
        || lower.contains("dns")
        || lower.contains("unavailable")
}

fn infer_local(
    s: &Store,
    model: &Value,
    system: &str,
    turns: &[ChatTurn],
    input: &Value,
    opts: &BrainOpts<'_>,
    sink: &mut dyn FnMut(&str),
) -> Res<(String, bool)> {
    if let Some(image) = opts.image_base64 {
        let limit = if opts.requires_image { opts.max_tokens.clamp(24, 80) } else { opts.max_tokens.max(240) };
        if model["runtime"] == "Beside" {
            if let Some(projector) = crate::weights::projector(s, model) {
                let mut cfg = model.clone();
                cfg["projector"] = json!(projector);
                let mut messages = vec![json!({"role": "system", "content": system})];
                for turn in turns {
                    messages.push(json!({"role": "user", "content": turn.user}));
                    messages.push(json!({"role": "assistant", "content": turn.assistant}));
                }
                messages.push(json!({
                    "role": "user",
                    "content": [
                        {"type": "text", "text": input.to_string()},
                        {"type": "image_url", "image_url": {"url": format!("data:image/jpeg;base64,{image}")}}
                    ]
                }));
                return bundled::infer(&cfg, json!(messages), limit, None, Some(sink))
                    .map(|answer| (answer, true));
            }
        } else if (model["runtime"] == "Ollama" && models::vision(model))
            || matches!(model["runtime"].as_str(), Some("MLX" | "LM Studio"))
        {
            return models::photo_reply(
                model,
                system,
                input,
                limit,
                &[json!(image)],
                Some(sink),
            ).map(|answer| (answer, true));
        }
    }
    if opts.requires_image {
        let message = if bundled::is_inkling(model) {
            "I need to see the image to check that. In Sources, open Visual screenshots & vision and choose Prepare local vision."
        } else {
            "I need to see the image to check that. The selected model can read text only."
        };
        sink(message);
        return Ok((message.into(), false));
    }
    // Optional images may accompany text requests. Describe the actual evidence
    // to the model if this runtime can only use the extracted text.
    let text_only;
    let system = if opts.image_base64.is_some() {
        text_only = format!("{system}\nYou received only text. Ground the answer in that text. If a detail requires seeing the image, say so.");
        &text_only
    } else { system };
    if bundled::is_inkling(model) {
        return bundled::inkling_chat(
            model,
            system,
            turns,
            input,
            opts.max_tokens,
            opts.effort,
            sink,
        ).map(|answer| (answer, false));
    }
    models::infer_messages(
        model,
        system,
        turns,
        &input.to_string(),
        opts.max_tokens,
        None,
        Some(sink),
    ).map(|answer| (answer, false))
}

fn infer_cloud(
    s: &Store,
    grant: &CloudGrant,
    verify_window: &dyn Fn() -> Res<()>,
    provider_id: &str,
    system: &str,
    turns: &[ChatTurn],
    tool_turns: &[NativeToolTurn],
    input: &str,
    opts: &BrainOpts<'_>,
    tools: Option<&[NativeToolSpec]>,
    sink: &mut dyn FnMut(&str),
) -> Res<InferOutcome> {
    let p = s.doc("provider", provider_id)?;
    let configured_model = p["model"].as_str().unwrap_or("");
    let model = resolve_cloud_model(s, provider_id, configured_model, turns, opts.effort);
    let limit = match opts.effort {
        ThinkingEffort::Deep => opts.max_tokens.max(1024),
        ThinkingEffort::Balanced => opts.max_tokens.max(512),
        ThinkingEffort::Fast => opts.max_tokens.max(256),
    };
    let photos: Vec<Value> = opts
        .image_base64
        .map(|b| vec![json!(b)])
        .unwrap_or_default();

    grant.check(s, provider_id)?;
    verify_window()?;
    let outcome = match provider_id {
        "Tinfoil" => {
            let cred = providers::credential(s, provider_id)?;
            let key = cred["key"].as_str().ok_or("Reconnect Tinfoil.")?;
            let packed = if turns.is_empty() {
                input.to_owned()
            } else {
                json!({"history": turns.iter().map(|t| json!({"user": t.user, "assistant": t.assistant})).collect::<Vec<_>>(), "current": input}).to_string()
            };
            let text = confidential::exchange(
                key,
                Some(json!({"model":model,"system":system,"input":packed,"maxTokens":limit})),
                || {
                    grant.check(s, provider_id)?;
                    verify_window()?;
                    providers::reserve(s, &s.doc("provider", provider_id)?)
                },
                sink,
            )?;
            InferOutcome::Text(text)
        }
        "ChatGPT" => {
            let (api, token) = plan::session(s)?;
            plan::complete_messages_tools(
                &api,
                &token,
                &model,
                system,
                turns,
                tool_turns,
                input,
                limit,
                None,
                tools,
                Some(sink),
                &photos,
            )?
        }
        "OpenAI" => {
            let cred = providers::credential(s, provider_id)?;
            let key = cred["key"].as_str().ok_or("Reconnect OpenAI.")?;
            plan::complete_messages_tools(
                plan::API,
                key,
                &model,
                system,
                turns,
                tool_turns,
                input,
                limit,
                None,
                tools,
                Some(sink),
                &photos,
            )?
        }
        "Anthropic" => {
            let cred = providers::credential(s, provider_id)?;
            let key = cred["key"].as_str().ok_or("Reconnect Claude.")?;
            let mut messages = Vec::with_capacity(turns.len() * 2 + tool_turns.len() * 2 + 1);
            for (idx, turn) in turns.iter().enumerate() {
                messages.push(json!({"role": "user", "content": turn.user}));
                if idx + 1 == turns.len() {
                    // Place an ephemeral prompt cache breakpoint at the end of prior history.
                    messages.push(json!({
                        "role": "assistant",
                        "content": [{"type": "text", "text": turn.assistant, "cache_control": {"type": "ephemeral"}}]
                    }));
                } else {
                    messages.push(json!({"role": "assistant", "content": turn.assistant}));
                }
            }
            let mut content = vec![json!({"type": "text", "text": input})];
            if let Some(img) = opts.image_base64 {
                content.push(json!({
                    "type": "image",
                    "source": {"type": "base64", "media_type": "image/jpeg", "data": img}
                }));
            }
            messages.push(json!({"role": "user", "content": content}));
            for tt in tool_turns {
                let mut assistant_blocks = Vec::new();
                if !tt.assistant_text.is_empty() {
                    assistant_blocks.push(json!({"type": "text", "text": tt.assistant_text}));
                }
                for call in &tt.calls {
                    assistant_blocks.push(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": call.arguments
                    }));
                }
                messages.push(json!({"role": "assistant", "content": assistant_blocks}));
                let result_blocks: Vec<Value> = tt
                    .results
                    .iter()
                    .map(|(id, _name, out)| json!({
                        "type": "tool_result",
                        "tool_use_id": id,
                        "content": out
                    }))
                    .collect();
                messages.push(json!({"role": "user", "content": result_blocks}));
            }
            let mut payload = json!({
                "model": model,
                "max_tokens": limit,
                "stream": true,
                "system": [{"type": "text", "text": system, "cache_control": {"type": "ephemeral"}}],
                "messages": messages
            });
            if let Some(specs) = tools.filter(|s| !s.is_empty()) {
                let mut tool_defs: Vec<Value> = specs
                    .iter()
                    .map(|t| {
                        json!({
                            "name": t.name,
                            "description": t.description,
                            "input_schema": {
                                "type": "object",
                                "properties": {
                                    t.param_name: {"type": "string", "description": t.param_description}
                                },
                                "required": [t.param_name]
                            }
                        })
                    })
                    .collect();
                if let Some(last) = tool_defs.last_mut() {
                    last["cache_control"] = json!({"type": "ephemeral"});
                }
                payload["tools"] = json!(tool_defs);
            }
            let resp = providers::http()
                .post("https://api.anthropic.com/v1/messages")
                .set("x-api-key", key)
                .set("anthropic-version", "2023-06-01")
                .send_json(payload)
                .map_err(|e| format!("Claude request failed: {e}"))?;
            consume_anthropic_sse_tools(resp.into_reader(), sink)?
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
            let mut contents = Vec::with_capacity(turns.len() * 2 + tool_turns.len() * 2 + 1);
            for turn in turns {
                contents.push(json!({"role": "user", "parts": [{"text": turn.user}]}));
                contents.push(json!({"role": "model", "parts": [{"text": turn.assistant}]}));
            }
            let mut parts = vec![json!({"text": input})];
            if let Some(img) = opts.image_base64 {
                parts.push(json!({"inlineData": {"mimeType": "image/jpeg", "data": img}}));
            }
            contents.push(json!({"role": "user", "parts": parts}));
            for tt in tool_turns {
                let mut model_parts = Vec::new();
                if !tt.assistant_text.is_empty() {
                    model_parts.push(json!({"text": tt.assistant_text}));
                }
                for call in &tt.calls {
                    model_parts.push(json!({
                        "functionCall": {"name": call.name, "args": call.arguments}
                    }));
                }
                contents.push(json!({"role": "model", "parts": model_parts}));
                let response_parts: Vec<Value> = tt
                    .results
                    .iter()
                    .map(|(_id, name, out)| json!({
                        "functionResponse": {
                            "name": name,
                            "response": {"result": out}
                        }
                    }))
                    .collect();
                contents.push(json!({"role": "user", "parts": response_parts}));
            }
            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{model}:streamGenerateContent?alt=sse"
            );
            let mut gen_cfg = json!({"maxOutputTokens": limit});
            // On Gemini 2.5 Flash, disable hidden thinking for Fast asks to slash TTFT,
            // and scale thinking budget up for Balanced / Deep tasks.
            if model.contains("gemini-2.5-flash") {
                let budget = match opts.effort {
                    ThinkingEffort::Fast => 0,
                    ThinkingEffort::Balanced => 1024,
                    ThinkingEffort::Deep => 4096,
                };
                gen_cfg["thinkingConfig"] = json!({"thinkingBudget": budget});
            }
            let mut body = json!({
                "systemInstruction": {"parts": [{"text": system}]},
                "contents": contents,
                "generationConfig": gen_cfg
            });
            if let Some(specs) = tools.filter(|s| !s.is_empty()) {
                body["tools"] = json!([{
                    "functionDeclarations": specs.iter().map(|t| json!({
                        "name": t.name,
                        "description": t.description,
                        "parameters": {
                            "type": "OBJECT",
                            "properties": {
                                t.param_name: {"type": "STRING", "description": t.param_description}
                            },
                            "required": [t.param_name]
                        }
                    })).collect::<Vec<_>>()
                }]);
            }
            let resp = providers::http()
                .post(&url)
                .set("x-goog-api-key", key)
                .send_json(body)
                .map_err(|e| format!("Gemini request failed: {e}"))?;
            consume_gemini_sse_tools(resp.into_reader(), sink)?
        }
        "OpenRouter" | "OpenCode" | "Compatible" => {
            let cred = providers::credential(s, provider_id)?;
            let key = cred["key"].as_str().ok_or("Reconnect provider.")?;
            let base = if provider_id == "OpenRouter" {
                "https://openrouter.ai/api/v1"
            } else if provider_id == "OpenCode" {
                "https://opencode.ai/zen/v1"
            } else {
                p["endpoint"].as_str().ok_or("Missing endpoint.")?
            };
            let mut messages = vec![json!({"role": "system", "content": system})];
            for turn in turns {
                messages.push(json!({"role": "user", "content": turn.user}));
                messages.push(json!({"role": "assistant", "content": turn.assistant}));
            }
            let user_content = if let Some(img) = opts.image_base64 {
                json!([
                    {"type": "text", "text": input},
                    {"type": "image_url", "image_url": {"url": format!("data:image/jpeg;base64,{img}")}}
                ])
            } else {
                json!(input)
            };
            messages.push(json!({"role": "user", "content": user_content}));
            for tt in tool_turns {
                messages.push(json!({
                    "role": "assistant",
                    "content": if tt.assistant_text.is_empty() { Value::Null } else { json!(tt.assistant_text) },
                    "tool_calls": tt.calls.iter().map(|c| json!({
                        "id": c.id,
                        "type": "function",
                        "function": {"name": c.name, "arguments": c.arguments.to_string()}
                    })).collect::<Vec<_>>()
                }));
                for (id, _name, out) in &tt.results {
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": id,
                        "content": out
                    }));
                }
            }
            let mut req = providers::http()
                .post(&format!("{}/chat/completions", base.trim_end_matches('/')))
                .set("Authorization", &format!("Bearer {key}"));
            if provider_id == "OpenRouter" {
                req = req
                    .set("HTTP-Referer", "https://pranavdulepet.github.io/beside/")
                    .set("X-Title", "Beside");
            }
            let mut body = json!({
                "model": model,
                "stream": true,
                "max_tokens": limit,
                "messages": messages
            });
            if let Some(specs) = tools.filter(|s| !s.is_empty()) {
                body["tools"] = json!(specs.iter().map(|t| json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": {
                            "type": "object",
                            "properties": {
                                t.param_name: {"type": "string", "description": t.param_description}
                            },
                            "required": [t.param_name]
                        }
                    }
                })).collect::<Vec<_>>());
            }
            let resp = req
                .send_json(body)
                .map_err(|e| format!("Provider request failed: {e}"))?;
            models::consume_chat_tools(resp.into_reader(), false, limit, sink)?
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
    Ok(outcome)
}

#[cfg(test)]
fn consume_anthropic_sse(reader: impl Read, sink: &mut dyn FnMut(&str)) -> Res<String> {
    match consume_anthropic_sse_tools(reader, sink)? {
        InferOutcome::Text(text) => Ok(text),
        InferOutcome::ToolCalls { text, .. } => Ok(text),
    }
}

fn consume_anthropic_sse_tools(reader: impl Read, sink: &mut dyn FnMut(&str)) -> Res<InferOutcome> {
    use std::collections::BTreeMap;
    let mut answer = String::new();
    let mut tools: BTreeMap<usize, (String, String, String)> = BTreeMap::new();
    for line in BufReader::new(reader.take(2 * 1024 * 1024)).lines() {
        if turn_cancelled() {
            return Err("Cancelled.".into());
        }
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
        match event["type"].as_str() {
            Some("content_block_start") => {
                if event.pointer("/content_block/type").and_then(Value::as_str) == Some("tool_use") {
                    let idx = event["index"].as_u64().unwrap_or(0) as usize;
                    let id = event
                        .pointer("/content_block/id")
                        .and_then(Value::as_str)
                        .unwrap_or("toolu_0")
                        .to_owned();
                    let name = event
                        .pointer("/content_block/name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned();
                    tools.insert(idx, (id, name, String::new()));
                }
            }
            Some("content_block_delta") => {
                let delta_type = event.pointer("/delta/type").and_then(Value::as_str);
                if delta_type == Some("input_json_delta") {
                    let idx = event["index"].as_u64().unwrap_or(0) as usize;
                    if let (Some(entry), Some(partial)) = (
                        tools.get_mut(&idx),
                        event.pointer("/delta/partial_json").and_then(Value::as_str),
                    ) {
                        entry.2.push_str(partial);
                    }
                } else if let Some(delta) = event.pointer("/delta/text").and_then(Value::as_str) {
                    if !delta.is_empty() {
                        answer.push_str(delta);
                        sink(delta);
                    }
                }
            }
            Some("error") => {
                let msg = event
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("Claude stopped before finishing.");
                return Err(msg.into());
            }
            _ => {}
        }
    }
    if !tools.is_empty() {
        let calls: Vec<NativeToolCall> = tools
            .into_values()
            .filter(|(_, name, _)| !name.is_empty())
            .map(|(id, name, raw_json)| NativeToolCall {
                id,
                name,
                arguments: serde_json::from_str(&raw_json).unwrap_or_else(|_| json!({})),
            })
            .collect();
        if !calls.is_empty() {
            return Ok(InferOutcome::ToolCalls {
                text: answer.trim().to_owned(),
                calls,
            });
        }
    }
    if answer.trim().is_empty() {
        Err("Claude returned an empty reply.".into())
    } else {
        Ok(InferOutcome::Text(answer.trim().to_owned()))
    }
}

#[cfg(test)]
fn consume_gemini_sse(reader: impl Read, sink: &mut dyn FnMut(&str)) -> Res<String> {
    match consume_gemini_sse_tools(reader, sink)? {
        InferOutcome::Text(text) => Ok(text),
        InferOutcome::ToolCalls { text, .. } => Ok(text),
    }
}

fn consume_gemini_sse_tools(reader: impl Read, sink: &mut dyn FnMut(&str)) -> Res<InferOutcome> {
    let mut answer = String::new();
    let mut calls: Vec<NativeToolCall> = Vec::new();
    for line in BufReader::new(reader.take(2 * 1024 * 1024)).lines() {
        if turn_cancelled() {
            return Err("Cancelled.".into());
        }
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
                if let Some(fc) = part.get("functionCall").filter(|v| v.is_object()) {
                    if let Some(name) = fc["name"].as_str().filter(|s| !s.is_empty()) {
                        calls.push(NativeToolCall {
                            id: format!("gemini_{}", calls.len()),
                            name: name.to_owned(),
                            arguments: fc.get("args").cloned().unwrap_or_else(|| json!({})),
                        });
                    }
                } else if let Some(delta) = part["text"].as_str() {
                    if !delta.is_empty() {
                        answer.push_str(delta);
                        sink(delta);
                    }
                }
            }
        }
    }
    if !calls.is_empty() {
        return Ok(InferOutcome::ToolCalls {
            text: answer.trim().to_owned(),
            calls,
        });
    }
    if answer.trim().is_empty() {
        Err("Gemini returned an empty reply.".into())
    } else {
        Ok(InferOutcome::Text(answer.trim().to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_inflight_cloud_grant_expires_on_provider_mode_or_mask_changes() {
        let temp = tempfile::tempdir().unwrap();
        let s = Store::with_key(temp.path().into(), &[13; 32]).unwrap();
        crate::app::setup(&s).unwrap();
        let cloud = json!({"mode":"cloud", "provider":"Tinfoil", "cloudConsent":true});
        s.set("brainSettings", &cloud).unwrap();
        let grant = CloudGrant::capture(&s).unwrap();
        assert!(grant.check(&s, "Tinfoil").is_ok());
        assert!(grant.check(&s, "OpenAI").is_err());
        for (field, value) in [
            ("mode", json!("local")),
            ("maskCloudText", json!(true)),
            ("provider", json!("OpenAI")),
            ("cloudConsent", json!(false)),
        ] {
            let mut changed = cloud.clone();
            changed[field] = value;
            s.set("brainSettings", &changed).unwrap();
            assert!(grant.check(&s, "Tinfoil").is_err());
        }
    }

    #[test]
    fn a_pixel_question_cannot_be_answered_by_a_text_only_runtime() {
        let temp = tempfile::tempdir().unwrap();
        let s = Store::with_key(temp.path().join("book"), &[3; 32]).unwrap();
        let opts = BrainOpts { requires_image: true, ..Default::default() };
        let (missing, meta) = infer(&s, "", &Value::Null, &opts, &mut |_| {}).unwrap();
        assert!(missing.contains("don’t have an image"));
        assert_eq!(meta["usedImage"], false);

        let model = json!({"runtime":"Beside", "name":"Inkling 2 Mini", "path":temp.path().join("Inkling-2-Mini.gguf")});
        let opts = BrainOpts { image_base64: Some("unused-test-pixels"), ..opts };
        let (answer, used) = infer_local(&s, &model, "", &[], &Value::Null, &opts, &mut |_| {}).unwrap();
        assert!(!used);
        assert!(answer.contains("Prepare local vision"));
    }

    #[test]
    fn cloud_payload_excludes_private_memories_and_cross_provider_history() {
        let temp = tempfile::tempdir().unwrap();
        let s = Store::with_key(temp.path().join("book"), &[6;32]).unwrap();
        crate::app::setup(&s).unwrap();
        for (id, private) in [("public",false),("secret",true)] {
            s.put("note",&json!({"id":id,"topic":id,"text":id,"validFrom": now(),"private":private})).unwrap();
        }
        let input = json!({"question":"What did I say about public access?","personal_profile":[],"remembered_notes":["secret"],
            "history":[{"sharedWith":"ChatGPT","answer":"prior shared answer"},{"answer":"private answer"}],
            "personal_notes":["secret"]});
        let mut input = input;
        input["open_windows_on_this_mac"] =
            json!([{"title":"private window","excerpt":"private draft"}]);
        input["working_context"] = json!(["private task note"]);
        let clean = cloud_input(&s, &input, "ChatGPT").unwrap();
        assert!(clean.get("open_windows_on_this_mac").is_none());
        assert!(clean.get("working_context").is_none());
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

    #[test]
    fn extracts_multi_turn_history_and_formats_inkling_prefix_for_kv_cache() {
        let input = json!({
            "question": "And what about caching?",
            "history": [
                {"user": "How does Inkling work?", "assistant": "It uses a looped transformer."},
                {"question": "How many layers?", "answer": "24 physical, 48 effective."}
            ]
        });
        let (turns, current) = extract_turns(&input);
        assert_eq!(turns.len(), 2);
        assert!(current.get("history").is_none());
        assert_eq!(current["question"], "And what about caching?");
        let prompt = crate::bundled::format_inkling_prompt("System rules.", &turns, &current, ThinkingEffort::Fast);
        assert!(prompt.contains("<|message_user|><|content_text|>How does Inkling work?<|end_message|><|message_model|><|content_text|>It uses a looped transformer.<|end_message|><|content_model_end_sampling|>"));
        assert!(prompt.ends_with("<|message_model|><|content_text|>"));
        let deep = crate::bundled::format_inkling_prompt("System rules.", &[], &current, ThinkingEffort::Deep);
        assert!(deep.contains("Thinking effort level: 0.5"));
    }

    #[test]
    fn classifies_thinking_effort_and_cancels_inflight_turns() {
        assert_eq!(classify_effort("what is this"), ThinkingEffort::Fast);
        assert_eq!(classify_effort("debug this Rust borrow checker error"), ThinkingEffort::Balanced);
        assert_eq!(classify_effort("think deeply about our storage architecture"), ThinkingEffort::Deep);

        let _guard = TurnGuard::start();
        assert!(!turn_cancelled());
        cancel_turns();
        assert!(turn_cancelled());
    }

    #[test]
    fn supports_opencode_and_custom_endpoint_providers() {
        assert!(CLOUD_PROVIDERS.contains(&"OpenCode"));
        assert!(CLOUD_PROVIDERS.contains(&"Compatible"));
        assert_eq!(provider_label("OpenCode"), "OpenCode Zen");
        assert_eq!(provider_label("Compatible"), "Custom Endpoint");
    }

    #[test]
    fn parses_native_tool_calls_for_anthropic_gemini_and_openai_chat() {
        let anthropic = concat!(
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"search_web\",\"input\":{}}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"query\\\":\\\"tauri\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\" 2 docs\\\"}\"}}\n\n"
        );
        let out = consume_anthropic_sse_tools(anthropic.as_bytes(), &mut |_| {}).unwrap();
        assert_eq!(
            out,
            InferOutcome::ToolCalls {
                text: String::new(),
                calls: vec![NativeToolCall {
                    id: "toolu_1".into(),
                    name: "search_web".into(),
                    arguments: json!({"query": "tauri 2 docs"}),
                }]
            }
        );

        let gemini = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"fetch_webpage\",\"args\":{\"url\":\"https://example.com\"}}}]}}]}\n\n";
        let out = consume_gemini_sse_tools(gemini.as_bytes(), &mut |_| {}).unwrap();
        assert_eq!(
            out,
            InferOutcome::ToolCalls {
                text: String::new(),
                calls: vec![NativeToolCall {
                    id: "gemini_0".into(),
                    name: "fetch_webpage".into(),
                    arguments: json!({"url": "https://example.com"}),
                }]
            }
        );

        let openai_chat = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_a\",\"function\":{\"name\":\"search_memory\",\"arguments\":\"{\\\"query\\\":\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"flight\\\"}\"}}]}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let out = models::consume_chat_tools(openai_chat.as_bytes(), false, 128, &mut |_| {}).unwrap();
        assert_eq!(
            out,
            InferOutcome::ToolCalls {
                text: String::new(),
                calls: vec![NativeToolCall {
                    id: "call_a".into(),
                    name: "search_memory".into(),
                    arguments: json!({"query": "flight"}),
                }]
            }
        );
    }

    #[test]
    fn auto_tiers_standalone_turns_and_keeps_multi_turn_threads_sticky_for_kv_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Store::with_key(tmp.path().join("book"), &[9; 32]).unwrap();
        s.set("model", &json!({"runtime":"ChatGPT","name":"gpt-5","fast":"gpt-5-mini"})).unwrap();

        let prior = [ChatTurn { user: "hi".into(), assistant: "hello".into() }];
        // Standalone Fast uses fast tier; Balanced/Deep or multi-turn stays on main model.
        assert_eq!(resolve_cloud_model(&s, "ChatGPT", "auto", &[], ThinkingEffort::Fast), "gpt-5-mini");
        assert_eq!(resolve_cloud_model(&s, "ChatGPT", "auto", &prior, ThinkingEffort::Fast), "gpt-5");
        assert_eq!(resolve_cloud_model(&s, "Anthropic", "auto", &[], ThinkingEffort::Fast), "claude-haiku-4-5");
        assert_eq!(resolve_cloud_model(&s, "Anthropic", "auto", &prior, ThinkingEffort::Fast), "claude-sonnet-4-5");
        assert_eq!(resolve_cloud_model(&s, "Anthropic", "claude-opus-4-5", &[], ThinkingEffort::Fast), "claude-opus-4-5");
        assert_eq!(resolve_cloud_model(&s, "Gemini", "auto", &[], ThinkingEffort::Fast), "gemini-2.5-flash");
        assert_eq!(resolve_cloud_model(&s, "Gemini", "auto", &[], ThinkingEffort::Deep), "gemini-2.5-pro");
        assert_eq!(resolve_cloud_model(&s, "OpenAI", "auto", &[], ThinkingEffort::Fast), "gpt-4.1-mini");
        assert_eq!(resolve_cloud_model(&s, "OpenAI", "auto", &[], ThinkingEffort::Balanced), "gpt-4.1");

        assert!(is_transient_cloud_error("Provider returned HTTP 503."));
        assert!(is_transient_cloud_error("Request timed out"));
        assert!(!is_transient_cloud_error("Cancelled."));
        assert!(!is_transient_cloud_error("Sign in to ChatGPT again."));
        assert!(!is_transient_cloud_error("Provider returned HTTP 401."));
    }
}
