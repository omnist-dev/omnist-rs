# Python divergences

A single index of every disclosed, live-checked behavioral divergence
from the Python reference (`~/dev/omnist`) found across this port's
merged PR history. Each entry names what Python does, what this port
does, the reasoning from the PR that introduced it, and whether it's a
permanent representational choice or something revisitable later.

This page is an index, not a replacement for
[`docs/limitations.md`](limitations.md) or the per-format pages under
[`docs/formats/`](formats/) -- those already carry the full detail for
the `i64`/temporal/`any` gaps. Where an entry below is already documented
there, this page states it briefly and links out rather than repeating
it.

## Integer representation: `i64` vs Python's arbitrary-precision `int`

**Python**: `int` is arbitrary precision; a decimal literal is only
rejected past `sys.set_int_max_str_digits`'s 4300-digit cap.
**Rust**: `document::Scalar::Int` is `i64` (max ~19 significant decimal
digits), so any literal over that -- even one comfortably under the
4300-digit cap -- is a `ParseError` ("out of range for a 64-bit
integer").

Origin: PR #5 (`document.rs`) chose `i64` for the Document model itself
and, as a direct consequence, dropped Python's `_check_int_digits` guard
entirely rather than ship "permanently-dead code with no reachable
branch to test" -- the guard exists in Python to stop a superlinear
`str()` conversion of a huge `int`, which can't happen when the type
tops out at 19 digits. PR #11 (`oml.rs`) then had to reintroduce a
4300-digit cap anyway, but at the *text-scanning* layer (`MAX_INT_DIGITS`
in `oml.rs`) rather than the Document model, because OML source text is
"the one place an arbitrarily-long digit run can actually reach the
parser" before the `i64` conversion fails. PR #17 (`json.rs`) verified
live against CPython's `json.loads` that the same 4300-digit cap fires
independently of the 64-bit ceiling, then applied the identical
text-layer cap. PR #23 (`xml.rs`) found a genuinely different Python
behavior for this same gap: Python's XML `_coerce` falls through a
20-4300-digit numeral to a `float` rather than erroring, so the Rust port
matches that specific control flow (falls through to `Scalar::Float`)
instead of raising.

Permanent: yes. It's a direct, disclosed consequence of #5's
Document-model decision (`Scalar` has exactly five variants, `Int` is
`i64`), applied consistently rather than special-cased per format. See
[`limitations.md`](limitations.md#representational-limits-from-scalarint-being-i64)
for the format-by-format table.

## TOML's hex/octal/binary digit-cap (corrected finding, PR #21)

**Python**: `tomllib` applies the 4300-digit cap to decimal literals
(same `sys.set_int_max_str_digits` mechanism as JSON), but **exempts**
hex/octal/binary (power-of-two-base) literals from that cap entirely --
confirmed live: `tomllib.loads("x = 0x" + "f"*10000)` parses with no
error at all.

**Rust**: applies the same `i64` bound to every radix uniformly, via
`toml_edit`'s parser. Any oversized hex/octal/binary literal is rejected
the same way an oversized decimal one is.

An earlier draft of PR #21 (and its module doc) incorrectly claimed hex
literals hit the identical `ValueError` as decimal ones in Python -- that
claim was wrong and was corrected mid-review once live-checked against
`tomllib` directly. The corrected finding: Python's radix exemption is
real, and this port does not replicate it, because `Scalar::Int` is
`i64`-backed regardless of the literal's source radix -- there's no
representational path for an "uncapped hex" literal past 64 bits in this
port regardless of what the cap logic does.

Permanent: yes, for the same reason as the general `i64` gap above --
revisiting it would require a non-`i64` `Scalar::Int`.

## Date-shaped strings as native temporal literals (TOML/YAML) -- resolved by issue #105

**Formerly**: `Scalar` had no temporal variant, so `toml.rs`/`yaml.rs`
could not distinguish "a string that happens to look like a date" from "a
value that was genuinely read as a date," and shape-guessed: any
`Scalar::Str` whose contents matched the temporal shape check wrote back
as a native temporal literal, unlike Python, which only ever promotes a
real `datetime` object.

**Now**: issue #105 gave `Scalar`/`Value` real `Date(String)`/
`Time(String)`/`Datetime(String)` variants. `toml.rs`/`yaml.rs` construct
the real variant directly from `toml_edit`'s own already-validated
date/time fields (TOML) or from `normalize_timestamp`'s output (YAML) --
never by shape-guessing a `Str` -- and write only a genuine temporal
variant as a native literal; a plain string that merely looks like a date
now stays a quoted string on write, matching Python exactly. This
divergence is resolved, not permanent -- see
[`limitations.md`](limitations.md#temporal-kinds-have-no-arithmetic) for
what temporal kinds still don't do (arithmetic).

## Bare time literal round-tripping (OML, PR #11; corrected by issue #90)

**Python**: parses a bare time literal like `12:00` into a real
`datetime.time` object, then always writes it back via
`isoformat()`, which normalizes to `12:00:00` (seconds always present).

**Rust, as of PR #11**: `Scalar` stored the literal's exact source
spelling and round-tripped it byte-for-byte, so `12:00` stayed `12:00` --
characterized at the time as "a *stronger* round-trip guarantee than
Python's own, not a bug against Python's docs," no upstream issue filed.

**Rust, as of issue #90** (found while building the [conformance
harness](conformance.md) against omnist-spec): that stronger guarantee
was itself a spec violation -- omnist-spec's OML grammar treats
`time`/`datetime` literal text as canonicalized on read, the same way
Python's `isoformat()` normalizes it. `read_oml`'s scanner now fills a
missing `:SS` to `:00` and zero-pads under-padded fractional seconds to 6
digits, so `12:00` now reads as `Scalar::Str("12:00:00")` -- matching
Python's output exactly, not diverging from it. `date` literals (no
optional grammar components) are unaffected.

