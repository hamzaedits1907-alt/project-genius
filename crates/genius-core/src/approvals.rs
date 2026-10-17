//! Approval gate for sensitive operations. A request is shown in the activity feed as
//! "Awaiting approval" with the exact details of what will happen; the operation proceeds
//! only after the user explicitly approves it (or when the user's policy allows it).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::error::{GeniusError, Result};
use crate::events::{EventBus, EventKind, EventStatus};
use crate::settings::Policy;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub id: String,
    pub project_id: Option<String>,
    pub task_id: Option<String>,
    pub category: String,
    pub title: String,
    pub detail: Value,
    pub event_id: String,
}

type Pending = HashMap<String, (ApprovalRequest, oneshot::Sender<bool>)>;

#[derive(Clone, Default)]
pub struct Approvals {
    pending: Arc<Mutex<Pending>>,
}

impl Approvals {
    pub fn pending(&self, project_id: Option<&str>) -> Vec<ApprovalRequest> {
        self.pending.lock().unwrap().values().map(|(r, _)| r.clone()).filter(|r| project_id.is_none() || r.project_id.as_deref() == project_id).collect()
    }

    pub fn respond(&self, id: &str, approved: bool) -> Result<()> {
        let entry = self.pending.lock().unwrap().remove(id);
        match entry {
            Some((_, tx)) => {
                let _ = tx.send(approved);
                Ok(())
            }
            None => Err(GeniusError::NotFound("approval request (it may have expired or been cancelled)".into())),
        }
    }

    /// Ask for approval (unless policy is Auto). Returns Ok(()) when approved.
    #[allow(clippy::too_many_arguments)]
    pub async fn request(&self, bus: &EventBus, project_id: Option<&str>, task_id: Option<&str>, category: &str, policy: Policy, title: &str, detail: Value, cancel: &CancellationToken) -> Result<()> {
        if policy == Policy::Auto {
            bus.record(
                project_id,
                task_id,
                EventKind::Approval,
                EventStatus::Completed,
                format!("Auto-approved by your settings: {title}"),
                json!({ "category": category, "policy": "auto", "detail": detail }),
            );
            return Ok(());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let mut ev = bus.start(project_id, task_id, EventKind::Approval, EventStatus::AwaitingApproval, title, json!({ "approvalId": id, "category": category, "detail": detail }));
        let (tx, rx) = oneshot::channel();
        let req = ApprovalRequest {
            id: id.clone(),
            project_id: project_id.map(String::from),
            task_id: task_id.map(String::from),
            category: category.into(),
            title: title.into(),
            detail,
            event_id: ev.id().to_string(),
        };
        self.pending.lock().unwrap().insert(id.clone(), (req, tx));
        let decision = tokio::select! {
            r = rx => r.unwrap_or(false),
            _ = cancel.cancelled() => {
                self.pending.lock().unwrap().remove(&id);
                ev.update(EventStatus::Cancelled, json!({ "approved": false, "note": "Task cancelled" }));
                return Err(GeniusError::Cancelled);
            }
        };
        if decision {
            ev.update(EventStatus::Completed, json!({ "approved": true }));
            Ok(())
        } else {
            ev.update(EventStatus::Cancelled, json!({ "approved": false }));
            Err(GeniusError::ApprovalDenied(title.to_string()))
        }
    }

    pub fn cancel_all(&self) {
        self.pending.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::events::NullEmitter;

    fn bus() -> EventBus {
        EventBus::new(Arc::new(Db::open_in_memory().unwrap()), Arc::new(NullEmitter))
    }

    #[tokio::test]
    async fn waits_for_user_decision() {
        let a = Approvals::default();
        let b = bus();
        let a2 = a.clone();
        let h = tokio::spawn(async move { a2.request(&b, None, None, "dependency_install", Policy::Ask, "npm install", json!({}), &CancellationToken::new()).await });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let p = a.pending(None);
        assert_eq!(p.len(), 1);
        a.respond(&p[0].id, false).unwrap();
        assert!(matches!(h.await.unwrap(), Err(GeniusError::ApprovalDenied(_))));
    }

    #[tokio::test]
    async fn approve_and_auto_policy() {
        let a = Approvals::default();
        let b = bus();
        assert!(a.request(&b, None, None, "x", Policy::Auto, "t", json!({}), &CancellationToken::new()).await.is_ok());
        let a2 = a.clone();
        let b2 = b.clone();
        let h = tokio::spawn(async move { a2.request(&b2, None, None, "x", Policy::Ask, "t", json!({}), &CancellationToken::new()).await });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let id = a.pending(None)[0].id.clone();
        a.respond(&id, true).unwrap();
        assert!(h.await.unwrap().is_ok());
        assert!(a.respond(&id, true).is_err(), "single use");
    }

    #[tokio::test]
    async fn cancellation_releases_request() {
        let a = Approvals::default();
        let b = bus();
        let c = CancellationToken::new();
        c.cancel();
        assert!(matches!(a.request(&b, None, None, "x", Policy::Ask, "t", json!({}), &c).await, Err(GeniusError::Cancelled)));
        assert!(a.pending(None).is_empty());
    }
}
