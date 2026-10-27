//! Scopes: what a token, or an action calling another, may do.
//!
//! A non-root token carries a [`ScopeSet`], a list of grants such as
//! `read:document`, `write:action:/my/tools`, `write:script` or
//! `exec:/packages/solx-google`. An action declares its own set in
//! `action_config.scopes` ([`declared_scopes`]); that set acts as the
//! action's token. When A calls B, B is checked against A's scopes; a call
//! with no calling action is checked against the token that authenticated it;
//! a local entry point (the CLI, the desktop app) is [`Grant::Unlimited`].
//!
//! The grammar:
//!
//! ```text
//! scope          := resource-scope | kind-scope | exec-scope | "all"
//! resource-scope := ("read" | "write") ":" resource [":" path]
//! resource       := "document" | "type" | "action" | "file"
//! kind-scope     := "write:" ("script" | "wasm" | "command" | "webhook")
//! exec-scope     := "exec:" path
//! path           := "/" [segments] ["/*"]
//! ```
//!
//! A scope path matches the way an `include_paths` pattern does
//! ([`crate::query::exclude_pattern_matches`]): it admits an entity when it
//! matches the entity's folder **or** its full reference (`<path>/<name>`).
//! There are two forms:
//!
//! * `/path/name` admits the entity `name` in `/path`, and every entity
//!   directly in the folder `/path/name` (not deeper);
//! * `/path/*` admits everything in the folder `/path` and beneath it. `/*`,
//!   which is also what a resource scope with no path means, admits
//!   everything.
//!
//! So `exec:/builtin/web/http-request` admits that action, and
//! `exec:/builtin/web/*` every web built-in, `/builtin/web/stream/start`
//! included. `*` is allowed only as that final segment, and `?` and `[` not
//! at all, so whether one scope covers another stays a string test
//! ([`ScopeSet::covers`]) and a scope path is a list filter as it stands
//! ([`ScopeSet::path_filter`]). Matching is case-sensitive, like the stores'
//! SQL `GLOB`. Scopes don't imply each other: `write:document` doesn't grant
//! `read:document`.
//!
//! Nothing here does I/O; the stores and dispatch call it.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::entities::ActionType;
use crate::error::{Result, SolxError};
use crate::path::{normalize_path, validate_name};
use crate::query::exclude_pattern_matches;

/// Reading or changing a resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Access {
    Read,
    Write,
}

impl Access {
    fn name(self) -> &'static str {
        match self {
            Access::Read => "read",
            Access::Write => "write",
        }
    }
}

/// What a resource scope governs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Resource {
    Document,
    Type,
    Action,
    /// Paths are the file's relative path under the files root.
    File,
}

impl Resource {
    /// The resource as a scope names it: `document`, `type`, ….
    pub fn name(self) -> &'static str {
        match self {
            Resource::Document => "document",
            Resource::Type => "type",
            Resource::Action => "action",
            Resource::File => "file",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "document" => Resource::Document,
            "type" => Resource::Type,
            "action" => Resource::Action,
            "file" => Resource::File,
            _ => return None,
        })
    }
}

/// An action type whose creation, change or deletion needs its own
/// `write:<kind>` scope on top of `write:action`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Script,
    Wasm,
    Command,
    Webhook,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Script => "script",
            Kind::Wasm => "wasm",
            Kind::Command => "command",
            Kind::Webhook => "webhook",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "script" => Kind::Script,
            "wasm" => Kind::Wasm,
            "command" => Kind::Command,
            "webhook" => Kind::Webhook,
            _ => return None,
        })
    }

    /// The kind scope a row of this type needs. `None` for `Internal`:
    /// built-ins are sealed in the store, so no scope can reach them.
    pub fn for_action_type(ty: ActionType) -> Option<Self> {
        match ty {
            ActionType::Script => Some(Kind::Script),
            ActionType::Wasm => Some(Kind::Wasm),
            ActionType::Command => Some(Kind::Command),
            ActionType::Webhook => Some(Kind::Webhook),
            ActionType::Internal => None,
        }
    }
}

/// One grant — see the module doc for the grammar.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Everything a client token could do before scopes existed. Never the
    /// root-only operations, which no scope grants.
    All,
    /// `read|write:<resource>[:path]`; `path` is canonical, `/*` when omitted.
    Resource {
        access: Access,
        resource: Resource,
        path: String,
    },
    /// `write:<kind>`.
    Kind(Kind),
    /// `exec:<path>`.
    Exec { path: String },
}

impl Scope {
    /// Whether this scope grants `need`.
    pub fn grants(&self, need: &Need<'_>) -> bool {
        match (self, need) {
            (Scope::All, _) => true,
            (
                Scope::Resource {
                    access,
                    resource,
                    path,
                },
                Need::Resource {
                    access: a,
                    resource: r,
                    path: p,
                    name,
                },
            ) => access == a && resource == r && admits(path, p, name),
            (Scope::Kind(k), Need::Kind(n)) => k == n,
            (Scope::Exec { path }, Need::Exec { path: p, name }) => admits(path, p, name),
            _ => false,
        }
    }

