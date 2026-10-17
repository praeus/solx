//! What a call chain runs as, carried from where it enters solx to every
//! nested frame.
//!
//! An execution has three facts about it that every re-entrant path must
//! carry together: the calling action ([`Caller`]), who authenticated the
//! chain ([`Principal`]), and which exclusions it is held to
//! ([`ExcludeMode`]). Dropping one is a security bug: without the caller an
//! action's call looks external, and without the mode a model reaches
//! actions excluded from tooling. Bundling them means a new path can't
//! forget one, and a new fact has one place to go.
//!
//! The three don't travel the same way:
//!
//! * `principal` and `exclude_mode` are fixed where the chain enters
//!   ([`ExecContext::local`], [`ExecContext::for_principal`],
//!   [`ExecContext::tooling`]) and inherited unchanged by every frame.
//! * `caller` is *replaced* at each frame that runs as an action (a Script
//!   or WASM action): [`ExecContext::enter`]. Anything that action invokes is
//!   invoked by it, with its own secrets, never the outer action's.
//!
//! [`ExecContext`] holds the full [`Caller`], secrets included, so it never
//! leaves the host. Handlers and the detached-run seam get
//! [`ExecContextInfo`], which holds only [`CallerInfo`].

use crate::auth::Principal;
use crate::internal_actions::{Caller, CallerInfo};
use crate::query::ExcludeMode;

/// The host-side context of one frame of a call chain — see the module doc.
/// Built only by the entry-point constructors and [`Self::enter`].
#[derive(Debug, Clone)]
pub struct ExecContext {
    caller: Option<Caller>,
    principal: Option<Principal>,
    exclude_mode: ExcludeMode,
}

impl ExecContext {
    /// A local entry point (the CLI, Tauri, in-process hosts): no caller, no
    /// principal, no exclusions.
    pub fn local() -> Self {
        ExecContext {
            caller: None,
            principal: None,
            exclude_mode: ExcludeMode::None,
        }
    }

    /// An authenticated route (`ActionManager::exec_for`): `principal` is
    /// who `TokenAuthority::authenticate` just identified.
    pub fn for_principal(principal: Principal) -> Self {
        ExecContext {
            principal: Some(principal),
            ..Self::local()
        }
    }

    /// The MCP server (`ActionManager::exec_tooling`): the whole chain is
    /// held to the tooling exclusions.
    pub fn tooling(principal: Option<Principal>) -> Self {
        ExecContext {
            caller: None,
            principal,
            exclude_mode: ExcludeMode::Tooling,
        }
    }

    /// The context for a frame that runs as an action: `frame` becomes the
    /// caller of everything it invokes; the principal and exclusion mode
    /// carry over.
    pub fn enter(&self, frame: Caller) -> Self {
        ExecContext {
            caller: Some(frame),
            ..self.clone()
        }
    }

    /// The action this frame runs on behalf of, if any.
    pub fn caller(&self) -> Option<&Caller> {
        self.caller.as_ref()
    }

    pub fn principal(&self) -> Option<&Principal> {
        self.principal.as_ref()
    }

    /// The exclusions the chain is held to. Only [`ExcludeMode::Tooling`] is
    /// enforced along the chain; `User` exclusions are a display
    /// convenience, not access control, so `User` acts as `None` here. No
    /// entry point sets it.
    pub fn exclude_mode(&self) -> ExcludeMode {
        self.exclude_mode
    }

    /// The secret-free form, for handlers and detached runs.
    pub fn info(&self) -> ExecContextInfo {
        ExecContextInfo {
            caller: self.caller.as_ref().map(Caller::info),
            principal: self.principal.clone(),
            exclude_mode: self.exclude_mode,
        }
    }
}

/// [`ExecContext`] without the caller's secrets: what an
/// `InternalActionHandler` sees, and what an `action-start` run carries
/// across its `tokio::spawn`.
#[derive(Debug, Clone)]
pub struct ExecContextInfo {
    caller: Option<CallerInfo>,
    principal: Option<Principal>,
    exclude_mode: ExcludeMode,
}

impl ExecContextInfo {
    /// The non-secret identity of the action this frame runs on behalf of.
    pub fn caller(&self) -> Option<&CallerInfo> {
        self.caller.as_ref()
    }

