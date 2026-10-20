//! Account sync, desktop side: the outbox triggers, collect/ack, and merging server changes into the local
//! database. No network: the interface does the HTTP part; these tests drive the same local functions it calls.

use genius_core::app::Genius;
use genius_core::events::NullEmitter;
use genius_core::secrets::MemoryStore;
use genius_core::sync::{RemoteBatch, SyncConv, SyncMsg};
use serde_json::json;
use std::sync::Arc;

fn genius() -> (tempfile::TempDir, Arc<Genius>) {
    let tmp = tempfile::tempdir().unwrap();
    let g = Genius::new(&tmp.path().join("data"), Arc::new(NullEmitter), Arc::new(MemoryStore::default())).unwrap();
    (tmp, g)
}

fn remote_conv(id: &str, title: &str, meta: &str) -> SyncConv {
    SyncConv { id: id.into(), mode: "chat".into(), title: title.into(), origin: json!(null), created_at: "2026-10-01T10:00:00.000000+00:00".into(), updated_at: "2026-10-01T10:00:05.000000+00:00".into(), meta_updated_at: meta.into(), deleted_at: None }
}
fn remote_msg(id: &str, conv: &str, role: &str, content: &str, created: &str, seq: i64) -> SyncMsg {
    SyncMsg { id: id.into(), conversation_id: conv.into(), role: role.into(), kind: "chat".into(), content: content.into(), meta: json!({}), task_id: None, created_at: created.into(), updated_at: created.into(), deleted_at: None, seq }
}

