# TLS and certificate pinning: implementation plan

Status: **Phase 1 done** (`solx-certs`: file and CNG backends, CLI). Phases
2–4 are not started. It builds on
the token registry ([token-registry-plan.md](token-registry-plan.md)), which
is being implemented in the working tree. Phase 2 below has to be
coordinated with that plan's server-route step.

## Goal

Close known gap 9 in [security-design.md](security-design.md): off loopback,
`solx-server` speaks plain HTTP and the bearer token travels in cleartext.
Right now `main.rs` only prints a warning that suggests a reverse proxy. The
token registry makes this more pressing, because connect links and
`/auth/exchange` are meant to be used from other machines.

This plan adds:

- **`solx-certs`**, a new core crate that is both a library and a CLI. It
  creates a local root CA and signs a server certificate with it. On Windows
  both keys are created **non-exportable** in the OS key store, preferably on
  the TPM.
- **TLS serving in `solx-server`**, from either the Windows certificate store
  or PEM files. Loopback peers can keep using plain HTTP on the same port.
- **Automatic provisioning in solx-browser.** The desktop app creates and
  renews the certificates. A standalone `solx-server` user runs
  `solx-certs` themselves, or supplies their own (for example
  OpenSSL-made) certificates.
- **Pinning**, in a separate phase. A client learns the server's root during
  connect (from the link, the exchange, or a pin given with a token) and
  refuses any other root from then on.

## Decisions

| Question | Decision |
|---|---|
| What issues the server certificate? | A per-appdata local root CA. Clients trust the root, never the leaf, so leaf renewal is invisible to them. |
| Where does the root key live on Windows? | CNG, with export policy none. The Microsoft Platform Crypto Provider (TPM 2.0) is tried first, then the Microsoft Software KSP. |
| Where does the leaf key live on Windows? | Also CNG and non-exportable, served through `rustls-cng`. No private key is ever written to disk. It defaults to the Software KSP, not the TPM: it signs every handshake, and TPM signing measured about 100 times slower (see "TPM speed"). |
| Who provisions certificates automatically? | solx-browser only. Standalone `solx-server` takes a store thumbprint or PEM files, which `solx-certs` can produce. |
| Plain HTTP once TLS is on? | Only from loopback peers. A single port sniffs each connection's first byte. Command children, the CLI and the host-running probe keep working unchanged. |
| Linux and macOS? | Designed for now, built later: a `pkcs11` backend (TPM, YubiKey or HSM) and a macOS `keychain` backend. Until then, the portable `file` backend. |
| An OpenSSL mode? | No. See "Why not OpenSSL". OpenSSL-made PEMs work through the file mode. |
| TLS library | rustls 0.23 with the `ring` provider, which is what reqwest 0.12's `rustls-tls` already uses. Avoids aws-lc's cmake/NASM build on Windows. |
| What is pinned? | The root's SubjectPublicKeyInfo, as SHA-256 base64url. The client stores the root certificate itself, so the full chain can be verified. |

## Where things stand

| Piece | Where | Today |
|---|---|---|
| Bind address | `solx-server/src/lib.rs` `bind_addr` | `server_host`/`server_port`; loopback by default. |
| Serving | `solx-server/src/lib.rs` `serve`, `main.rs` | `axum::serve` over a `TcpListener`, plain HTTP. axum 0.7's `serve` can't take a TLS listener. |
| Off-loopback warning | `solx-server/src/main.rs` | Printed whenever the bind is not loopback. |
| URL handed to Command children | `solx-server/src/lib.rs` `server_url` | `http://<bound>`, with unspecified binds reached over loopback. |
| Advertised URL | `solx-registry/src/lib.rs` `advertised_url`, `server_public_url` | Used for connect links and the exchange's `server_url`. |
| Embedded server in the desktop app | `browser/src-tauri/src/server.rs` `start` | Calls `solx_server::serve`. Its `/health` probe is raw HTTP/1.0 to `127.0.0.1`. |
| Client HTTP | `solx-client` (`actions.rs`, `docs.rs`, `types.rs`, `files.rs`) | A separate `reqwest::Client::new()` per manager, trusting webpki roots only. |
| Client credentials | `solx-config/src/credentials.rs` | Per `(appdata, server URL)` credential-manager entries (`client:…`). The root token is offered only to loopback URLs. |
| Connect input | `credentials::parse_connect_input` (+ TS twin) | Code, link or token; the link's origin fills in the server URL. |
| Appdata-scoped naming | `credentials::appdata_key` | FNV-1a of the canonical appdata path. Reuse it for CA names. |

