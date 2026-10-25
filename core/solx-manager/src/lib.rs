//! Wires the solx manager implementations together — the same sequence
//! `solx-cli`'s `App::build()` used to do inline. Extracted so `solx-mcp`
//! (and any other future long-lived consumer) gets identical, correct
//! wiring for free: seeding the built-in `get-env`/`set-env` environment
//! store before any `get-env` call, and the `set_self_ref` dance
//! `LocalActionManager` needs for recursive WASM/internal action-exec
//! calls, are both easy to silently get wrong in a second, duplicated
//! copy, and neither failure mode shows up until the corresponding feature
//! (env access, OAuth loopback, recursive `action-exec`) is actually
//! exercised.
//!
//! `App` transparently builds either **local** managers (`Local*` from
//! `solx-types`/`solx-docs`/`solx-actions`/`solx-files`, today's exact
//! behavior — the default for every existing user) or **remote** ones
//! (`Remote*` from `solx-client`, HTTP-proxying to a `solx-server`),
//! depending on whether `server_url` is set in config. `solx-cli`/
//! `solx-mcp` never need to know or care which mode they're in — they only
//! ever see `Arc<dyn TypeManager/DocManager/ActionManager/FileStore>` via
//! the `Solx` trait.
//!
//! The local managers sit behind the default `local` feature. Built without
//! it, only remote wiring exists, so a thin client links neither wasmtime
//! nor libsql.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};

#[cfg(feature = "local")]
use solx_actions::LocalActionManager;
use solx_config::{ConfigService, EntityKind};
#[cfg(feature = "local")]
use solx_docs::LocalDocManager;
#[cfg(feature = "local")]
use solx_files::LocalFileStore;
use solx_surface::auth::TokenAuthority;
use solx_surface::managers::{ActionManager, DocManager, FileStore, Solx, TypeManager};
use solx_surface::query::ExcludeMode;
#[cfg(feature = "local")]
use solx_types::LocalTypeManager;

/// The wired-together backend (local or remote). All fields are `Arc`, so
/// cloning `App` shares the same underlying managers.
#[derive(Clone)]
pub struct App {
    pub config: Arc<ConfigService>,
    types: Arc<dyn TypeManager>,
    docs: Arc<dyn DocManager>,
    actions: Arc<dyn ActionManager>,
    files: Arc<dyn FileStore>,
    /// `Some(server_url)` when proxying to a remote `solx-server`.
    remote_url: Option<String>,
    /// Decides which bearer tokens this process's HTTP surface accepts, and
    /// credentials spawned Command actions. `Some` only for local wiring: a
    /// remote `App` serves nothing and runs no actions itself.
    authority: Option<Arc<dyn TokenAuthority>>,
    /// The same authority as the token registry, for the routes and hosts
    /// that manage client tokens and connect codes. `Some` only for local
    /// wiring.
    #[cfg(feature = "registry")]
    registry: Option<Arc<solx_registry::Registry>>,
    /// The concrete action manager behind `actions`, for checks that aren't
    /// on the `ActionManager` trait (`authorize_*_change`). `Some` only for
    /// local wiring; a remote host enforces them itself.
    #[cfg(feature = "local")]
    local_actions: Option<Arc<LocalActionManager>>,
}

impl App {
    /// Build the app against the default appdata directory (`solx-config`'s
    /// `SOLX_APPDATA_DIR` override, else the platform default). Builds
    /// local or remote managers depending on config's `server_url`.
    pub async fn build() -> Result<Arc<Self>> {
        let config = Arc::new(ConfigService::open().context("open config")?);
        Self::build_with_config(config).await
    }

    /// Build the app against an explicit appdata directory — for isolated
    /// testing (mirrors `ConfigService::open_in`). Still local-or-remote
    /// per config, like `build()`.
    pub async fn build_in(appdata: impl Into<PathBuf>) -> Result<Arc<Self>> {
        let config = Arc::new(ConfigService::open_in(appdata).context("open config")?);
        Self::build_with_config(config).await
    }