#[tokio::test(flavor = "multi_thread")]
async fn local_changes_are_collected_once_and_acknowledged() {
    let (_t, g) = genius();
    let c = g.create_chat_conversation("chat", Some("First")).unwrap();
    let ws = g.general_workspace().unwrap();
    g.db.add_message_in(&c.row.id, &ws.id, None, "user", "chat", "hello", &json!({})).unwrap();
    g.db.add_message_in(&c.row.id, &ws.id, None, "assistant", "answer", "hi there", &json!({"model":"x"})).unwrap();
    let b = g.sync_collect(100).unwrap();
    assert_eq!(b.conversations.len(), 1);
    assert_eq!(b.conversations[0].title, "First");
    assert_eq!(b.conversations[0].mode, "chat");
    assert_eq!(b.messages.len(), 2);
    assert!(b.messages.iter().any(|m| m.content == "hi there" && m.kind == "answer" && m.meta["model"] == "x"));
    g.sync_ack(&b.keys).unwrap();
    assert_eq!(g.sync_pending().unwrap(), 0);
    assert!(g.sync_collect(100).unwrap().keys.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_made_while_the_upload_was_in_flight_stays_queued() {
    let (_t, g) = genius();
    let c = g.create_chat_conversation("chat", Some("A")).unwrap();
    let b = g.sync_collect(100).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    g.db.rename_conversation(&c.row.id, "A renamed").unwrap(); // while "uploading"
    g.sync_ack(&b.keys).unwrap();
    let again = g.sync_collect(100).unwrap();
    assert_eq!(again.conversations.len(), 1);
    assert_eq!(again.conversations[0].title, "A renamed");
    assert!(again.conversations[0].meta_updated_at > b.conversations[0].meta_updated_at, "a rename moves the title's age");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_message_does_not_make_the_title_look_newer() {
    let (_t, g) = genius();
    let c = g.create_chat_conversation("chat", Some("Title")).unwrap();
    let first = g.sync_collect(100).unwrap().conversations[0].meta_updated_at.clone();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let ws = g.general_workspace().unwrap();
    g.db.add_message_in(&c.row.id, &ws.id, None, "user", "chat", "later", &json!({})).unwrap();
    let after = g.sync_collect(100).unwrap();
    assert_eq!(after.conversations[0].meta_updated_at, first, "only a rename may overwrite another device's title");
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_and_rewinding_become_tombstones() {
    let (_t, g) = genius();
    let c = g.create_chat_conversation("chat", Some("T")).unwrap();
    let ws = g.general_workspace().unwrap();
    let m1 = g.db.add_message_in(&c.row.id, &ws.id, None, "user", "chat", "one", &json!({})).unwrap();
    let m2 = g.db.add_message_in(&c.row.id, &ws.id, None, "assistant", "answer", "two", &json!({})).unwrap();
    let b = g.sync_collect(100).unwrap();
    g.sync_ack(&b.keys).unwrap();
    g.db.delete_messages(&c.row.id, std::slice::from_ref(&m2.id)).unwrap(); // rewind
    let r = g.sync_collect(100).unwrap();
    assert_eq!(r.messages.len(), 1);
    assert_eq!(r.messages[0].id, m2.id);
    assert!(r.messages[0].deleted_at.is_some());
    g.sync_ack(&r.keys).unwrap();
    g.db.delete_conversation(&c.row.id).unwrap();
    let d = g.sync_collect(100).unwrap();
    assert!(d.conversations.iter().any(|x| x.id == c.row.id && x.deleted_at.is_some()), "conversation tombstone");
    assert!(d.messages.iter().all(|x| x.id != m1.id), "messages of a deleted conversation are not tombstoned one by one");
    g.sync_ack(&d.keys).unwrap();
    assert_eq!(g.sync_pending().unwrap(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn server_changes_are_merged_in_order_without_echoing_back() {
    let (_t, g) = genius();
    let id = "00000000-0000-4000-8000-000000000001";
    let batch = RemoteBatch {
        conversations: vec![remote_conv(id, "From the web", "2026-10-01T10:00:00.000000+00:00")],
        messages: vec![
            remote_msg("00000000-0000-4000-8000-0000000000a2", id, "assistant", "second", "2026-10-01T10:00:01.500000+00:00", 2),
            remote_msg("00000000-0000-4000-8000-0000000000a1", id, "user", "first", "2026-10-01T10:00:01.500000+00:00", 1),
            remote_msg("00000000-0000-4000-8000-0000000000a3", id, "user", "third", "2026-10-01T10:00:09.000000+00:00", 3),
        ],
    };
    let rep = g.sync_apply(&batch).unwrap();
    assert_eq!((rep.conversations, rep.messages), (1, 3));
    assert_eq!(g.sync_pending().unwrap(), 0, "nothing the server sent is queued for upload");
    let list = g.list_chat_conversations(None, None).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].row.title, "From the web");
    let msgs = g.db.list_conversation_messages(id, 50).unwrap();
    assert_eq!(msgs.iter().map(|m| m.content.as_str()).collect::<Vec<_>>(), vec!["first", "second", "third"]);
    assert!(msgs[0].created_at.ends_with('Z') && msgs[0].created_at.contains(".500"), "timestamps use the local format: {}", msgs[0].created_at);
    // the same batch again changes nothing (retries and overlapping pulls are harmless)
    let again = g.sync_apply(&batch).unwrap();
    assert_eq!(again.messages, 0);
    assert_eq!(g.db.list_conversation_messages(id, 50).unwrap().len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_newest_title_wins_and_server_deletions_apply() {
    let (_t, g) = genius();
    let id = "00000000-0000-4000-8000-000000000002";
    g.sync_apply(&RemoteBatch { conversations: vec![remote_conv(id, "Old", "2026-10-01T10:00:00.000000+00:00")], messages: vec![] }).unwrap();
    g.sync_apply(&RemoteBatch { conversations: vec![remote_conv(id, "Renamed elsewhere", "2026-10-02T10:00:00.000000+00:00")], messages: vec![] }).unwrap();
    assert_eq!(g.chat_conversation(id).unwrap().row.title, "Renamed elsewhere");
    g.sync_apply(&RemoteBatch { conversations: vec![remote_conv(id, "Stale", "2026-10-01T12:00:00.000000+00:00")], messages: vec![] }).unwrap();
    assert_eq!(g.chat_conversation(id).unwrap().row.title, "Renamed elsewhere", "an older rename never overwrites a newer one");
    assert_eq!(g.sync_pending().unwrap(), 0);

    let mut gone = remote_conv(id, "", "2026-10-03T10:00:00.000000+00:00");
    gone.deleted_at = Some("2026-10-03T10:00:00.000000+00:00".into());
    let rep = g.sync_apply(&RemoteBatch { conversations: vec![gone], messages: vec![] }).unwrap();
    assert_eq!(rep.deleted, 1);
    assert!(g.list_chat_conversations(None, None).unwrap().is_empty());
    assert_eq!(g.sync_pending().unwrap(), 0, "a deletion that came from the server is not sent back");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unsent_local_edit_is_not_overwritten_by_an_older_server_copy() {
    let (_t, g) = genius();
    let c = g.create_chat_conversation("chat", Some("T")).unwrap();
    let ws = g.general_workspace().unwrap();
    let m = g.db.add_message_in(&c.row.id, &ws.id, None, "assistant", "answer", "local text", &json!({})).unwrap();
    let mut stale = remote_msg(&m.id, &c.row.id, "assistant", "server text", &m.created_at, 1);
    stale.updated_at = "2099-01-01T00:00:00+00:00".into();
    g.sync_apply(&RemoteBatch { conversations: vec![], messages: vec![stale.clone()] }).unwrap();
    assert_eq!(g.db.list_conversation_messages(&c.row.id, 10).unwrap()[0].content, "local text", "queued local changes win until they are uploaded");
    let b = g.sync_collect(100).unwrap();
    g.sync_ack(&b.keys).unwrap();
    g.sync_apply(&RemoteBatch { conversations: vec![], messages: vec![stale] }).unwrap();
    assert_eq!(g.db.list_conversation_messages(&c.row.id, 10).unwrap()[0].content, "server text", "once uploaded, a newer server edit applies");
}

#[tokio::test(flavor = "multi_thread")]
async fn first_sync_queues_every_existing_conversation() {
    let (_t, g) = genius();
    let c = g.create_chat_conversation("research", Some("Old research")).unwrap();
    let ws = g.general_workspace().unwrap();
    g.db.add_message_in(&c.row.id, &ws.id, None, "user", "chat", "q", &json!({})).unwrap();
    let b = g.sync_collect(100).unwrap();
    g.sync_ack(&b.keys).unwrap();
    assert_eq!(g.sync_pending().unwrap(), 0);
    let n = g.sync_enqueue_all().unwrap();
    assert_eq!(n, 2);
    let again = g.sync_collect(100).unwrap();
    assert_eq!(again.conversations[0].mode, "research");
    assert_eq!(again.messages.len(), 1);
}
