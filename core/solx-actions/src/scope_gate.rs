//! Scope checks: whether a call is within the scopes of whoever makes it,
//! under the configured `scope_enforcement`.
//!
//! Every surface builds a [`ScopeCheck`] from what it knows about the
//! caller — an [`ExecContext`] (dispatch and the built-ins), an [`Actor`]
//! (the `authorize_*` entry points) or a [`Principal`] (REST) — and asks it
//! one of three things:
//!
//! * [`ScopeCheck::require`]: may this one call happen?
//! * [`ScopeCheck::narrow`]: write the list filter that keeps a list,
//!   search or path facet to what the scopes admit;
//! * [`ScopeCheck::filter_file_reads`]: which files of a listing to show,
//!   since the file store has no SQL to narrow.
//!
//! What a failed check does depends on [`Enforcement`]: nothing (`off`), a
//! log line, once per party and need (`audit`), or a refusal with
//! [`SolxError::Forbidden`] (`enforce`). The mode is read on every check, so
//! a change to `solx-config.json` applies to the next call.

use std::collections::HashSet;
use std::fmt;
use std::sync::{Mutex, OnceLock};

use solx_config::{ConfigService, EntityKind};
use solx_surface::error::{Result, SolxError};
use solx_surface::scope::{Access, Enforcement, Grant, Need, PathFilter, Resource};
use solx_surface::{Actor, ExecContext, ExecContextInfo, Principal};

/// One caller's scopes, ready to check calls against. See the module doc.
pub struct ScopeCheck<'a> {
    config: &'a ConfigService,
    grant: &'a Grant,
    /// Who the scopes belong to, for messages: `action /p/a`, `client
    /// 'laptop' (id)`, `the caller`.
    party: String,
}

impl<'a> ScopeCheck<'a> {
    pub fn new(config: &'a ConfigService, grant: &'a Grant, party: String) -> Self {
        ScopeCheck {
            config,
            grant,
            party,
        }
    }

    /// The checks for a call made from this frame: against the calling
    /// action's scopes, else the token's ([`ExecContext::grant`]).
    pub fn for_exec(config: &'a ConfigService, ctx: &'a ExecContext) -> Self {
        let party = describe(ctx.caller().map(|c| c.action_ref()), ctx.principal());
        Self::new(config, ctx.grant(), party)
    }

    /// [`Self::for_exec`] for a detached run that hasn't started yet.
    pub fn for_exec_info(config: &'a ConfigService, ctx: &'a ExecContextInfo) -> Self {
        let party = describe(ctx.caller().map(|c| c.action_ref()), ctx.principal());
        Self::new(config, ctx.grant(), party)
    }

    /// The checks for a change `actor` makes.
    pub fn for_actor(config: &'a ConfigService, actor: Actor<'a>) -> Self {
        Self::new(config, actor.grant(), describe(actor.acting(), None))
    }

    /// The checks for a REST call made with `principal`'s token.
    pub fn for_principal(config: &'a ConfigService, principal: &'a Principal) -> Self {
        Self::new(config, principal.grant(), describe(None, Some(principal)))
    }

