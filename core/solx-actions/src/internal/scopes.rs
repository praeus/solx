//! The resource scopes a built-in needs, beyond the `exec:` scope dispatch
//! already checked: `entity-get-document` needs `read:document` on the
//! document as well as `exec` on itself, so one `exec:/*` grant doesn't
//! silently read everything.
//!
//! [`gate`] runs in dispatch, before the handler, so a refusal keeps its
//! [`SolxError::Forbidden`] kind; a handler's own errors are plain text.
//! It covers one-entity reads and the document and type writes, and refuses
//! a list outright when the scopes admit none of what it lists. The rest is
//! checked where the data is:
//!
//! * action and file writes in `authorize_action_change`/
//!   `authorize_file_change`, which every surface already calls;
//! * list narrowing in [`super::exclude`]'s `apply_*` helpers;
//! * per-entry file reads in `file-list` and `dir-copy`.

use serde_json::Value;
use solx_surface::error::Result;
use solx_surface::scope::{Access, Need, Resource};

use super::{doc_path_or_root, path_or_root};
use crate::ScopeCheck;

/// Refuse `fn_name` with `params` when it reads or writes an entity, or
/// lists a resource, outside `scopes`. Built-ins that touch no stored data
/// (`http-request`, `get-env`, consoles, …) are governed by their `exec:`
/// scope alone.
pub(crate) fn gate(fn_name: &str, params: &Value, scopes: &ScopeCheck) -> Result<()> {
    use Access::{Read, Write};
    use Resource::{Action, Document, File, Type};

    let entity = |access, resource, path: &str| -> Result<()> {
        // A missing name is the handler's error to report.
        match params.get("name").and_then(Value::as_str) {
            Some(name) => scopes.require(&Need::Resource {
                access,
                resource,
                path,
                name,
            }),
            None => Ok(()),
        }
    };
    let file = |access, key: &str| -> Result<()> {
        // Judged as the path the store will use; one it can't normalize is
        // the handler's error to report.
        let Some(raw) = params.get(key).and_then(Value::as_str) else {
            return Ok(());
        };
        match solx_files::normalize_rel_path(raw) {
            Ok(rel) => scopes.require(&Need::file(access, &rel)),
            Err(_) => Ok(()),
        }
    };

    match fn_name {
        "entity-get-document" | "get-field" => entity(Read, Document, path_or_root(params)),
        "entity-save-document" | "entity-delete-document" | "set-field" => {
            entity(Write, Document, path_or_root(params))
        }
        "get-field-at-path" => entity(Read, Document, doc_path_or_root(params)),
        "set-field-at-path" => entity(Write, Document, doc_path_or_root(params)),
        "entity-get-type" => entity(Read, Type, path_or_root(params)),
        "entity-save-type" | "entity-delete-type" => entity(Write, Type, path_or_root(params)),
        "entity-get-action" => entity(Read, Action, path_or_root(params)),
        "file-get" => file(Read, "rel_path"),
        "file-copy" => file(Read, "source"),

        "entity-list-documents" | "entity-list-document-paths" | "search-documents" => {
            scopes.require_some_reads(Document)
        }
        "entity-list-types" | "entity-list-type-paths" => scopes.require_some_reads(Type),
        "entity-list-actions" | "entity-list-action-paths" | "search-actions" => {
            scopes.require_some_reads(Action)
        }
        "file-list" | "dir-copy" => scopes.require_some_reads(File),
        _ => Ok(()),
    }
}
