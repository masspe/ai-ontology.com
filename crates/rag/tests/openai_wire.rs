// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Winven-Commercial
// Copyright (C) 2026 Winven AI Sarl
// Route de Crassier 7, 1262 Eysins, VD, CH
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Winven AI Sarl. See LICENSE and LICENSE-COMMERCIAL.md.

//! Wire-level checks for `OpenAiModel` against a one-shot TCP stub, with the
//! Infomaniak AI Tools layout as the primary case.
//!
//! What matters here is the *request line*: Infomaniak serves chat
//! completions at `/2/ai/{product_id}/openai/v1/chat/completions`, and the
//! previous hard-coded `format!("{base}/v1/chat/completions")` silently
//! produced a 404 for every base URL that already carried its own `/v1`.
//! These tests pin the resolved path for both base-URL spellings.

use futures::StreamExt;
use ontology_rag::{LanguageModel, LlmRequest, Message, OpenAiModel, StreamChunk};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

const COMPLETION_JSON: &[u8] = br#"{"id":"chatcmpl-1","object":"chat.completion","model":"mixtral","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":11,"completion_tokens":2,"prompt_tokens_details":{"cached_tokens":5}}}"#;

/// Read one full HTTP request (headers + declared body) off `sock`.
async fn read_request(sock: &mut tokio::net::TcpStream) -> String {
    let mut buf = vec![0u8; 16 * 1024];
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

/// One-shot stub that captures the request and replies with `COMPLETION_JSON`.
async fn run_stub(captured: Arc<Mutex<String>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        *captured.lock().await = read_request(&mut sock).await;
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            COMPLETION_JSON.len(),
        );
        sock.write_all(head.as_bytes()).await.unwrap();
        sock.write_all(COMPLETION_JSON).await.unwrap();
        sock.shutdown().await.ok();
    });
    port
}

fn request_line(request: &str) -> &str {
    request.lines().next().unwrap_or("")
}

fn header(request: &str, name: &str) -> Option<String> {
    let head = request.split("\r\n\r\n").next().unwrap_or("");
    let prefix = format!("{}:", name.to_ascii_lowercase());
    head.split("\r\n")
        .find(|l| l.to_ascii_lowercase().starts_with(&prefix))
        .map(|l| l[prefix.len()..].trim().to_string())
}

fn body_of(request: &str) -> &str {
    request.split("\r\n\r\n").nth(1).unwrap_or("")
}

#[tokio::test]
async fn infomaniak_layout_hits_the_product_scoped_v1_chat_path() {
    let captured = Arc::new(Mutex::new(String::new()));
    let port = run_stub(captured.clone()).await;

    // Same shape as `infomaniak_base_url("101112")`, pointed at the stub:
    // the base already ends in `/v1`, which must not be duplicated.
    let model = OpenAiModel::infomaniak("tok", "101112")
        .with_base_url(format!("http://127.0.0.1:{port}/2/ai/101112/openai/v1"))
        .with_model("mixtral");

    let resp = model
        .generate(&LlmRequest {
            system: Some("be terse".into()),
            cached_context: Some("ONTOLOGY".into()),
            messages: vec![Message::user("ping")],
            max_tokens: 64,
            temperature: 0.2,
        })
        .await
        .unwrap();

    assert_eq!(resp.content, "ok");
    assert_eq!(resp.model, "mixtral");
    assert_eq!(resp.stop_reason.as_deref(), Some("stop"));
    assert_eq!(resp.usage.input_tokens, 11);
    assert_eq!(resp.usage.cache_read_input_tokens, 5);

    let req = captured.lock().await.clone();
    assert_eq!(
        request_line(&req),
        "POST /2/ai/101112/openai/v1/chat/completions HTTP/1.1",
        "unexpected request line in:\n{req}"
    );
    assert_eq!(
        header(&req, "authorization").as_deref(),
        Some("Bearer tok"),
        "missing/invalid bearer auth in:\n{req}"
    );

    let v: serde_json::Value = serde_json::from_str(body_of(&req)).unwrap();
    assert_eq!(v["model"], "mixtral");
    assert_eq!(v["max_tokens"], 64);
    assert_eq!(v["temperature"], 0.2);
    // `stream` is skipped when false, so a non-streaming call must omit it.
    assert!(v.get("stream").is_none(), "stream leaked: {v}");
    // cached_context is folded ahead of the system instruction.
    assert_eq!(v["messages"][0]["role"], "system");
    assert_eq!(v["messages"][0]["content"], "ONTOLOGY\n\nbe terse");
    assert_eq!(v["messages"][1]["content"], "ping");
}

