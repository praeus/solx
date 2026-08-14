//! `solx-bindings` — the single Neon binary exposing seven sub-modules
//! to Node.js under one `.node` file.
//!
//! ```js
//! const solx = require("solx.node");
//! const mgr  = solx.typesOpen("/path/to/types.db");   // sync
//! const t    = await solx.typesSave(mgr, "/types/custom", "Person", JSON.stringify(input));
//! ```
//!
//! Each sub-module is registered by its own `register_*_module` function
//! which exports camelCase function names with the sub-module name as
//! a prefix (e.g. `typesOpen`, `configOpen`, `docsSave`). The
//! TS-side `Local*` classes are thin wrappers around these primitives
//! — they exist to give users proper async, error types, and DTO
//! conversion, none of which are ergonomic to do in raw `solx.node`.

pub mod async_runtime;
pub mod types;
pub mod config;
pub mod files;
pub mod docs;
pub mod actions;
pub mod scripts;
pub mod surface;

use neon::prelude::*;

#[neon::main]
fn main(mut cx: ModuleContext) -> NeonResult<()> {
    surface::register_surface_module(&mut cx)?;
    types::register_types_module(&mut cx)?;
    config::register_config_module(&mut cx)?;
    files::register_files_module(&mut cx)?;
    docs::register_docs_module(&mut cx)?;
    actions::register_actions_module(&mut cx)?;
    scripts::register_scripts_module(&mut cx)?;
    Ok(())
}