    /// The config these checks read their mode from.
    pub fn config(&self) -> &'a ConfigService {
        self.config
    }

    /// Refuse `need` unless the scopes grant it (under `enforce`).
    pub fn require(&self, need: &Need<'_>) -> Result<()> {
        if self.grant.grants(need) {
            return Ok(());
        }
        match self.config.scope_enforcement() {
            Enforcement::Off => Ok(()),
            Enforcement::Audit => {
                audit(&self.party, need);
                Ok(())
            }
            Enforcement::Enforce => Err(SolxError::Forbidden(format!(
                "{} may not do this: it needs {need}",
                self.party
            ))),
        }
    }

    /// Whether the scopes grant `need`, whatever the mode: for a caller
    /// that checks many needs and reports only the first it lacks.
    pub fn grants(&self, need: &Need<'_>) -> bool {
        self.grant.grants(need)
    }

    /// Keep the files of a listing the scopes let the caller read (under
    /// `enforce`; otherwise all of them). Under `audit` a listing that
    /// would lose some is logged once, not once per file.
    pub fn filter_file_reads(&self, entries: Vec<String>) -> Vec<String> {
        let readable = |e: &String| self.grants(&Need::file(Access::Read, e));
        let Some(first) = entries.iter().find(|e| !readable(e)) else {
            return entries;
        };
        match self.config.scope_enforcement() {
            Enforcement::Off => entries,
            Enforcement::Audit => {
                audit(&self.party, &Need::file(Access::Read, first));
                entries
            }
            Enforcement::Enforce => entries.into_iter().filter(readable).collect(),
        }
    }

    /// Narrow a list, search or path facet to what `filter` admits, by
    /// writing `include_paths`. `scope` names the grant it needed (e.g.
    /// `read:document`), for messages. Under `enforce`, a filter that
    /// admits nothing refuses the call, since an empty `include_paths`
    /// would mean everything; and `include_paths` already set by trusted
    /// code is refused rather than overwritten, since one list can't hold
    /// the intersection of two.
    pub fn narrow(&self, filter: PathFilter, scope: &str, include: &mut Vec<String>) -> Result<()> {
        let mode = self.config.scope_enforcement();
        match (filter, mode) {
            (PathFilter::Unrestricted, _) | (_, Enforcement::Off) => Ok(()),
            (PathFilter::Nothing, Enforcement::Audit) => {
                audit_text(&self.party, &format!("{scope} (any path)"));
                Ok(())
            }
            (PathFilter::Only(paths), Enforcement::Audit) => {
                audit_text(&self.party, &format!("{scope} beyond {}", paths.join(", ")));
                Ok(())
            }
            (PathFilter::Nothing, Enforcement::Enforce) => Err(SolxError::Forbidden(format!(
                "{} may not do this: it needs {scope}",
                self.party
            ))),
            (PathFilter::Only(paths), Enforcement::Enforce) => {
                if !include.is_empty() {
                    return Err(SolxError::Invalid(
                        "include_paths is already set; it can't also be narrowed to the \
                         caller's scopes"
                            .into(),
                    ));
                }
                *include = paths;
                Ok(())
            }
        }
    }

    /// [`Self::narrow`] for reading `resource`.
    pub fn narrow_reads(&self, resource: Resource, include: &mut Vec<String>) -> Result<()> {
        self.narrow(
            self.grant.path_filter(Access::Read, resource),
            &format!("read:{}", resource_name(resource)),
            include,
        )
    }

    /// [`Self::narrow`] for the actions the caller may call.
    pub fn narrow_exec(&self, include: &mut Vec<String>) -> Result<()> {
        self.narrow(self.grant.exec_filter(), "exec", include)
    }

    /// Refuse a list of `resource` outright when the scopes admit none of
    /// it (under `enforce`). [`Self::narrow_reads`] does the same, but
    /// callers whose errors lose their kind check this first.
    pub fn require_some_reads(&self, resource: Resource) -> Result<()> {
        if self.grant.path_filter(Access::Read, resource) != PathFilter::Nothing {
            return Ok(());
        }
        self.narrow(
            PathFilter::Nothing,
            &format!("read:{}", resource_name(resource)),
            &mut Vec::new(),
        )
    }
}

/// The resource a configured entity kind is.
pub fn resource_of(kind: EntityKind) -> Resource {
    match kind {
        EntityKind::Document => Resource::Document,
        EntityKind::Type => Resource::Type,
        EntityKind::Action => Resource::Action,
    }
}

fn resource_name(resource: Resource) -> &'static str {
    match resource {
        Resource::Document => "document",
        Resource::Type => "type",
        Resource::Action => "action",
        Resource::File => "file",
    }
}

/// Who a call's scopes belong to, for messages: the acting action if there
/// is one, else the token.
pub fn describe(acting: Option<&str>, principal: Option<&Principal>) -> String {
    if let Some(action) = acting {
        return format!("action {action}");
    }
    match principal {
        Some(Principal::Client {
            id,
            label: Some(label),
            ..
        }) => format!("client '{label}' ({id})"),
        Some(Principal::Client { id, .. }) => format!("client {id}"),
        Some(Principal::Action { action_ref, .. }) => format!("action {action_ref}"),
        _ => "the caller".into(),
    }
}

/// Log, once per process, that `party` made a call its scopes don't allow.
fn audit(party: &str, need: &Need<'_>) {
    audit_text(party, &need.to_string());
}

