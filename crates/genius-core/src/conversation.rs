//! Conversation-level operations: rewinding to an earlier message (with the file changes made
//! since undone, like checkpoints) and compacting long conversations into a summary.

use crate::app::Genius;
use crate::changes::ChangeStore;
use crate::error::{GeniusError, Result};
use crate::nvidia::ChatMessage;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use tokio_util::sync::CancellationToken;

impl Genius {
    /// Remove `message_id` and everything after it from a conversation. With `revert_files`,
    /// changes applied by those turns are reverted (newest first) and their pending proposals
    /// rejected, so the project returns to its state before that message.
    pub fn rewind_conversation(&self, conversation_id: &str, message_id: &str, revert_files: bool) -> Result<Value> {
        let conv = self.db.get_conversation(conversation_id)?;
        if self.active_tasks(Some(&conv.project_id)).iter().any(|t| t.kind == "agent") {
            return Err(GeniusError::Conflict("Genius is still working in this project. Stop it first, then rewind.".into()));
        }
        let msgs = self.db.list_conversation_messages(conversation_id, 100_000)?;
        let idx = msgs.iter().position(|m| m.id == message_id).ok_or_else(|| GeniusError::NotFound("message".into()))?;
        let later = &msgs[idx..];
        let tasks: BTreeSet<&str> = later.iter().filter_map(|m| m.task_id.as_deref()).collect();
        let mut reverted = 0;
        if revert_files && !tasks.is_empty() {
            let ws = self.workspace(&conv.project_id)?;
            let store = ChangeStore::new(&self.db);
            // newest first (list is newest first)
            for cs in store.list(&conv.project_id, 500)?.into_iter().filter(|c| c.task_id.as_deref().is_some_and(|t| tasks.contains(t))) {
                match cs.status.as_str() {
                    "proposed" => {
                        self.reject_changeset(&cs.id)?;
                    }
                    "applied" => {
                        match store.propose_revert(&ws, &cs.id, true) {
                            Ok(r) => {
                                self.apply_changeset_inner(&conv.project_id, None, &r.id)?;
                                reverted += 1;
                            }
                            Err(GeniusError::Validation(_)) => {} // already matches the earlier state
                            Err(e) => return Err(e),
                        }
                    }
                    _ => {}
                }
            }
        }
        let ids: Vec<String> = later.iter().map(|m| m.id.clone()).collect();
        let removed = self.db.delete_messages(conversation_id, &ids)?;
        Ok(json!({ "removedMessages": removed, "revertedChangesets": reverted }))
    }

    /// Summarise a long conversation so later requests carry the gist instead of every turn.
    pub async fn compact_conversation(&self, conversation_id: &str, cancel: &CancellationToken) -> Result<Value> {
        let conv = self.db.get_conversation(conversation_id)?;
        let mut msgs = self.db.list_conversation_messages(conversation_id, 400)?;
        if let Some(i) = msgs.iter().rposition(|m| m.kind == "summary") {
            msgs.drain(..i); // keep the previous summary as the start
        }
        if msgs.len() < 4 {
            return Err(GeniusError::Validation("This conversation is still short; there is nothing to compact yet.".into()));
        }
        let mut transcript = String::new();
        for m in &msgs {
            let files: Vec<&str> = m.meta.get("files").and_then(|f| f.as_array()).map(|a| a.iter().filter_map(|f| f.get("path").and_then(|p| p.as_str())).collect()).unwrap_or_default();
            let content: String = m.content.chars().take(2500).collect();
            transcript.push_str(&format!("[{}{}]\n{}\n{}\n\n", m.role, if m.kind == "summary" { " — earlier summary" } else { "" }, content, if files.is_empty() { String::new() } else { format!("(files changed: {})", files.join(", ")) }));
        }
        let transcript: String = transcript.chars().rev().take(80_000).collect::<Vec<_>>().into_iter().rev().collect();
        let messages = vec![
            ChatMessage::system("You compress a conversation between a user and Genius, an AI software engineer building the user's app, so work can continue without the full transcript. Write a faithful summary in the user's language, under 350 words, Markdown bullets under these headings: Goal, What exists now (features, key files), Decisions & preferences (design, tech, things the user liked or rejected), Open issues / next steps. Facts only — no new suggestions."),
            ChatMessage::user(format!("CONVERSATION:\n{transcript}")),
        ];
        let task = format!("compact-{}", uuid::Uuid::new_v4());
        let (res, _) = self.ai_call(&conv.project_id, &task, "Summarising the conversation", &messages, false, cancel).await?;
        let summary = res.content.trim();
        if summary.is_empty() {
            return Err(GeniusError::Validation("The model returned an empty summary; try again.".into()));
        }
        let m = self.db.add_message_in(conversation_id, &conv.project_id, None, "assistant", "summary", &format!("**Conversation summary** (earlier messages are kept above but no longer sent to the model)\n\n{summary}"), &json!({ "compactedMessages": msgs.len() }))?;
        Ok(json!({ "messageId": m.id, "compactedMessages": msgs.len() }))
    }
}