## Design

### Certificates

- **Root CA.**
  - ECDSA P-256, valid 10 years.
  - `BasicConstraints CA:true, pathlen:0`; `KeyUsage keyCertSign, cRLSign`.
  - Subject `CN=solx local CA (<computer name>) <appdata key>, O=solx`. The
    appdata key keeps isolated appdata dirs from sharing a CA, as with
    tokens.
- **Server (leaf).**
  - ECDSA P-256, valid 397 days.
  - `KeyUsage digitalSignature`; `ExtendedKeyUsage serverAuth`.
  - SANs:
    - `localhost`, `127.0.0.1`, `::1`
    - the computer name and `<name>.local`
    - the bound IP, or every interface address when bound to `0.0.0.0`/`::`
    - the host of `server_public_url`
- **Renewal.** At startup, the leaf is reissued when it has under 30 days
  left or its SAN set differs from the one wanted now (for example, a new
  LAN address). Signing is local, so this is cheap.
- **Pin.** `base64url(SHA-256(root SPKI))`. Shown to people as a 64-bit
  grouped prefix, `3F7A-91C2-0B4E-D518`, alongside connect codes.
- **No name constraints.** The leaf's names change with the network, so a
  constraint fixed at root creation would break renewals. A pinning client
  only ever trusts this root for this one server, which limits a misused root
  key more usefully than name constraints would.

### `core/solx-certs`

A library plus a `[[bin]] solx-certs`. No axum or server dependency.

#### Backends

Certificate building is shared (`Authority` in `lib.rs`), so every backend
issues identical certificates. A backend (`KeyBackend` in `backend.rs`) only
handles keys and where certificates are installed:

- `create_key(role)`: a new key, which may sit somewhere temporary
- `commit_key(key)`: make it permanent once its certificate is issued
- `open_key`, `key_exists`, `delete_key`
- `install_cert(der, key)` and `remove_cert(sha256)`: no-ops for files
- `describe_key`: where the key is, whether it's hardware, and whether it
  can be exported

Keys implement `rcgen::SigningKey`, so `Authority` signs through them
without knowing where they are. `Authority` is the public API: `create`,
`open`, `issue_server`, `ensure_server` (what solx-browser will call at
startup), `delete`, and `trust_on_this_machine`.

| Backend | Platforms | Key protection | Status |
|---|---|---|---|
| `cng` | Windows (feature `cng`, default there) | Non-exportable; TPM where available | Phase 1 |
| `file` | All | PKCS#8 PEM. The CA key is wrapped with AES-256-GCM under a key in the OS credential manager. | Phase 1 |
| `pkcs11` | Linux mainly (`cryptoki`) | Non-exportable on a TPM (`tpm2-pkcs11`), YubiKey or HSM | Later |
| `keychain` | macOS (`security-framework`) | Secure Enclave P-256 | Later |

#### The CNG backend

- **Creating keys.** Neither `rustls-cng` nor `schannel` creates keys, so
  this uses `windows-sys` (`Win32_Security_Cryptography`):
  1. `NCryptOpenStorageProvider(MS_PLATFORM_CRYPTO_PROVIDER)`, and on
     failure `MS_KEY_STORAGE_PROVIDER`.
  2. `NCryptCreatePersistedKey(BCRYPT_ECDSA_P256_ALGORITHM, <container>)`.
     The container is named `solx-ca-<dir key>-<unique>` or
     `solx-server-<dir key>-<unique>`, where the dir key is the FNV hash of
     the certificates directory.
  3. Set `NCRYPT_EXPORT_POLICY_PROPERTY` to 0 and
     `NCRYPT_KEY_USAGE_PROPERTY` to `NCRYPT_ALLOW_SIGNING_FLAG`. The TPM
     provider may refuse these properties; that is tolerated, since its keys
     can't leave the TPM anyway.
  4. `NCryptFinalizeKey`.
  5. `NCryptExportKey(BCRYPT_ECCPUBLIC_BLOB)`, which exports the public key
     only.

  The provider used is recorded in `tls.json`, and shown in `show` and
  Settings.
