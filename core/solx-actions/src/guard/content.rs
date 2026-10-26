//! What an [`Actor`] may change: the one place every surface's action and
//! file writes are authorized.
//!
//! Each surface names who is acting and calls
//! [`LocalActionManager::authorize_action_change`] or
//! [`LocalActionManager::authorize_file_change`] (the hosts reach both
//! through `solx_manager::App`). Operators (the CLI, a package install, REST
//! with the root token, the Settings window) pass every check. Content (an
//! MCP tool call whatever token its client holds, a WASM guest, a script,
//! REST with a client or action token, the desktop app's main window) is
//! held to the checks below, and to its scopes ([`Actor::grant`]):
//! `write:action` plus the kind scope (`write:script`, `write:command`, …)
//! for an action, `write:file` for a file. The desktop app's main window is
//! limited by no scope.
//!
//! Built-ins additionally refuse Command and Webhook rows; for REST,
//! `write:command` and `write:webhook` decide it.

use solx_surface::entities::{Action, ActionInput, ActionType};
use solx_surface::error::{Result, SolxError};
use solx_surface::managers::ActionManager;
use solx_surface::scope::{Access, Kind, Need, Resource};
use solx_surface::{secret_scope, Actor};

use crate::{LocalActionManager, ScopeCheck};

impl LocalActionManager {
    /// Refuse a save (`incoming`) or delete (`None`) of `path`/`name` that
    /// `actor` may not make — see [`check_action_change`]. Reads the stored
    /// row itself, since several checks depend on it: `save` is a
    /// merge-upsert, so the payload alone doesn't say what the row will be.
    pub async fn authorize_action_change(
        &self,
        actor: Actor<'_>,
        path: &str,
        name: &str,
        incoming: Option<&ActionInput>,
    ) -> Result<()> {
        if actor == Actor::Operator {
            return Ok(());
        }
        let existing = match ActionManager::get(self, path, name).await {
            Ok(a) => Some(a),
            Err(SolxError::NotFound(_)) => None,
            Err(e) => return Err(e),
        };
        check_action_change(actor, existing.as_ref(), incoming, path, name)
            .map_err(SolxError::Unauthorized)?;
        // The actor's scopes: `write:action` on the action, and the kind
        // scope of both the incoming and the stored type, since a
        // merge-upsert that omits `action_type` keeps the stored one.
        let scopes = ScopeCheck::for_actor(&self.config, actor);
        scopes.require(&Need::Resource {
            access: Access::Write,
            resource: Resource::Action,
            path,
            name,
        })?;
        let types = [
            incoming.and_then(|i| i.action_type),
            existing.as_ref().and_then(|a| a.action_type),
        ];
        let kinds = types
            .into_iter()
            .flatten()
            .filter_map(Kind::for_action_type);
        for kind in kinds {
            scopes.require(&Need::Kind(kind))?;
        }
        Ok(())
    }

    /// Refuse when `actor` may not write or delete any of `targets`
    /// (store-relative paths): for content, a locked action's pinned
    /// artifact. A directory operation passes every entry before touching
    /// any. See `crate::lock`.
    pub async fn authorize_file_change<'t>(
        &self,
        actor: Actor<'_>,
        targets: impl IntoIterator<Item = &'t str>,
    ) -> Result<()> {
        if actor == Actor::Operator {
            return Ok(());
        }
        let targets: Vec<&str> = targets.into_iter().collect();
        let scopes = ScopeCheck::for_actor(&self.config, actor);
        for target in &targets {
            // Checked against where the store will really write, so an
            // alias of a path (`a//b`, `a\b`) is judged as the path it is.
            let stored = solx_files::normalize_rel_path(target)?;
            scopes.require(&Need::file(Access::Write, &stored))?;
        }
        self.guard_pinned_files(targets).await
    }
}

/// Refuse, for content, a save (`incoming`) or delete (`None`) of
/// `path`/`name` that only an operator may make:
///
/// * one outside the acting package's own namespace
///   ([`guard_package_namespace`]);
/// * a keyed action ([`crate::guard_keyed_action`]);
/// * any change to a locked one ([`crate::guard_locked_action`]);
/// * through a built-in, any Command or Webhook row, stored or incoming
///   (`guard_executable_action`).
///
/// `existing` is the stored row, if any. An operator passes.
pub fn check_action_change(
    actor: Actor<'_>,
    existing: Option<&Action>,
    incoming: Option<&ActionInput>,
    path: &str,
    name: &str,
) -> std::result::Result<(), String> {
    if actor == Actor::Operator {
        return Ok(());
    }
    let reference = format!("{path}/{name}");
    if matches!(actor, Actor::Builtin { .. }) {
        guard_executable_action(
            existing.and_then(|a| a.action_type),
            incoming.and_then(|i| i.action_type),
            &reference,
            if incoming.is_some() {
                "create or modify"
            } else {
                "delete"
            },
        )?;
    }
    guard_package_namespace(path, name, actor.acting())?;
    if let Some(input) = incoming {
        crate::guard_keyed_action(input, &reference)?;
    }
    crate::guard_locked_action(existing, incoming, &reference)
}

