//! LIVE Genius Chat test against the real NVIDIA API (nothing mocked). Skipped unless
//! `NVIDIA_API_KEY` is set. Two tiny requests:
//!
//! 1. an image whose **pixels are red** but whose **file name says "blue"** — a model that only
//!    reads the name or the surrounding text answers "blue"; one that sees the pixels answers "red";
//! 2. a plain greeting — timing and the metadata the UI would show.
//!
//!   NVIDIA_API_KEY=nvapi-… cargo test -p genius-core --test live_chat -- --nocapture

use genius_core::app::Genius;
use genius_core::chat_mode::ChatRequest;
use genius_core::events::{EventKind, NullEmitter};
use genius_core::secrets::MemoryStore;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
    out
}

/// A real, decodable `size`×`size` RGB PNG of one colour (zlib "stored" blocks, no compression).
fn solid_png(size: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut raw = Vec::new();
    for _ in 0..size {
        raw.extend(std::iter::once(0u8).chain(std::iter::repeat_n(rgb, size as usize).flatten())); // filter byte 0 (none), then the row
    }
    let mut z = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65_535).peekable();
    while let Some(c) = chunks.next() {
        z.push(if chunks.peek().is_none() { 1 } else { 0 });
        z.extend_from_slice(&(c.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
        z.extend_from_slice(c);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut ihdr = size.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend(chunk(b"IHDR", &ihdr));
    png.extend(chunk(b"IDAT", &z));
    png.extend(chunk(b"IEND", &[]));
    png
}

#[test]
fn the_synthetic_png_is_well_formed() {
    let p = solid_png(32, [220, 20, 20]);
    assert_eq!(genius_core::attachments::sniff_image(&p), Some("image/png"));
    assert_eq!(&p[12..16], b"IHDR");
    assert_eq!(crc32(b"IEND"), 0xAE42_6082, "crc32 implementation matches the PNG spec's IEND crc");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_chat_sees_pixels_not_filenames_and_greets_plainly() {
    let Ok(key) = std::env::var("NVIDIA_API_KEY") else {
        eprintln!("SKIPPED: set NVIDIA_API_KEY to run the live chat test");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let g = Genius::new(&tmp.path().join("data"), Arc::new(NullEmitter), Arc::new(MemoryStore::default())).unwrap();
    let mut s = g.settings().unwrap();
    s.model = None;
    s.model_mode = "auto".into();
    s.workspace_dir = Some(tmp.path().join("projects").to_string_lossy().to_string());
    g.save_settings(&s).unwrap();
    g.save_api_key(&key).unwrap();

    let conv = g.create_chat_conversation("chat", None).unwrap();
    let pid = conv.row.project_id.clone();
    let ask = |message: &str, atts: Vec<String>| {
        let g = g.clone();
        let (cid, pid, message) = (conv.row.id.clone(), pid.clone(), message.to_string());
        async move {
            let started = Instant::now();
            let t = g.start_chat(&cid, ChatRequest { message, attachment_ids: atts, effort: None }).unwrap();
            for _ in 0..2400 {
                if g.active_tasks(Some(&pid)).is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let msgs = g.db.list_conversation_messages(&cid, 50).unwrap();
            let events: Vec<_> = g.bus.list(Some(&pid), 500).unwrap().into_iter().filter(|e| e.task_id.as_deref() == Some(t.id.as_str()) && e.kind == EventKind::Ai).collect();
            (msgs.into_iter().rev().find(|m| m.role == "assistant").unwrap(), events, started.elapsed())
        }
    };

    // 1. pixels vs. file name
    let ws = g.general_workspace().unwrap();
    let att = g.import_attachment_bytes(&ws.id, "blue.png", &solid_png(64, [220, 20, 20]), "message", Some(&conv.row.id)).unwrap();
    assert_eq!(att.status, "ready", "{:?}", att.error);
    let (a, events, took) = ask("What is the dominant colour of this image? Answer with one word.", vec![att.id.clone()]).await;
    let report = &a.meta["report"];
    println!("live image: kind={} answer={:?} took={took:?}", a.kind, a.content);
    println!("live image: modelCalls={}", report["modelCalls"]);
    println!("live image: vision={} model={} reported={} timing={}", report["vision"], report["model"], report["reportedModel"], report["timing"]);
    for e in &events {
        println!("live image: event {:?} {}", e.status, e.title);
    }
    assert_eq!(a.kind, "answer", "{}", a.content);
    assert!(a.content.to_lowercase().contains("red"), "the model must report the pixels (red), not the file name (blue): {}", a.content);
    assert!(!a.content.to_lowercase().contains("blue"), "{}", a.content);
    assert_eq!(report["vision"]["route"], "native");
    assert_eq!(report["vision"]["answeredBy"], report["model"], "the model that answered is the one that received the image");

    // 2. greeting
    let (a, _, took) = ask("Hi! Who are you?", vec![]).await;
    let report = &a.meta["report"];
    println!("live greeting: answer={:?} took={took:?} model={} reported={} timing={}", a.content, report["model"], report["reportedModel"], report["timing"]);
    println!("live greeting: modelCalls={}", report["modelCalls"]);
    assert_eq!(a.kind, "answer");
}

/// The provider diagnostics against the real NVIDIA API (opt-in, three tiny requests): every stage
/// must pass for a working key, and the report must not contain the key.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_provider_diagnostics_pass_and_do_not_leak_the_key() {
    let Ok(key) = std::env::var("NVIDIA_API_KEY") else {
        eprintln!("SKIPPED: set NVIDIA_API_KEY to run the live diagnostics test");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let g = Genius::new(&tmp.path().join("data"), Arc::new(NullEmitter), Arc::new(MemoryStore::default())).unwrap();
    g.save_api_key(&key).unwrap();
    let r = g.diagnose_provider(true).await.unwrap();
    for s in &r.steps {
        println!("live diag: {:?} {} — {}", s.status, s.title, s.detail);
    }
    println!("live diag: verdict: {}", r.verdict);
    assert!(r.ok, "{}", r.verdict);
    assert_eq!(r.integration_verified, "live");
    let json = serde_json::to_string(&r).unwrap();
    assert!(!json.contains(&key), "the report must never contain the key");
}
