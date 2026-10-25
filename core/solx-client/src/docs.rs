use async_trait::async_trait;
use solx_surface::entities::{Document, DocumentInput};
use solx_surface::error::Result;
use solx_surface::managers::DocManager;
use solx_surface::query::{ListOptions, Page, PathFacet, SearchQuery};

use crate::http::{
    collection_url, delete, entity_crud_url, explicit_mode, get_json, get_json_query, put_json,
};

/// HTTP-proxy [`DocManager`] talking to a `solx-server`.
pub struct RemoteDocManager {
    base_url: String,
    token: String,
    http: reqwest::Client,
}

impl RemoteDocManager {
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
        RemoteDocManager {
            base_url: base_url.into(),
            token: token.into(),
            http,
        }
    }
}

#[async_trait]
impl DocManager for RemoteDocManager {
    async fn save(&self, path: &str, name: &str, input: DocumentInput) -> Result<Document> {
        let url = entity_crud_url(&self.base_url, "docs", path, name)?;
        put_json(&self.http, &self.token, url, &input).await
    }

    async fn get(&self, path: &str, name: &str) -> Result<Document> {
        let url = entity_crud_url(&self.base_url, "docs", path, name)?;
        get_json(&self.http, &self.token, url).await
    }

    async fn delete(&self, path: &str, name: &str) -> Result<()> {
        let url = entity_crud_url(&self.base_url, "docs", path, name)?;
        delete(&self.http, &self.token, url).await
    }

    async fn list(&self, mut opts: ListOptions) -> Result<Page<Document>> {
        let url = collection_url(&self.base_url, "docs")?;
        explicit_mode(&mut opts.exclude_mode);
        get_json_query(&self.http, &self.token, url, &opts).await
    }

    async fn paths(&self, mut opts: ListOptions) -> Result<Page<PathFacet>> {
        let url = collection_url(&self.base_url, "docs-paths")?;
        explicit_mode(&mut opts.exclude_mode);
        get_json_query(&self.http, &self.token, url, &opts).await
    }

    /// `GET /search` — a top-level route rather than `/docs/search`, which
    /// as a static sibling of the `/docs/{*ref}` catch-all would shadow any
    /// document named `search` at the root.
    async fn search(&self, mut query: SearchQuery) -> Result<Page<Document>> {
        let url = collection_url(&self.base_url, "search")?;
        explicit_mode(&mut query.exclude_mode);
        get_json_query(&self.http, &self.token, url, &query).await
    }
}
