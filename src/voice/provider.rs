//! Bounded, off-render-thread transcription requests. Never log request bodies or credentials.
use super::Settings;
use base64::Engine;
use serde_json::Value;
use std::time::Duration;

pub const OPENAI_ENDPOINT: &str = "https://api.openai.com/v1/audio/transcriptions";

fn endpoint(settings: &Settings) -> &str {
    if settings.custom {
        &settings.endpoint
    } else {
        OPENAI_ENDPOINT
    }
}

pub fn validate(settings: &Settings) -> Result<(), String> {
    let url = endpoint(settings);
    let uri: ureq::http::Uri = url.parse().map_err(|_| "Invalid endpoint URL")?;
    let local = matches!(uri.host(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if uri.scheme_str() != Some("https") && !(uri.scheme_str() == Some("http") && local) {
        return Err("Use HTTPS, or HTTP on localhost for a local provider".into());
    }
    if uri.host().is_none() || url.contains('@') || url.contains('#') {
        return Err("Endpoint must have a host and no credentials or fragment".into());
    }
    if settings.model.trim().is_empty() {
        return Err("Choose a transcription model".into());
    }
    if settings.custom {
        let headers: Value =
            serde_json::from_str(&settings.headers).map_err(|_| "Headers must be a JSON object")?;
        let Some(headers) = headers
            .as_object()
            .filter(|o| o.values().all(Value::is_string))
        else {
            return Err("Headers must be a JSON object of strings".into());
        };
        for (name, val) in headers {
            ureq::http::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| "Invalid header name")?;
            ureq::http::HeaderValue::from_str(val.as_str().unwrap())
                .map_err(|_| "Invalid header value")?;
            if ["host", "content-length", "content-type"]
                .contains(&name.to_ascii_lowercase().as_str())
            {
                return Err(
                    "Host, Content-Length and Content-Type are managed automatically".into(),
                );
            }
        }
        let body: Value =
            serde_json::from_str(&settings.body).map_err(|_| "Body must be valid JSON")?;
        let Some(body) = body.as_object() else {
            return Err("Body must be a JSON object".into());
        };
        if !settings.json_body && !body.values().all(Value::is_string) {
            return Err("Multipart extra fields must be strings".into());
        }
        if !settings.response_pointer.is_empty() && !settings.response_pointer.starts_with('/') {
            return Err("Response JSON pointer must start with / (for example /text)".into());
        }
    }
    Ok(())
}

pub fn wav(samples: &[i16], rate: u32) -> Vec<u8> {
    let size = samples.len() as u32 * 2;
    let mut out = Vec::with_capacity(size as usize + 44);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(size + 36).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt \x10\0\0\0\x01\0\x01\0");
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(b"\x02\0\x10\0data");
    out.extend_from_slice(&size.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

fn substitute(value: &mut Value, audio: &str, model: &str) {
    match value {
        Value::String(s) => {
            *s = s
                .replace("{{audio_base64}}", audio)
                .replace("{{model}}", model)
        }
        Value::Object(o) => {
            for v in o.values_mut() {
                substitute(v, audio, model);
            }
        }
        Value::Array(a) => {
            for v in a {
                substitute(v, audio, model);
            }
        }
        _ => (),
    }
}

fn multipart(settings: &Settings, audio: &[u8], boundary: &str) -> Result<Vec<u8>, String> {
    let mut fields = serde_json::Map::new();
    fields.insert("model".into(), Value::String(settings.model.clone()));
    if settings.custom {
        let extra: Value = serde_json::from_str(&settings.body).map_err(|_| "Invalid fields")?;
        for (key, val) in extra.as_object().ok_or("Invalid fields")? {
            if key == "file" || key.contains(['\r', '\n', '"']) {
                return Err("Invalid multipart field name".into());
            }
            fields.insert(key.clone(), val.clone());
        }
    }
    let mut out = Vec::new();
    for (key, val) in fields {
        let val = val.as_str().ok_or("Multipart fields must be strings")?;
        out.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{key}\"\r\n\r\n{val}\r\n"
            )
            .as_bytes(),
        );
    }
    out.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"voice.wav\"\r\nContent-Type: audio/wav\r\n\r\n").as_bytes());
    out.extend_from_slice(audio);
    out.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    Ok(out)
}