    /// Build the app against the default appdata directory, **always**
    /// local — never proxies, regardless of what `server_url` might say.
    /// This is what `solx-server` itself must use: it must never
    /// accidentally proxy to itself if its own config happens to contain a
    /// stray `server_url`.
    #[cfg(feature = "local")]
    pub async fn build_local() -> Result<Arc<Self>> {
        let config = Arc::new(ConfigService::open().context("open config")?);
        Self::wire_local(config).await
    }

    /// Like [`Self::build_local`], against an explicit appdata directory.
    #[cfg(feature = "local")]
    pub async fn build_local_in(appdata: impl Into<PathBuf>) -> Result<Arc<Self>> {
        let config = Arc::new(ConfigService::open_in(appdata).context("open config")?);
        Self::wire_local(config).await
    }

    async fn build_with_config(config: Arc<ConfigService>) -> Result<Arc<Self>> {
        let snap = config.snapshot();
        // SOLX_SERVER_URL must be able to *enable* remote mode on its own
        // (mirroring SOLX_APPDATA_DIR's independent-of-config precedence),
        // not merely override an already-configured `server_url` — a
        // config with no `server_url` at all is the common case for every
        // client pointed at a server via env var alone.
        let server_url = std::env::var("SOLX_SERVER_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| snap.server_url.clone().filter(|s| !s.trim().is_empty()));
        match server_url {
            Some(server_url) => {
                let token = solx_config::credentials::client_token(&config, &server_url)
                    .with_context(|| {
                        format!(
                            "server_url is configured but no token is saved for {server_url} \
                             (start solx-server against this appdata dir first, or set \
                             SOLX_SERVER_TOKEN)"
                        )
                    })?;
                Self::remote(config, server_url, token)
            }
            #[cfg(feature = "local")]
            None => Self::wire_local(config).await,
            #[cfg(not(feature = "local"))]
            None => anyhow::bail!(
                "no server_url is configured, and this build has no local managers                  (solx-manager was built without the `local` feature)"
            ),
        }
    }

    /// Today's exact wiring, unchanged: open the three local databases and
    /// the local file store, wire `LocalActionManager`'s self-reference.
    #[cfg(feature = "local")]
    async fn wire_local(config: Arc<ConfigService>) -> Result<Arc<Self>> {
        // Seed the built-in `get-env`/`set-env` environment store once at
        // startup (decoupled from any single exec() call — see
        // internal::init_env_mappings).
        solx_actions::internal::init_env_mappings(
            config.snapshot().env_mappings.unwrap_or_default(),
        );
        // Then load variables persisted by `set-env { persist: true }`, so a
        // resume cursor (or any other durable variable) survives a restart.
        // Ordering matters: these win over an `env_mappings` key of the same
        // name, since they were written deliberately rather than mirrored
        // from the ambient system environment.
        solx_actions::internal::init_persisted_env(config.env_vars());

        let types: Arc<dyn TypeManager> = Arc::new(
            LocalTypeManager::open(&config.types_db_path())
                .await
                .context("open types db")?,
        );
        let docs: Arc<dyn DocManager> = Arc::new(
            LocalDocManager::open(&config.docs_db_path(), types.clone())
                .await
                .context("open docs db")?,
        );
        let files: Arc<dyn FileStore> = Arc::new(LocalFileStore::from_config(&config));

        // Built as a concrete Arc first so `set_self_ref` can hand it a
        // `Weak<LocalActionManager>` to itself (needed for WASM guests'
        // recursive action-exec calls, which go through the concrete
        // `exec_as` so they can carry the calling action's identity), then
        // stored as the trait object like the other managers.
        let actions_concrete = Arc::new(
            LocalActionManager::open(
                &config.actions_db_path(),
                config.clone(),
                types.clone(),
                docs.clone(),
                files.clone(),
            )
            .await
            .context("open actions db")?,
        );
        let authority = build_authority(&config).await?;
        // Plugins must be added before `set_self_ref`, which merges them.
        #[cfg(feature = "registry")]
        actions_concrete
            .add_plugin(solx_registry::actions::plugin(authority.registry.clone()))
            .await
            .context("register the token registry's builtins")?;
        actions_concrete.set_self_ref(Arc::downgrade(&actions_concrete));
        actions_concrete.set_token_authority(authority.authority.clone());
        let local_actions = Some(actions_concrete.clone());
        let actions: Arc<dyn ActionManager> = actions_concrete;

        Ok(Arc::new(App {
            config,
            types,
            docs,
            actions,
            files,
            remote_url: None,
            authority: Some(authority.authority),
            #[cfg(feature = "registry")]
            registry: Some(authority.registry),
            local_actions,
        }))
    }

    /// Proxy every manager over HTTP to a running `solx-server`. `config`
    /// itself stays local — packages/config are always local-file
    /// concerns, never proxied, in either mode. None of `wire_local`'s
    /// local-only setup (env mappings, opening local DBs, `set_self_ref`)
    /// applies here — there's no local `LocalActionManager`/
    /// `LocalDocManager` in this process; all of that lives in whichever
    /// process is running `solx-server`.
    ///
    /// Public so a host can choose remote mode explicitly, with a URL/token
    /// kept outside `config` (e.g. a desktop client's own settings, so it
    /// doesn't flip every other solx tool sharing the appdata dir to remote).
    ///
    /// A `#pin=` on `server_url` (see `solx_config::credentials`) makes the
    /// connection trust only that root.
    pub fn remote(
        config: Arc<ConfigService>,
        server_url: String,
        token: String,
    ) -> Result<Arc<Self>> {
        let (server_url, pin) = solx_config::credentials::split_server_pin(&server_url)?;
        let remote_url = Some(server_url.clone());
        // One client for all four, trusting `server_ca_file`/`SOLX_CA_FILE`
        // besides the built-in and OS roots, or only the pinned root.
        let http =
            solx_client::http_client_for(config.server_ca_file().as_deref(), pin.as_deref())?;
        let types: Arc<dyn TypeManager> = Arc::new(solx_client::RemoteTypeManager::with_client(
            server_url.clone(),
            token.clone(),
            http.clone(),
        ));
        let docs: Arc<dyn DocManager> = Arc::new(solx_client::RemoteDocManager::with_client(
            server_url.clone(),
            token.clone(),
            http.clone(),
        ));
        let actions: Arc<dyn ActionManager> =
            Arc::new(solx_client::RemoteActionManager::with_client(
                server_url.clone(),
                token.clone(),
                http.clone(),
            ));
        let files: Arc<dyn FileStore> = Arc::new(solx_client::RemoteFileStore::with_client(
            server_url, token, http,
        ));

        Ok(Arc::new(App {
            config,
            types,
            docs,
            actions,
            files,
            remote_url,
            authority: None,
            #[cfg(feature = "registry")]
            registry: None,
            #[cfg(feature = "local")]
            local_actions: None,
        }))
    }

    /// The `solx-server` this app proxies to, or `None` for local managers.
    pub fn remote_url(&self) -> Option<&str> {
        self.remote_url.as_deref()
    }

    /// The token authority for serving this app over HTTP. `None` for a
    /// remote app, which has nothing local to serve.
    pub fn authority(&self) -> Option<Arc<dyn TokenAuthority>> {
        self.authority.clone()
    }

    /// The token registry behind [`Self::authority`]. `None` for a remote
    /// app.
    #[cfg(feature = "registry")]
    pub fn registry(&self) -> Option<Arc<solx_registry::Registry>> {
        self.registry.clone()
    }
}

/// What [`build_authority`] chose.
#[cfg(feature = "local")]
struct BuiltAuthority {
    authority: Arc<dyn TokenAuthority>,
    #[cfg(feature = "registry")]
    registry: Arc<solx_registry::Registry>,
}

/// The one place the token authority is chosen: the token registry with
/// the `registry` feature, else the single shared root token.
#[cfg(feature = "local")]
async fn build_authority(config: &Arc<ConfigService>) -> Result<BuiltAuthority> {
    #[cfg(feature = "registry")]
    {
        let registry = solx_registry::Registry::open(config.clone())
            .await
            .context("open the token registry")?;
        Ok(BuiltAuthority {
            authority: registry.clone(),
            registry,
        })
    }
    #[cfg(not(feature = "registry"))]
    {
        let authority = solx_config::credentials::SingleTokenAuthority::from_config(config)
            .context("prepare server token")?;
        Ok(BuiltAuthority {
            authority: Arc::new(authority),
        })
    }
}

impl App {
    /// Refuse a write or delete of any of `targets` that `actor` may not
    /// make: for content, a locked action's pinned artifact. See
    /// `solx_actions::LocalActionManager::authorize_file_change`. A no-op
    /// for a remote app, whose server authorizes the request itself.
    pub async fn authorize_file_change(
        &self,
        actor: solx_surface::Actor<'_>,
        targets: &[&str],
    ) -> solx_surface::error::Result<()> {
        #[cfg(feature = "local")]
        if let Some(local) = &self.local_actions {
            return local
                .authorize_file_change(actor, targets.iter().copied())
                .await;
        }
        let _ = (actor, targets);
        Ok(())
    }