    /// Who authenticated the call this chain started from; `None` for local
    /// entry points. Inherited by nested frames, so on its own it does
    /// **not** mean "an external party called this directly" — use
    /// [`Self::external_principal`] for any authorization decision.
    pub fn principal(&self) -> Option<&Principal> {
        self.principal.as_ref()
    }

    /// See [`ExecContext::exclude_mode`].
    pub fn exclude_mode(&self) -> ExcludeMode {
        self.exclude_mode
    }

    /// The principal to authorize against, failing closed: `None` when an
    /// action is in the call chain (whatever principal it inherited), when
    /// the principal is itself an action token, or when there is no
    /// principal at all. A handler that gates on this must treat `None` as
    /// *deny*, never as root.
    pub fn external_principal(&self) -> Option<&Principal> {
        external_principal(self.caller.is_some(), self.principal.as_ref())
    }

    /// The full context again, for a detached run: `caller` is the frame the
    /// host rebuilt (secrets included) from the row [`Self::caller`] names,
    /// and must be present exactly when that is. Panics otherwise: resuming
    /// without the caller would let the run look like an external call.
    pub fn resume(&self, caller: Option<Caller>) -> ExecContext {
        assert_eq!(
            caller.as_ref().map(Caller::action_ref),
            self.caller.as_ref().map(CallerInfo::action_ref),
            "resumed with a different caller"
        );
        ExecContext {
            caller,
            principal: self.principal.clone(),
            exclude_mode: self.exclude_mode,
        }
    }
}

/// The decision behind [`ExecContextInfo::external_principal`], split out
/// so it can be tested on its own.
fn external_principal(has_caller: bool, principal: Option<&Principal>) -> Option<&Principal> {
    if has_caller {
        return None;
    }
    match principal {
        Some(Principal::Action { .. }) | None => None,
        Some(p) => Some(p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entering_a_frame_replaces_only_the_caller() {
        let root = ExecContext::tooling(Some(Principal::Root));
        let inner = root.enter(Caller::from_action("/p/a", None));
        assert_eq!(inner.caller().map(Caller::action_ref), Some("/p/a"));
        assert_eq!(inner.principal(), Some(&Principal::Root));
        assert_eq!(inner.exclude_mode(), ExcludeMode::Tooling);

        let deeper = inner.enter(Caller::from_action("/p/b", None));
        assert_eq!(deeper.caller().map(Caller::action_ref), Some("/p/b"));
    }

    fn client() -> Principal {
        Principal::Client {
            id: "c1".into(),
            label: None,
        }
    }

    fn action() -> Principal {
        Principal::Action {
            action_ref: "/pkg/a".into(),
            invocation_id: "i1".into(),
        }
    }

    #[test]
    fn external_principal_allows_root_and_client_with_no_caller() {
        assert_eq!(
            external_principal(false, Some(&Principal::Root)),
            Some(&Principal::Root)
        );
        assert_eq!(external_principal(false, Some(&client())), Some(&client()));
    }

    #[test]
    fn external_principal_fails_closed() {
        // No principal at all: a local entry point, never treated as root.
        assert_eq!(external_principal(false, None), None);
        // An action token is not an external party.
        assert_eq!(external_principal(false, Some(&action())), None);
        // An action in the chain wins over whatever principal it inherited.
        assert_eq!(external_principal(true, Some(&Principal::Root)), None);
        assert_eq!(external_principal(true, Some(&client())), None);
        // ...which is what entering a frame does.
        let nested =
            ExecContext::for_principal(Principal::Root).enter(Caller::from_action("/p/a", None));
        assert_eq!(nested.info().external_principal(), None);
    }

    #[test]
    fn resume_restores_the_full_context() {
        let ctx =
            ExecContext::tooling(Some(Principal::Root)).enter(Caller::from_action("/p/a", None));
        let resumed = ctx.info().resume(Some(Caller::from_action("/p/a", None)));
        assert_eq!(resumed.caller().map(Caller::action_ref), Some("/p/a"));
        assert_eq!(resumed.principal(), Some(&Principal::Root));
        assert_eq!(resumed.exclude_mode(), ExcludeMode::Tooling);
    }

    #[test]
    #[should_panic(expected = "resumed with a different caller")]
    fn resume_refuses_to_drop_the_caller() {
        let ctx = ExecContext::local().enter(Caller::from_action("/p/a", None));
        ctx.info().resume(None);
    }
}
