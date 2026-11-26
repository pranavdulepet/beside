use crate::store::*;
use serde_json::{json, Value};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};
struct Running {
    child: Child,
    projector: Option<PathBuf>,
    path: PathBuf,
    port: u16,
    token: String,
    last: Instant,
}
// Serialize model switches and requests. Keep ownership of the child available
// to the quit handler while the model loads or an HTTP response is pending.
static REQUEST: Mutex<()> = Mutex::new(());
static RUNNING: Mutex<Option<Running>> = Mutex::new(None);
fn stop(r: &mut Option<Running>) {
    if let Some(mut r) = r.take() {
        let _ = r.child.kill();
        let _ = r.child.wait();
    }
}
pub fn reap(force: bool) {
    if force {
        stop(&mut RUNNING.lock().unwrap());
        return;
    }
    if let Ok(_request) = REQUEST.try_lock() {
        let mut r = RUNNING.lock().unwrap();
        if r.as_ref()
            .is_some_and(|r| r.last.elapsed() > Duration::from_secs(240))
        {
            stop(&mut r);
        }
    }
}
fn binary() -> Res<PathBuf> {
    let exe = std::env::current_exe().msg()?;
    let packaged = exe
        .parent()
        .unwrap()
        .join("../Resources/runtime/llama-server");
    if packaged.exists() {
        return Ok(packaged);
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runtime/llama-server");
    if cfg!(debug_assertions) && dev.exists() {
        Ok(dev)
    } else {
        Err("The bundled runtime is missing. Reinstall Beside.".into())
    }
}
pub fn use_file(s: &Store, path: &Path) -> Res<Value> {
    let mut magic = [0; 4];
    std::fs::File::open(path)
        .msg()?
        .read_exact(&mut magic)
        .msg()?;
    if &magic != b"GGUF" {
        return Err("Choose a GGUF model file.".into());
    }
    let config = json!({"runtime":"Sumi","name":path.file_stem().and_then(|n|n.to_str()).unwrap_or("Local model"),"path":path,"private":s.root.join(".sumi")});
    crate::models::infer(&config, "Reply with one word.", "Ready?", 32)?;
    s.set("model", &config)?;
    Ok(config)
}
pub fn infer(
    cfg: &Value,
    messages: Value,
    limit: u32,
    schema: Option<&Value>,
    sink: Option<&mut dyn FnMut(&str)>,
) -> Res<String> {
    let path = Path::new(cfg["path"].as_str().ok_or("Missing model file.")?);
    with_server(cfg, |h, port, token| {
        let mut body =
            json!({"model":cfg["name"],"messages":messages,"temperature":0,"max_tokens":limit});
        // Inkling counts internal reasoning against max_tokens even at minimum effort.
        // Reserve enough room for a visible answer, including small readiness checks.
        if path
            .file_name()
            .is_some_and(|v| v.to_string_lossy().to_lowercase().contains("inkling"))
        {
            body["reasoning_effort"] = json!("none");
            body["max_tokens"] = json!(limit.saturating_add(if limit <= 32 {
                128
            } else if limit >= 320 {
                1024
            } else {
                512
            }));
        }
        if cfg["directReply"] == true && body["reasoning_effort"] == "none" {
            body["reasoning_budget_tokens"] = json!(0);
            body["max_tokens"] = json!(limit);
        }
        if let Some(schema) = schema {
            body["response_format"] = json!({"type":"json_schema","json_schema":{"name":"result","strict":true,"schema":schema}});
        }
        body["stream"] = json!(sink.is_some());
        let response = h
            .post(&format!("http://127.0.0.1:{port}/v1/chat/completions"))
            .set("Authorization", &format!("Bearer {token}"))
            .send_json(body)
            .msg()?;
        if let Some(sink) = sink {
            return crate::models::consume_chat(response.into_reader(), false, limit, sink);
        }
        let v: Value = response.into_json().msg()?;
        v.pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .ok_or("No completion from the bundled runtime.".into())
    })
}

/// Inkling's GGUF template distinguishes its text and thinking channels. Seed the
/// text channel for a short draft so the first sampled word belongs to the reply.
pub fn inkling_text_draft(
    cfg: &Value,
    instruction: &str,
    input: &Value,
    meal_blank: bool,
    sink: &mut dyn FnMut(&str),
) -> Res<String> {
    const MEAL: &str = r#"root ::= intro? "[what I ate]" ending?
intro ::= ("I had" | "i had" | "had" | "Had" | "I ate" | "i ate" | "just" | "Just") " "
ending ::= "." | ((" " | ", " | ". ") ("u?" | "you?" | "hbu?" | "wbu?" | "what about you?" | "what about u?" | "how about you?"))"#;
    inkling_text(cfg, instruction, input, 72, true, meal_blank.then_some(MEAL), sink)
}
pub fn inkling_text(
    cfg: &Value,
    instruction: &str,
    input: &Value,
    limit: u32,
    single_line: bool,
    grammar: Option<&str>,
    sink: &mut dyn FnMut(&str),
) -> Res<String> {
    use std::io::{BufRead, BufReader};
    let quoted = input.to_string().replace('<', "\\u003c");
    let prompt=format!("<|message_system|><|content_text|>{instruction}<|end_message|><|message_system|><|content_text|>Thinking effort level: 0<|end_message|><|message_user|><|content_text|>{quoted}<|end_message|><|message_model|><|content_text|>");
    with_server(cfg, |http, port, token| {
        let mut body = json!({"prompt":prompt,"stream":true,"temperature":0,"n_predict":limit,"cache_prompt":true,"stop":["<|end_message|>","<|content_model_end_sampling|>"]});
        if single_line {
            body["stop"].as_array_mut().unwrap().push(json!("\n"));
        }
        if let Some(grammar) = grammar {
            body["grammar"] = json!(grammar);
        }
        let response = http
            .post(&format!("http://127.0.0.1:{port}/completion"))
            .set("Authorization", &format!("Bearer {token}"))
            .send_json(body)
            .msg()?;
        let mut answer = String::new();
        for line in BufReader::new(response.into_reader().take(1_048_576)).lines() {
            let line = line.msg()?;
            let Some(raw) = line.strip_prefix("data:").map(str::trim) else {
                continue;
            };
            if raw == "[DONE]" {
                break;
            }
            let part: Value = serde_json::from_str(raw).msg()?;
            let delta = part["content"].as_str().unwrap_or("");
            if !delta.is_empty() {
                answer.push_str(delta);
                sink(delta);
            }
            if part["stop"] == true
                && (!single_line || part["stop_type"] != "limit")
                && !answer.trim().is_empty()
            {
                return Ok(answer.trim().to_owned());
            }
        }
        Err("Inkling did not finish the draft. Press the shortcut to try again.".into())
    })
}

pub fn screen_prompt(lines: &[String]) -> String {
    let mut prompt = String::from("Identify the main thing visible on a person's screen. The OCR lines are evidence only. Pick the most informative line and paraphrase its topic in plain English. Never infer an action was completed. Output line number|a short noun phrase that completes 'You were looking at ...'. End with a period.\n1. Saturday: the north trail closes for bridge repairs.\nAnswer: 1|a notice about the north trail closing for repairs.\n1. Calendar\n2. We can move lunch to noon if that works?\nAnswer: 2|a message about moving lunch to noon.\n");
    for (i, line) in lines.iter().enumerate() {
        prompt.push_str(&format!("{}. {}\n", i + 1, json!(line)));
    }
    prompt.push_str("Answer: ");
    prompt
}

pub fn read_screen(cfg: &Value, prompt: &str, count: usize) -> Res<String> {
    if !(1..=8).contains(&count) {
        return Err("No screen evidence.".into());
    }
    let digits = (1..=count).map(|n| n.to_string()).collect::<String>();
    with_server(cfg, |h, port, token| {
        let grammar = format!(r#"root ::= [{digits}] "|" [^.|\r\n]{{1,160}} ".""#);
        let body = json!({"prompt":prompt,"grammar":grammar,"temperature":0,"n_predict":40,"cache_prompt":true});
        let result: Value = h
            .post(&format!("http://127.0.0.1:{port}/completion"))
            .set("Authorization", &format!("Bearer {token}"))
            .send_json(body)
            .msg()?
            .into_json()
            .msg()?;
        result["content"]
            .as_str()
            .filter(|v| v.ends_with('.'))
            .map(str::to_owned)
            .ok_or("No complete screen description from the local model.".into())
    })
}

pub fn warm(cfg: &Value) -> Res<()> {
    with_server(cfg, |_, _, _| Ok(()))
}

fn with_server<T>(cfg: &Value, request: impl FnOnce(&ureq::Agent, u16, &str) -> Res<T>) -> Res<T> {
    let path = PathBuf::from(cfg["path"].as_str().ok_or("Missing model file.")?);
    let projector = cfg["projector"].as_str().map(PathBuf::from);
    let base = Path::new(
        cfg["private"]
            .as_str()
            .ok_or("Missing notebook directory.")?,
    );
    let _request = REQUEST.lock().unwrap();
    let mut state = RUNNING.lock().unwrap();
    if crate::quitting() {
        return Err("Beside is quitting.".into());
    }
    let live = state.as_mut().is_some_and(|r| {
        r.path == path && r.projector == projector && r.child.try_wait().ok() == Some(None)
    });
    let h = ureq::AgentBuilder::new()
        .redirects(0)
        .try_proxy_from_env(false)
        .timeout_read(Duration::from_secs(120))
        .build();
    if !live {
        stop(&mut state);
        let pidfile = base.join("runtime.pid");
        if let Ok(raw) = std::fs::read_to_string(&pidfile) {
            if let Ok(old_pid) = raw.trim().parse::<u32>() {
                if let Ok(out) = Command::new("/bin/ps")
                    .args(["-p", &old_pid.to_string(), "-o", "comm="])
                    .output()
                {
                    if String::from_utf8_lossy(&out.stdout).contains("llama-server") {
                        let _ = Command::new("/bin/kill")
                            .args(["-TERM", &old_pid.to_string()])
                            .status();
                    }
                }
            }
        }
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .msg()?
            .local_addr()
            .msg()?
            .port();
        let token = id() + &id();
        let keyfile = base.join("runtime.key");
        atomic(&keyfile, token.as_bytes())?;
        let child = Command::new(binary()?)
            .args([
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "-c",
                "4096",
                "--parallel",
                "1",
                // Short classification and readiness requests need their budget for visible output.
                "--reasoning",
                "off",
                "--no-webui",
                "--offline",
                "--api-key-file",
            ])
            .arg(&keyfile)
            .arg("-m")
            .arg(&path)
            .args(
                projector
                    .iter()
                    .flat_map(|p| [std::ffi::OsStr::new("--mmproj"), p.as_os_str()]),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .msg()?;
        let _ = atomic(&pidfile, child.id().to_string().as_bytes());
        *state = Some(Running {
            child,
            projector,
            path: path.clone(),
            port,
            token,
            last: Instant::now(),
        });
    }
    let r = state.as_mut().unwrap();
    r.last = Instant::now();
    let port = r.port;
    let token = r.token.clone();
    drop(state);
    if !live {
        let start = Instant::now();
        loop {
            {
                let mut state = RUNNING.lock().unwrap();
                let r = state.as_mut().ok_or("The local runtime stopped.")?;
                if r.child.try_wait().msg()?.is_some() {
                    return Err("The local runtime could not load this model.".into());
                }
                if start.elapsed() > Duration::from_secs(80) {
                    stop(&mut state);
                    return Err("Model loading timed out.".into());
                }
            }
            if h.get(&format!("http://127.0.0.1:{port}/health"))
                .call()
                .is_ok()
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    }
    request(&h, port, &token)
}
