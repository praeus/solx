//! Who is asking to change an action or a file.
//!
//! Every surface that saves or deletes an action, or writes or deletes a
//! file, on someone's behalf names its [`Actor`] and hands it to one
//! authorization entry point (`LocalActionManager::authorize_action_change`
//! and `authorize_file_change`, reached through `solx_manager::App` from the
//! hosts). The surfaces decide only *who* is acting; *what* that actor may
//! do is decided in one place, so a new check lands once instead of at
//! every route.

use crate::auth::Principal;
use crate::exec_context::ExecContext;

/// Who is asking to change an action or a file — see the module doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor<'a> {
    /// An operator: the CLI, a package install, REST with the root token,
    /// the desktop app's Settings window. Operators call
    /// `ActionManager::save`/`delete` directly and skip the content checks
    /// on purpose.
    Operator,
    /// Content on a direct route: REST with a client or action token, the
    /// desktop app's main window. `acting` is the action it acts as, if any.
    Content { acting: Option<&'a str> },
    /// Content arriving through a built-in (`entity-save-action`,
    /// `entity-delete-action`, the file built-ins): an MCP tool call, a WASM
    /// guest, a script. Held to everything [`Actor::Content`] is, and may
    /// not create, change or delete Command or Webhook actions.
    Builtin { acting: Option<&'a str> },
}

impl<'a> Actor<'a> {
    /// The actor behind an authenticated REST request: the root token is an
    /// operator; any other token is content, acting as the action an action
    /// token was issued to.
    pub fn for_principal(principal: &'a Principal) -> Self {
        match principal {
            Principal::Root => Actor::Operator,
            principal => Actor::Content {
                acting: principal_action(principal),
            },
        }
    }

    /// The actor behind a built-in: always content, whatever principal the
    /// chain carries. It acts as the calling action on a re-entrant hop (a
    /// Script or WASM action), else as the Command action whose token
    /// authenticated the chain.
    pub fn builtin(ctx: &'a ExecContext) -> Self {
        Actor::Builtin {
            acting: ctx
                .caller()
                .map(|c| c.action_ref())
                .or_else(|| ctx.principal().and_then(principal_action)),
        }
    }

    /// The action this actor acts as, if any. `None` for an operator.
    pub fn acting(&self) -> Option<&'a str> {
        match *self {
            Actor::Operator => None,
            Actor::Content { acting } | Actor::Builtin { acting } => acting,
        }
    }
}

/// The action a [`Principal::Action`] token was issued to.
fn principal_action(principal: &Principal) -> Option<&str> {
    match principal {
        Principal::Action { action_ref, .. } => Some(action_ref),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_is_an_operator_and_tokens_are_content() {
        assert_eq!(Actor::for_principal(&Principal::Root), Actor::Operator);
        let client = Principal::Client {
            id: "c".into(),
            label: None,
        };
        assert_eq!(
            Actor::for_principal(&client),
            Actor::Content { acting: None }
        );
        let action = Principal::Action {
            action_ref: "/packages/p/a".into(),
            invocation_id: "i".into(),
        };
        assert_eq!(
            Actor::for_principal(&action),
            Actor::Content {
                acting: Some("/packages/p/a")
            }
        );
    }

    #[test]
    fn a_builtin_is_content_even_for_root() {
        let ctx = ExecContext::for_principal(Principal::Root);
        assert_eq!(Actor::builtin(&ctx), Actor::Builtin { acting: None });
        assert_eq!(Actor::builtin(&ExecContext::local()).acting(), None);
    }
}