- **Signing certificates.** rcgen 0.14 builds both certificates. `CngKey`
  implements `rcgen::SigningKey` and `PublicKeyData`:
  1. SHA-256 the message.
  2. Call `NCryptSignHash` directly through `windows-sys`. `rustls-cng` isn't
     needed until phase 2 serves TLS.
  3. DER-encode CNG's raw `r‖s` as `Ecdsa-Sig-Value`.

  `rcgen::Issuer::from_ca_cert_der(root, root_key)` then signs the leaf.
- **Storing certificates.** With `schannel`: `CertContext::new(der)`, then
  `set_key_prov_info()` to bind the certificate to its container and
  provider, then `CertStore::open_current_user("My").add_cert`. No admin
  rights are needed.
- **Trusting the root on this machine** is opt-in (`solx-certs trust`, or a
  Settings button). It adds the root to `CurrentUser\Root`, and Windows
  shows its own confirmation dialog. The host doesn't need to trust its own
  root for serving, so this is never done silently. The call blocks until
  the person answers, so it must never run from a test or an unattended
  script.
- **What "non-exportable" means.** On the TPM, the key can't leave the chip.
  Code running as the user can still *use* it to sign while it runs, but it
  can't copy the key elsewhere. On the Software KSP, the protection is an
  enforced policy, but malware running as the user, or an admin, can defeat
  it. SECURITY.md should say which provider is in use and what each one
  guarantees.
- **TPM speed.** Measured on an Intel PTT TPM 2.0 (2026-10-16, the ignored
  `signing_speed` test):

  | Provider | Create key | One P-256 signature |
  |---|---|---|
  | Microsoft Platform Crypto Provider (TPM) | 282 ms | 24 ms |
  | Microsoft Software Key Storage Provider | 39 ms | 0.24 ms |

  The root signs only when a certificate is issued, so it goes on the TPM.
  Server keys default to the Software KSP, still non-exportable, because
  they sign every TLS handshake. `--server-provider tpm` is available for
  anyone who prefers hardware for both.

#### The file backend

- Keys are written as standard PKCS#8 PEM and certificates as X.509 PEM, so
  OpenSSL-made files interoperate both ways.
- The CA key file is AES-256-GCM-encrypted. The wrapping key is kept in the
  OS credential manager through the existing `keyring` dependency, using the
  `secrets.rs` pattern. On Linux, that means Secret Service. With no
  credential manager, the key is plaintext with mode 0600 and a warning is
  logged.
- The leaf key is plaintext PEM (mode 0600), since rustls has to load it.
- This backend is honest about its limits: any process running as the user
  can read the keys.

#### Why not OpenSSL

- An OpenSSL key is non-exportable only through a provider such as
  `tpm2-openssl`, which is the TPM again. PKCS#11 reaches the same TPMs and
  HSMs, plus YubiKeys, across platforms.
- It would add a system C library to a rustls stack, and the server would
  still need a custom rustls signing adapter.
- What people usually want from it is compatibility, and PEM in the file
  mode gives that. A recipe for standalone users:

  ```sh
  openssl ecparam -name prime256v1 -genkey -noout -out root.key
  openssl req -x509 -new -key root.key -sha256 -days 3650 -subj "/CN=my solx CA" \
    -addext "basicConstraints=critical,CA:true,pathlen:0" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" -out root.crt
  openssl ecparam -name prime256v1 -genkey -noout -out server.key
  openssl req -new -key server.key -subj "/CN=solx" -out server.csr
  openssl x509 -req -in server.csr -CA root.crt -CAkey root.key -days 397 -sha256 \
    -extfile <(printf "subjectAltName=DNS:myhost,IP:192.168.1.10\nextendedKeyUsage=serverAuth") \
    -out server.crt
  cat root.crt >> server.crt
  ```

#### Output

These are written to `<appdata>/certs/`:

- `solx-root.crt`: the root, in PEM, to import into clients
- `solx-server.crt`: the leaf followed by the root
- `tls.json`: backend, provider, store thumbprints, pin, and expiry dates
- the file backend also writes `solx-root.key` (wrapped) and
  `solx-server.key`

#### CLI

Every command takes `--dir DIR` (default `<appdata>/certs`).

- `solx-certs init [--backend cng|file] [--provider auto|tpm|software]
  [--server-provider auto|tpm|software] [--no-wrap] [--san NAME]… [--days N]`
