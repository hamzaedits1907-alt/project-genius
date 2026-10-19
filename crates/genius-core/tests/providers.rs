//! Provider contract tests at the HTTP level, plus the execution budgets.
//!
//! A local wiremock server plays Anthropic, Gemini and an OpenAI-compatible endpoint. These tests
//! prove what Genius really **sends** (URL, authentication header, body shape, where the key is and
//! is not) and how it **reads** replies, streams, errors and cancellation. They do NOT prove that
//! the live Anthropic, Gemini or OpenAI services accept those requests: no live call to them has
//! been made. (NVIDIA is covered live by `live_nvidia.rs` / `live_chat.rs`.)

use genius_core::app::Genius;
use genius_core::budget::TaskLimits;
use genius_core::chat_mode::ChatRequest;
use genius_core::events::{EventKind, NullEmitter};
use genius_core::nvidia::{ChatMessage, ChatOptions, NvidiaClient, ProviderError};
use genius_core::provider::ProviderKind;
use genius_core::secrets::MemoryStore;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const KEY: &str = "sk-contract-test-key-ABCDEF0123456789";

fn sse(events: &[Value]) -> ResponseTemplate {
    let body: String = events.iter().map(|e| format!("data: {e}\n\n")).collect();
    ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")
}

fn anthropic_stream(text: &str) -> Vec<Value> {
    vec![
        json!({"type":"message_start","message":{"model":"claude-contract-1","usage":{"input_tokens":21,"output_tokens":1}}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":9}}),
        json!({"type":"message_stop"}),
    ]
}

fn client(server: &MockServer, kind: ProviderKind, base_path: &str) -> NvidiaClient {
    let mut c = NvidiaClient::new(&format!("http://127.0.0.1:{}{base_path}", server.address().port()), Some(KEY.into())).unwrap().with_kind(kind);
    c.max_retries = 0; // no sleeping between attempts in tests
    c
}

fn msgs() -> Vec<ChatMessage> {
    vec![ChatMessage::system("Be brief."), ChatMessage::user("Say hi")]
}

async fn requests(server: &MockServer) -> Vec<Request> {
    server.received_requests().await.unwrap()
}

#[tokio::test]
async fn anthropic_streams_with_native_headers_body_and_measured_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/messages")).respond_with(sse(&anthropic_stream("Hello from Claude"))).mount(&server).await;
    let c = client(&server, ProviderKind::Anthropic, "/v1");
    let mut seen = String::new();
    let r = c.chat("claude-contract-1", &msgs(), &ChatOptions::default(), &CancellationToken::new(), |d| seen.push_str(d)).await.unwrap();
    assert_eq!((seen.as_str(), r.content.as_str()), ("Hello from Claude", "Hello from Claude"));
    assert_eq!(r.finish_reason.as_deref(), Some("stop"));
    assert_eq!(r.reported_model.as_deref(), Some("claude-contract-1"));
    assert_eq!(r.model, "claude-contract-1");
    assert_eq!(r.usage.unwrap(), json!({ "prompt_tokens": 21, "completion_tokens": 9, "total_tokens": 30 }));

    let req = &requests(&server).await[0];
    assert_eq!(req.headers.get("x-api-key").unwrap().to_str().unwrap(), KEY);
    assert_eq!(req.headers.get("anthropic-version").unwrap().to_str().unwrap(), "2023-06-01");
    assert!(req.headers.get("authorization").is_none(), "Anthropic does not use a bearer token");
    let body: Value = serde_json::from_slice(&req.body).unwrap();
    assert_eq!((body["system"].as_str(), body["stream"].clone(), body["model"].as_str()), (Some("Be brief."), json!(true), Some("claude-contract-1")));
    assert_eq!(body["messages"], json!([{ "role": "user", "content": "Say hi" }]));
    assert!(!String::from_utf8_lossy(&req.body).contains(KEY), "the key never goes into the body");
}

#[tokio::test]
async fn anthropic_non_streamed_reply_and_a_model_that_rejects_temperature() {
    let server = MockServer::start().await;
    let calls = Arc::new(Mutex::new(Vec::<Value>::new()));
    struct R(Arc<Mutex<Vec<Value>>>);
    impl Respond for R {
        fn respond(&self, req: &Request) -> ResponseTemplate {
            let b: Value = serde_json::from_slice(&req.body).unwrap();
            let has_temp = b.get("temperature").is_some();
            self.0.lock().unwrap().push(b);
            if has_temp {
                ResponseTemplate::new(400).set_body_json(json!({"type":"error","error":{"type":"invalid_request_error","message":"`temperature` is deprecated for this model."}}))
            } else {
                ResponseTemplate::new(200).set_body_json(json!({"type":"message","model":"claude-contract-1","content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":1}}))
            }
        }
    }
    Mock::given(method("POST")).and(path("/v1/messages")).respond_with(R(calls.clone())).mount(&server).await;
    let c = client(&server, ProviderKind::Anthropic, "/v1");
    let opts = ChatOptions { stream: false, ..Default::default() };
    let r = c.chat("claude-contract-1", &msgs(), &opts, &CancellationToken::new(), |_| {}).await.unwrap();
    assert_eq!(r.content, "ok");
    assert_eq!(r.usage.unwrap()["total_tokens"], 4);
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2, "one rejected attempt, one retry without the rejected parameter");
    assert!(calls[0].get("temperature").is_some() && calls[1].get("temperature").is_none());
    assert!(c.rejected_params("claude-contract-1").contains("temperature"), "remembered so later calls do not repeat the failure");
}

