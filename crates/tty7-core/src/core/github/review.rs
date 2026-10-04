//! Submitting a review on a pull request: a comment, or an approval.

use super::api::{ApiError, Transport, repo_path};
use super::remote::RepoSlug;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewEvent {
    Comment,
    Approve,
}

impl ReviewEvent {
    fn as_str(self) -> &'static str {
        match self {
            ReviewEvent::Comment => "COMMENT",
            ReviewEvent::Approve => "APPROVE",
        }
    }
}

/// GitHub rejects a comment review with an empty body; an approval may have
/// none.
pub fn submit_review(
    t: &dyn Transport,
    slug: &RepoSlug,
    number: u64,
    event: ReviewEvent,
    body: &str,
) -> Result<(), ApiError> {
    let payload = serde_json::json!({ "body": body, "event": event.as_str() });
    t.post(
        &format!("{}/pulls/{number}/reviews", repo_path(slug)),
        payload.to_string().as_bytes(),
    )
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::github::api::Reply;
    use crate::core::github::api::tests::slug;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorder {
        posted: Mutex<Vec<(String, serde_json::Value)>>,
    }

    impl Transport for Recorder {
        fn get(&self, _: &str) -> Result<Reply, ApiError> {
            Err(ApiError::NotFound)
        }

        fn post(&self, path: &str, body: &[u8]) -> Result<Reply, ApiError> {
            let body = serde_json::from_slice(body).unwrap();
            self.posted.lock().unwrap().push((path.to_string(), body));
            Ok(Reply::default())
        }

        fn authenticated(&self) -> bool {
            true
        }
    }

    #[test]
    fn a_review_posts_its_event_and_body_to_the_pulls_reviews() {
        let t = Recorder::default();
        submit_review(&t, &slug(), 13, ReviewEvent::Approve, "").unwrap();
        submit_review(&t, &slug(), 13, ReviewEvent::Comment, "looks \"good\"").unwrap();
        let posted = t.posted.lock().unwrap();
        assert_eq!(
            *posted,
            vec![
                (
                    "/repos/l0ng-ai/tty7/pulls/13/reviews".to_string(),
                    serde_json::json!({"body": "", "event": "APPROVE"}),
                ),
                (
                    "/repos/l0ng-ai/tty7/pulls/13/reviews".to_string(),
                    serde_json::json!({"body": "looks \"good\"", "event": "COMMENT"}),
                ),
            ]
        );
    }

    #[test]
    fn a_read_only_transport_refuses_a_review() {
        let t = crate::core::github::api::tests::Fixture::new();
        assert_eq!(
            submit_review(&t, &slug(), 13, ReviewEvent::Comment, "x"),
            Err(ApiError::Http(405))
        );
    }
}