- `solx-certs issue [--san NAME]… [--no-default-sans] [--days N]`. The
  defaults are `localhost`, `127.0.0.1`, `::1` and the computer name.
- `solx-certs show [--json]`: backend, provider, fingerprints, pin and expiry dates
- `solx-certs export-root [PATH]`
- `solx-certs trust` / `untrust`. On Linux these print the
  `update-ca-certificates` / `trust anchor` command, since that needs root.
- `solx-certs delete --yes`: removes the keys and certificates. Every
  client then has to re-pin.

#### Dependencies

Added to `[workspace.dependencies]`:

- `rcgen = "0.14"`
- `rustls = { version = "0.23", default-features = false, features = ["ring", "std", "tls12"] }`
- `rustls-pki-types`
- `tokio-rustls`
- `hyper-util`
- `sha2`
- Windows-only: `rustls-cng = "0.7"`, `schannel = "0.1"`, `windows-sys`

### Serving TLS (`solx-server`)

#### Configuration

- Feature `tls`, default on. As with `registry`, `browser/src-tauri` depends
  on it with `default-features = false`, so its `server` feature must
  forward `solx-server/tls` explicitly.
- The config key `server_tls` takes one of:
  - `"off"`
  - `"auto"`: provisioned by solx-browser from `<appdata>/certs/tls.json`
  - `{ "cert_file": "...", "key_file": "..." }`
  - `{ "store_thumbprint": "<sha256 hex>" }`, Windows only
- It goes in `RESTART_KEYS`, and validation goes in `solx-config/src/lib.rs`.
- The default is `"off"`. Behaviour doesn't change for anyone until they opt
  in, or the desktop app does.

#### `solx-server/src/tls.rs`

- Turns a `TlsSource` into an `Arc<rustls::ServerConfig>`, with ALPN `h2`
  and `http/1.1`.
- **Files:** PEM via `rustls-pki-types`.
- **Store:** `rustls_cng::store::CertStore::open(CurrentUser, "My")`, then
  `find_by_sha256`. The certificate's key becomes a `CngSigningKey` inside a
  `ResolvesServerCert` that also returns the chain up to the root.

#### The accept loop

- One shared `serve_listener(listener, router, tls, shutdown)` replaces the
  bodies of `serve` and `main.rs`'s serving code. It is built on
  `hyper-util` (`server::conn::auto::Builder`, `GracefulShutdown`). For each
  accepted connection:
  1. `peek` the first byte.
  2. `0x16` (a TLS handshake record): run `tokio_rustls::TlsAcceptor`, then
     serve.
  3. Anything else from a loopback peer: serve plain HTTP.
  4. Anything else from any other peer: reply `400` with a body saying
     "this server requires https://", then close.
  5. Either way, insert `ConnectInfo<SocketAddr>` for the connection. The
     registry's loopback-only root check depends on it.
- With TLS off, the loop behaves exactly like `axum::serve` does today.
- **Reverse-proxy caveat.** Every request through a proxy on the same
  machine looks like loopback. The registry plan already raises this.
  Treating peers as non-loopback when `server_public_url` is set covers this
  case too.

#### URLs and warnings

- `server_url(bound)`, the URL handed to Command children as
  `SOLX_SERVER_URL`, stays `http://127.0.0.1:…`. Sniffing keeps it working,
  so packages need no CA and no changes.
- The registry's `advertised_url` uses `https://` when TLS is on.
  `server_public_url` is taken as written.
- The off-loopback warning is printed only when TLS is off. Its wording
  should point to `solx-certs`.
- `spawn_embedded` (loopback only) and the desktop's raw `/health` probe are
  unchanged.

### solx-browser

- `start()` in `browser/src-tauri/src/server.rs`, when `server_tls` is
  `"auto"`:
  1. `solx_certs::ensure_server_cert(&config, wanted_sans)` creates the root
     if it's missing and reissues the leaf if it's stale.
  2. It returns `TlsSource::Store`, which is passed to `solx_server::serve`.
- `"auto"` is the effective default when `server_host` is not loopback on
  Windows. Loopback defaults stay plain.
- **Settings** gets a "Secure connections" section. Its commands go in the
  Settings-only `config` permission set in `solx-tauri-plugin`:
  - status, backend and provider, root fingerprint, leaf expiry
  - "Open certificates folder" and "Trust on this machine"
  - "Reissue server certificate"
  - "Reset CA", with a warning that every client has to reconnect