#[tokio::test]
async fn gemini_streams_with_header_key_and_the_model_in_the_url() {
    let server = MockServer::start().await;
    let ev = [
        json!({"candidates":[{"content":{"role":"model","parts":[{"text":"Hi "}]}}]}),
        json!({"candidates":[{"content":{"role":"model","parts":[{"text":"there"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":6,"candidatesTokenCount":2,"totalTokenCount":8},"modelVersion":"gemini-contract-001"}),
    ];
    Mock::given(method("POST")).and(path("/v1beta/models/gemini-contract:streamGenerateContent")).respond_with(sse(&ev)).mount(&server).await;
    let c = client(&server, ProviderKind::Gemini, "/v1beta");
    let r = c.chat("models/gemini-contract", &msgs(), &ChatOptions::default(), &CancellationToken::new(), |_| {}).await.unwrap();
    assert_eq!(r.content, "Hi there");
    assert_eq!(r.usage.unwrap(), json!({ "prompt_tokens": 6, "completion_tokens": 2, "total_tokens": 8 }));
    assert_eq!(r.reported_model.as_deref(), Some("gemini-contract-001"));
    let req = &requests(&server).await[0];
    assert_eq!(req.url.query(), Some("alt=sse"), "no key in the query string");
    assert_eq!(req.headers.get("x-goog-api-key").unwrap().to_str().unwrap(), KEY);
    let body: Value = serde_json::from_slice(&req.body).unwrap();
    assert_eq!(body["systemInstruction"]["parts"][0]["text"], "Be brief.");
    assert_eq!(body["contents"][0], json!({ "role": "user", "parts": [{ "text": "Say hi" }] }));
}

#[tokio::test]
async fn openai_and_custom_endpoints_use_a_bearer_token_on_chat_completions() {
    let server = MockServer::start().await;
    let ev = [json!({"model":"gpt-contract","choices":[{"index":0,"delta":{"content":"yo"},"finish_reason":null}]}), json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":1,"total_tokens":5}})];
    let body: String = ev.iter().map(|e| format!("data: {e}\n\n")).chain(std::iter::once("data: [DONE]\n\n".to_string())).collect();
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")).mount(&server).await;
    for kind in [ProviderKind::OpenAi, ProviderKind::Custom] {
        let r = client(&server, kind, "/v1").chat("gpt-contract", &msgs(), &ChatOptions::default(), &CancellationToken::new(), |_| {}).await.unwrap();
        assert_eq!((r.content.as_str(), r.reported_model.as_deref()), ("yo", Some("gpt-contract")));
    }
    for req in requests(&server).await {
        assert_eq!(req.headers.get("authorization").unwrap().to_str().unwrap(), format!("Bearer {KEY}"));
        assert!(req.headers.get("x-api-key").is_none());
    }
}

#[tokio::test]
async fn model_listing_per_provider() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/models")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"claude-contract-1","type":"model"}],"has_more":false}))).mount(&server).await;
    Mock::given(method("GET")).and(path("/v1beta/models")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"models":[{"name":"models/gemini-contract","supportedGenerationMethods":["generateContent"]},{"name":"models/embedding-x","supportedGenerationMethods":["embedContent"]}]}))).mount(&server).await;
    let a = client(&server, ProviderKind::Anthropic, "/v1").list_models().await.unwrap();
    assert_eq!(a.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["claude-contract-1"]);
    let g = client(&server, ProviderKind::Gemini, "/v1beta").list_models().await.unwrap();
    assert_eq!(g.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["gemini-contract"]);
    let reqs = requests(&server).await;
    assert!(reqs[0].headers.get("x-api-key").is_some() && reqs[1].headers.get("x-goog-api-key").is_some());
}

#[tokio::test]
async fn provider_errors_are_classified_and_never_echo_the_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/messages")).respond_with(ResponseTemplate::new(401).set_body_json(json!({"type":"error","error":{"type":"authentication_error","message":format!("invalid x-api-key {KEY}")}}))).mount(&server).await;
    Mock::given(method("POST")).and(path("/v1beta/models/g:generateContent")).respond_with(ResponseTemplate::new(400).set_body_json(json!({"error":{"code":400,"message":"API key not valid. Please pass a valid API key.","status":"INVALID_ARGUMENT"}}))).mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/models")).respond_with(ResponseTemplate::new(529).set_body_json(json!({"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}))).mount(&server).await;
    let opts = ChatOptions { stream: false, ..Default::default() };
    let e = client(&server, ProviderKind::Anthropic, "/v1").chat("m", &msgs(), &opts, &CancellationToken::new(), |_| {}).await.unwrap_err();
    assert!(matches!(e, ProviderError::Auth { status: 401, .. }), "{e:?}");
    assert!(!e.to_string().contains("ABCDEF0123456789"), "key redacted from the message: {e}");
    let e = client(&server, ProviderKind::Gemini, "/v1beta").chat("g", &msgs(), &opts, &CancellationToken::new(), |_| {}).await.unwrap_err();
    assert!(matches!(e, ProviderError::Auth { .. }), "Gemini's 400 for a bad key is an authentication problem: {e:?}");
    let e = client(&server, ProviderKind::Anthropic, "/v1").list_models().await.unwrap_err();
    assert!(matches!(e, ProviderError::Server { status: 529, .. }), "{e:?}");
}

