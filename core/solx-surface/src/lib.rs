//! `solx-surface` — the dependency-light foundation for the solx workspace.
//!
//! Holds the entity DTOs, the shared [`error::SolxError`]/[`error::Result`],
//! list/search wire types, path/name helpers, and the manager traits that form
//! the client/server seam. It deliberately depends only on serde/uuid/chrono/
//! async-trait so it can back both the local impls and a future HTTP
//! client/server.

pub mod auth;
pub mod entities;
pub mod error;
pub mod exec_context;
pub mod internal_actions;
pub mod managers;
pub mod path;
pub mod query;
pub mod wire;

pub use auth::{ActionCredential, Principal, TokenAuthority};
pub use exec_context::{ExecContext, ExecContextInfo};
pub use error::{Result, SolxError};

pub use entities::{
    Action, ActionExecResult, ActionInput, ActionType, ArtifactPin, DocLink, Document, DocumentInput, FileRef,
    LinkKind, TypeEntity, TypeInput,
};
pub use internal_actions::{
    is_keyed, secret_names, secret_scope, uses_keyring, ActionExecutor, ActionSecrets, Caller, CallerInfo,
    InternalActionHandler, InternalActionRegistry, InternalCallCtx, SecretHandle, SeedAction,
};
pub use managers::{ActionManager, DocManager, FileStore, Solx, TypeManager};
pub use query::{ExcludeMode, ListOptions, Page, PathFacet, SearchQuery, SortOrder};