fn audit_text(party: &str, needed: &dyn fmt::Display) {
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    // Bounded, so a long-lived host can't grow it without limit; past the
    // bound a repeat is logged again, which is harmless.
    const MAX_SEEN: usize = 10_000;
    let key = format!("{party}\u{0}{needed}");
    if let Ok(mut seen) = SEEN.get_or_init(Mutex::default).lock() {
        if seen.len() >= MAX_SEEN {
            seen.clear();
        }
        if !seen.insert(key) {
            return;
        }
    }
    tracing::warn!(
        target: "solx::scopes",
        "scope audit: {party} needs {needed}, which its scopes don't grant \
         (allowed because scope_enforcement is \"audit\")"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use solx_surface::scope::ScopeSet;

    fn config(mode: &str) -> (tempfile::TempDir, ConfigService) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ConfigService::open_in(dir.path()).unwrap();
        cfg.set("scope_enforcement", mode.into()).unwrap();
        (dir, cfg)
    }

    fn grant(scopes: &[&str]) -> Grant {
        Grant::scoped(ScopeSet::parse(scopes).unwrap())
    }

    fn read_doc<'a>(path: &'a str, name: &'a str) -> Need<'a> {
        Need::Resource {
            access: Access::Read,
            resource: Resource::Document,
            path,
            name,
        }
    }

    #[test]
    fn only_enforce_refuses() {
        let g = grant(&["read:document:/a/*"]);
        for (mode, refused) in [("off", false), ("audit", false), ("enforce", true)] {
            let (_d, cfg) = config(mode);
            let check = ScopeCheck::new(&cfg, &g, "client c".into());
            assert!(check.require(&read_doc("/a", "x")).is_ok());
            let outside = check.require(&read_doc("/b", "x"));
            assert_eq!(outside.is_err(), refused, "{mode}");
            if let Err(e) = outside {
                assert!(matches!(e, SolxError::Forbidden(_)), "{e:?}");
                assert!(e.to_string().contains("client c"), "{e}");
                assert!(e.to_string().contains("read:document:/b/x"), "{e}");
            }
        }
    }

    #[test]
    fn narrowing_writes_the_filter_only_when_enforcing() {
        let g = grant(&["read:document:/a/*"]);
        for (mode, narrowed) in [("off", false), ("audit", false), ("enforce", true)] {
            let (_d, cfg) = config(mode);
            let check = ScopeCheck::new(&cfg, &g, "c".into());
            let mut include = Vec::new();
            check
                .narrow_reads(Resource::Document, &mut include)
                .unwrap();
            assert_eq!(include.is_empty(), !narrowed, "{mode}");
        }

        let (_d, cfg) = config("enforce");
        let check = ScopeCheck::new(&cfg, &g, "c".into());
        let mut include = Vec::new();
        check
            .narrow_reads(Resource::Document, &mut include)
            .unwrap();
        assert_eq!(include, vec!["/a/*".to_string()]);

        // A trusted caller's own filter is refused, not overwritten.
        let mut preset = vec!["/z/*".to_string()];
        assert!(check.narrow_reads(Resource::Document, &mut preset).is_err());
        assert_eq!(preset, vec!["/z/*".to_string()]);
    }

    /// No grant for the resource refuses the list rather than leaving
    /// `include_paths` empty, which would mean everything.
    #[test]
    fn a_list_with_no_grant_is_refused_not_unrestricted() {
        let (_d, cfg) = config("enforce");
        let g = grant(&["read:type"]);
        let check = ScopeCheck::new(&cfg, &g, "c".into());
        let mut include = Vec::new();
        let err = check
            .narrow_reads(Resource::Document, &mut include)
            .unwrap_err();
        assert!(matches!(err, SolxError::Forbidden(_)), "{err:?}");
        assert!(check.require_some_reads(Resource::Document).is_err());
        assert!(check.require_some_reads(Resource::Type).is_ok());

        let unlimited = Grant::Unlimited;
        let check = ScopeCheck::new(&cfg, &unlimited, "c".into());
        check
            .narrow_reads(Resource::Document, &mut include)
            .unwrap();
        assert!(include.is_empty());
    }

    /// A file listing drops what may not be read only when enforcing.
    #[test]
    fn file_listings_filter_only_when_enforcing() {
        let g = grant(&["read:file:/files/a/*"]);
        let entries = || vec!["files/a/x".to_string(), "files/b/y".to_string()];
        for (mode, kept) in [("off", 2), ("audit", 2), ("enforce", 1)] {
            let (_d, cfg) = config(mode);
            let check = ScopeCheck::new(&cfg, &g, "c".into());
            assert_eq!(check.filter_file_reads(entries()).len(), kept, "{mode}");
        }
    }

    #[test]
    fn describes_the_party() {
        let client = Principal::Client {
            id: "id1".into(),
            label: Some("laptop".into()),
            grant: Grant::none(),
        };
        assert_eq!(describe(None, Some(&client)), "client 'laptop' (id1)");
        assert_eq!(describe(Some("/p/a"), Some(&client)), "action /p/a");
        assert_eq!(describe(None, None), "the caller");
    }
}