#[tokio::test]
async fn cancelling_stops_a_slow_native_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/messages")).respond_with(sse(&anthropic_stream("late")).set_delay(Duration::from_secs(30))).mount(&server).await;
    let c = client(&server, ProviderKind::Anthropic, "/v1");
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        c2.cancel();
    });
    let started = std::time::Instant::now();
    let e = c.chat("m", &msgs(), &ChatOptions::default(), &cancel, |_| {}).await.unwrap_err();
    assert!(matches!(e, ProviderError::Cancelled), "{e:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

// ---------------------------------------------------------------------------- Genius level

struct Anthropic {
    log: Arc<Mutex<Vec<Value>>>,
}
impl Respond for Anthropic {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        self.log.lock().unwrap().push(serde_json::from_slice(&req.body).unwrap());
        sse(&anthropic_stream("I am Genius, running on Claude."))
    }
}

async fn genius_with(server: &MockServer, provider: &str, base_path: &str, model: &str) -> (tempfile::TempDir, Arc<Genius>) {
    let tmp = tempfile::tempdir().unwrap();
    let g = Genius::new(&tmp.path().join("data"), Arc::new(NullEmitter), Arc::new(MemoryStore::default())).unwrap();
    let mut s = g.settings().unwrap();
    s.provider = provider.into();
    s.base_url = format!("http://127.0.0.1:{}{base_path}", server.address().port());
    s.model_mode = "manual".into();
    s.model = Some(model.into());
    s.workspace_dir = Some(tmp.path().join("projects").to_string_lossy().to_string());
    g.save_settings(&s).unwrap();
    g.save_api_key(KEY).unwrap();
    (tmp, g)
}

