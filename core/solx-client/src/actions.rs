use async_trait::async_trait;
use serde_json::Value;
use solx_surface::auth::{EXEC_SCOPE_HEADER, NO_PRINCIPAL_HEADER};
use solx_surface::entities::{Action, ActionExecResult, ActionInput};
use solx_surface::error::Result;
use solx_surface::managers::ActionManager;
use solx_surface::query::{ActionSearchQuery, ListOptions, Page, PathFacet};
use solx_surface::Principal;

use crate::error::{read_response, transport_err};
use crate::http::{
    collection_url, delete, entity_crud_url, entity_url, explicit_mode, get_json, get_json_query,
    post_json, put_json,
};

/// HTTP-proxy [`ActionManager`] talking to a `solx-server`. `exec()` runs
/// entirely server-side — including WASM execution and every `Internal`
/// built-in (entity CRUD, search, secrets, OAuth loopback) — since that's
/// where the one `LocalActionManager` this proxies to actually lives.
pub struct RemoteActionManager {
    base_url: String,
    token: String,
    http: reqwest::Client,
}

impl RemoteActionManager {
    /// A manager with its own client, trusting the roots `SOLX_CA_FILE`
    /// names besides the built-in and OS ones, or only the root a `#pin=`
    /// on `base_url` names. Prefer [`with_client`](Self::with_client) with
    /// one shared [`crate::http_client`].
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        let (base_url, http) = crate::tls::client_for_url(base_url.into());
        Self::with_client(base_url, token, http)
    }

    /// A manager using `http`, normally one [`crate::http_client`] shared by
    /// every manager talking to the same server.
    pub fn with_client(
        base_url: impl Into<String>,
        token: impl Into<String>,
        http: reqwest::Client,
    ) -> Self {
        RemoteActionManager {
            base_url: base_url.into(),
            token: token.into(),
            http,
        }
    }
}

#[async_trait]
impl ActionManager for RemoteActionManager {
    async fn save(&self, path: &str, name: &str, input: ActionInput) -> Result<Action> {
        let url = entity_crud_url(&self.base_url, "actions", path, name)?;
        put_json(&self.http, &self.token, url, &input).await
    }

    async fn get(&self, path: &str, name: &str) -> Result<Action> {
        let url = entity_crud_url(&self.base_url, "actions", path, name)?;
        get_json(&self.http, &self.token, url).await
    }

    async fn delete(&self, path: &str, name: &str) -> Result<()> {
        let url = entity_crud_url(&self.base_url, "actions", path, name)?;
        delete(&self.http, &self.token, url).await
    }

    async fn list(&self, mut opts: ListOptions) -> Result<Page<Action>> {
        let url = collection_url(&self.base_url, "actions")?;
        explicit_mode(&mut opts.exclude_mode);
        get_json_query(&self.http, &self.token, url, &opts).await
    }

    async fn paths(&self, mut opts: ListOptions) -> Result<Page<PathFacet>> {
        let url = collection_url(&self.base_url, "actions-paths")?;
        explicit_mode(&mut opts.exclude_mode);
        get_json_query(&self.http, &self.token, url, &opts).await
    }

    /// `GET /actions-search` — a top-level route rather than
    /// `/actions/search`, for the same reason document search lives at
    /// `/search` rather than `/docs/search`; see `solx_client::docs`.
    async fn search(&self, mut query: ActionSearchQuery) -> Result<Page<Action>> {
        let url = collection_url(&self.base_url, "actions-search")?;
        query.list.exclude_mode = query.effective_mode();
        query.exclude_hidden = false;
        explicit_mode(&mut query.list.exclude_mode);
        get_json_query(&self.http, &self.token, url, &query).await
    }

    /// `POST` on the action's own URL, with the params as the body, asking
    /// the server to run it with no principal ([`NO_PRINCIPAL_HEADER`]), as
    /// `exec` means locally. The server drops only the root token's
    /// principal; a client token keeps its own, and with it its scopes.
    async fn exec(&self, path: &str, name: &str, params: Value) -> Result<ActionExecResult> {
        let url = entity_url(&self.base_url, "actions", path, name)?;
        let resp = self
            .http
            .post(url)
            .bearer_auth(&self.token)
            .header(NO_PRINCIPAL_HEADER, "none")
            .json(&params)
            .send()
            .await
            .map_err(transport_err)?;
        read_response(resp).await
    }

    /// The same request without [`NO_PRINCIPAL_HEADER`]. The principal
    /// itself never crosses the wire: the server re-derives it from this
    /// client's bearer token, so the one passed in here has nothing to
    /// contribute beyond "run with one".
    async fn exec_for(
        &self,
        _principal: &Principal,
        path: &str,
        name: &str,
        params: Value,
    ) -> Result<ActionExecResult> {
        let url = entity_url(&self.base_url, "actions", path, name)?;
        post_json(&self.http, &self.token, url, &params).await
    }

    /// The same request with [`EXEC_SCOPE_HEADER`], so a local MCP server
    /// proxying here keeps the whole chain in the tooling scope on the
    /// server. A `None` principal also sends [`NO_PRINCIPAL_HEADER`], as
    /// `exec` does.
    async fn exec_tooling(
        &self,
        principal: Option<&Principal>,
        path: &str,
        name: &str,
        params: Value,
    ) -> Result<ActionExecResult> {
        let url = entity_url(&self.base_url, "actions", path, name)?;
        let mut req = self
            .http
            .post(url)
            .bearer_auth(&self.token)
            .header(EXEC_SCOPE_HEADER, "tooling");
        if principal.is_none() {
            req = req.header(NO_PRINCIPAL_HEADER, "none");
        }
        let resp = req.json(&params).send().await.map_err(transport_err)?;
        read_response(resp).await
    }
}
