//! Verifies the wire format the AnthropicModel sends — specifically that
//! `cached_context` becomes a system block carrying `cache_control: ephemeral`,
//! and that `temperature` is omitted on Claude Opus 4.7. Uses a one-shot
//! tokio TCP server speaking the minimum HTTP needed by reqwest.

use ontology_rag::{AnthropicModel, LanguageModel, LlmRequest, Message};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

async fn run_stub(captured: Arc<Mutex<String>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 16 * 1024];
        let mut total = 0;
        // Read until we have headers + the declared body.
        let mut content_length: Option<usize> = None;
        let mut header_end: Option<usize> = None;
        loop {
            let n = sock.read(&mut buf[total..]).await.unwrap();
            if n == 0 { break; }
            total += n;
            let view = &buf[..total];
            if header_end.is_none() {
                if let Some(pos) = find_subseq(view, b"\r\n\r\n") {
                    header_end = Some(pos + 4);
                    let headers = std::str::from_utf8(&view[..pos]).unwrap_or("");
                    for line in headers.split("\r\n") {
                        let lower = line.to_ascii_lowercase();
                        if let Some(rest) = lower.strip_prefix("content-length:") {
                            content_length = rest.trim().parse().ok();
                        }
                    }
                }
            }
            if let (Some(he), Some(cl)) = (header_end, content_length) {
                if total >= he + cl { break; }
            }
            if total == buf.len() { break; }
        }
        let request = String::from_utf8_lossy(&buf[..total]).to_string();
        *captured.lock().await = request;

        let body = br#"{"id":"msg_x","type":"message","role":"assistant","model":"claude-opus-4-7","stop_reason":"end_turn","content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":10,"output_tokens":2,"cache_creation_input_tokens":1234,"cache_read_input_tokens":0}}"#;
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len(),
        );
        sock.write_all(resp.as_bytes()).await.unwrap();
        sock.write_all(body).await.unwrap();
        sock.shutdown().await.ok();
    });
    port
}

fn find_subseq(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn body_of(request: &str) -> &str {
    request.split("\r\n\r\n").nth(1).unwrap_or("")
}

#[tokio::test]
async fn cached_context_serializes_with_cache_control_and_no_temperature_on_opus_4_7() {
    let captured = Arc::new(Mutex::new(String::new()));
    let port = run_stub(captured.clone()).await;

    let model = AnthropicModel::new("test-key")
        .with_base_url(format!("http://127.0.0.1:{port}"))
        .with_model("claude-opus-4-7");

    let resp = model.generate(&LlmRequest {
        system: Some("You answer using context.".into()),
        cached_context: Some("# Ontology\n- Person :: human\n".into()),
        messages: vec![Message::user("What is a person?")],
        max_tokens: 64,
        temperature: 0.7,
    }).await.unwrap();

    assert_eq!(resp.usage.cache_creation_input_tokens, 1234);

    let req = captured.lock().await.clone();
    let body = body_of(&req);
    let v: serde_json::Value = serde_json::from_str(body)
        .unwrap_or_else(|e| panic!("not JSON: {e}; body=`{body}`"));

    // temperature must be absent on Opus 4.7.
    assert!(v.get("temperature").is_none(), "temperature leaked on Opus 4.7: {v}");

    // system must be a 2-block array; the cached block carries cache_control.
    let sys = v.get("system").expect("system field present");
    let arr = sys.as_array().expect("system rendered as block array");
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["text"], "You answer using context.");
    assert!(arr[0].get("cache_control").is_none());
    assert!(arr[1]["text"].as_str().unwrap().contains("Ontology"));
    assert_eq!(arr[1]["cache_control"]["type"], "ephemeral");
}

#[tokio::test]
async fn temperature_sent_for_non_opus_4_7_models() {
    let captured = Arc::new(Mutex::new(String::new()));
    let port = run_stub(captured.clone()).await;

    let model = AnthropicModel::new("test-key")
        .with_base_url(format!("http://127.0.0.1:{port}"))
        .with_model("claude-opus-4-6");

    let _ = model.generate(&LlmRequest {
        system: Some("hi".into()),
        cached_context: None,
        messages: vec![Message::user("ping")],
        max_tokens: 16,
        temperature: 0.3,
    }).await.unwrap();

    let req = captured.lock().await.clone();
    let v: serde_json::Value = serde_json::from_str(body_of(&req)).unwrap();
    assert_eq!(v["temperature"], 0.3);
    // No cached_context → system rendered as plain string.
    assert_eq!(v["system"], "hi");
}
