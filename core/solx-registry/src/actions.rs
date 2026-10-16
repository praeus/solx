//! `/builtin/auth/*` — managing client tokens and connect codes, registered
//! into `solx-actions`' dispatcher through `LocalActionManager::add_plugin`
//! (the same plugin API `solx-console` uses).
//!
//! Every handler is **root only**, judged by
//! `InternalCallCtx::external_principal()`, which fails closed: a client
//! token, an action token, any action in the call chain and a call with no
//! principal at all (stdio MCP, local CLI `exec`, plain Tauri IPC) are all
//! refused. The trusted local ways in — `solx token …` and the desktop
//! Settings window's `auth_exec` — run these builtins with
//! `exec_for(&Principal::Root, …)` themselves.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use solx_surface::internal_actions::{
    InternalActionHandler, InternalActionRegistry, InternalCallCtx, SeedAction,
};
use solx_surface::Principal;

use crate::Registry;

pub const AUTH_PATH: &str = "/builtin/auth";

/// The builtins' names under [`AUTH_PATH`], and so the only names the
/// trusted entry points accept.
pub const AUTH_ACTIONS: &[&str] = &[
    "create-connect-code",
    "list-clients",
    "revoke-client",
    "issue-client",
];

#[derive(Clone, Copy)]
enum Op {
    CreateConnectCode,
    ListClients,
    RevokeClient,
    IssueClient,
}

struct AuthHandler {
    registry: Arc<Registry>,
    op: Op,
}

fn require_root(ctx: &InternalCallCtx) -> Result<(), String> {
    match ctx.external_principal() {
        Some(Principal::Root) => Ok(()),
        _ => Err(
            "this action needs the root token. Run it on the solx host with \
                  `solx token …`, from the desktop app's Settings, or over HTTP from \
                  this machine with the root token."
                .into(),
        ),
    }
}

fn opt_str<'a>(params: &'a Value, field: &str) -> Option<&'a str> {
    params.get(field).and_then(Value::as_str)
}

fn opt_secs(params: &Value, field: &str) -> Result<Option<Duration>, String> {
    match params.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .map(|s| Some(Duration::from_secs(s)))
            .ok_or_else(|| format!("{field} must be a whole number of seconds")),
    }
}

#[async_trait]
impl InternalActionHandler for AuthHandler {
    async fn call(&self, params: &Value, ctx: &InternalCallCtx) -> Result<Value, String> {
        require_root(ctx)?;
        let r = &self.registry;
        let result = match self.op {
            Op::CreateConnectCode => {
                let code = r
                    .create_connect_code(
                        opt_str(params, "label"),
                        opt_secs(params, "code_ttl_secs")?,
                        opt_secs(params, "token_ttl_secs")?,
                        opt_str(params, "public_url"),
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                json!({
                    "display_code": code.display_code,
                    "url": code.url,
                    "server_url": code.server_url,
                    "expires_at": code.expires_at,
                })
            }
            Op::ListClients => {
                json!({ "clients": r.list_clients().await.map_err(|e| e.to_string())? })
            }
            Op::RevokeClient => {
                let id = opt_str(params, "id").ok_or("missing required param: id")?;
                r.revoke_client(id).await.map_err(|e| e.to_string())?;
                json!({ "revoked": id })
            }
            Op::IssueClient => {
                let issued = r
                    .issue_client(
                        opt_str(params, "label"),
                        opt_secs(params, "token_ttl_secs")?,
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                json!({
                    "token": issued.token,
                    "client_id": issued.client_id,
                    "expires_at": issued.expires_at,
                })
            }
        };
        Ok(result)
    }
}

fn seed(name: &'static str, fn_name: &'static str, description: &'static str) -> SeedAction {
    SeedAction {
        path: AUTH_PATH,
        name,
        fn_name,
        description,
        param_type: None,
    }
}

/// The catalogue entries backing [`plugin`]'s handlers.
pub fn seed_actions() -> Vec<SeedAction> {
    vec![
        seed(
            "create-connect-code",
            "auth-create-connect-code",
            "Root only. Create a single-use 8-digit connect code another device redeems for its own client token. Params: label?, code_ttl_secs? (default 600), token_ttl_secs?, public_url? (the address to put in the link instead of server_public_url, e.g. a Tailscale IP; a bare host gets this server's port). Returns display_code, url, server_url, expires_at.",
        ),
        seed(
            "list-clients",
            "auth-list-clients",
            "Root only. List working client tokens: ids, labels and timestamps, never the tokens.",
        ),
        seed(
            "revoke-client",
            "auth-revoke-client",
            "Root only. Revoke a client token by id. Params: id.",
        ),
        seed(
            "issue-client",
            "auth-issue-client",
            "Root only. Mint a client token directly. Params: label, token_ttl_secs?. Returns token, client_id, expires_at.",
        ),
    ]
}

/// The `/builtin/auth/*` handlers over `registry`, with their seeds.
pub fn plugin(registry: Arc<Registry>) -> InternalActionRegistry {
    let mut plugin = InternalActionRegistry::new();
    let ops = [
        ("auth-create-connect-code", Op::CreateConnectCode),
        ("auth-list-clients", Op::ListClients),
        ("auth-revoke-client", Op::RevokeClient),
        ("auth-issue-client", Op::IssueClient),
    ];
    for (fn_name, op) in ops {
        plugin.register(
            &[fn_name],
            Arc::new(AuthHandler {
                registry: registry.clone(),
                op,
            }),
        );
    }
    plugin.add_seeds(seed_actions());
    plugin
}
