// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Failure modes and framing edge cases of the two HTTP clients against a
//! loopback TCP stub: non-retryable statuses, decode errors, role mapping,
//! and SSE streams delivered in arbitrary byte slices or closed without a
//! terminator. No network is touched.

use futures::StreamExt;
use ontology_rag::{
    AnthropicModel, LanguageModel, LlmError, LlmRequest, Message, OpenAiModel, StreamChunk,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

/// Read one full HTTP request (headers + declared body) off `sock`.
async fn read_request(sock: &mut TcpStream) -> String {
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0;
    let mut content_length: Option<usize> = None;
    let mut header_end: Option<usize> = None;
    loop {
        let n = sock.read(&mut buf[total..]).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        total += n;
        let view = &buf[..total];
        if header_end.is_none() {
            if let Some(pos) = view.windows(4).position(|w| w == b"\r\n\r\n") {
                header_end = Some(pos + 4);
                let headers = std::str::from_utf8(&view[..pos]).unwrap_or("");
                for line in headers.split("\r\n") {
                    if let Some(rest) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        content_length = rest.trim().parse().ok();
                    }
                }
            }
        }
        if let (Some(he), Some(cl)) = (header_end, content_length) {
            if total >= he + cl {
                break;
            }
        }
        if total == buf.len() {
            break;
        }
    }
    String::from_utf8_lossy(&buf[..total]).to_string()
}

fn body_of(request: &str) -> &str {
    request.split("\r\n\r\n").nth(1).unwrap_or("")
}

fn header(request: &str, name: &str) -> Option<String> {
    let head = request.split("\r\n\r\n").next().unwrap_or("");
    let prefix = format!("{}:", name.to_ascii_lowercase());
    head.split("\r\n")
        .find(|l| l.to_ascii_lowercase().starts_with(&prefix))
        .map(|l| l[prefix.len()..].trim().to_string())
}

/// Serve every connection with the same `status` line and body; count them.
async fn serve_status(status: &'static str, body: &'static str) -> (u16, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let count = Arc::new(AtomicUsize::new(0));
    let count2 = count.clone();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            count2.fetch_add(1, Ordering::SeqCst);
            let _ = read_request(&mut sock).await;
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(head.as_bytes()).await;
            let _ = sock.write_all(body.as_bytes()).await;
            let _ = sock.shutdown().await;
        }
    });
    (port, count)
}

/// Serve one connection: capture the request, then write `pieces` one by
/// one (flushing between them) after the given raw response head.
async fn serve_once(head: &'static str, pieces: Vec<Vec<u8>>, captured: Arc<Mutex<String>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        *captured.lock().await = read_request(&mut sock).await;
        sock.write_all(head.as_bytes()).await.unwrap();
        for p in pieces {
            sock.write_all(&p).await.unwrap();
            sock.flush().await.unwrap();
            tokio::task::yield_now().await;
        }
        sock.shutdown().await.ok();
    });
    port
}

fn json_head(body_len: usize) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {body_len}\r\nConnection: close\r\n\r\n"
    )
}

const SSE_HEAD: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";

fn slices(s: &str, n: usize) -> Vec<Vec<u8>> {
    s.as_bytes().chunks(n).map(|c| c.to_vec()).collect()
}

fn one_turn() -> LlmRequest {
    LlmRequest {
        messages: vec![Message::user("hi")],
        max_tokens: 8,
        ..Default::default()
    }
}

// -- Anthropic ----------------------------------------------------------