async fn ask(g: &Arc<Genius>, message: &str) -> (genius_core::events::EventStatus, genius_core::db::MessageRow) {
    let conv = g.create_chat_conversation("chat", None).unwrap();
    let pid = conv.row.project_id.clone();
    let t = g.start_chat(&conv.row.id, ChatRequest { message: message.into(), attachment_ids: vec![], effort: Some("balanced".into()) }).unwrap();
    for _ in 0..400 {
        if g.active_tasks(Some(&pid)).is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let ev = g.bus.list(Some(&pid), 3000).unwrap().into_iter().find(|e| e.task_id.as_deref() == Some(t.id.as_str()) && e.kind == EventKind::Task && e.detail.get("taskKind").is_some()).unwrap();
    let last = g.db.list_conversation_messages(&conv.row.id, 50).unwrap().into_iter().rev().find(|m| m.role == "assistant").unwrap();
    (ev.status, last)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn genius_chats_through_anthropic_with_the_chosen_model_and_provider_usage() {
    let server = MockServer::start().await;
    let log = Arc::new(Mutex::new(Vec::new()));
    Mock::given(method("POST")).and(path("/v1/messages")).respond_with(Anthropic { log: log.clone() }).mount(&server).await;
    let (_tmp, g) = genius_with(&server, "anthropic", "/v1", "claude-contract-1").await;
    let (status, a) = ask(&g, "Hi! Who are you?").await;
    assert_eq!(format!("{status:?}"), "Completed", "{a:?}");
    assert_eq!(a.kind, "answer", "{}", a.content);
    assert!(a.content.contains("I am Genius"));
    let r = &a.meta["report"];
    assert_eq!(r["model"], "claude-contract-1");
    assert_eq!(r["reportedModel"], "claude-contract-1");
    assert_eq!(r["usage"]["totalTokens"], 30, "the provider's own counts");
    assert_eq!(r["usage"]["source"], "measured");
    assert_eq!(r["usage"]["estimatedTokens"], 0);
    let body = log.lock().unwrap()[0].clone();
    assert_eq!(body["model"], "claude-contract-1");
    let system = body["system"].as_str().unwrap();
    assert!(!system.contains("claude") && !system.contains("Anthropic"), "the prompt does not name a model: identity comes from the app");
}

#[tokio::test]
async fn every_provider_has_its_own_key_slot() {
    let server = MockServer::start().await;
    let (_tmp, g) = genius_with(&server, "anthropic", "/v1", "claude-contract-1").await;
    assert!(g.key_status().stored);
    let mut s = g.settings().unwrap();
    s.provider = "gemini".into();
    g.save_settings(&s).unwrap();
    assert!(!g.key_status().stored, "the Anthropic key is not offered to Gemini");
    s.provider = "nvidia".into();
    g.save_settings(&s).unwrap();
    assert!(!g.key_status().stored);
    s.provider = "anthropic".into();
    g.save_settings(&s).unwrap();
    assert!(g.key_status().stored);
    s.provider = "nope".into();
    assert!(g.save_settings(&s).is_err(), "unknown providers are rejected");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn usage_is_labelled_estimated_when_the_provider_reports_none() {
    let server = MockServer::start().await;
    let ev = [json!({"model":"gpt-contract","choices":[{"index":0,"delta":{"content":"No usage block here."},"finish_reason":"stop"}]})];
    let body: String = ev.iter().map(|e| format!("data: {e}\n\n")).chain(std::iter::once("data: [DONE]\n\n".to_string())).collect();
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")).mount(&server).await;
    let (_tmp, g) = genius_with(&server, "custom", "/v1", "gpt-contract").await;
    let (_, a) = ask(&g, "hello").await;
    let u = &a.meta["report"]["usage"];
    assert_eq!(u["totalTokens"], 0, "no invented provider counts");
    assert_eq!(u["source"], "estimated");
    assert!(u["estimatedTokens"].as_u64().unwrap() > 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_model_call_budget_stops_the_task_as_blocked_with_a_truthful_message() {
    let server = MockServer::start().await;
    // The stand-in model never finishes: it keeps calling a tool.
    let looping = "<tool>{\"name\":\"runtime_info\",\"args\":{}}</tool>";
    let ev = [json!({"model":"m","choices":[{"index":0,"delta":{"content":looping},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":5,"total_tokens":10}})];
    let body: String = ev.iter().map(|e| format!("data: {e}\n\n")).chain(std::iter::once("data: [DONE]\n\n".to_string())).collect();
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")).mount(&server).await;
    let (_tmp, g) = genius_with(&server, "custom", "/v1", "m").await;
    let mut s = g.settings().unwrap();
    s.limits = TaskLimits { max_model_calls: 3, ..Default::default() };
    g.save_settings(&s).unwrap();
    let (_, a) = ask(&g, "keep going forever").await;
    let n = requests(&server).await.iter().filter(|r| r.method.as_str() == "POST").count();
    assert_eq!(n, 3, "exactly the budgeted number of model calls");
    let r = &a.meta["report"];
    assert_eq!(r["lifecycle"], "blocked", "{a:?}");
    assert_eq!(r["budget"]["stoppedBy"]["limit"], "maxModelCalls");
    assert_eq!(r["budget"]["stoppedBy"]["max"], 3);
    assert!(a.content.contains("maxModelCalls"), "the user is told why: {}", a.content);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_concurrent_task_limit_refuses_an_extra_task() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(ResponseTemplate::new(200).set_body_raw("data: [DONE]\n\n", "text/event-stream").set_delay(Duration::from_secs(5))).mount(&server).await;
    let (_tmp, g) = genius_with(&server, "custom", "/v1", "m").await;
    let mut s = g.settings().unwrap();
    s.limits.max_concurrent_tasks = 1;
    g.save_settings(&s).unwrap();
    let a = g.create_chat_conversation("chat", None).unwrap();
    let t = g.start_chat(&a.row.id, ChatRequest { message: "first".into(), attachment_ids: vec![], effort: None }).unwrap();
    let b = g.create_project("Other", "second project").unwrap();
    let e = g.start_agent(&b.id, genius_core::agent::AgentRequest { conversation_id: g.db.create_conversation(&b.id, "x").unwrap().id, message: "second".into(), ..Default::default() }).unwrap_err();
    assert!(e.to_string().contains("1 tasks are already running"), "{e}");
    g.cancel_task(&t.id).unwrap();
}

// ------------------------------------------------------------------------ diagnostics

fn step_of<'a>(r: &'a genius_core::provider_diag::DiagReport, id: &str) -> &'a genius_core::provider_diag::DiagStep {
    r.steps.iter().find(|s| s.id == id).unwrap_or_else(|| panic!("no step {id}"))
}

struct ModelAware;
impl Respond for ModelAware {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let body = String::from_utf8_lossy(&req.body).to_string();
        sse(&anthropic_stream(if body.contains("runtime_info") { "<tool>{\"name\": \"runtime_info\", \"args\": {}}</tool>" } else { "ready" }))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn diagnostics_walk_every_stage_on_a_healthy_provider() {
    use genius_core::provider_diag::StepStatus::*;
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/models")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"claude-contract-1","type":"model"}]}))).mount(&server).await;
    Mock::given(method("POST")).and(path("/v1/messages")).respond_with(ModelAware).mount(&server).await;
    let (_tmp, g) = genius_with(&server, "anthropic", "/v1", "claude-contract-1").await;
    let r = g.diagnose_provider(true).await.unwrap();
    assert!(r.ok, "{}", r.verdict);
    assert_eq!(r.integration_verified, "contract-tests", "the report is honest that Anthropic was not run live");
    assert_eq!(r.steps.iter().map(|s| s.id).collect::<Vec<_>>(), ["provider", "credentials", "base_url", "models", "model", "chat", "vision", "tools"]);
    assert_eq!(step_of(&r, "credentials").status, Ok);
    assert_eq!(step_of(&r, "models").status, Ok);
    assert_eq!(step_of(&r, "chat").status, Ok);
    assert_eq!(step_of(&r, "tools").status, Ok);
    assert_eq!(step_of(&r, "vision").status, Info);
    assert_eq!(step_of(&r, "base_url").status, Ok, "the stand-in's 127.0.0.1 host is allowed without a warning");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn diagnostics_without_a_key_make_no_network_call_and_say_what_to_do() {
    use genius_core::provider_diag::StepStatus::*;
    let server = MockServer::start().await;
    let (_tmp, g) = genius_with(&server, "anthropic", "/v1", "claude-contract-1").await;
    g.remove_api_key().unwrap();
    let r = g.diagnose_provider(false).await.unwrap();
    assert!(!r.ok);
    assert_eq!(step_of(&r, "credentials").status, Fail);
    assert!(step_of(&r, "credentials").hint.as_deref().unwrap().contains("Each provider has its own key"));
    for id in ["models", "model", "chat"] {
        assert_eq!(step_of(&r, id).status, Skipped, "{id}");
    }
    assert!(requests(&server).await.is_empty(), "nothing was sent without a key");
    assert!(r.verdict.starts_with("Not ready: API key"), "{}", r.verdict);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn diagnostics_classify_a_rejected_key_and_never_leak_it() {
    use genius_core::provider_diag::StepStatus::*;
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/models")).respond_with(ResponseTemplate::new(401).set_body_json(json!({"type":"error","error":{"type":"authentication_error","message":format!("invalid x-api-key: {KEY}")}}))).mount(&server).await;
    let (_tmp, g) = genius_with(&server, "anthropic", "/v1", "claude-contract-1").await;
    let r = g.diagnose_provider(true).await.unwrap();
    assert!(!r.ok);
    let m = step_of(&r, "models");
    assert_eq!(m.status, Fail);
    assert!(m.hint.as_deref().unwrap().contains("did not accept this key"));
    assert_eq!(step_of(&r, "chat").status, Skipped);
    assert_eq!(step_of(&r, "tools").status, Skipped);
    let json = serde_json::to_string(&r).unwrap();
    assert!(!json.contains("ABCDEF0123456789") && !json.contains(KEY), "the report must never contain the key: {json}");
    let events = serde_json::to_string(&g.bus.list(None, 500).unwrap()).unwrap();
    assert!(!events.contains("ABCDEF0123456789"), "nor may the event log");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn diagnostics_flag_a_model_the_key_cannot_use_and_suggest_similar_ones() {
    use genius_core::provider_diag::StepStatus::*;
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/models")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"claude-sonnet-contract"},{"id":"claude-haiku-contract"}]}))).mount(&server).await;
    let (_tmp, g) = genius_with(&server, "anthropic", "/v1", "claude-opus-contract").await;
    let r = g.diagnose_provider(false).await.unwrap();
    let m = step_of(&r, "model");
    assert_eq!(m.status, Fail);
    assert!(m.detail.contains("claude-opus-contract"));
    assert_eq!(step_of(&r, "chat").status, Skipped);
}
