//! Format registry -- read/write/check a [`crate::document::Doc`] by format
//! *name* at runtime, plus register your own format plugins. Ported from
//! `~/dev/omnist/omnist/registry.py` (issue #31; see also the TypeScript
//! port's `registry.ts` for the same architecture-freedom call made there).
//!
//! Python's registry is a plain `dict[str, Format]` of arbitrary callables --
//! genuine runtime plugin registration, exercised by
//! `tests/test_canonical.py::TestRegistry`: a caller can `register_format`
//! an arbitrary `(name, read, write, check?)` tuple at runtime and every
//! `Doc`-level API that takes a format name (`from_format`/`to_format`/
//! `check_format`) transparently picks it up. A closed `enum` dispatch (the
//! `omnist-cli` `Fmt` enum's approach, or a `match` over the five builtins)
//! can't express "register a new format at runtime under an arbitrary
//! name," so this module reaches for the same dynamic-dispatch idiom Rust
//! uses in place of Python's first-class functions: `Arc<dyn Fn(...) + Send
//! + Sync>` trait objects, keyed by name in an `IndexMap` behind an
//! `RwLock` inside a `OnceLock` (this crate's only piece of global mutable
//! state). `Arc` (not `Box`) so [`get_format`] can hand back an owned,
//! independently usable [`Format`] without holding the registry lock across
//! the caller's use of it -- mirroring Python's `_LOCK`-guarded dict lookup,
//! which also releases the lock before the caller touches the returned
//! `Format`.
//!
//! ## Uniform signatures across five differently-shaped codecs
//!
//! [`crate::formats::json::write_json`] takes an extra `indent: Option<
//! usize>` the other three format writers don't, and [`crate::oml`]'s
//! `read_oml`/`write_oml` operate on [`crate::document::RawNode`] rather
//! than `Doc` directly (see `oml.rs`'s own module doc on why). The
//! [`ReadFn`]/[`WriteFn`] the registry stores are `Doc`-in/`Doc`-out with no
//! format-specific options, matching Python's registry entries as actually
//! invoked from this port's zero-arg call sites (Python's `Doc.to_format`
//! forwards `**o` through, but nothing in the Python test suite or
//! `docs/api.md` exercises that with the builtins, so this port keeps the
//! simpler no-options signature and documents the gap here rather than
//! silently reproducing untested surface). The five builtins are registered
//! as thin wrapper closures around the existing per-format functions with
//! their default options (`indent: None`, `strict: false`, no report
//! requested for writers; `Doc::from_raw`/`to_raw` bridging OML's `RawNode`
//! shape) -- `get_format("json").read`/`.write` are *not* literally
//! `read_json`/`write_json` (Rust can't express "the same fn item" through
//! an `Arc<dyn Fn>` the way Python's `is` can point at the same function
//! object), but they call straight through with no other logic, matching
//! Python's actual invariant in spirit: no behavior is added or changed at
//! the registry boundary.
//!
//! ## OML's `check_oml`
//!
//! Rust's port had no `check_oml` before this issue -- OML is lossless for
//! every `Doc` (see `oml.rs`'s module doc: "no adjustment ever needed"), so
//! nothing needed to call it. Python's `check_oml` exists purely to satisfy
//! the registry `Format` tuple's fourth slot and always returns an empty
//! `WriteReport`; this issue adds the same trivial function to `oml.rs` for
//! the same reason (used only via the `"oml"` registry entry's `check`).

use std::sync::{Arc, OnceLock, RwLock};

use indexmap::IndexMap;

use crate::document::Doc;
use crate::error::{FormatError, OmnistError};
use crate::report::WriteReport;

/// `text -> Doc` reader callable.
pub type ReadFn = dyn Fn(&str) -> Result<Doc, OmnistError> + Send + Sync;
/// `Doc -> text` writer callable.
pub type WriteFn = dyn Fn(&Doc) -> Result<String, OmnistError> + Send + Sync;
/// `Doc -> WriteReport` check callable; simulates a write without producing
/// output.
pub type CheckFn = dyn Fn(&Doc) -> WriteReport + Send + Sync;