    /// Whether everything `other` grants, this grants too.
    pub fn covers(&self, other: &Scope) -> bool {
        match (self, other) {
            (Scope::All, _) => true,
            (
                Scope::Resource {
                    access,
                    resource,
                    path,
                },
                Scope::Resource {
                    access: a,
                    resource: r,
                    path: p,
                },
            ) => access == a && resource == r && path_covers(path, p),
            (Scope::Kind(k), Scope::Kind(o)) => k == o,
            (Scope::Exec { path }, Scope::Exec { path: p }) => path_covers(path, p),
            _ => false,
        }
    }
}

impl FromStr for Scope {
    type Err = SolxError;

    fn from_str(raw: &str) -> Result<Self> {
        let s = raw.trim();
        let bad = |why: &str| SolxError::Invalid(format!("scope '{raw}': {why}"));
        if s == "all" {
            return Ok(Scope::All);
        }
        let (verb, rest) = s
            .split_once(':')
            .ok_or_else(|| bad("expected 'all', 'read:…', 'write:…' or 'exec:…'"))?;
        match verb {
            "read" | "write" => {
                let access = if verb == "read" {
                    Access::Read
                } else {
                    Access::Write
                };
                let (what, path) = match rest.split_once(':') {
                    Some((what, path)) => (what, Some(path)),
                    None => (rest, None),
                };
                if let Some(resource) = Resource::from_name(what) {
                    let path = match path {
                        Some(p) => {
                            scope_path(p, resource == Resource::File).map_err(|e| bad(&e))?
                        }
                        None => "/*".to_string(),
                    };
                    Ok(Scope::Resource {
                        access,
                        resource,
                        path,
                    })
                } else if let Some(kind) = Kind::from_name(what) {
                    if access == Access::Read {
                        return Err(bad("a kind scope is 'write:' only"));
                    }
                    if path.is_some() {
                        return Err(bad("a kind scope takes no path"));
                    }
                    Ok(Scope::Kind(kind))
                } else {
                    Err(bad(&format!("unknown resource or kind '{what}'")))
                }
            }
            "exec" => Ok(Scope::Exec {
                path: scope_path(rest, false).map_err(|e| bad(&e))?,
            }),
            _ => Err(bad(&format!("unknown verb '{verb}'"))),
        }
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Scope::All => f.write_str("all"),
            Scope::Resource {
                access,
                resource,
                path,
            } if path == "/*" => write!(f, "{}:{}", access.name(), resource.name()),
            Scope::Resource {
                access,
                resource,
                path,
            } => write!(f, "{}:{}:{path}", access.name(), resource.name()),
            Scope::Kind(kind) => write!(f, "write:{}", kind.name()),
            Scope::Exec { path } => write!(f, "exec:{path}"),
        }
    }
}

/// Validate and canonicalize a scope path: it must start with `/`, may end
/// in a `/*` segment and hold no other `*`, `?` or `[`, and otherwise follows
/// [`normalize_path`]. A file scope path may also not end a segment in `.`:
/// Windows drops a trailing dot, so `/files/docs.` would reach `files/docs`.
fn scope_path(raw: &str, file: bool) -> std::result::Result<String, String> {
    let p = raw.trim();
    if !p.starts_with('/') {
        return Err("a path must start with '/'".into());
    }
    let (base, prefix) = match p.strip_suffix("/*") {
        Some(base) => (base, true),
        None => (p, false),
    };
    if let Some(c) = base.chars().find(|c| matches!(c, '*' | '?' | '[')) {
        return Err(format!(
            "unexpected {c:?}: the only wildcard is a final '/*' segment"
        ));
    }
    let base = normalize_path(base).map_err(|e| e.to_string())?;
    if file && base.split('/').any(|seg| seg.ends_with('.')) {
        return Err("a file path segment may not end in '.'".into());
    }
    Ok(match (prefix, base.as_str()) {
        (false, _) => base,
        (true, "/") => "/*".to_string(),
        (true, _) => format!("{base}/*"),
    })
}

/// Whether the canonical scope path `outer` admits everything `inner` does.
/// A `/*` path admits whatever folder or reference starts with its base and
/// a `/`, and every path `inner` admits starts with `inner`'s own text, so a
/// string prefix test is exact. A path without `/*` covers only itself.
fn path_covers(outer: &str, inner: &str) -> bool {
    match outer.strip_suffix('*') {
        Some(base) => inner.starts_with(base),
        None => outer == inner,
    }
}

/// Whether the scope path `scope` admits the entity `name` in the folder
/// `path`, by the `include_paths` rule. `path` may lack its leading `/` (a
/// file's folder). An invalid path or name is admitted by nothing.
fn admits(scope: &str, path: &str, name: &str) -> bool {
    let Ok(path) = normalize_path(path) else {
        return false;
    };
    validate_name(name).is_ok() && exclude_pattern_matches(scope, &path, name.trim())
}

/// What one call needs to be allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need<'a> {
    /// Reading or writing the entity `name` in the folder `path`; for a
    /// file, see [`Need::file`].
    Resource {
        access: Access,
        resource: Resource,
        path: &'a str,
        name: &'a str,
    },
    /// Creating, changing or deleting an action of this kind.
    Kind(Kind),
    /// Calling the action `name` in the folder `path`.
    Exec { path: &'a str, name: &'a str },
}

