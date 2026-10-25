//! What a configured server needs: the certificates directory of an appdata
//! dir, the names its server certificate should carry, and creating or
//! renewing both certificates. solx-browser calls [`provision`] at startup
//! for `server_tls: "auto"`, and its Settings use the rest.

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

use solx_config::ConfigService;

use crate::{default_sans, normalize_sans, Authority, CreateOptions, Result, San};

/// `<appdata>/certs` for the appdata dir `config` belongs to: where
/// `server_tls: "auto"` finds its certificates.
pub fn certs_dir(config: &ConfigService) -> PathBuf {
    config.appdata().join("certs")
}

/// The names the server `config` describes should have on its certificate:
/// [`default_sans`], the bound address (or, bound on `0.0.0.0`/`::`, every
/// interface address that's up), and the host of `server_public_url`.
///
/// A `0.0.0.0` bind gets IPv4 addresses only, since that's all it accepts
/// (and IPv6 temporary addresses would reissue the certificate whenever
/// they rotate). Link-local addresses are left out: they need a zone to be
/// dialled and change with the interface.
pub fn wanted_sans(config: &ConfigService) -> Vec<San> {
    let snap = config.snapshot();
    let host = snap
        .server_host
        .as_deref()
        .and_then(|h| h.trim().parse::<IpAddr>().ok())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    let mut sans = default_sans();
    if host.is_unspecified() {
        let v4_only = host.is_ipv4();
        sans.extend(
            if_addrs::get_if_addrs()
                .unwrap_or_default()
                .into_iter()
                .filter(|i| i.is_oper_up() && !i.is_loopback() && !i.is_link_local())
                .map(|i| i.ip())
                .filter(|ip| !v4_only || ip.is_ipv4())
                .map(San::Ip),
        );
    } else {
        sans.push(San::Ip(host));
    }
    if let Some(public) = config.server_public_url() {
        if let Some(name) = url::Url::parse(&public)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .and_then(|h| h.parse::<San>().ok())
        {
            sans.push(name);
        }
    }
    normalize_sans(&sans)
}

/// The CA in [`certs_dir`], created with the platform defaults if there is
/// none, and a server certificate for [`wanted_sans`], reissued if it's
/// missing, stale or names something else. Returns the authority and
/// whether a server certificate was issued.
pub fn provision(config: &ConfigService) -> Result<(Authority, bool)> {
    let mut ca = Authority::open_or_create(certs_dir(config), &CreateOptions::default())?;
    let issued = ca.ensure_server(&wanted_sans(config))?;
    Ok((ca, issued))
}