/// A 401 is a client error: exactly one connection, and the error carries
/// both the status and the server's body so the user sees the real cause.
#[tokio::test]
async fn anthropic_401_is_not_retried_and_surfaces_status_and_body() {
    let (port, count) = serve_status(
        "401 Unauthorized",
        r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#,
    )
    .await;
    let model = AnthropicModel::new("bad-key")
        .with_base_url(format!("http://127.0.0.1:{port}"))
        .with_max_retries(3)
        .with_initial_backoff(Duration::from_millis(1));
    let err = model.generate(&one_turn()).await.unwrap_err();
    match err {
        LlmError::Api(msg) => {
            assert!(msg.starts_with("401"), "{msg}");
            assert!(msg.contains("invalid x-api-key"), "{msg}");
        }
        other => panic!("expected Api, got {other:?}"),
    }
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

/// A 200 whose body is not the expected JSON is a `Decode` error, not a
/// silent empty answer.
#[tokio::test]
async fn anthropic_invalid_json_body_is_a_decode_error() {
    let (port, _) = serve_status("200 OK", "<html>gateway</html>").await;
    let model = AnthropicModel::new("k").with_base_url(format!("http://127.0.0.1:{port}"));
    let err = model.generate(&one_turn()).await.unwrap_err();
    assert!(matches!(err, LlmError::Decode(_)), "got {err:?}");
}

/// `System`-role messages never reach the `messages` array (the API
/// rejects them there); assistant turns are kept in order; auth and
/// version headers are set; `max_tokens` is forwarded verbatim.
#[tokio::test]
async fn anthropic_drops_system_role_messages_and_keeps_the_assistant_turn() {
    let captured = Arc::new(Mutex::new(String::new()));
    let body = br#"{"model":"claude-opus-4-6","stop_reason":"end_turn","content":[{"type":"text","text":"a"},{"type":"text","text":"b"}],"usage":{"input_tokens":1,"output_tokens":1}}"#;
    let head: &'static str = Box::leak(json_head(body.len()).into_boxed_str());
    let port = serve_once(head, vec![body.to_vec()], captured.clone()).await;
    let model = AnthropicModel::new("secret-key")
        .with_base_url(format!("http://127.0.0.1:{port}"))
        .with_model("claude-opus-4-6");
    let resp = model
        .generate(&LlmRequest {
            system: None,
            cached_context: None,
            messages: vec![
                Message::system("inline system"),
                Message::user("q1"),
                Message::assistant("a1"),
                Message::user("q2"),
            ],
            max_tokens: 4321,
            temperature: 0.0,
        })
        .await
        .unwrap();
    assert_eq!(resp.content, "ab", "text blocks are concatenated");
    let req = captured.lock().await.clone();
    assert_eq!(header(&req, "x-api-key").as_deref(), Some("secret-key"));
    assert_eq!(
        header(&req, "anthropic-version").as_deref(),
        Some("2023-06-01")
    );
    assert!(req.starts_with("POST /v1/messages HTTP/1.1"), "{req}");
    let v: serde_json::Value = serde_json::from_str(body_of(&req)).unwrap();
    assert_eq!(v["max_tokens"], 4321);
    assert!(
        v.get("system").is_none(),
        "no system field without instruction"
    );
    let roles: Vec<&str> = v["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, ["user", "assistant", "user"]);
    assert_eq!(v["messages"][1]["content"], "a1");
}

/// SSE frames delivered in 7-byte slices (event boundaries split anywhere)
/// still reassemble into the full text and a single `End`.
#[tokio::test]
async fn anthropic_stream_reassembles_frames_split_at_arbitrary_bytes() {
    let frames = "event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-opus-4-7\",\"usage\":{\"input_tokens\":5}}}\n\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hé \"}}\n\n\
: keep-alive\n\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"漢字!\"}}\n\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":3}}\n\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\n";
    let captured = Arc::new(Mutex::new(String::new()));
    let port = serve_once(SSE_HEAD, slices(frames, 7), captured).await;
    let model = AnthropicModel::new("k").with_base_url(format!("http://127.0.0.1:{port}"));
    let chunks: Vec<StreamChunk> = model
        .generate_stream(&one_turn())
        .await
        .unwrap()
        .map(|c| c.expect("no transport error"))
        .collect()
        .await;
    let text: String = chunks
        .iter()
        .filter_map(|c| match c {
            StreamChunk::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "Hé 漢字!");
    let ends: Vec<&StreamChunk> = chunks
        .iter()
        .filter(|c| matches!(c, StreamChunk::End { .. }))
        .collect();
    assert_eq!(ends.len(), 1, "exactly one End: {chunks:?}");
    match ends[0] {
        StreamChunk::End {
            usage,
            stop_reason,
            model,
        } => {
            assert_eq!(model, "claude-opus-4-7");
            assert_eq!(stop_reason.as_deref(), Some("end_turn"));
            assert_eq!((usage.input_tokens, usage.output_tokens), (5, 3));
        }
        _ => unreachable!(),
    }
    assert!(matches!(chunks.last(), Some(StreamChunk::End { .. })));
}

/// The streaming path has no retry loop: an overloaded 529 fails the call
/// up front with an `Api` error after a single connection.
#[tokio::test]
async fn anthropic_stream_error_status_fails_before_streaming() {
    let (port, count) = serve_status("529 Overloaded", r#"{"error":"overloaded"}"#).await;
    let model = AnthropicModel::new("k")
        .with_base_url(format!("http://127.0.0.1:{port}"))
        .with_max_retries(3);
    match model.generate_stream(&one_turn()).await {
        Err(LlmError::Api(msg)) => assert!(msg.contains("529") && msg.contains("overloaded")),
        Err(other) => panic!("expected Api, got {other:?}"),
        Ok(_) => panic!("expected an error, got a stream"),
    }
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

// -- OpenAI-compatible --------------------------------------------------

/// 429 is retried `max_retries` times and then surfaced as `Api` with the
/// status; a 400 is never retried.
#[tokio::test]
async fn openai_429_retries_up_to_max_then_surfaces_and_400_is_not_retried() {
    let (port, count) = serve_status(
        "429 Too Many Requests",
        r#"{"error":{"message":"slow down"}}"#,
    )
    .await;
    let model = OpenAiModel::new("k")
        .with_base_url(format!("http://127.0.0.1:{port}"))
        .with_max_retries(1)
        .with_initial_backoff(Duration::from_millis(1));
    let err = model.generate(&one_turn()).await.unwrap_err();
    match err {
        LlmError::Api(msg) => assert!(msg.starts_with("429") && msg.contains("slow down"), "{msg}"),
        other => panic!("expected Api, got {other:?}"),
    }
    assert_eq!(count.load(Ordering::SeqCst), 2, "1 attempt + 1 retry");

    let (port, count) = serve_status("400 Bad Request", r#"{"error":{"message":"bad"}}"#).await;
    let model = OpenAiModel::new("k")
        .with_base_url(format!("http://127.0.0.1:{port}"))
        .with_max_retries(5)
        .with_initial_backoff(Duration::from_millis(1));
    let err = model.generate(&one_turn()).await.unwrap_err();
    assert!(
        matches!(err, LlmError::Api(ref m) if m.starts_with("400")),
        "{err:?}"
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

/// A completion with no `choices` yields an empty answer and no stop
/// reason rather than a decode failure; usage is still read.
#[tokio::test]
async fn openai_empty_choices_yield_empty_content() {
    let (port, _) = serve_status(
        "200 OK",
        r#"{"id":"x","model":"m","choices":[],"usage":{"prompt_tokens":3,"completion_tokens":0}}"#,
    )
    .await;
    let model = OpenAiModel::new("k").with_base_url(format!("http://127.0.0.1:{port}"));
    let resp = model.generate(&one_turn()).await.unwrap();
    assert_eq!(resp.content, "");
    assert!(resp.stop_reason.is_none());
    assert_eq!(resp.usage.input_tokens, 3);
}

/// A stream closed by the server without `data: [DONE]` still ends with
/// one synthesized `End` carrying the last `finish_reason`; `data:` without
/// a space and comment lines are accepted; frames may arrive in slices.
#[tokio::test]
async fn openai_stream_without_done_synthesizes_end_from_finish_reason() {
    let frames = ": ping\n\n\
data:{\"id\":\"1\",\"model\":\"mixtral\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"A\"}}]}\n\n\
data: {\"id\":\"1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"B\"},\"finish_reason\":\"length\"}]}\n\n\
data: {\"id\":\"1\",\"choices\":[],\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":2}}\n\n";
    let captured = Arc::new(Mutex::new(String::new()));
    let port = serve_once(SSE_HEAD, slices(frames, 5), captured.clone()).await;
    let model = OpenAiModel::new("k")
        .with_base_url(format!("http://127.0.0.1:{port}"))
        .with_model("mixtral");
    let chunks: Vec<StreamChunk> = model
        .generate_stream(&one_turn())
        .await
        .unwrap()
        .map(|c| c.expect("no transport error"))
        .collect()
        .await;
    let text: String = chunks
        .iter()
        .filter_map(|c| match c {
            StreamChunk::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "AB");
    assert_eq!(
        chunks
            .iter()
            .filter(|c| matches!(c, StreamChunk::End { .. }))
            .count(),
        1
    );
    match chunks.last().unwrap() {
        StreamChunk::End {
            usage,
            stop_reason,
            model,
        } => {
            assert_eq!(stop_reason.as_deref(), Some("length"));
            assert_eq!(model, "mixtral", "first model seen is kept");
            assert_eq!(usage.output_tokens, 2);
        }
        other => panic!("expected End last, got {other:?}"),
    }
    let req = captured.lock().await.clone();
    assert_eq!(header(&req, "authorization").as_deref(), Some("Bearer k"));
    assert_eq!(header(&req, "accept").as_deref(), Some("text/event-stream"));
}
