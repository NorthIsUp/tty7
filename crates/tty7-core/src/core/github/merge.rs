//! Merging a pull request: now, by one of the methods the repository allows,
//! or on auto-merge once its requirements pass.

use serde::Deserialize;

use super::api::{ApiError, Transport, decode, repo_path};
use super::remote::RepoSlug;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl MergeMethod {
    fn as_rest(self) -> &'static str {
        match self {
            MergeMethod::Merge => "merge",
            MergeMethod::Squash => "squash",
            MergeMethod::Rebase => "rebase",
        }
    }

    fn as_graphql(self) -> &'static str {
        match self {
            MergeMethod::Merge => "MERGE",
            MergeMethod::Squash => "SQUASH",
            MergeMethod::Rebase => "REBASE",
        }
    }
}

/// What the repository's settings allow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeRules {
    /// In GitHub's order: merge commit, squash, rebase.
    pub methods: Vec<MergeMethod>,
    pub auto_merge: bool,
}

/// GitHub only sends the `allow_*` settings to a token that can push. Without
/// them every method is offered and GitHub's refusal, if any, is shown; no
/// auto-merge is offered, since it may well be off.
#[derive(Default, Deserialize)]
struct RawRepo {
    allow_merge_commit: Option<bool>,
    allow_squash_merge: Option<bool>,
    allow_rebase_merge: Option<bool>,
    allow_auto_merge: Option<bool>,
}

impl Default for MergeRules {
    fn default() -> MergeRules {
        RawRepo::default().into_rules()
    }
}

impl RawRepo {
    fn into_rules(self) -> MergeRules {
        let methods = [
            (self.allow_merge_commit, MergeMethod::Merge),
            (self.allow_squash_merge, MergeMethod::Squash),
            (self.allow_rebase_merge, MergeMethod::Rebase),
        ]
        .into_iter()
        .filter(|(allowed, _)| allowed.unwrap_or(true))
        .map(|(_, m)| m)
        .collect();
        MergeRules {
            methods,
            auto_merge: self.allow_auto_merge.unwrap_or(false),
        }
    }
}

pub fn merge_rules(t: &dyn Transport, slug: &RepoSlug) -> Result<MergeRules, ApiError> {
    Ok(decode::<RawRepo>(&t.get(&repo_path(slug))?)?.into_rules())
}

pub fn merge(
    t: &dyn Transport,
    slug: &RepoSlug,
    number: u64,
    method: MergeMethod,
) -> Result<(), ApiError> {
    let payload = serde_json::json!({ "merge_method": method.as_rest() });
    t.put(
        &format!("{}/pulls/{number}/merge", repo_path(slug)),
        payload.to_string().as_bytes(),
    )
    .map(|_| ())
}

/// `Some(method)` turns auto-merge on, `None` turns it off. REST cannot do
/// either, so this is GraphQL, keyed by the pull request's `node_id`.
pub fn set_auto_merge(
    t: &dyn Transport,
    node_id: &str,
    method: Option<MergeMethod>,
) -> Result<(), ApiError> {
    let payload = match method {
        Some(method) => serde_json::json!({
            "query": "mutation($id: ID!, $method: PullRequestMergeMethod!) { \
                enablePullRequestAutoMerge(input: {pullRequestId: $id, mergeMethod: $method}) \
                { clientMutationId } }",
            "variables": { "id": node_id, "method": method.as_graphql() },
        }),
        None => serde_json::json!({
            "query": "mutation($id: ID!) { \
                disablePullRequestAutoMerge(input: {pullRequestId: $id}) { clientMutationId } }",
            "variables": { "id": node_id },
        }),
    };
    let reply = t.post("/graphql", payload.to_string().as_bytes())?;
    graphql_errors(&reply.body)
}

