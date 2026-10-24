//! The checks every surface applies before it saves or deletes an action on
//! content's behalf.
//!
//! Content is anything that isn't an operator: an MCP tool call (whatever
//! token its client holds), a WASM guest, a script, REST with a client or
//! action token, the desktop app's main window. Operators (the CLI, a
//! package install, REST with the root token, the Settings window) call
//! `ActionManager::save`/`delete` directly and skip these on purpose.
//!
//! `entity-save-action`/`entity-delete-action` additionally refuse Command
//! and Webhook rows (`internal::guard_executable_action`); REST and the
//! desktop app don't yet (known gap 7 in `docs/security-design.md`).

use solx_surface::entities::{Action, ActionInput};
use solx_surface::exec_context::ExecContext;
use solx_surface::{secret_scope, Principal};

/// Refuse, for content, a save (`incoming`) or delete (`None`) of
/// `path`/`name` that only an operator may make: one outside the acting
/// package's own namespace ([`guard_package_namespace`]), a keyed action
/// ([`crate::guard_keyed_action`]), or any change to a locked one
/// ([`crate::guard_locked_action`]).
///
/// `existing` is the stored row, if any. `acting` is the action content is
/// acting as, if any ([`acting_action`]).
pub fn guard_content_action_change(
    existing: Option<&Action>,
    incoming: Option<&ActionInput>,
    path: &str,
    name: &str,
    acting: Option<&str>,
) -> Result<(), String> {
    let reference = format!("{path}/{name}");
    guard_package_namespace(path, name, acting)?;
    if let Some(input) = incoming {
        crate::guard_keyed_action(input, &reference)?;
    }
    crate::guard_locked_action(existing, incoming, &reference)
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
) -> Result<(), String> {
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

/// The action a call chain is acting as: the calling action on a re-entrant
/// hop (a Script or WASM action), else the Command action whose token
/// authenticated it. `None` for a chain no action started.
pub fn acting_action(ctx: &ExecContext) -> Option<&str> {
    ctx.caller()
        .map(|c| c.action_ref())
        .or_else(|| principal_action(ctx.principal()))
}

/// The action an [`Principal::Action`] token was issued to.
pub fn principal_action(principal: Option<&Principal>) -> Option<&str> {
    match principal {
        Some(Principal::Action { action_ref, .. }) => Some(action_ref),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