- The "Connect another device" panel from the registry plan also shows the
  fingerprint next to the code (phase 4).

### Clients before pinning

- **One shared HTTP client.** The managers' separate `reqwest::Client::new()`
  calls become one `solx_client::http_client(&ClientTls)`, shared by every
  `Remote*` manager built in `App::remote`.
- `ClientTls` holds:
  - webpki roots, plus OS roots (`rustls-native-certs`), so a root the user
    imported is honoured
  - extra roots from the config key `server_ca_file`, or `SOLX_CA_FILE`, for
    manual setups
  - later, a pinned root (phase 4), which replaces all of the above for
    that server
- **Node bindings** (`js/crates/solx-bindings`) inherit this through
  `solx-client`.
- **Real browsers.** The web UI in a real browser, and other HTTPS clients
  such as VS Code's MCP client, can't be pinned by solx. They need the root
  in the OS or browser trust store. Document this:
  - Windows: `certutil -user -addstore Root solx-root.crt`
  - macOS: Keychain Access
  - Linux: `update-ca-certificates`
  - Firefox: its own certificate store

### Pinning

The goal is that a client learns the root once, from a source it can trust,
and from then on accepts only chains that end in it.

#### Where the pin comes from

- **Connect link:** `<public url>/connect#code=12345678&pin=<b64url>`.
  - `create_connect_code` adds `pin` to `url`. `ConnectCode` and
    `create-connect-code` gain `tls_pin`.
  - `parse_connect_input` and the TypeScript twin extract `pin`.
  - The pin is checked **before** the code is sent. This is the strong path,
    because the pin arrives out of band with the code.
- **Typed code** (address plus `1234-5678`, no pin): trust on first use.
  - The client shows the root fingerprint, and the person compares it with
    the host's "Connect another device" panel.
  - An 8-digit code can't authenticate the channel. A man in the middle that
    sees the code can brute-force any proof derived from it offline, and
    fixing that would take a PAKE. Prefer the link; the doc says so.
- **Token given directly** (`issue-client`, `solx token issue`, a pasted
  token, `SOLX_SERVER_TOKEN`). There is no exchange, so the client resolves
  trust in this order:
  1. **A pin given with the token.**
     - `issue-client` returns `tls_pin` next to `token`, and
       `solx token issue` prints both.
     - The server-address field accepts `https://host:8766#pin=<b64url>`.
     - The CLI takes `solx connect URL TOKEN --pin <b64url>`.
     - Non-interactive clients (Node bindings, MCP configs) read
       `SOLX_SERVER_PIN`.
  2. **`server_ca_file` / `SOLX_CA_FILE`.** An explicitly configured root
     counts as the pin and is stored as one.
  3. **The OS trust store.** Accepted, but not stored as a pin.
  4. **Interactive trust on first use**, in the CLI and desktop client only.
  5. **Otherwise, fail closed** with an error that includes the server's
     fingerprint and suggests `--pin`.

#### Never send a secret before the pin is settled

- First contact is an unauthenticated `GET /health` through a recording
  verifier. It is a custom `rustls::client::danger::ServerCertVerifier`,
  passed to reqwest with `use_preconfigured_tls`.
- That verifier checks that the chain is internally valid for the host name
  and records the root.
- Only once the root matches a given pin, or the person confirms it, does
  the client send the code to `/auth/exchange` or the token in
  `Authorization`.
- This applies to codes and tokens alike. Otherwise a man in the middle
  would collect the credential during the very handshake being judged.

#### The exchange

- `/auth/exchange` returns `tls_root_cert` (PEM) as well.
- The client checks that its SPKI hash equals the root it saw in the
  handshake, then stores it. This catches a confused deployment, such as a
  proxy terminating TLS with a different CA. It is not a defence against an
  active attacker; the pin source above is.

#### Storage and enforcement

- **Storage.**
  - A credential-manager entry `pin:<appdata>:<server url>`, holding the
    root DER as base64. It sits next to `client:…` in `credentials.rs`.
  - Helpers `server_pin`, `set_server_pin`, `forget_server_pin`.
    `forget_client_token` also forgets the pin.
  - The value isn't secret, but keeping it next to the token gives both the
    same lifetime and the same protection.
- **Enforcement.** With a stored pin, `http_client` builds reqwest with
  `tls_built_in_root_certs(false)` and
  `add_root_certificate(<pinned root>)`. Host names and validity are still
  checked, and no custom verifier is needed after first contact.