#[tokio::test]
async fn base_url_without_v1_still_reaches_v1_chat_completions() {
    let captured = Arc::new(Mutex::new(String::new()));
    let port = run_stub(captured.clone()).await;

    let model = OpenAiModel::new("tok")
        .with_base_url(format!("http://127.0.0.1:{port}"))
        .with_model("gpt-4o-mini");

    model
        .generate(&LlmRequest {
            messages: vec![Message::user("ping")],
            max_tokens: 8,
            ..Default::default()
        })
        .await
        .unwrap();

    let req = captured.lock().await.clone();
    assert_eq!(
        request_line(&req),
        "POST /v1/chat/completions HTTP/1.1",
        "unexpected request line in:\n{req}"
    );
}

/// One-shot stub replying with an OpenAI-style SSE stream.
async fn run_streaming_stub(captured: Arc<Mutex<String>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        *captured.lock().await = read_request(&mut sock).await;

        let head = "HTTP/1.1 200 OK\r\n\
                    Content-Type: text/event-stream\r\n\
                    Cache-Control: no-cache\r\n\
                    Connection: close\r\n\r\n";
        sock.write_all(head.as_bytes()).await.unwrap();

        let frames = [
            ": keep-alive comment\n\n",
            "data: {\"id\":\"1\",\"model\":\"mixtral\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hel\"}}]}\n\n",
            "data: {\"id\":\"1\",\"model\":\"mixtral\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo!\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"id\":\"1\",\"model\":\"mixtral\",\"choices\":[],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":2,\"prompt_tokens_details\":{\"cached_tokens\":4}}}\n\n",
            "data: [DONE]\n\n",
        ];
        for f in frames {
            sock.write_all(f.as_bytes()).await.unwrap();
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        sock.shutdown().await.ok();
    });
    port
}

#[tokio::test]
async fn infomaniak_streaming_yields_deltas_then_end_with_usage() {
    let captured = Arc::new(Mutex::new(String::new()));
    let port = run_streaming_stub(captured.clone()).await;

    let model = OpenAiModel::infomaniak("tok", "101112")
        .with_base_url(format!("http://127.0.0.1:{port}/2/ai/101112/openai/v1"))
        .with_model("mixtral");

    let stream = model
        .generate_stream(&LlmRequest {
            messages: vec![Message::user("ping")],
            max_tokens: 32,
            ..Default::default()
        })
        .await
        .unwrap();

    let chunks: Vec<_> = stream.collect().await;
    assert!(
        chunks.iter().all(|c| c.is_ok()),
        "stream contained an error: {chunks:?}"
    );

    let mut text = String::new();
    let mut end_found = false;
    for chunk in chunks.into_iter().flatten() {
        match chunk {
            StreamChunk::Text(t) => text.push_str(&t),
            StreamChunk::End {
                usage,
                stop_reason,
                model,
            } => {
                assert_eq!(text, "Hello!");
                assert_eq!(stop_reason.as_deref(), Some("stop"));
                assert_eq!(model, "mixtral");
                assert_eq!(usage.input_tokens, 9);
                assert_eq!(usage.output_tokens, 2);
                assert_eq!(usage.cache_read_input_tokens, 4);
                end_found = true;
            }
            StreamChunk::KeepAlive => {}
        }
    }
    assert!(end_found, "stream did not emit End");

    let req = captured.lock().await.clone();
    assert_eq!(
        request_line(&req),
        "POST /2/ai/101112/openai/v1/chat/completions HTTP/1.1",
        "unexpected request line in:\n{req}"
    );
    let v: serde_json::Value = serde_json::from_str(body_of(&req)).unwrap();
    assert_eq!(v["stream"], true);
    assert_eq!(v["stream_options"]["include_usage"], true);
}