/// GraphQL answers a refused mutation with a 200 and an `errors` list.
fn graphql_errors(body: &[u8]) -> Result<(), ApiError> {
    #[derive(Deserialize)]
    struct Error {
        message: String,
    }
    #[derive(Deserialize)]
    struct Response {
        #[serde(default)]
        errors: Vec<Error>,
    }
    let errors = serde_json::from_slice::<Response>(body)
        .map_err(|e| ApiError::Decode(e.to_string()))?
        .errors;
    if errors.is_empty() {
        return Ok(());
    }
    Err(ApiError::Rejected(
        errors
            .into_iter()
            .map(|e| e.message)
            .collect::<Vec<_>>()
            .join("; "),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::github::api::Reply;
    use crate::core::github::api::tests::{Fixture, slug};
    use std::sync::Mutex;

    /// Records every write and answers each with `reply`.
    struct Recorder {
        sent: Mutex<Vec<(&'static str, String, serde_json::Value)>>,
        reply: &'static str,
    }

    impl Recorder {
        fn answering(reply: &'static str) -> Recorder {
            Recorder {
                sent: Mutex::new(Vec::new()),
                reply,
            }
        }

        fn record(&self, method: &'static str, path: &str, body: &[u8]) -> Result<Reply, ApiError> {
            let body = serde_json::from_slice(body).unwrap();
            self.sent
                .lock()
                .unwrap()
                .push((method, path.to_string(), body));
            Ok(Reply {
                body: self.reply.as_bytes().to_vec(),
                has_next: false,
            })
        }
    }

    impl Transport for Recorder {
        fn get(&self, _: &str) -> Result<Reply, ApiError> {
            Err(ApiError::NotFound)
        }

        fn post(&self, path: &str, body: &[u8]) -> Result<Reply, ApiError> {
            self.record("POST", path, body)
        }

        fn put(&self, path: &str, body: &[u8]) -> Result<Reply, ApiError> {
            self.record("PUT", path, body)
        }

        fn authenticated(&self) -> bool {
            true
        }
    }

    #[test]
    fn a_merge_puts_its_method_to_the_pulls_merge() {
        let t = Recorder::answering(r#"{"merged": true}"#);
        merge(&t, &slug(), 13, MergeMethod::Squash).unwrap();
        assert_eq!(
            *t.sent.lock().unwrap(),
            vec![(
                "PUT",
                "/repos/l0ng-ai/tty7/pulls/13/merge".to_string(),
                serde_json::json!({"merge_method": "squash"}),
            )]
        );
    }

    #[test]
    fn auto_merge_is_turned_on_and_off_through_graphql() {
        let t = Recorder::answering(r#"{"data": {}}"#);
        set_auto_merge(&t, "PR_kw13", Some(MergeMethod::Rebase)).unwrap();
        set_auto_merge(&t, "PR_kw13", None).unwrap();
        let sent = t.sent.lock().unwrap();
        let (method, path, on) = &sent[0];
        assert_eq!((*method, path.as_str()), ("POST", "/graphql"));
        assert!(
            on["query"]
                .as_str()
                .unwrap()
                .contains("enablePullRequestAutoMerge")
        );
        assert_eq!(
            on["variables"],
            serde_json::json!({"id": "PR_kw13", "method": "REBASE"})
        );
        let off = &sent[1].2;
        assert!(
            off["query"]
                .as_str()
                .unwrap()
                .contains("disablePullRequestAutoMerge")
        );
        assert_eq!(off["variables"], serde_json::json!({"id": "PR_kw13"}));
    }

    #[test]
    fn graphql_errors_in_a_200_are_a_rejection() {
        let t = Recorder::answering(
            r#"{"data": null, "errors": [{"message": "Pull request is in clean status"},
                {"message": "second"}]}"#,
        );
        assert_eq!(
            set_auto_merge(&t, "PR_kw13", Some(MergeMethod::Merge)),
            Err(ApiError::Rejected(
                "Pull request is in clean status; second".into()
            ))
        );
    }

    #[test]
    fn repo_rules_offer_what_is_allowed_and_everything_when_unsaid() {
        let mut t = Fixture::new();
        t.on(
            "/repos/l0ng-ai/tty7",
            r#"{"allow_merge_commit": false, "allow_squash_merge": true,
                "allow_rebase_merge": true, "allow_auto_merge": true}"#,
            false,
        );
        assert_eq!(
            merge_rules(&t, &slug()),
            Ok(MergeRules {
                methods: vec![MergeMethod::Squash, MergeMethod::Rebase],
                auto_merge: true,
            })
        );
        t.on("/repos/l0ng-ai/tty7", r#"{"name": "tty7"}"#, false);
        assert_eq!(
            merge_rules(&t, &slug()),
            Ok(MergeRules {
                methods: vec![MergeMethod::Merge, MergeMethod::Squash, MergeMethod::Rebase],
                auto_merge: false,
            })
        );
        assert_eq!(merge_rules(&t, &slug()).unwrap(), MergeRules::default());
    }

    #[test]
    fn a_read_only_transport_refuses_a_merge() {
        let t = Fixture::new();
        assert_eq!(
            merge(&t, &slug(), 13, MergeMethod::Merge),
            Err(ApiError::Http(405))
        );
    }
}