/// Refuse a change to a Command or Webhook action, judged by both the
/// stored and the incoming type: a merge upsert that omits `action_type`
/// can't silently rewrite what an established Command action shells out to.
/// Built-ins are the only route to action creation an MCP client, a WASM
/// guest or a `.solx` script has, so this keeps them from granting
/// themselves shell or outbound HTTP.
fn guard_executable_action(
    existing: Option<ActionType>,
    incoming: Option<ActionType>,
    reference: &str,
    verb: &str,
) -> std::result::Result<(), String> {
    for ty in [incoming, existing].into_iter().flatten() {
        let label = match ty {
            ActionType::Command => "command",
            ActionType::Webhook => "webhook",
            _ => continue,
        };
        return Err(format!(
            "cannot {verb} '{label}' actions from an action, MCP tool call, or script \
             (attempted on {reference}); use the solx CLI"
        ));
    }
    Ok(())
}

/// Refuse, for content, a save or delete under `/packages/`, except by one
/// of that package's own actions within its own `/packages/<name>/`.
///
/// What a package installs is the package's: content saving a row there
/// ahead of an install would have it merged into the install's `save
/// action` (which keeps any field the script doesn't set), locked with the
/// package and recorded as its own. A package's own action may still manage
/// its namespace at run time (`solx-mcp-actions` imports tools as actions
/// under `/packages/solx-mcp-actions/`). The first segment is matched
/// case-insensitively, so `/Packages/x/` is refused too.
pub fn guard_package_namespace(
    path: &str,
    name: &str,
    acting: Option<&str>,
) -> std::result::Result<(), String> {
    let path = solx_surface::path::normalize_path(path).map_err(|e| e.to_string())?;
    let first = path.split('/').find(|s| !s.is_empty());
    if !first.is_some_and(|s| s.eq_ignore_ascii_case("packages")) {
        return Ok(());
    }
    let target = secret_scope(&path, name);
    let own = acting
        .and_then(|r| r.rsplit_once('/'))
        .map(|(p, n)| secret_scope(if p.is_empty() { "/" } else { p }, n));
    if target.starts_with("package:") && own.as_deref() == Some(target.as_str()) {
        return Ok(());
    }
    Err(format!(
        "{path}/{name} is under /packages/, which only an operator (the solx CLI, a \
         package install, the root token, or the desktop app's Settings window) or \
         that package's own actions can change"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command() -> ActionInput {
        ActionInput {
            action_type: Some(ActionType::Command),
            fn_name: Some("k".into()),
            ..Default::default()
        }
    }

    /// Only a built-in is refused Command rows; every actor but the
    /// operator is held to the package namespace.
    #[test]
    fn each_actor_gets_its_own_checks() {
        let grant = solx_surface::scope::UNLIMITED;
        let builtin = Actor::Builtin {
            acting: None,
            grant,
        };
        let content = Actor::Content {
            acting: None,
            grant,
        };
        let err = check_action_change(builtin, None, Some(&command()), "/t", "a").unwrap_err();
        assert!(err.contains("'command' actions"), "{err}");
        assert!(check_action_change(content, None, Some(&command()), "/t", "a").is_ok());
        for actor in [builtin, content] {
            assert!(check_action_change(actor, None, None, "/packages/p", "a").is_err());
        }
        assert!(
            check_action_change(Actor::Operator, None, Some(&command()), "/packages/p", "a")
                .is_ok()
        );
    }

    #[test]
    fn outside_packages_anything_goes() {
        assert!(guard_package_namespace("/my/tools", "a", None).is_ok());
        assert!(guard_package_namespace("/", "packages", None).is_ok());
    }

    #[test]
    fn content_may_not_write_under_packages() {
        for (path, name) in [
            ("/packages/p", "a"),
            ("/packages/p/deep", "a"),
            ("/packages", "p"),
            ("/Packages/p", "a"),
            ("packages//p/", "a"),
        ] {
            let err = guard_package_namespace(path, name, None).unwrap_err();
            assert!(err.contains("under /packages/"), "{path}/{name}: {err}");
        }
    }

    #[test]
    fn a_package_may_manage_only_its_own_namespace() {
        let own = Some("/packages/p/importer");
        assert!(guard_package_namespace("/packages/p/server", "tool", own).is_ok());
        assert!(guard_package_namespace("/packages/q", "tool", own).is_err());
        // `/packages/p` the action is not package `p`.
        assert!(guard_package_namespace("/packages/p", "tool", Some("/packages/p")).is_err());
        assert!(guard_package_namespace("/Packages/p", "tool", own).is_err());
    }
}