    /// Refuse a save (`incoming`) or delete (`None`) of the action at
    /// `path`/`name` that `actor` may not make. For content: any change to a
    /// locked action, locking or unlocking, a save of a keyed action, and
    /// anything under another package's `/packages/<name>/`. See
    /// `solx_actions::check_action_change`. A no-op for a remote app, whose
    /// server authorizes the request itself.
    pub async fn authorize_action_change(
        &self,
        actor: solx_surface::Actor<'_>,
        path: &str,
        name: &str,
        incoming: Option<&solx_surface::entities::ActionInput>,
    ) -> solx_surface::error::Result<()> {
        #[cfg(feature = "local")]
        if let Some(local) = &self.local_actions {
            return local
                .authorize_action_change(actor, path, name, incoming)
                .await;
        }
        let _ = (actor, path, name, incoming);
        Ok(())
    }

    /// Refuse direct access to an entity excluded under `mode` — the
    /// get/save/delete half of the exclusion lists, shared by every surface
    /// that has a mode to honour (solx-server's routes, the Tauri plugin).
    ///
    /// Reads and deletes of an excluded entity fail as `NotFound`, worded
    /// like a genuinely missing one; writes fail as `Invalid`. Under
    /// [`ExcludeMode::Tooling`] an action is read first so its own
    /// `solx:hidden` tag counts too. A no-op for [`ExcludeMode::None`].
    pub async fn guard_excluded(
        &self,
        kind: EntityKind,
        mode: ExcludeMode,
        path: &str,
        name: &str,
        write: bool,
    ) -> solx_surface::error::Result<()> {
        if mode == ExcludeMode::None {
            return Ok(());
        }
        let policy = self.config.exclusion_policy();
        policy.guard(kind, mode, path, name, write)?;
        if kind == EntityKind::Action && mode == ExcludeMode::Tooling {
            if let Ok(action) = self.actions.get(path, name).await {
                policy.guard_action(mode, &action, write)?;
            }
        }
        Ok(())
    }
}

impl Solx for App {
    fn types(&self) -> Arc<dyn TypeManager> {
        self.types.clone()
    }
    fn files(&self) -> Arc<dyn FileStore> {
        self.files.clone()
    }
    fn docs(&self) -> Arc<dyn DocManager> {
        self.docs.clone()
    }
    fn actions(&self) -> Arc<dyn ActionManager> {
        self.actions.clone()
    }
}
