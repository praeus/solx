//! `solx-certs`: create and manage the local CA that `solx-server` serves
//! TLS from. The desktop app does all of this itself; this is for a
//! standalone `solx-server`, for scripting, and for inspecting what the app
//! made.

use std::path::PathBuf;

use anyhow::{bail, Context};
use clap::{Parser, Subcommand, ValueEnum};
use serde_json::json;

use solx_certs::{
    default_sans, Authority, BackendKind, CreateOptions, FileProtection, KeyReport, Provider, San,
    SERVER_VALIDITY_DAYS,
};

#[derive(Parser)]
#[command(
    name = "solx-certs",
    about = "Local certificate authority for solx-server TLS",
    version
)]
struct Cli {
    /// Certificates directory. Defaults to `<appdata>/certs`, the one the
    /// desktop app uses.
    #[arg(long, global = true)]
    dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a root CA and a server certificate signed by it.
    Init {
        /// Where keys live. `cng` (Windows only, the default there) keeps
        /// them non-exportable in the OS key store; `file` writes PEM files.
        #[arg(long, value_enum)]
        backend: Option<BackendArg>,
        /// CNG: key storage provider for the root key.
        #[arg(long, value_enum, default_value_t = ProviderArg::Auto)]
        provider: ProviderArg,
        /// CNG: key storage provider for server keys. Software by default:
        /// the server key signs every TLS handshake, and TPM signing is
        /// about 100 times slower.
        #[arg(long, value_enum, default_value_t = ProviderArg::Software)]
        server_provider: ProviderArg,
        /// File backend: write the root key unencrypted instead of wrapping
        /// it under a key in the OS credential manager.
        #[arg(long)]
        no_wrap: bool,
        #[command(flatten)]
        names: Names,
    },
    /// Issue a new server certificate, replacing the current one.
    Issue {
        #[command(flatten)]
        names: Names,
    },
    /// Show the CA, the server certificate, and where their keys live.
    Show {
        #[arg(long)]
        json: bool,
    },
    /// Write the root certificate (PEM) to PATH, or print it.
    ExportRoot { path: Option<PathBuf> },
    /// Trust the root on this machine (Windows: CurrentUser\Root, after a
    /// confirmation dialog). Elsewhere, prints the command to run.
    Trust,
    /// Remove the root from this machine's trust store.
    Untrust,
    /// Delete the CA: keys, certificates, and files. Every client has to
    /// trust or pin the new root afterwards.
    Delete {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(clap::Args)]
struct Names {
    /// Extra DNS name or IP address for the server certificate. Repeatable.
    #[arg(long = "san", value_name = "NAME_OR_IP")]
    sans: Vec<String>,
    /// Leave out the defaults: localhost, 127.0.0.1, ::1 and this
    /// computer's name.
    #[arg(long)]
    no_default_sans: bool,
    /// Server certificate validity in days.
    #[arg(long, default_value_t = SERVER_VALIDITY_DAYS)]
    days: i64,
}

impl Names {
    fn resolve(&self) -> anyhow::Result<Vec<San>> {
        let mut sans = if self.no_default_sans {
            Vec::new()
        } else {
            default_sans()
        };
        for name in &self.sans {
            sans.push(name.parse()?);
        }
        if sans.is_empty() {
            bail!("no names: pass --san, or drop --no-default-sans");
        }
        Ok(sans)
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum BackendArg {
    Cng,
    File,
}

#[derive(Clone, Copy, ValueEnum)]
enum ProviderArg {
    Auto,
    Tpm,
    Software,
}

impl From<ProviderArg> for Provider {
    fn from(p: ProviderArg) -> Self {
        match p {
            ProviderArg::Auto => Provider::Auto,
            ProviderArg::Tpm => Provider::Tpm,
            ProviderArg::Software => Provider::Software,
        }
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let dir = cli.dir.clone().unwrap_or_else(solx_certs::default_dir);
    match cli.command {
        Command::Init {
            backend,
            provider,
            server_provider,
            no_wrap,
            names,
        } => {
            let sans = names.resolve()?;
            let options = CreateOptions {
                backend: match backend {
                    Some(BackendArg::Cng) => BackendKind::Cng,
                    Some(BackendArg::File) => BackendKind::File,
                    None => solx_certs::default_backend(),
                },
                root_provider: provider.into(),
                server_provider: server_provider.into(),
                file_protection: if no_wrap {
                    FileProtection::Plain
                } else {
                    FileProtection::Keyring
                },
            };
            let mut ca = Authority::create(&dir, &options)?;
            ca.issue_server(&sans, names.days)?;
            println!("Created a certificate authority in {}.", dir.display());
            println!();
            show(&ca);
            println!();
            println!(
                "Clients import {} (or pin {}).",
                ca.root_cert_path().display(),
                ca.pin()
            );
        }
        Command::Issue { names } => {
            let mut ca = open(&dir)?;
            let sans = names.resolve()?;
            ca.issue_server(&sans, names.days)?;
            println!("Issued a new server certificate.");
            println!();
            show(&ca);
        }
        Command::Show { json } => {
            let ca = open(&dir)?;
            if json {
                let (root_key, server_key) = ca.key_reports();
                let out = json!({
                    "dir": ca.dir(),
                    "state": ca.state(),
                    "fingerprint": ca.fingerprint(),
                    "root_key": root_key,
                    "server_key": server_key,
                    "trusted_on_this_machine": ca.trusted_on_this_machine(),
                    "root_cert_file": ca.root_cert_path(),
                    "server_cert_file": ca.server_cert_path(),
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                show(&ca);
            }
        }
        Command::ExportRoot { path } => {
            let ca = open(&dir)?;
            let pem = std::fs::read_to_string(ca.root_cert_path())
                .with_context(|| format!("read {}", ca.root_cert_path().display()))?;
            match path {
                Some(path) => {
                    std::fs::write(&path, pem)
                        .with_context(|| format!("write {}", path.display()))?;
                    println!("Wrote {}.", path.display());
                }
                None => print!("{pem}"),
            }
        }
        Command::Trust => {
            let ca = open(&dir)?;
            ca.trust_on_this_machine()?;
            println!(
                "This machine now trusts the solx root ({}).",
                ca.fingerprint()
            );
        }
        Command::Untrust => {
            let ca = open(&dir)?;
            ca.untrust_on_this_machine()?;
            println!("This machine no longer trusts the solx root.");
        }
        Command::Delete { yes } => {
            let ca = open(&dir)?;
            if !yes {
                bail!(
                    "this deletes the CA in {} for good, and every client will have to trust \
                     the new root. Run again with --yes",
                    dir.display()
                );
            }
            if ca.trusted_on_this_machine() == Some(true) {
                eprintln!("Removing the root from this machine's trust store first.");
            }
            ca.delete()?;
            println!("Deleted the certificate authority in {}.", dir.display());
        }
    }
    Ok(())
}

fn open(dir: &std::path::Path) -> anyhow::Result<Authority> {
    Authority::open(dir)?.with_context(|| {
        format!(
            "no certificate authority in {} (run `solx-certs init`)",
            dir.display()
        )
    })
}

fn show(ca: &Authority) {
    let state = ca.state();
    let (root_key, server_key) = ca.key_reports();
    println!("directory    {}", ca.dir().display());
    println!("backend      {}", state.backend.as_str());
    println!("root         {}", state.root.subject);
    println!("  sha256     {}", state.root.sha256);
    println!("  expires    {}", state.root.not_after.format("%Y-%m-%d"));
    println!("  key        {}", describe(&root_key));
    match ca.trusted_on_this_machine() {
        Some(true) => println!("  trusted    yes, on this machine"),
        Some(false) => println!("  trusted    not on this machine (`solx-certs trust`)"),
        None => {}
    }
    println!("pin          {}", ca.pin());
    println!("fingerprint  {}", ca.fingerprint());
    match (&state.server, server_key) {
        (Some(server), Some(key)) => {
            println!("server       {}", server.subject);
            println!("  names      {}", server.sans.join(", "));
            println!("  sha256     {}", server.sha256);
            println!("  expires    {}", server.not_after.format("%Y-%m-%d"));
            println!("  key        {}", describe(&key));
        }
        _ => println!("server       none (`solx-certs issue`)"),
    }
    println!("files        {}", ca.root_cert_path().display());
    println!("             {}", ca.server_cert_path().display());
}

fn describe(key: &KeyReport) -> String {
    if !key.present {
        return format!("{} -- MISSING", key.location);
    }
    let mut notes = Vec::new();
    if key.hardware {
        notes.push("TPM");
    }
    match key.exportable {
        Some(false) => notes.push("non-exportable"),
        Some(true) => notes.push("exportable"),
        None => {}
    }
    if notes.is_empty() {
        key.location.clone()
    } else {
        format!("{} [{}]", key.location, notes.join(", "))
    }
}