impl<'a> Need<'a> {
    /// Reading or writing the file at `rel_path`, relative to the files
    /// root (`files/docs/doc-1/note.txt`): its folder and its file name.
    pub fn file(access: Access, rel_path: &'a str) -> Self {
        let (path, name) = rel_path.rsplit_once('/').unwrap_or(("", rel_path));
        Need::Resource {
            access,
            resource: Resource::File,
            path,
            name,
        }
    }
}

/// The narrowest scope that grants the need, e.g. `read:document:/a/b` or
/// `write:script`: what a refusal names, and what an audit log tells an
/// operator to grant.
impl fmt::Display for Need<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let target = |path: &str, name: &str| {
            let path = path.trim_matches('/');
            if path.is_empty() {
                format!("/{name}")
            } else {
                format!("/{path}/{name}")
            }
        };
        match *self {
            Need::Resource {
                access,
                resource,
                path,
                name,
            } => write!(
                f,
                "{}:{}:{}",
                access.name(),
                resource.name(),
                target(path, name)
            ),
            Need::Kind(kind) => write!(f, "write:{}", kind.name()),
            Need::Exec { path, name } => write!(f, "exec:{}", target(path, name)),
        }
    }
}

/// How a list, search or path facet is narrowed for one resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathFilter {
    /// Everything is admitted; the query runs as it is.
    Unrestricted,
    /// Only entities matching one of these `include_paths` patterns.
    Only(Vec<String>),
    /// Nothing is admitted. The call must be refused before the query runs:
    /// an empty `include_paths` means *unrestricted*.
    Nothing,
}

/// A set of scopes, kept sorted (by their string form) and without
/// duplicates. Scopes that another in the set covers are kept, so a listing
/// shows what was granted. Serializes as a list of scope strings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<String>", into = "Vec<String>")]
pub struct ScopeSet(Vec<Scope>);

impl ScopeSet {
    pub fn new(scopes: impl IntoIterator<Item = Scope>) -> Self {
        let mut keyed: Vec<(String, Scope)> =
            scopes.into_iter().map(|s| (s.to_string(), s)).collect();
        keyed.sort_by(|a, b| a.0.cmp(&b.0));
        keyed.dedup_by(|a, b| a.0 == b.0);
        ScopeSet(keyed.into_iter().map(|(_, s)| s).collect())
    }

    /// The set holding only [`Scope::All`].
    pub fn all() -> Self {
        ScopeSet(vec![Scope::All])
    }

    /// Parse scope strings, failing on the first bad one.
    pub fn parse<I, S>(scopes: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let parsed = scopes
            .into_iter()
            .map(|s| s.as_ref().parse())
            .collect::<Result<Vec<Scope>>>()?;
        Ok(Self::new(parsed))
    }

    /// The scopes to issue a token with: `scopes` plus what `preset` expands
    /// to. Expanded here, at issue time, so changing a preset later never
    /// widens a token already issued.
    pub fn expand<I, S>(scopes: I, preset: Option<&str>) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut set = Self::parse(scopes)?.0;
        if let Some(preset) = preset {
            set.extend(preset.parse::<Preset>()?.scopes());
        }
        Ok(Self::new(set))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Scope> {
        self.0.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether any scope in the set grants `need`.
    pub fn grants(&self, need: &Need<'_>) -> bool {
        self.0.iter().any(|s| s.grants(need))
    }

    /// Whether every scope in `other` is covered by one in this set: the
    /// test a package's actions are held to against its manifest.
    pub fn covers(&self, other: &ScopeSet) -> bool {
        other.0.iter().all(|o| self.0.iter().any(|s| s.covers(o)))
    }

    /// The `include_paths` patterns that admit what this set grants for
    /// `access` on `resource`: its scope paths, as they stand, since they
    /// already follow the `include_paths` rule.
    pub fn path_filter(&self, access: Access, resource: Resource) -> PathFilter {
        self.filter(|scope| match scope {
            Scope::Resource {
                access: a,
                resource: r,
                path,
            } if *a == access && *r == resource => Some(path),
            _ => None,
        })
    }

    /// [`Self::path_filter`] for the actions this set may call: what a
    /// model is shown when it discovers tools.
    pub fn exec_filter(&self) -> PathFilter {
        self.filter(|scope| match scope {
            Scope::Exec { path } => Some(path),
            _ => None,
        })
    }

    /// The paths of the scopes `select` picks, as a filter.
    fn filter<'s>(&'s self, select: impl Fn(&'s Scope) -> Option<&'s String>) -> PathFilter {
        let mut paths = Vec::new();
        for scope in &self.0 {
            if *scope == Scope::All {
                return PathFilter::Unrestricted;
            }
            if let Some(path) = select(scope) {
                if path == "/*" {
                    return PathFilter::Unrestricted;
                }
                paths.push(path.clone());
            }
        }
        if paths.is_empty() {
            return PathFilter::Nothing;
        }
        PathFilter::Only(paths)
    }
}

impl TryFrom<Vec<String>> for ScopeSet {
    type Error = SolxError;