pub fn transcribe(
    settings: &Settings,
    key: &str,
    samples: &[i16],
    rate: u32,
) -> Result<String, String> {
    validate(settings)?;
    if samples.is_empty() {
        return Err("No microphone audio captured".into());
    }
    let audio = wav(samples, rate);
    if audio.len() > 24_000_000 {
        return Err("Recording exceeds the 24 MB upload limit".into());
    }
    let boundary = format!("tm-voice-{}", super::now_ms());
    let (body, content_type) = if settings.custom && settings.json_body {
        let mut body: Value = serde_json::from_str(&settings.body).map_err(|_| "Invalid body")?;
        substitute(
            &mut body,
            &base64::engine::general_purpose::STANDARD.encode(&audio),
            &settings.model,
        );
        (
            serde_json::to_vec(&body).map_err(|_| "Invalid body")?,
            "application/json".to_string(),
        )
    } else {
        (
            multipart(settings, &audio, &boundary)?,
            format!("multipart/form-data; boundary={boundary}"),
        )
    };
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(90)))
        .max_redirects(0)
        .build()
        .into();
    let headers: serde_json::Map<String, Value> = if settings.custom {
        serde_json::from_str(&settings.headers).map_err(|_| "Invalid headers")?
    } else {
        Default::default()
    };
    let custom_authorization = headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case("authorization"));
    let mut req = agent
        .post(endpoint(settings))
        .header("Content-Type", &content_type);
    if !key.is_empty() && !custom_authorization {
        req = req.header("Authorization", format!("Bearer {key}"));
    }
    for (name, val) in &headers {
        req = req.header(name, val.as_str().unwrap().replace("{{api_key}}", key));
    }
    let mut response = req.send(body.as_slice()).map_err(|e| match e {
        ureq::Error::StatusCode(code) => {
            format!("Transcription HTTP {code}. Check endpoint, API key, model and account quota.")
        }
        _ => "Transcription connection failed or timed out. Check endpoint and network.".into(),
    })?;
    let body = response
        .body_mut()
        .with_config()
        .limit(1_000_000)
        .read_to_string()
        .map_err(|_| "Cannot read transcription response")?;
    let pointer = if settings.custom {
        settings.response_pointer.as_str()
    } else {
        "/text"
    };
    let text = if pointer.is_empty() {
        body
    } else {
        let json: Value =
            serde_json::from_str(&body).map_err(|_| "Provider did not return valid JSON")?;
        json.pointer(pointer)
            .and_then(Value::as_str)
            .ok_or("Response text not found at the configured JSON pointer")?
            .to_string()
    };
    let text = text.trim().to_string();
    if text.is_empty() {
        Err("No speech detected".into())
    } else {
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wav_header_and_samples() {
        let bytes = wav(&[-32768, 0, 32767], 24000);
        assert_eq!(bytes.len(), 50);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[24..28], &24000u32.to_le_bytes());
        assert_eq!(&bytes[44..], &[0, 128, 0, 0, 255, 127]);
    }
    #[test]
    fn json_templates_escape_text_and_recurse() {
        let mut value = serde_json::json!({"model":"{{model}}","audio":["{{audio_base64}}"]});
        substitute(&mut value, "YWJj", "model\"x");
        assert_eq!(value["model"], "model\"x");
        assert_eq!(value["audio"][0], "YWJj");
    }
    #[test]
    fn reject_unsafe_urls_and_headers() {
        let mut s = Settings {
            custom: true,
            ..Settings::default()
        };
        s.endpoint = "http://example.com/audio".into();
        assert!(validate(&s).is_err());
        s.endpoint = "http://127.0.0.1:8000/audio".into();
        assert!(validate(&s).is_ok());
        s.headers = r#"{"Content-Length":"0"}"#.into();
        assert!(validate(&s).is_err());
    }
    #[test]
    fn multipart_contains_binary_and_extra_fields() {
        let s = Settings {
            custom: true,
            body: r#"{"language":"pt"}"#.into(),
            ..Settings::default()
        };
        let body = multipart(&s, &[0, 255, 1], "boundary").unwrap();
        assert!(body.windows(3).any(|w| w == [0, 255, 1]));
        assert!(String::from_utf8_lossy(&body).contains("name=\"language\"\r\n\r\npt"));
        assert!(body.ends_with(b"--boundary--\r\n"));
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use std::io::{Read, Write};
    fn server(status: &str, body: &str) -> (String, std::thread::JoinHandle<Vec<u8>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            loop {
                let mut buf = [0u8; 4096];
                let count = socket.read(&mut buf).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..count]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&request[..end]).to_lowercase();
                    let length: usize = head
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            socket.write_all(response.as_bytes()).unwrap();
            request
        });
        (format!("http://{address}/audio"), thread)
    }
    #[test]
    fn post_json_maps_response_and_sends_saved_key_placeholder() {
        let (endpoint, server) = server("200 OK", r#"{"result":{"transcript":"Olá mundo"}}"#);
        let s = Settings {
            custom: true,
            endpoint,
            json_body: true,
            headers: r#"{"X-API-Key":"{{api_key}}"}"#.into(),
            body: r#"{"audio":"{{audio_base64}}","model":"{{model}}"}"#.into(),
            response_pointer: "/result/transcript".into(),
            ..Settings::default()
        };
        assert_eq!(
            transcribe(&s, "fixture-secret", &[100, 200], 24000).unwrap(),
            "Olá mundo"
        );
        let request = String::from_utf8(server.join().unwrap()).unwrap();
        assert!(request.starts_with("POST /audio HTTP/1.1"));
        assert!(request.to_lowercase().contains("x-api-key: fixture-secret"));
        let body = request.split("\r\n\r\n").nth(1).unwrap();
        let json: Value = serde_json::from_str(body).unwrap();
        let audio = base64::engine::general_purpose::STANDARD
            .decode(json["audio"].as_str().unwrap())
            .unwrap();
        assert_eq!(audio, wav(&[100, 200], 24000));
    }
    #[test]
    fn multipart_accepts_plain_text_response() {
        let (endpoint, server) = server("200 OK", "hello");
        let s = Settings {
            custom: true,
            endpoint,
            response_pointer: String::new(),
            ..Settings::default()
        };
        assert_eq!(transcribe(&s, "", &[0, 1], 24000).unwrap(), "hello");
        let request = server.join().unwrap();
        assert!(String::from_utf8_lossy(&request).contains("filename=\"voice.wav\""));
    }
    #[test]
    fn error_body_and_secrets_are_not_exposed() {
        let (endpoint, server) = server(
            "401 Unauthorized",
            "fixture-secret private provider response",
        );
        let s = Settings {
            custom: true,
            endpoint,
            ..Settings::default()
        };
        let error = transcribe(&s, "fixture-secret", &[1], 24000).unwrap_err();
        assert!(error.contains("401"));
        assert!(!error.contains("fixture-secret"));
        server.join().unwrap();
    }
    #[test]
    fn rejects_missing_response_text() {
        let (endpoint, server) = server("200 OK", "{}");
        let s = Settings {
            custom: true,
            endpoint,
            ..Settings::default()
        };
        assert!(transcribe(&s, "", &[1], 24000)
            .unwrap_err()
            .contains("JSON pointer"));
        server.join().unwrap();
    }
}