- **Mismatch.** A distinct error: "the server's certificate authority
  changed; reconnect with a new connect link (or `--pin`)". It never falls
  back to other roots.
- **Wiring:**
  - the CLI's `solx connect`
  - the desktop client build's `connect(server_url, credential)`
  - the web `/connect` page, which can only display the fingerprint, since
    the browser enforces trust itself

### Security notes for SECURITY.md and security-design.md

- §3.2 gets a TLS paragraph: sources, the sniffing rule, and loopback plain
  HTTP.
- §9 gap 9 becomes: "Off-loopback TLS is opt-in (automatic in the desktop
  app); plain HTTP is accepted only from loopback peers."
- A new gap: "Clients that can't pin (browsers, third-party MCP clients)
  rely on the OS trust store, so the solx root is trusted for every host
  name on those machines."
- What each key provider guarantees (see "What non-exportable means").

## Implementation steps

1. **`solx-certs`.** Done.
   - Crate skeleton, `KeyBackend`, and the file backend.
   - The CNG backend: key creation (TPM, then software), the rcgen signing
     adapter, and store binding.
   - The CLI.
   - Measure TPM signing.
2. **Server TLS.** Coordinate with the registry plan's step 4, which adds
   `ConnectInfo` and `/connect`.
   - `server_tls` config and validation.
   - `tls.rs`.
   - The sniffing `serve_listener`, used by `serve` and `main.rs`.
   - `https` advertised URLs and the conditional warning.
3. **Desktop app, clients and docs.**
   - `ensure_server_cert` in solx-browser and the Settings section.
   - The shared `solx_client::http_client` with OS roots and
     `server_ca_file`.
   - SECURITY.md, security-design.md, http-api.md, README.
4. **Pinning.**
   - `pin=` in links and server URLs; `tls_pin` from `create-connect-code`
     and `issue-client`.
   - The recording first-contact verifier and the fingerprint prompt.
   - `tls_root_cert` from the exchange.
   - Credential-store pins and their enforcement.
   - The CLI, desktop and web flows.
5. **Later:** the `pkcs11` and `keychain` backends, and `"auto"`
   provisioning on Linux and macOS.

## Tests

Run `cargo test --workspace`, plus `cargo check -p solx-server
--no-default-features` and `cargo check -p solx-certs --no-default-features`
(the file backend only, so it builds anywhere). Tests that touch
credentials call `credentials::use_memory_store_for_tests()`.

- **solx-certs, file backend:**
  - init → issue → the leaf verifies against the root (webpki) with the
    expected SANs and EKU
  - reissue on SAN change and near expiry
  - the CA key on disk is wrapped
- **solx-certs, CNG** (Windows only, in throwaway containers that are
  deleted afterwards):
  - export policy reads back as 0
  - exporting the private blob fails
  - a leaf signed through the adapter verifies against the root
  - the TPM-to-software fallback is recorded
- **Server** (`solx-server/tests/tls.rs`, with file-backend certificates in
  a tempdir):
  - HTTPS with the root trusted works, over both h2 and http/1.1
  - plain HTTP from `127.0.0.1` works
  - the sniff decision for a non-loopback plain peer is a 400, as a unit test
    of the predicate
  - `ConnectInfo` reaches `require_bearer`
  - graceful shutdown drains requests
- **Pinning:**
  - a link with the right pin connects and stores the pin
  - a wrong pin is refused before the code is sent: `failed_exchanges` is
    unchanged and the code is still pending
  - a token with `--pin`, `#pin=` or `SOLX_SERVER_PIN` connects
  - a wrong pin is refused before `Authorization` is sent, checked against
    a server that records request headers
  - with no pin and no trusted root, a non-interactive client fails, and the
    error carries the fingerprint
  - a CA swap after pinning gives the mismatch error
  - the classifier extracts `pin` (Rust and TS share the cases)
- **Manual (Windows):**
  - solx-browser with `server_host: 0.0.0.0`: `certmgr.msc` shows both
    certificates under Personal with keys, `certutil -user -store My` reports
    the keys as non-exportable, and the provider is the TPM where one exists
  - a second machine connects through a link (`solx connect`), and through a
    browser after importing `solx-root.crt`
  - reissuing the leaf leaves clients working; resetting the CA gives the
    mismatch error