    fn try_from(scopes: Vec<String>) -> Result<Self> {
        Self::parse(scopes)
    }
}

impl From<ScopeSet> for Vec<String> {
    fn from(set: ScopeSet) -> Self {
        set.0.iter().map(Scope::to_string).collect()
    }
}

impl fmt::Display for ScopeSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<String> = self.0.iter().map(Scope::to_string).collect();
        f.write_str(&names.join(" "))
    }
}

/// What the party a call is checked against may do: its token's or its
/// calling action's scopes, or, for a local entry point, everything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Grant {
    Unlimited,
    Scoped(Arc<ScopeSet>),
}

/// A `'static` [`Grant::Unlimited`], for constants that borrow a grant.
pub const UNLIMITED: &Grant = &Grant::Unlimited;

impl Grant {
    pub fn scoped(set: ScopeSet) -> Self {
        Grant::Scoped(Arc::new(set))
    }

    /// A grant of nothing at all.
    pub fn none() -> Self {
        Self::scoped(ScopeSet::default())
    }

    /// The grant for a token row's stored scopes. `None` is a row from
    /// before scopes existed, which keeps working as `all`.
    pub fn from_stored(stored: Option<ScopeSet>) -> Self {
        Self::scoped(stored.unwrap_or_else(ScopeSet::all))
    }

    /// The grant the action in `path` runs with: [`action_scopes`].
    pub fn for_action(path: &str, action_config: Option<&Value>) -> Self {
        Self::scoped(action_scopes(path, action_config))
    }

    pub fn is_unlimited(&self) -> bool {
        matches!(self, Grant::Unlimited)
    }

    pub fn grants(&self, need: &Need<'_>) -> bool {
        match self {
            Grant::Unlimited => true,
            Grant::Scoped(set) => set.grants(need),
        }
    }

    /// See [`ScopeSet::path_filter`].
    pub fn path_filter(&self, access: Access, resource: Resource) -> PathFilter {
        match self {
            Grant::Unlimited => PathFilter::Unrestricted,
            Grant::Scoped(set) => set.path_filter(access, resource),
        }
    }

    /// See [`ScopeSet::exec_filter`].
    pub fn exec_filter(&self) -> PathFilter {
        match self {
            Grant::Unlimited => PathFilter::Unrestricted,
            Grant::Scoped(set) => set.exec_filter(),
        }
    }
}

/// How a failed scope check is treated: `scope_enforcement` in
/// `solx-config.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Enforcement {
    /// Checks pass whatever they find.
    Off,
    /// A failed check is logged and allowed, so the scopes an action or a
    /// client really uses can be found before they are enforced.
    Audit,
    /// A failed check is refused with [`SolxError::Forbidden`]. The default.
    #[default]
    Enforce,
}

impl Enforcement {
    pub fn name(self) -> &'static str {
        match self {
            Enforcement::Off => "off",
            Enforcement::Audit => "audit",
            Enforcement::Enforce => "enforce",
        }
    }
}

impl FromStr for Enforcement {
    type Err = SolxError;

    fn from_str(s: &str) -> Result<Self> {
        Ok(match s.trim() {
            "off" => Enforcement::Off,
            "audit" => Enforcement::Audit,
            "enforce" => Enforcement::Enforce,
            other => {
                return Err(SolxError::Invalid(format!(
                    "unknown scope_enforcement '{other}' (expected off, audit or enforce)"
                )))
            }
        })
    }
}

/// A named bundle of scopes offered when issuing a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// Read every resource.
    ReadOnly,
    /// Discover and call actions.
    Tools,
    /// `ReadOnly` plus writing documents, types and files.
    Editor,
    /// `all`.
    Full,
}

impl Preset {
    pub fn scopes(self) -> Vec<Scope> {
        let resource = |access, resource| Scope::Resource {
            access,
            resource,
            path: "/*".into(),
        };
        let read_all = || {
            [
                Resource::Document,
                Resource::Type,
                Resource::Action,
                Resource::File,
            ]
            .map(|r| resource(Access::Read, r))
        };
        match self {
            Preset::ReadOnly => read_all().into(),
            Preset::Tools => vec![
                resource(Access::Read, Resource::Action),
                Scope::Exec { path: "/*".into() },
            ],
            Preset::Editor => {
                let mut scopes: Vec<Scope> = read_all().into();
                scopes.extend(
                    [Resource::Document, Resource::Type, Resource::File]
                        .map(|r| resource(Access::Write, r)),
                );
                scopes
            }
            Preset::Full => vec![Scope::All],
        }
    }
}

impl FromStr for Preset {
    type Err = SolxError;

    fn from_str(s: &str) -> Result<Self> {
        Ok(match s.trim() {
            "read-only" => Preset::ReadOnly,
            "tools" => Preset::Tools,
            "editor" => Preset::Editor,
            "full" => Preset::Full,
            other => {
                return Err(SolxError::Invalid(format!(
                    "unknown preset '{other}' (expected read-only, tools, editor or full)"
                )))
            }
        })
    }
}