/// A registered format: a name plus `read`/`write` callables and an
/// optional `check`. Mirrors Python's `Format` `NamedTuple` (`name, read,
/// write, check`); a plugin registered with [`Format::new`] alone has no
/// `check`, and [`crate::document::Doc::check_format`] errors cleanly (not
/// a panic) if invoked on it -- matching
/// `test_plugin_without_check_raises_on_check_format`.
#[derive(Clone)]
pub struct Format {
    pub name: String,
    pub read: Arc<ReadFn>,
    pub write: Arc<WriteFn>,
    pub check: Option<Arc<CheckFn>>,
}

impl Format {
    /// Build a `Format` with no `check` callable. Use [`Format::with_check`]
    /// to attach one.
    pub fn new(
        name: impl Into<String>,
        read: impl Fn(&str) -> Result<Doc, OmnistError> + Send + Sync + 'static,
        write: impl Fn(&Doc) -> Result<String, OmnistError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            read: Arc::new(read),
            write: Arc::new(write),
            check: None,
        }
    }

    /// Attach a `check` callable, returning `self` for chaining.
    pub fn with_check(
        mut self,
        check: impl Fn(&Doc) -> WriteReport + Send + Sync + 'static,
    ) -> Self {
        self.check = Some(Arc::new(check));
        self
    }
}

fn registry() -> &'static RwLock<IndexMap<String, Format>> {
    static REGISTRY: OnceLock<RwLock<IndexMap<String, Format>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(builtins()))
}

fn builtins() -> IndexMap<String, Format> {
    use crate::document::RawNode;
    use crate::formats::{json, toml, xml, yaml};
    use crate::oml;

    let mut m = IndexMap::new();
    let mut add = |fmt: Format| {
        m.insert(fmt.name.clone(), fmt);
    };

    add(Format::new("json", json::read_json, |doc| {
        json::write_json(doc, None, false, None).map_err(Into::into)
    })
    .with_check(json::check_json));
    add(Format::new("yaml", yaml::read_yaml, |doc| {
        yaml::write_yaml(doc, false, None).map_err(Into::into)
    })
    .with_check(yaml::check_yaml));
    add(Format::new("toml", toml::read_toml, |doc| {
        toml::write_toml(doc, false, None).map_err(Into::into)
    })
    .with_check(toml::check_toml));
    add(Format::new("xml", xml::read_xml, |doc| {
        xml::write_xml(doc, false, None).map_err(Into::into)
    })
    .with_check(xml::check_xml));
    add(Format::new(
        "oml",
        |text| {
            let raw: RawNode = oml::read_oml(text)?;
            Doc::from_raw(raw).map_err(Into::into)
        },
        |doc| oml::write_oml(&doc.to_raw(), 2).map_err(Into::into),
    )
    .with_check(oml::check_oml));

    m
}

/// Register (or replace) a format plugin, usable everywhere a format name is
/// accepted, including [`crate::document::Doc::from_format`]/`to_format`/
/// `check_format`.
pub fn register_format(fmt: Format) {
    registry().write().unwrap().insert(fmt.name.clone(), fmt);
}

/// The registered [`Format`] for `name`. An [`OmnistError::Format`] if
/// unknown, naming every currently-registered format name, sorted --
/// mirrors Python's `get_format`'s `f"unknown format {name!r}; registered:
/// {known}"` message. Unlike Python, there is no `"(none)"` fallback for an
/// empty registry: [`register_format`] only ever adds entries and the five
/// builtins always register on first access (see [`builtins`]), so the
/// registry can never actually be empty here -- an untestable dead branch
/// for that case was deliberately not carried over (playbook's "unreachable
/// dead code" gap classification), rather than kept under an unreachable
/// coverage-ignore.
pub fn get_format(name: &str) -> Result<Format, OmnistError> {
    let reg = registry().read().unwrap();
    reg.get(name).cloned().ok_or_else(|| {
        let mut known: Vec<&str> = reg.keys().map(String::as_str).collect();
        known.sort_unstable();
        FormatError::new(format!(
            "unknown format {name:?}; registered: {}",
            known.join(", ")
        ))
        .into()
    })
}

/// The names of all registered formats, sorted.
pub fn formats() -> Vec<String> {
    let reg = registry().read().unwrap();
    let mut names: Vec<String> = reg.keys().cloned().collect();
    names.sort_unstable();
    names
}

#[cfg(test)]
mod tests;
