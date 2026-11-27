use crate::store::*;
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn executable() -> Res<PathBuf> {
    let bin = std::env::current_exe().msg()?;
    let packaged = bin
        .parent()
        .and_then(|p| p.parent())
        .ok_or("Missing app resources.")?
        .join("Resources/runtime/beside-confidential");
    if packaged.is_file() {
        return Ok(packaged);
    }
    #[cfg(debug_assertions)]
    {
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runtime/beside-confidential");
        if dev.is_file() {
            return Ok(dev);
        }
    }
    Err("The confidential transport is missing. Reinstall Beside.".into())
}

fn frame(reader: &mut impl BufRead) -> Res<Value> {
    let mut line = String::new();
    use std::io::Read;
    let count = reader.take(256 * 1024).read_line(&mut line).msg()?;
    if count == 0 || !line.ends_with('\n') {
        return Err(
            "The confidential connection closed before it could be verified or completed.".into(),
        );
    }
    let result: Value = serde_json::from_str(&line).msg()?;
    if result["error"].is_string() {
        return Err("The confidential connection could not be verified or completed.".into());
    }
    Ok(result)
}

pub(super) fn exchange(
    key: &str,
    prompt: Option<Value>,
    before_send: impl FnOnce() -> Res<()>,
    sink: &mut dyn FnMut(&str),
) -> Res<String> {
    let mut process = Process(
        Command::new(executable()?)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .msg()?,
    );
    let mut input = process
        .0
        .stdin
        .take()
        .ok_or("Missing confidential connection.")?;
    let mut reader = BufReader::new(
        process
            .0
            .stdout
            .take()
            .ok_or("Missing confidential response.")?,
    );
    let answer = protocol(key, prompt, &mut input, &mut reader, before_send, sink)?;
    if !process.0.wait().msg()?.success() {
        return Err("The confidential connection failed.".into());
    }
    Ok(answer)
}

fn protocol(
    key: &str,
    prompt: Option<Value>,
    input: &mut impl Write,
    mut reader: impl BufRead,
    before_send: impl FnOnce() -> Res<()>,
    sink: &mut dyn FnMut(&str),
) -> Res<String> {
    writeln!(
        input,
        "{}",
        json!({"op":if prompt.is_some(){"ask"}else{"verify"},"key":key})
    )
    .msg()?;
    if frame(&mut reader)?["verified"] != true {
        return Err("The confidential router did not pass attestation.".into());
    }
    let mut answer = String::new();
    if let Some(body) = prompt {
        before_send()?;
        writeln!(input, "{body}").msg()?;
        loop {
            let record = frame(&mut reader)?;
            if record["done"] == true {
                break;
            }
            let part = record["delta"]
                .as_str()
                .ok_or("Unexpected confidential response.")?;
            if answer.len() + part.len() > 2 * 1024 * 1024 {
                return Err("The response exceeded its size limit.".into());
            }
            answer.push_str(part);
            sink(part);
        }
        if answer.trim().is_empty() {
            return Err("The confidential model returned an empty response.".into());
        }
    }
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Contacts the public attestation service without an API key or inference"]
    fn verify_real_attestation_without_sending_a_notebook_prompt() {
        assert_eq!(
            exchange("", None, || Ok(()), &mut |_| panic!(
                "unexpected model output"
            ))
            .unwrap(),
            ""
        );
    }
    #[test]
    fn an_unverified_helper_never_receives_the_prompt() {
        let secret = json!({"input":"private synthetic notebook passage"});
        for response in [
            "",
            "{}\n",
            "{\"verified\":false}\n",
            "{\"delta\":\"untrusted\"}\n",
        ] {
            let mut wire = Vec::new();
            assert!(protocol(
                "fixture",
                Some(secret.clone()),
                &mut wire,
                response.as_bytes(),
                || panic!("unverified request reserved"),
                &mut |_| panic!("unexpected text")
            )
            .is_err());
            let sent = String::from_utf8(wire).unwrap();
            assert!(!sent.contains("synthetic"));
            assert_eq!(sent.lines().count(), 1);
        }
        let mut wire = Vec::new();
        let mut text = String::new();
        let answer = protocol(
            "fixture",
            Some(secret),
            &mut wire,
            b"{\"verified\":true}\n{\"delta\":\"Hello\"}\n{\"done\":true}\n".as_slice(),
            || Ok(()),
            &mut |part| text.push_str(part),
        )
        .unwrap();
        assert_eq!(answer, text);
        assert_eq!(text, "Hello");
        assert!(String::from_utf8(wire).unwrap().contains("synthetic"));
    }
}