/// The scopes an action declares in `action_config.scopes`, which it is
/// held to when it calls anything. `None` when it declares none, which
/// grants nothing. Anything but a list of valid scope strings is an error.
pub fn declared_scopes(action_config: Option<&Value>) -> Result<Option<ScopeSet>> {
    let Some(scopes) = action_config.and_then(|c| c.get("scopes")) else {
        return Ok(None);
    };
    let not_a_list =
        || SolxError::Invalid("action_config.scopes must be a list of scope strings".into());
    let items = scopes.as_array().ok_or_else(not_a_list)?;
    let names = items
        .iter()
        .map(|v| v.as_str().ok_or_else(not_a_list))
        .collect::<Result<Vec<&str>>>()?;
    ScopeSet::parse(names).map(Some)
}

/// The scopes the action in `path` runs with: those it declares in
/// `action_config.scopes` (none if it declares none, or a malformed list,
/// which a save refuses), plus, for a package's action
/// (`/packages/<name>/...`), `exec:/packages/<name>/*`, so a package can
/// always call its own actions.
pub fn action_scopes(path: &str, action_config: Option<&Value>) -> ScopeSet {
    let declared = declared_scopes(action_config)
        .ok()
        .flatten()
        .unwrap_or_default();
    match package_of(path) {
        Some(package) => ScopeSet::new(declared.iter().cloned().chain([Scope::Exec {
            path: format!("/packages/{package}/*"),
        }])),
        None => declared,
    }
}

