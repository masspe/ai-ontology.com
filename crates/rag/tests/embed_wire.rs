// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! The embeddings client on the wire: the request it sends, the vectors it
//! returns in order, the dimension it probes, and what a refusal or a
//! malformed answer becomes — a zero vector, never a wrong one.

use ontology_index::Embedder;
use ontology_rag::RemoteEmbedder;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

async fn read_request(sock: &mut tokio::net::TcpStream) -> String {
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

/// A server answering every request with `status` and a body computed
/// from the number of inputs; keeps what it received.
async fn serve(
    status: &'static str,
    body_for: fn(usize) -> String,
) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let req = read_request(&mut sock).await;
            let inputs = req.matches("\"").count(); // not used for the count below
            let _ = inputs;
            let n = req
                .split_once("\"input\":[")
                .map(|(_, rest)| rest.split(']').next().unwrap_or("").split("\",\"").count())
                .unwrap_or(0);
            log.lock().await.push(req);
            let body = body_for(n);
            let resp = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
            let _ = sock.shutdown().await;
        }
    });
    (format!("http://{addr}"), seen)
}

fn vectors(n: usize) -> String {
    // Vector i is [i, 1, 0], reversed in the answer: order comes from `index`.
    let data: Vec<String> = (0..n)
        .rev()
        .map(|i| format!("{{\"index\":{i},\"embedding\":[{i}.0,1.0,0.0]}}"))
        .collect();
    format!("{{\"object\":\"list\",\"data\":[{}]}}", data.join(","))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn probes_the_dimension_then_embeds_in_order_with_the_model_and_the_key() {
    let (base, seen) = serve("200 OK", vectors).await;
    let e = RemoteEmbedder::connect("sk-test", base, "text-embedding-3-small").unwrap();
    assert_eq!(e.dim(), 3);
    assert_eq!(e.model(), "text-embedding-3-small");
    assert!(e.name().starts_with("text-embedding-3-small@http://"));

    let got = e.embed_batch(&["a", "b", "c"]);
    assert_eq!(
        got,
        vec![
            vec![0.0, 1.0, 0.0],
            vec![1.0, 1.0, 0.0],
            vec![2.0, 1.0, 0.0]
        ]
    );
    assert_eq!(e.embed("solo"), vec![0.0, 1.0, 0.0]);

    let log = seen.lock().await;
    assert_eq!(log.len(), 3, "probe, batch, single");
    let batch = &log[1];
    assert!(batch.starts_with("POST /v1/embeddings HTTP/1.1"), "{batch}");
    assert!(
        batch.contains("authorization: Bearer sk-test")
            || batch.contains("Authorization: Bearer sk-test"),
        "{batch}"
    );
    assert!(batch.contains("\"model\":\"text-embedding-3-small\""));
    assert!(batch.contains("\"input\":[\"a\",\"b\",\"c\"]"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_base_url_ending_in_v1_is_not_doubled() {
    let (base, seen) = serve("200 OK", vectors).await;
    let e = RemoteEmbedder::connect("k", format!("{base}/v1"), "m").unwrap();
    e.embed("x");
    let log = seen.lock().await;
    assert!(
        log.iter().all(|r| r.starts_with("POST /v1/embeddings ")),
        "{log:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn more_texts_than_a_batch_take_several_requests() {
    let (base, seen) = serve("200 OK", vectors).await;
    let e = RemoteEmbedder::connect("k", base, "m").unwrap();
    let texts: Vec<String> = (0..ontology_rag::EMBED_BATCH + 1)
        .map(|i| format!("t{i}"))
        .collect();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let got = e.embed_batch(&refs);
    assert_eq!(got.len(), texts.len());
    assert_eq!(
        got[ontology_rag::EMBED_BATCH],
        vec![0.0, 1.0, 0.0],
        "second request starts at 0"
    );
    assert_eq!(seen.lock().await.len(), 3, "probe + two batches");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refusal_is_a_connect_error_and_later_a_zero_vector() {
    let (base, _) = serve("401 Unauthorized", |_| "{\"error\":\"bad key\"}".into()).await;
    let err = RemoteEmbedder::connect("k", base.clone(), "m")
        .err()
        .expect("refused");
    assert!(err.to_string().contains("401"), "{err}");

    // Connected fine, then the provider breaks: the text gets no score.
    let (ok_base, _) = serve("200 OK", vectors).await;
    let e = RemoteEmbedder::connect("k", ok_base, "m").unwrap();
    let (bad_base, _) = serve("200 OK", |_| "{\"data\":[]}".into()).await;
    let _ = bad_base;
    // A malformed answer (too few vectors) on the first server is simulated
    // by asking for an empty batch's shape: the client refuses an answer
    // whose count differs from the request.
    let shape = serve("200 OK", |_| {
        "{\"data\":[{\"index\":0,\"embedding\":[1.0,0.0,0.0]}]}".into()
    })
    .await;
    let e2 = RemoteEmbedder::connect("k", shape.0, "m").unwrap();
    let got = e2.embed_batch(&["a", "b"]);
    assert_eq!(
        got,
        vec![vec![0.0; 3], vec![0.0; 3]],
        "one vector for two texts: none kept"
    );
    assert_eq!(e.embed("fine"), vec![0.0, 1.0, 0.0]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_that_is_not_there_is_a_connect_error() {
    let err = RemoteEmbedder::connect("k", "http://127.0.0.1:1", "m")
        .err()
        .expect("no server");
    assert!(err.to_string().starts_with("embeddings:"), "{err}");
}
