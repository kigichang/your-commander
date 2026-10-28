//! PlantUML rendering for Quick View. PlantUML is a Java program with no
//! usable JS/WASM build, so diagrams render here in Rust: either through a
//! local plantuml.jar or a PlantUML server (MPE's two modes). Mode "off" is
//! the default — nothing ever leaves the machine silently.

use crate::state::AppState;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::sync::Arc;
use tauri::State;

/// PlantUML's own base64 variant for URL text encoding.
const PLANTUML_ALPHABET: &[u8; 64] =
    b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz-_";

/// Raw-deflate the UML text and encode it with the PlantUML alphabet.
fn encode_for_server(uml: &str) -> Result<String, String> {
    let mut enc =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(uml.as_bytes()).map_err(|e| e.to_string())?;
    let deflated = enc.finish().map_err(|e| e.to_string())?;

    let mut out = String::with_capacity(deflated.len().div_ceil(3) * 4);
    for chunk in deflated.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(PLANTUML_ALPHABET[(n >> 18) as usize & 0x3f] as char);
        out.push(PLANTUML_ALPHABET[(n >> 12) as usize & 0x3f] as char);
        if chunk.len() > 1 {
            out.push(PLANTUML_ALPHABET[(n >> 6) as usize & 0x3f] as char);
        }
        if chunk.len() > 2 {
            out.push(PLANTUML_ALPHABET[n as usize & 0x3f] as char);
        }
    }
    Ok(out)
}

/// Wrap bare diagram text so both .puml files and shorthand fences work.
fn with_start_end(code: &str) -> String {
    if code.contains("@start") {
        code.to_string()
    } else {
        format!("@startuml\n{code}\n@enduml")
    }
}

fn render_via_server(server: &str, uml: &str) -> Result<String, String> {
    let encoded = encode_for_server(uml)?;
    let url = format!("{}/svg/{}", server.trim_end_matches('/'), encoded);
    if url.len() > 8000 {
        return Err("Diagram too large for server rendering — use a local plantuml.jar".into());
    }
    match ureq::get(&url)
        .timeout(std::time::Duration::from_secs(15))
        .call()
    {
        Ok(resp) => {
            let body = resp.into_string().map_err(|e| e.to_string())?;
            if body.contains("<svg") {
                Ok(body)
            } else {
                Err("Unexpected response from PlantUML server".to_string())
            }
        }
        // PlantUML answers syntax errors with 4xx + an error diagram SVG.
        Err(ureq::Error::Status(status, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            if body.contains("<svg") {
                Ok(body)
            } else {
                Err(format!("PlantUML server HTTP {status}"))
            }
        }
        Err(e) => Err(e.to_string()),
    }
}

fn render_via_jar(jar: &str, uml: &str) -> Result<String, String> {
    // A silent probe: invoking bare `java` without a JDK pops macOS's
    // "install Java" dialog, java_home just fails quietly.
    let has_java = std::process::Command::new("/usr/libexec/java_home")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !has_java {
        return Err("Java runtime not found (install a JDK to use plantuml.jar)".into());
    }
    if !std::path::Path::new(jar).exists() {
        return Err(format!("plantuml.jar not found: {jar}"));
    }
    let mut child = std::process::Command::new("java")
        .args([
            "-Djava.awt.headless=true",
            "-jar",
            jar,
            "-pipe",
            "-tsvg",
            "-charset",
            "UTF-8",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("java: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("no stdin")?
        .write_all(uml.as_bytes())
        .map_err(|e| e.to_string())?;
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    let svg = String::from_utf8_lossy(&output.stdout).to_string();
    // PlantUML writes an error *diagram* for bad syntax; only an empty
    // stdout means the run itself failed.
    if svg.contains("<svg") {
        Ok(svg)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[tauri::command]
pub async fn render_plantuml(
    state: State<'_, AppState>,
    code: String,
) -> Result<String, String> {
    let (mode, server, jar) = {
        let config = state.config.lock().unwrap();
        (
            config.plantuml_mode.clone(),
            config.plantuml_server.clone(),
            config.plantuml_jar.clone(),
        )
    };
    if mode != "server" && mode != "jar" {
        return Err("PlantUML rendering is disabled — enable it in Settings".into());
    }

    let key = {
        let mut hasher = std::hash::DefaultHasher::new();
        (&mode, &server, &jar, &code).hash(&mut hasher);
        hasher.finish()
    };
    if let Some(svg) = state.plantuml_cache.lock().unwrap().get(&key) {
        return Ok(svg.as_ref().clone());
    }

    let svg = tauri::async_runtime::spawn_blocking(move || {
        let uml = with_start_end(&code);
        if mode == "jar" {
            render_via_jar(&jar, &uml)
        } else {
            render_via_server(&server, &uml)
        }
    })
    .await
    .map_err(|e| e.to_string())??;

    state
        .plantuml_cache
        .lock()
        .unwrap()
        .insert(key, Arc::new(svg.clone()));
    Ok(svg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// Decode the PlantUML alphabet + inflate — test-only inverse.
    fn decode(encoded: &str) -> Vec<u8> {
        let value = |c: u8| PLANTUML_ALPHABET.iter().position(|&a| a == c).unwrap() as u32;
        let chars: Vec<u8> = encoded.bytes().collect();
        let mut bytes = Vec::new();
        for chunk in chars.chunks(4) {
            let mut n = 0u32;
            for (i, &c) in chunk.iter().enumerate() {
                n |= value(c) << (18 - 6 * i as u32);
            }
            bytes.push((n >> 16) as u8);
            if chunk.len() > 2 {
                bytes.push((n >> 8) as u8);
            }
            if chunk.len() > 3 {
                bytes.push(n as u8);
            }
        }
        let mut out = Vec::new();
        flate2::read::DeflateDecoder::new(&bytes[..])
            .read_to_end(&mut out)
            .unwrap();
        out
    }

    #[test]
    fn server_encoding_roundtrips() {
        let uml = "@startuml\nBob -> Alice : hello\n@enduml";
        let encoded = encode_for_server(uml).unwrap();
        assert!(encoded
            .bytes()
            .all(|c| PLANTUML_ALPHABET.contains(&c)));
        assert_eq!(decode(&encoded), uml.as_bytes());
    }

    #[test]
    fn bare_code_gets_wrapped() {
        assert!(with_start_end("A -> B").starts_with("@startuml"));
        assert_eq!(with_start_end("@startmindmap\nx\n@endmindmap"), "@startmindmap\nx\n@endmindmap");
    }
}
