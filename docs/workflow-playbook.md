# DevOps playbook (ported from omnist-ts, originally from omnist Python)

Standing operating procedure for AI-agent-driven development on this repo,
carried over from [omnist-dev/omnist](https://github.com/omnist-dev/omnist)
(Python) via [omnist-dev/omnist-ts](https://github.com/omnist-dev/omnist-ts)
(TypeScript). Public writeup of the original version:
[lee.yt/posts/the-devops-team-that-never-sleeps](https://lee.yt/posts/the-devops-team-that-never-sleeps/).
This file is the working reference, adapted for a Rust toolchain.

## The eight rules

1. **Agree on the plan before any code.** Argue the design out first; once
   settled, it doesn't get re-litigated mid-build.
2. **Make the spec precise enough that no one has to come ask.**
   Agreed-but-fuzzy isn't done — pin the awkward cases or the builder
   guesses.
3. **The agent that builds it never gets to sign it off.** A different
   agent (or the controller, re-running it) reviews every change.
4. **Match the model to the task** (tier table below) — scale-dependent,
   pays off once there's enough independent work to split up. For small
   in-session increments, doing mechanical/algorithmic/design work directly
   is fine.
5. **Run in parallel only what's truly independent.** Check *informational*
   dependency too, not just file ownership — two tasks that don't share a
   file can still share meaning (e.g. both need the same upstream design
   decision resolved first).
6. **Test like you don't trust yourself.** 100% coverage, fuzzing/property
   tests, doc examples that run as tests, red-before-green for behavior
   changes. A claim that code is *unreachable* needs the same empirical
   proof as a claim that code is *covered*.
7. **Ship code, tests, and docs together, or don't ship.** The version bump
   and changelog entry land in the *same* PR as the change, never a
   follow-up.
8. **Leave a trail for everything.** Spec, outcome, and every surprise
   found along the way all get written down.

## Rust-specific additions to rule 1 (plan before code)

Three decisions belong in the **design doc**, before issue #1 (the document
model) is even filed, because retrofitting them later is expensive in Rust
specifically:

- **Node/Edge representation**: arena/index-based tree vs.
  `Rc`/`RefCell`-based — shapes the depth-guard and mutation story for the
  whole port.
- **Ordered-map type**: `IndexMap`/`BTreeMap` picked before the first
  algebra function is written, not after a nondeterminism bug turns up.
  Rust's `HashMap` is randomized by default (more aggressively than
  Python's `PYTHONHASHSEED`-gated randomization) — anywhere Python/TS used
  an ordered structure to dodge this, the Rust port needs an explicit
  ordered type or sort, not `HashMap`.
- **Error hierarchy**: `thiserror`-based, mirroring
  `OmnistError`/`SchemaError`/`ParseError`/`WriteError`/`DocumentError`,
  decided up front — Rust's `Result`-based control flow makes retrofitting
  a shared hierarchy costlier than in an exception-based language.

## The tech lead and the team

One **master agent** (the controller) plays tech lead — doesn't write
code, runs the team: breaks the signed-off plan into tasks, assigns tier,
decides parallel-vs-sequential, starts/tracks workers, sends finished work
to a *different* worker to review, files an issue when work uncovers a new
problem, merges only on clean review.

**Worker tiers:**

| Tier | Model | Good for | Example tasks |
|---|---|---|---|
| Mechanical | Cheapest, fastest | Rote, mechanical right answer | find-and-replace, version bumps, running the suite, opening a branch/PR |
| Algorithmic | Balanced | Real logic, but a spec already says what "correct" is | implementing a feature to a written issue, root-causing a failed test, verifying another agent's work against the spec |
| Design | Most capable | Open-ended, no spec yet, consequences that outlive the change | deciding a design question and writing down why, turning that into a spec, ordering a multi-step release so no half-finished state is unsafe to ship |

Why tier at all: cheap work on a cheap model keeps the token bill sane.
Quality doesn't ride on the call anyway — a second agent reviews everything
regardless of who built it. This is a scale-dependent rule — skip tiering
for small in-session tasks.

## The loop (9 steps, each with its own audit-trail artifact)

1. File an issue with the actual spec in the body — artifact: the issue
   body.
2. Cut a branch, pick the tier, start an implementer — artifact: branch
   name references the issue.
3. Build to the spec — artifact: commits + opened PR.
4. Run the full suite in CI — artifact: pass/fail logs on the PR.
5. Verify independently, correctness + performance — artifact: a written
   verdict with evidence.
6. If verification finds a problem, fix and re-verify; if the plan itself
   has to change, say so on the issue — artifact: a comment, never a
   silent patch.
7. If work uncovers a *separate* problem, file a new issue — artifact: new
   issue linked back to where it was found.
8. Merge, tag, release — artifact: changelog entry, tag, release notes.
9. Close the issue with a summary of what actually happened, including any
   divergence from plan — artifact: the closing comment.

**Plan-approval gate is a distinct step from filing the issue.** Filing a
well-specified issue is not the same as having read-and-approved sign-off —
launching an implementer waits for explicit approval after the plan is
shown, every time.

## Red before green

For any change to library code, the implementer writes the test(s) first,
runs them, and shows the actual **failure** output, before touching the
implementation. Only then implement to green, showing the actual
**passing** output too. The independent verifier's job explicitly includes
**reproducing the red state itself**.

**Scoped by tier:**
- **Design-tier changes**: test obligations get specified in the plan/issue
  itself, before implementation starts.
- **Algorithmic-tier changes**: implementer owns red-then-green; verifier
  reproduces it.
- **Mechanical/docs-only work**: no TDD — existing behavior gets a
  **characterization test** instead.
- A mechanical-looking task that turns out to be a bug fix graduates to the
  algorithmic-tier rule regardless of diff size.

## 100% coverage standard

Every release ships at 100% line coverage across the whole repo, via
`cargo llvm-cov`. Gaps are classified, not open-ended:

1. **Defensive trip-wires** — annotate with the toolchain's coverage-ignore
   equivalent plus a one-line reason.
2. **Rare-but-real branches** — force deterministically with a direct unit
   test or seeded example, not left to random-seed luck.
3. **Unreachable dead code** — confirmed via an actual exhaustive/
   adversarial check before deleting.

Exact pragma syntax needs re-deriving for `cargo llvm-cov` (not the same as
Istanbul's `/* c8 ignore next */` or Python's `# pragma: no cover`) — check
when first needed. Update every doc that quotes the coverage percentage in
the same release that changes it.

## Doc-example CI gate

Every code example shown in the docs must be tested and CI-enforced per
PR. Pattern ported from Python's `tools/check_doc_examples.py` /
omnist-ts's equivalent: a script diffs `docs/**/*.md` against the PR's base
branch; every fenced code block added or changed needs a
`<!-- verified-by: path/to/test.rs::test_name -->` marker naming the test
that asserts the block's exact literal output, or
`<!-- doc-illustrative -->` as an explicit opt-out. Wired into CI as its
own `pull_request`-only job with full git history fetched.

**Known, deliberate gap** (carried over from omnist and omnist-ts): this
only checks a marker is *present*, not that it's *honest*. Evaluated twice
already and deliberately not built — handled instead by periodic manual
multi-agent re-audits before each minor release.

## When the port disagrees with upstream Python or TypeScript

Porting will surface cases where this implementation's behavior differs
from the Python original or the TypeScript port. Two different situations,
two different responses:

1. **The difference is a bug in *this* port** (a mistranslation of the
   spec, a missed edge case). Fix the Rust port to match the spec. Default
   assumption — check it first.
2. **The difference exists because Python's or TypeScript's behavior is
   itself wrong** against the formal spec, and there's concrete evidence
   (a worked spec example either gets wrong, a violated algebra property, a
   genuine language-independent correctness bug):
   - **Do not replicate the wrong behavior** just to match a sibling
     implementation. Implement the Rust port against the *spec*.
   - **File an issue on the affected repo** —
     [omnist-dev/omnist](https://github.com/omnist-dev/omnist) if Python is
     wrong, [omnist-dev/omnist-ts](https://github.com/omnist-dev/omnist-ts)
     if TypeScript is wrong (check whether the bug is shared by both or
     specific to one before deciding where to file). Include the spec
     section violated, a concrete input/output pair, and a suggested fix
     direction — enough for that repo's maintainer to act on without
     re-deriving the finding.
   - Note the divergence and the upstream issue number in `omnist-rs`'s own
     issue/PR, so the history explains why Rust doesn't match a sibling
     implementation's current released behavior.
   - The sibling repo's maintainer decides whether/how/when to fix it — the
     Rust port's job is to surface the evidence, not to wait for or assume
     a fix before shipping.

Same evidence bar as rule 6 (red-before-green): a reproducible input/output
pair or a specific spec citation, not "this looks off."

## Architecture freedom

Structural mirroring of the Python/TS code (module layout, function names,
data-flow shape) is a *starting point*, not a requirement. Where Rust
idioms give a better answer — ownership model, trait boundaries, error
handling, module/crate organization, workspace layout — optimize for Rust,
not for line-by-line resemblance to the other two implementations.

**The one constraint: observable behavior must not change** relative to
the spec (same inputs → same outputs, same validation results, same error
conditions). Anything below that line is free to diverge for idiomatic
Rust, and *should*, rather than importing a Python- or TypeScript-shaped
design that fights the language.

This means the "Rust-specific decisions made up front" list above (Node/
Edge representation, ordered-map choice, error hierarchy) is a from-scratch
Rust design decision, informed by but not bound to how Python/TS solved the
same problem.

## Toolchain mapping (Python / TypeScript → Rust)

| Python | TypeScript | Rust |
|---|---|---|
| `pytest` | `vitest` | `cargo test` |
| `ruff check` | `eslint` | `clippy` + `rustfmt` |
| `mypy --strict` | `tsc --noEmit` strict | Rust's type system + `#[deny(warnings)]` |
| `coverage.py` | `vitest run --coverage` | `cargo llvm-cov` |
| `mkdocs build --strict` | N/A | N/A unless this repo adopts a doc-site generator |
| PyPI publish on tag push | npm publish on tag push | `cargo publish` on tag push — **gated pending review**, not automatic |
| `hypothesis` | `fast-check` | `proptest` |

## Tooling and multi-agent-workflow gotchas (language-agnostic)

- **HTTPS git clones hang waiting for credentials in this WSL setup —
  always clone via SSH.**
- **Dispatched agents' native file tools silently resolve onto the wrong
  filesystem** when working in a WSL-hosted repo from a Windows-hosted
  session. Route every file/git operation through
  `wsl -d Debian -e bash -lc "..."` explicitly.
- **Multi-line commit messages / PR bodies through a nested
  Windows-shell-wraps-WSL-shell-wraps-git pipeline get mangled by quote
  parsing.** Always write the message to a file first (`git commit -F
  file`, `gh pr create --body-file file`).
- **Parallel agents must use isolated `git worktree`s**, never a shared
  checkout.
- **A local commit can end up stranded on a "pull-only" mirror checkout** —
  don't discard it; diff it, recreate the change through the real
  source-of-truth checkout, push it properly, then reset the mirror.
- **`gh pr edit --base` silently no-ops** — use
  `gh api -X PATCH repos/O/R/pulls/N -f base=X` instead.
- **Background-agent "completed" notifications with very few tool calls
  and a report that just restates the task are a red flag** — verify
  against actual repo/GitHub state before trusting it.

## What NOT to carry over unexamined

- Coverage tooling differs enough (llvm-cov vs. Istanbul vs.
  `coverage.py`) that exact pragma syntax and gap-classification workflow
  need re-deriving, not copy-pasting.
- TS/Python leaned on runtime validation where their type systems had
  gaps; Rust's stronger type system may close some of those gaps for free
  — don't assume every validation path from the TS port is still needed,
  but don't drop one without checking first either.
- The audit-trail loop assumes a GitHub issue tracker — unchanged here
  since `omnist-rs` also uses GitHub.