/// The package an action in `path` belongs to: the segment after a
/// leading `/packages/`. The action `/packages/p` (in path `/packages`)
/// belongs to none. Matched exactly, like every scope path.
pub fn package_of(path: &str) -> Option<&str> {
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    match (segments.next(), segments.next()) {
        (Some("packages"), Some(package)) => Some(package),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{is_included_by, validate_exclude_pattern};
    use serde_json::json;

    fn set(scopes: &[&str]) -> ScopeSet {
        ScopeSet::parse(scopes).unwrap()
    }

    fn split(full: &str) -> (&str, &str) {
        match full.rsplit_once('/') {
            Some(("", name)) => ("/", name),
            Some((path, name)) => (path, name),
            None => ("/", full),
        }
    }

    fn read_doc(full: &str) -> Need<'_> {
        let (path, name) = split(full);
        Need::Resource {
            access: Access::Read,
            resource: Resource::Document,
            path,
            name,
        }
    }

    fn exec(full: &str) -> Need<'_> {
        let (path, name) = split(full);
        Need::Exec { path, name }
    }

    #[test]
    fn parses_and_displays_every_form() {
        for s in [
            "all",
            "read:document",
            "read:document:/",
            "write:type:/research/*",
            "read:action:/my/tools/build",
            "write:file:/files/docs/*",
            "write:script",
            "write:wasm",
            "write:command",
            "write:webhook",
            "exec:/*",
            "exec:/builtin/web/*",
            "exec:/builtin/web/http-request",
        ] {
            assert_eq!(s.parse::<Scope>().unwrap().to_string(), s);
        }
        assert_eq!(
            "read:document".parse::<Scope>().unwrap(),
            "read:document:/*".parse::<Scope>().unwrap()
        );
        assert_ne!(
            "read:document:/".parse::<Scope>().unwrap(),
            "read:document:/*".parse::<Scope>().unwrap()
        );
        assert_eq!(
            " exec://builtin//web//* "
                .parse::<Scope>()
                .unwrap()
                .to_string(),
            "exec:/builtin/web/*"
        );
    }

    #[test]
    fn parsing_is_strict() {
        for bad in [
            "",
            "everything",
            "delete:document",
            "read:docs",
            "read:script",
            "write:script:/x",
            "exec:",
            "exec:builtin/web",
            "exec:*",
            "read:document:",
            "write:document:/a*",
            "write:document:/*/a",
            "exec:/a/**",
            "exec:/a?",
            "exec:/a[b]",
            "read:document:/a:b",
            "exec:/a/../b",
            "write:file:/files/docs.",
            "write:file:/files/docs./*",
        ] {
            assert!(bad.parse::<Scope>().is_err(), "{bad:?} parsed");
        }
        let err = "read:docs".parse::<Scope>().unwrap_err().to_string();
        assert!(err.contains("read:docs"), "{err}");
    }

    #[test]
    fn a_plain_path_names_an_entity_and_its_folder() {
        let one = set(&["exec:/builtin/web/http-request"]);
        assert!(one.grants(&exec("/builtin/web/http-request")));
        assert!(!one.grants(&exec("/builtin/web/http-requests")));
        assert!(!one.grants(&exec("/builtin/web/stream-start")));

        // The include_paths rule: the entity, and the direct members of the
        // folder of that name, never deeper.
        let doc = set(&["read:document:/path/name"]);
        assert!(doc.grants(&read_doc("/path/name")));
        assert!(doc.grants(&read_doc("/path/name/x")));
        assert!(!doc.grants(&read_doc("/path/name/sub/x")));
        assert!(!doc.grants(&read_doc("/path/other")));

        let folder = set(&["exec:/builtin/web"]);
        assert!(folder.grants(&exec("/builtin/web/http-request")));
        assert!(!folder.grants(&exec("/builtin/web/stream/start")));
    }

    #[test]
    fn a_star_path_admits_a_subtree() {
        let web = set(&["exec:/builtin/web/*"]);
        assert!(web.grants(&exec("/builtin/web/http-request")));
        assert!(web.grants(&exec("/builtin/web/stream/start")));
        assert!(!web.grants(&exec("/builtin/webby/x")));
        assert!(!web.grants(&exec("/builtin/web")));

        assert!(set(&["exec:/*"]).grants(&exec("/anything/at/all")));
        assert!(set(&["exec:/*"]).grants(&exec("/top")));

        let docs = set(&["read:document:/path/*"]);
        for ok in ["/path/name", "/path/sub/x"] {
            assert!(docs.grants(&read_doc(ok)), "{ok}");
        }
        for no in ["/path", "/pathology/x", "/other/path"] {
            assert!(!docs.grants(&read_doc(no)), "{no}");
        }
    }

    #[test]
    fn root_without_a_star_is_its_direct_members() {
        let root = set(&["read:document:/"]);
        assert!(root.grants(&read_doc("/note")));
        assert!(!root.grants(&read_doc("/a/note")));
    }

    #[test]
    fn invalid_targets_are_admitted_by_nothing() {
        let all_docs = set(&["read:document"]);
        let bad = |path, name| Need::Resource {
            access: Access::Read,
            resource: Resource::Document,
            path,
            name,
        };
        assert!(!all_docs.grants(&bad("/a:b", "x")));
        assert!(!all_docs.grants(&bad("/a", "x/y")));
        assert!(!all_docs.grants(&bad("/a", "")));
    }

    #[test]
    fn matching_is_case_sensitive() {
        assert!(!set(&["read:document:/A/*"]).grants(&read_doc("/a/x")));
        assert!(!set(&["exec:/builtin/*"]).grants(&exec("/Builtin/web/x")));
    }

    #[test]
    fn file_needs_split_the_relative_path() {
        let files = set(&["write:file:/files/docs/*"]);
        let write = |rel| Need::file(Access::Write, rel);
        assert!(files.grants(&write("files/docs/doc-1/note.txt")));
        assert!(files.grants(&write("files/docs/note.txt")));
        assert!(!files.grants(&write("files/docsx/note.txt")));
        assert!(!files.grants(&write("files/other.txt")));
        assert!(set(&["write:file:/top.txt"]).grants(&write("top.txt")));
        assert_eq!(
            write("a/b/c.txt"),
            Need::Resource {
                access: Access::Write,
                resource: Resource::File,
                path: "a/b",
                name: "c.txt",
            }
        );
    }

    #[test]
    fn scopes_do_not_imply_each_other() {
        let s = set(&["write:document", "exec:/*"]);
        assert!(!s.grants(&read_doc("/a")));
        assert!(!s.grants(&Need::Resource {
            access: Access::Write,
            resource: Resource::Action,
            path: "/",
            name: "a",
        }));
        assert!(!s.grants(&Need::Kind(Kind::Script)));
        assert!(set(&["write:script"]).grants(&Need::Kind(Kind::Script)));
        assert!(!set(&["write:script"]).grants(&Need::Kind(Kind::Command)));
    }

    #[test]
    fn all_grants_everything() {
        let all = ScopeSet::all();
        assert!(all.grants(&read_doc("/x")));
        assert!(all.grants(&Need::Kind(Kind::Command)));
        assert!(all.grants(&exec("/builtin/env/get-env")));
    }

    #[test]
    fn covers_is_subset_by_coverage() {
        let wide = set(&["exec:/builtin/web/*", "read:document"]);
        assert!(wide.covers(&set(&["exec:/builtin/web/http-request"])));
        assert!(wide.covers(&set(&["exec:/builtin/web/stream/*"])));
        assert!(wide.covers(&set(&["read:document:/a", "read:document:/a/*"])));
        assert!(!wide.covers(&set(&["exec:/builtin/web"])));
        assert!(!wide.covers(&set(&["exec:/builtin/*"])));
        assert!(!wide.covers(&set(&["exec:/builtin/webby/*"])));
        assert!(!wide.covers(&set(&["write:document"])));

        let exact = set(&["exec:/builtin/web"]);
        assert!(exact.covers(&exact));
        assert!(!exact.covers(&set(&["exec:/builtin/web/http-request"])));
        assert!(!exact.covers(&set(&["exec:/builtin/web/*"])));

        assert!(ScopeSet::all().covers(&set(&["write:command", "exec:/*"])));
        assert!(!wide.covers(&ScopeSet::all()));
        assert!(wide.covers(&ScopeSet::default()));
    }

    /// `covers` must agree with what the scopes admit: whenever one scope
    /// covers another, it grants everything the other grants.
    #[test]
    fn covers_agrees_with_grants() {
        let paths = ["/a", "/a/*", "/a/b", "/a/b/*", "/ab", "/ab/*", "/", "/*"];
        let targets = [
            "/a", "/a/b", "/a/b/c", "/a/b/c/d", "/ab", "/ab/c", "/x", "/x/a",
        ];
        for outer in paths {
            for inner in paths {
                let o = set(&[format!("exec:{outer}").as_str()]);
                let i = set(&[format!("exec:{inner}").as_str()]);
                if !o.covers(&i) {
                    continue;
                }
                for t in targets {
                    if i.grants(&exec(t)) {
                        assert!(o.grants(&exec(t)), "{outer} covers {inner} but not {t}");
                    }
                }
            }
        }
    }

    #[test]
    fn presets_expand_to_their_tables() {
        let expand = |p| ScopeSet::expand(Vec::<String>::new(), Some(p)).unwrap();
        assert_eq!(
            expand("read-only"),
            set(&["read:document", "read:type", "read:action", "read:file"])
        );
        assert_eq!(expand("tools"), set(&["read:action", "exec:/*"]));
        assert_eq!(
            expand("editor"),
            set(&[
                "read:document",
                "read:type",
                "read:action",
                "read:file",
                "write:document",
                "write:type",
                "write:file",
            ])
        );
        assert!(expand("editor").covers(&expand("read-only")));
        assert_eq!(expand("full"), ScopeSet::all());
        assert_eq!(
            ScopeSet::expand(["exec:/packages/solx-google/*"], Some("tools")).unwrap(),
            set(&["read:action", "exec:/*", "exec:/packages/solx-google/*"])
        );
        assert!(ScopeSet::expand(Vec::<String>::new(), Some("admin")).is_err());
    }

    #[test]
    fn sets_are_sorted_and_deduplicated() {
        let s = set(&["exec:/*", "read:document:/*", "read:document", "exec:/a"]);
        assert_eq!(
            Vec::<String>::from(s),
            vec!["exec:/*", "exec:/a", "read:document"]
        );
    }

    #[test]
    fn path_filter_never_fails_open() {
        use PathFilter::*;
        let filter = |s: &[&str]| set(s).path_filter(Access::Read, Resource::Document);
        assert_eq!(filter(&[]), Nothing);
        assert_eq!(filter(&["write:document", "read:type"]), Nothing);
        assert_eq!(filter(&["read:document"]), Unrestricted);
        assert_eq!(filter(&["read:document:/a", "read:document"]), Unrestricted);
        assert_eq!(filter(&["all"]), Unrestricted);
        assert_eq!(
            filter(&["read:document:/a", "read:document:/b/c/*"]),
            Only(vec!["/a".into(), "/b/c/*".into()])
        );
        assert_eq!(filter(&["read:document:/"]), Only(vec!["/".into()]));
        if let Only(patterns) = filter(&["read:document:/a", "read:document:/b/*"]) {
            for p in patterns {
                assert_eq!(validate_exclude_pattern(&p).unwrap(), p);
            }
        }
        assert_eq!(
            UNLIMITED.path_filter(Access::Write, Resource::File),
            Unrestricted
        );
    }

    /// The list filter (`path_filter`, rendered to SQL by the stores) and the
    /// single-entity check (`grants`) must agree. `is_included_by` is the
    /// Rust twin of the SQL renderer, so this needs no database.
    #[test]
    fn path_filter_agrees_with_grants() {
        let rows = [
            ("/", "path"),
            ("/", "pathology"),
            ("/", "other"),
            ("/path", "name"),
            ("/path", "sub"),
            ("/path/name", "x"),
            ("/path/sub", "x"),
            ("/path/sub/deeper", "y"),
            ("/pathology", "x"),
            ("/other", "path"),
            ("/other/path", "x"),
            ("/a", "b"),
            ("/b", "a"),
            ("/ab", "c"),
        ];
        for grants in [
            vec!["read:document:/path"],
            vec!["read:document:/path/*"],
            vec!["read:document:/path/name"],
            vec!["read:document:/path/sub/deeper/*"],
            vec!["read:document:/a", "read:document:/b/*"],
            vec!["read:document:/other/path"],
            vec!["read:document:/"],
        ] {
            let s = set(&grants);
            let PathFilter::Only(patterns) = s.path_filter(Access::Read, Resource::Document) else {
                panic!("{grants:?} should narrow");
            };
            for (path, name) in rows {
                let need = Need::Resource {
                    access: Access::Read,
                    resource: Resource::Document,
                    path,
                    name,
                };
                assert_eq!(
                    s.grants(&need),
                    is_included_by(&patterns, path, name),
                    "{grants:?} on {path} {name}"
                );
            }
        }
    }

    #[test]
    fn serializes_as_a_list_of_strings() {
        let s = set(&["write:script", "exec:/builtin/web/*"]);
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v, json!(["exec:/builtin/web/*", "write:script"]));
        assert_eq!(serde_json::from_value::<ScopeSet>(v).unwrap(), s);
        assert!(serde_json::from_value::<ScopeSet>(json!(["exec:"])).is_err());
        assert!(serde_json::from_value::<ScopeSet>(json!("all")).is_err());
    }

    #[test]
    fn reads_declared_scopes() {
        assert_eq!(declared_scopes(None).unwrap(), None);
        assert_eq!(
            declared_scopes(Some(&json!({ "secrets": [] }))).unwrap(),
            None
        );
        assert_eq!(
            declared_scopes(Some(
                &json!({ "scopes": ["exec:/builtin/web/http-request"] })
            ))
            .unwrap(),
            Some(set(&["exec:/builtin/web/http-request"]))
        );
        assert_eq!(
            declared_scopes(Some(&json!({ "scopes": [] }))).unwrap(),
            Some(ScopeSet::default())
        );
        for bad in [
            json!({ "scopes": "all" }),
            json!({ "scopes": "inherit" }),
            json!({ "scopes": null }),
            json!({ "scopes": [1] }),
            json!({ "scopes": ["exec:"] }),
        ] {
            assert!(declared_scopes(Some(&bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn grants() {
        assert!(UNLIMITED.is_unlimited());
        assert!(UNLIMITED.grants(&Need::Kind(Kind::Webhook)));
        assert!(UNLIMITED.grants(&read_doc("/x")));

        let legacy = Grant::from_stored(None);
        assert!(!legacy.is_unlimited());
        assert!(legacy.grants(&exec("/builtin/env/get-env")));

        let narrow = Grant::scoped(set(&["exec:/builtin/web/http-request"]));
        assert!(narrow.grants(&exec("/builtin/web/http-request")));
        assert!(!narrow.grants(&exec("/builtin/env/get-env")));
        assert!(!Grant::none().grants(&exec("/a")));
    }

    /// A need displays as the narrowest scope that grants it.
    #[test]
    fn a_need_names_the_scope_that_grants_it() {
        let needs = [
            (read_doc("/a/b"), "read:document:/a/b"),
            (read_doc("/top"), "read:document:/top"),
            (
                exec("/builtin/web/http-request"),
                "exec:/builtin/web/http-request",
            ),
            (Need::Kind(Kind::Wasm), "write:wasm"),
            (
                Need::file(Access::Write, "files/docs/x.txt"),
                "write:file:/files/docs/x.txt",
            ),
            (Need::file(Access::Read, "top.txt"), "read:file:/top.txt"),
        ];
        for (need, shown) in needs {
            assert_eq!(need.to_string(), shown);
            let scope = set(&[shown]);
            assert!(scope.grants(&need), "{shown}");
        }
    }

    #[test]
    fn exec_filter_follows_exec_scopes_only() {
        use PathFilter::*;
        assert_eq!(set(&["read:action"]).exec_filter(), Nothing);
        assert_eq!(set(&["exec:/*"]).exec_filter(), Unrestricted);
        assert_eq!(ScopeSet::all().exec_filter(), Unrestricted);
        assert_eq!(
            set(&["exec:/t/*", "exec:/builtin/web/http-request", "read:action"]).exec_filter(),
            Only(vec!["/builtin/web/http-request".into(), "/t/*".into()])
        );
        assert_eq!(UNLIMITED.exec_filter(), Unrestricted);
    }

    #[test]
    fn enforcement_parses_strictly() {
        for e in [Enforcement::Off, Enforcement::Audit, Enforcement::Enforce] {
            assert_eq!(e.name().parse::<Enforcement>().unwrap(), e);
        }
        assert_eq!(Enforcement::default(), Enforcement::Enforce);
        assert!("strict".parse::<Enforcement>().is_err());
    }

    #[test]
    fn an_action_is_granted_what_it_declares_or_nothing() {
        let declared = json!({ "scopes": ["exec:/builtin/web/http-request"] });
        let grant = Grant::for_action("/t", Some(&declared));
        assert!(grant.grants(&exec("/builtin/web/http-request")));
        assert!(!grant.grants(&exec("/builtin/env/get-env")));

        for config in [None, Some(json!({})), Some(json!({ "scopes": "all" }))] {
            assert_eq!(Grant::for_action("/t", config.as_ref()), Grant::none());
        }
    }

    /// A package's action may always call the package's own actions, and
    /// nothing else it doesn't declare.
    #[test]
    fn a_package_action_may_call_its_own_package() {
        let grant = Grant::for_action("/packages/p/sub", None);
        assert!(grant.grants(&exec("/packages/p/other")));
        assert!(grant.grants(&exec("/packages/p/deep/x")));
        assert!(!grant.grants(&exec("/packages/q/x")));
        assert!(!grant.grants(&exec("/builtin/web/http-request")));

        let declared = json!({ "scopes": ["write:script"] });
        assert_eq!(
            action_scopes("/packages/p", Some(&declared)).to_string(),
            "exec:/packages/p/* write:script"
        );
        for outside in ["/packages", "/", "/Packages/p", "/my/packages/p"] {
            assert_eq!(package_of(outside), None, "{outside}");
        }
    }

    #[test]
    fn kinds_follow_action_types() {
        assert_eq!(
            Kind::for_action_type(ActionType::Script),
            Some(Kind::Script)
        );
        assert_eq!(Kind::for_action_type(ActionType::Wasm), Some(Kind::Wasm));
        assert_eq!(
            Kind::for_action_type(ActionType::Command),
            Some(Kind::Command)
        );
        assert_eq!(
            Kind::for_action_type(ActionType::Webhook),
            Some(Kind::Webhook)
        );
        assert_eq!(Kind::for_action_type(ActionType::Internal), None);
    }
}