Permanent: yes, but now in the sense of "matches Python," not "diverges
from it" -- this entry is kept for history; the actual current behavior
is *not* a divergence.

## OML's UTC-offset preservation (PR #11) / TOML's identical case (PR #21)

Both codecs preserve an explicit UTC offset (`-07:00`/`+07:00`) exactly
on round-trip, and both normalize a bare `Z` suffix to `+00:00` rather
than preserving `Z` literally -- a normalization choice inherited from
`yaml.rs`'s `normalize_timestamp`, applied consistently across format
codecs rather than re-decided per format. Not a divergence from Python's
observable output; noted here because it was explicitly cross-checked
(the omnist-ts#51-derived regression class) in both PRs.

## `infer()`'s API shape: one Python kwarg vs two Rust functions

**Python**: `infer(samples, root_name, allow_any=False)` -- a single
function, `allow_any` is a keyword argument.

**Rust**: `infer::infer(samples, root_name)` always behaves as
Python's `allow_any=False` path (any ambiguous field is a
`SchemaError`); `infer::infer_with_report(samples, root_name, allow_any)`
is a second function that also accepts `allow_any: true` and returns
every recorded `AnyFallback` alongside the schema, mirroring Python's
`infer_with_report`/`AnyFallback` pair rather than its single
`infer(..., allow_any=...)` surface.

