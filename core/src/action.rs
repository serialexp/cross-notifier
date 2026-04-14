//! Server-side execution of exclusive-notification actions. When an action
//! fires the server either opens the URL locally (useless on a headless
//! server, so we just report it) or issues the HTTP request on behalf of
//! the clicking client.

use std::time::Duration;

use reqwest::Method;

use crate::protocol::Action;

#[derive(Debug, thiserror::Error)]
pub enum ActionError {
    #[error("invalid http method: {0}")]
    InvalidMethod(String),
    #[error("build request: {0}")]
    Build(String),
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("request returned status {0}")]
    BadStatus(u16),
}

/// Executes an HTTP action. `Action.open == true` is skipped at the call
/// site — the caller decides whether a headless server should ignore those.
pub async fn execute_http_action(client: &reqwest::Client, a: &Action) -> Result<(), ActionError> {
    let method = Method::from_bytes(a.effective_method().as_bytes())
        .map_err(|e| ActionError::InvalidMethod(e.to_string()))?;

    let mut req = client.request(method, &a.url);
    for (k, v) in &a.headers {
        req = req.header(k, v);
    }
    if !a.body.is_empty() {
        req = req.body(a.body.clone());
    }

    let resp = req
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(ActionError::from)?;
    let status = resp.status();
    if !status.is_success() {
        return Err(ActionError::BadStatus(status.as_u16()));
    }
    Ok(())
}
