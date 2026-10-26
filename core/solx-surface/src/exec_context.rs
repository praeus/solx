//! What a call chain runs as, carried from where it enters solx to every
//! nested frame.
//!
//! An execution has four facts about it that every re-entrant path must
//! carry together: the calling action ([`Caller`]), who authenticated the
//! chain ([`Principal`]), which exclusions it is held to ([`ExcludeMode`]),
//! and the scopes the chain entered with ([`Grant`]). Dropping one is a security bug: without the caller an
//! action's call looks external, and without the mode a model reaches
//! actions excluded from tooling. Bundling them means a new path can't
//! forget one, and a new fact has one place to go.
//!
//! They don't all travel the same way:
//!
//! * `principal`, `exclude_mode` and the entry grant are fixed where the
//!   chain enters ([`ExecContext::local`], [`ExecContext::for_principal`],
//!   [`ExecContext::tooling`]) and inherited unchanged by every frame.
//! * `caller` is *replaced* at each frame that runs as an action (a Script
//!   or WASM action): [`ExecContext::enter`]. Anything that action invokes is
//!   invoked by it, with its own secrets and its own scopes, never the outer
//!   action's.
//!
//! So a call is checked against whoever makes it ([`ExecContext::grant`]):
//! the calling action's scopes on a nested frame, the token's on the first,
//! and nothing at a local entry point.
//!
//! [`ExecContext`] holds the full [`Caller`], secrets included, so it never
//! leaves the host. Handlers and the detached-run seam get
//! [`ExecContextInfo`], which holds only [`CallerInfo`].

use crate::auth::Principal;
use crate::internal_actions::{Caller, CallerInfo};
use crate::query::ExcludeMode;
use crate::scope::Grant;

/// The host-side context of one frame of a call chain — see the module doc.
/// Built only by the entry-point constructors and [`Self::enter`].
#[derive(Debug, Clone)]
pub struct ExecContext {
    caller: Option<Caller>,
    principal: Option<Principal>,
    exclude_mode: ExcludeMode,
    /// What a call with no calling action is checked against: the
    /// principal's scopes, or everything at a local entry point.
    grant: Grant,
}

impl ExecContext {
    /// A local entry point (the CLI, Tauri, in-process hosts): no caller, no
    /// principal, no exclusions, no scope limit.
    pub fn local() -> Self {
        ExecContext {
            caller: None,
            principal: None,
            exclude_mode: ExcludeMode::None,
            grant: Grant::Unlimited,
        }
    }

    /// An authenticated route (`ActionManager::exec_for`): `principal` is
    /// who `TokenAuthority::authenticate` just identified.
    pub fn for_principal(principal: Principal) -> Self {
        ExecContext {
            grant: principal.grant().clone(),
            principal: Some(principal),
            ..Self::local()
        }
    }

    /// The MCP server (`ActionManager::exec_tooling`): the whole chain is
    /// held to the tooling exclusions, and to `principal`'s scopes when
    /// there is one (stdio MCP has none: it is a local entry point).
    pub fn tooling(principal: Option<Principal>) -> Self {
        ExecContext {
            caller: None,
            grant: principal
                .as_ref()
                .map_or(Grant::Unlimited, |p| p.grant().clone()),
            principal,
            exclude_mode: ExcludeMode::Tooling,
        }
    }

    /// The context for a frame that runs as an action: `frame` becomes the
    /// caller of everything it invokes, and what it invokes is checked
    /// against `frame`'s scopes; the principal and exclusion mode carry
    /// over.
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

    /// What a call made from this frame is checked against: the calling
    /// action's scopes when there is one, else the entry grant.
    pub fn grant(&self) -> &Grant {
        self.caller.as_ref().map_or(&self.grant, Caller::grant)
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
            grant: self.grant.clone(),
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
    /// The entry grant, for [`Self::resume`]. The caller's own comes back
    /// with the frame the host rebuilds from its row.
    grant: Grant,
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

    /// See [`ExecContext::grant`].
    pub fn grant(&self) -> &Grant {
        self.caller.as_ref().map_or(&self.grant, CallerInfo::grant)
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
            grant: self.grant.clone(),
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
    use crate::scope::ScopeSet;

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
            grant: Grant::scoped(ScopeSet::parse(["read:document"]).unwrap()),
        }
    }

    fn action() -> Principal {
        Principal::Action {
            action_ref: "/pkg/a".into(),
            invocation_id: "i1".into(),
            grant: Grant::none(),
        }
    }

    fn declaring(action_ref: &str, scopes: &[&str]) -> Caller {
        Caller::from_action(action_ref, None)
            .with_grant(Grant::scoped(ScopeSet::parse(scopes).unwrap()))
    }

    #[test]
    fn a_call_is_checked_against_whoever_makes_it() {
        assert_eq!(ExecContext::local().grant(), &Grant::Unlimited);
        assert_eq!(ExecContext::tooling(None).grant(), &Grant::Unlimited);
        assert_eq!(
            ExecContext::for_principal(Principal::Root).grant(),
            &Grant::Unlimited
        );
        let from_client = ExecContext::for_principal(client());
        assert_eq!(from_client.grant(), client().grant());
        assert_eq!(
            ExecContext::tooling(Some(client())).grant(),
            client().grant()
        );

        // A nested frame is checked against its own action, never against
        // the token or an outer action.
        let a = from_client.enter(declaring("/p/a", &["exec:/p/b"]));
        assert_eq!(
            a.grant(),
            &Grant::scoped(ScopeSet::parse(["exec:/p/b"]).unwrap())
        );
        let b = a.enter(Caller::from_action("/p/b", None));
        assert_eq!(b.grant(), &Grant::none());
        // Even when the chain entered locally.
        let local = ExecContext::local().enter(Caller::from_action("/p/a", None));
        assert_eq!(local.grant(), &Grant::none());
    }

    #[test]
    fn resume_keeps_the_entry_grant() {
        let ctx = ExecContext::for_principal(client());
        assert_eq!(ctx.info().resume(None).grant(), client().grant());
        let nested = ctx.enter(declaring("/p/a", &["exec:/p/b"]));
        assert_eq!(nested.info().grant(), nested.grant());
        let resumed = nested
            .info()
            .resume(Some(declaring("/p/a", &["exec:/p/b"])));
        assert_eq!(resumed.grant(), nested.grant());
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