Origin: PR #15 (`infer.rs`) scoped `any` out of inference entirely,
leaving `infer` with no `allow_any` parameter at all -- an unforced
scoping choice, not something the deferred `any` governance question
required (see
[`limitations.md`](limitations.md#the-any-type-scoping-gap-deferred-not-forgotten)).
Issue #29 later flagged that scoping-out as a mistake and tracked
porting `any` for real. PR #33 (closing #29) fixed it by porting
`allow_any` as a second function, `infer_with_report`, rather than
adding an optional parameter to `infer` itself -- an intentional
API-shape split from Python's single-function-with-kwarg design,
flagged during PR #33's review as a deliberate divergence worth
recording rather than silently carrying forward.

Revisitable: yes, in principle -- collapsing back to a single function
with a defaulted parameter is a compatible API change if ever desired,
but the two-function split is deliberate for now.

## XML: scalar coercion narrowing (PR #23)

**Python**: `_coerce`'s rules, live-checked directly rather than
assumed. Two narrowings this port discloses:

- A 20-4300-digit numeral (too big for `i64`, still under the security
  cap) falls through to `Scalar::Float`, matching Python's own
  int-then-float control flow -- covered under the general `i64` gap
  above, not a separate issue.
- Unicode decimal digits (e.g. Arabic-indic digits) are **not**
  recognized by this port's coercion -- ASCII-digit-only, a narrowing
  from whatever Python's coercion accepts.

Permanent: yes, treated as a disclosed narrowing rather than a bug.

## XML: DTD/XXE safety by construction vs Python's `defusedxml` dependency

**Python**: `read_xml` requires `defusedxml` instead of the stdlib
`ElementTree`, specifically to guard against XXE/DTD-expansion attacks --
an explicit dependency choice to close a real vulnerability class.

**Rust**: uses `quick-xml` 0.41.0 (tokenization only, no `serde`
feature), which has no DTD/external-entity expansion support *at all*.
XXE safety is a structural property of the crate, not a guard that has
to be separately verified or maintained.

Not a behavioral divergence in observable output -- both are safe against
XXE -- but a divergence in *how* that safety is achieved, worth recording
because it removes an entire dependency Python's implementation needs.
Permanent (a consequence of the crate choice in PR #23).

## YAML: bool tag spellings and calendar-invalid timestamps (PR #19, bugs found and fixed, not divergences)

Two items from PR #19 are **not** divergences from Python -- they were
bugs in this port's own WIP checkpoint, found via live cross-check
against PyYAML and fixed to match Python exactly, before merge:

- Explicit `!!bool` tag construction was initially too narrow (only
  true/false spellings); PyYAML's `bool_values` accepts
  yes/no/on/off case-insensitively regardless of implicit vs explicit
  tagging. Fixed to match.
- A timestamp-shaped scalar naming an invalid calendar/clock value
  (`2024-13-01`, `2024-02-30`, hour 25, tz `+25:00`) initially fell back
  silently to a plain string; PyYAML actually raises and fails the whole
  document. Fixed to raise `ParseError`.

Listed here for completeness (issue #39 asked for both), but neither is
a live divergence today -- both match Python's behavior as of PR #19's
merge.

## Not a divergence: the `any`-type gap and the `i64`/temporal gaps documented elsewhere

Two structural gaps referenced above are covered in full in
[`limitations.md`](limitations.md) rather than repeated here:

- The `any`-type scoping gap (deferred pending the sibling `omnist`
  project's openness decision, per issue #29 and PR #33) --
  [`limitations.md`](limitations.md#the-any-type-scoping-gap-deferred-not-forgotten).
- The `i64` representational ceiling, per format --
  [`limitations.md`](limitations.md#representational-limits-from-scalarint-being-i64).
- Temporal kinds having no arithmetic (a real variant, but an opaque
  canonical string, not a `chrono`/`time` value) --
  [`limitations.md`](limitations.md#temporal-kinds-have-no-arithmetic).

## Summary table

| Divergence | Permanent or revisitable | Origin PR |
|---|---|---|
| `i64` vs arbitrary-precision `int` | Permanent | #5, #11, #17, #23 |
| TOML hex/octal/binary uncapped in Python, capped here | Permanent | #21 |
| Date-shaped string becomes native literal on write (TOML/YAML) | **No longer a divergence** -- resolved by #105's real temporal variants | #19, #21, #105 |
| Bare time literal round-trips exactly instead of normalizing | **No longer a divergence** -- corrected by #90 to match Python | #11, #90 |
| `infer()`/`infer_with_report()` split vs single `allow_any` kwarg | Revisitable | #15, #33 |
| XML coercion: over-`i64` numeral falls to float | Permanent | #23 |
| XML coercion: ASCII-only digit recognition | Permanent | #23 |
| XML XXE-safety by construction (`quick-xml`) vs `defusedxml` | Permanent (crate choice) | #23 |
