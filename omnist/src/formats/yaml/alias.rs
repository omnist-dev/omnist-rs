//! The YAML alias expansion limits (spec §2.4.1, D-18, D-18a, D-19, D-20, D-22).
//!
//! The checker consumes the parser's event stream and never builds a tree, so
//! an over-limit input is refused before anything is materialized (D-19): the
//! reader buffers the events and only replays them into the tree builder once
//! every candidate node has passed.
//!
//! For every candidate node `a` (every anchored node, and every mapping and
//! sequence whether anchored or not, the document root and an inline merge
//! source included; a scalar is never checked, `E = 1.00` trivially, and a
//! merge carrier is not a candidate) it computes, from the reference graph
//! alone:
//!
//! * `W(a)`, the value slots materialized when `a` is expanded;
//! * `S(a)`, the value slots written in `a`'s own definition;
//! * `E(a) = W(a) / S(a)`, rejected when it exceeds the maximum.
//!
//! A container and a scalar each count one slot; mapping keys are not value
//! slots. Inside a definition:
//!
//! * a plain alias `*b` counts one slot in `S` and contributes `W(b)` to `W`;
//! * a merge-key entry `<<: *b` counts one slot in `S` and contributes
//!   `W(b) - 1` to `W` (the container of `b` is flattened into the referring
//!   mapping, not reproduced);
//! * a sequence in merge-value position (`<<: [*p, *q]`) is a syntactic
//!   **carrier**, whether or not it is anchored (D-18a): it adds no slot of
//!   its own to `W` or `S` and is not a candidate. Its members, which must be
//!   mappings, each contribute `W - 1`; an alias member adds nothing to `S`,
//!   a mapping written (or first anchored) in it adds `S - 1`;
//! * `<<: *s`, an alias to a sequence, contributes what the same members
//!   written inline would: the sum over its mapping members of `W - 1`, and
//!   one slot (the `<<` entry) in `S`. A sequence written as an ordinary value
//!   and anchored there (`s: &s [*p, *q]`) is an ordinary candidate with
//!   `W = 1 + sum W(member)`; a plain alias to a carrier (`t: *s`) likewise
//!   materializes that list;
//! * an inline mapping merged in place (`<<: {k: 1}`): the `<<` entry is the
//!   one slot in the referrer's `S`, the inline mapping's own written values
//!   add theirs (`S - 1`, its container excluded) and its container is
//!   flattened away in `W` (contribution `W - 1`). It is also a candidate on
//!   its own subtree.
//!
//! **Merge shapes** are validated in the same pass (D-18a): a merge value that
//! is not a mapping or a sequence of mappings (a scalar, a scalar or sequence
//! member of a carrier, an alias to a scalar or to a sequence that holds
//! anything but mappings) is `parse.codec-syntax` at the offending node's
//! position, and it wins over every `document.limit.*` code, so the checker
//! keeps reading (and keeps counting, saturating) after it finds a limit
//! violation: only the first limit is kept, and a later syntax error replaces
//! it.
//!
//! **D-22**: at the end of a document containing at least one alias or merge
//! key, `W(root)` above the maximum expanded size is rejected with
//! `document.limit.expanded-size`, after the ratio check (a ratio violation
//! found earlier is reported instead). An input with neither is exempt.
//!
//! `W` is the structural count of the spec, deliberately blind to key
//! collisions (D-19): it may exceed what is finally materialized, never fall
//! below it.
//!
//! One pass, memoized per anchor, constant work per event, `E` checked as
//! each container completes so no `W` held exceeds `max * S` of the node in
//! hand. All arithmetic saturates at `u64::MAX` regardless, so a count can
//! never wrap under the limit. A reference to an anchor whose definition has
//! not completed is a cycle (D-20): `W` is unbounded and the input is rejected
//! under the same code without computing any `E`.

use std::collections::HashMap;

use yaml_rust2::parser::Event;
use yaml_rust2::scanner::TScalarStyle;

use crate::error::{DocumentError, OmnistError, ParseError};

/// The code every D-18/D-20 rejection carries.
pub(super) const CODE: &str = "document.limit.alias-expansion";

/// The code every D-22 rejection carries.
pub(super) const SIZE_CODE: &str = "document.limit.expanded-size";

/// What a completed node is, as far as a merge cares.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Scalar,
    Map,
    Seq,
}

/// Where a node sits in its parent: how it folds into the parent's counts.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pos {
    /// No parent: the document root.
    Root,
    /// A mapping key: keys are not value slots.
    Key,
    /// An ordinary mapping value.
    Value,
    /// The value of a `<<` key.
    MergeValue,
    /// An item of an ordinary sequence.
    SeqItem,
    /// An item of a merge carrier (which must be a mapping).
    CarrierMember,
}

/// What the checker remembers of a completed node.
#[derive(Clone, Copy)]
struct Info {
    kind: Kind,
    /// `W` as an ordinary value (a plain alias contributes this).
    w: u64,
    /// `S` as written (an alias counts one slot instead).
    s: u64,
    /// Sequence only: what `<<: *seq` contributes to `W`, the sum over its
    /// mapping members of `W - 1`.
    merge_w: u64,
    /// Carrier only: the written slots of its first-defined mapping members,
    /// less their containers, which a first-defined carrier adds to `S`.
    inline_s: u64,
    /// Sequence only: every member is a mapping (or an alias to one).
    all_map: bool,
}

const SCALAR: Info = Info {
    kind: Kind::Scalar,
    w: 1,
    s: 1,
    merge_w: 0,
    inline_s: 0,
    all_map: false,
};

/// One container being walked.
struct Frame {
    aid: usize,
    is_map: bool,
    pos: Pos,
    /// A sequence in merge-value position.
    carrier: bool,
    /// `W` and `S` of this node as it folds into its parent. For a carrier
    /// they are already net of the flattening of its members.
    w: u64,
    s: u64,
    /// Sequence only: `1 + sum W(member)`, what a plain alias to a carrier
    /// materializes (equal to `w` for an ordinary sequence).
    full_w: u64,
    /// Sequence only: sum over mapping members of `W - 1`.
    merge_w: u64,
    /// Sequence only: every member is a mapping.
    all_map: bool,
    /// Mapping only: the next child is a key.
    expect_key: bool,
    /// Mapping only: the key just read was `<<`, so the next child is a merge
    /// value.
    merge_value: bool,
}

/// The streaming checker. Feed it every event in order; it records the first
/// limit violation and the first malformed merge as it goes.
pub(super) struct AliasCheck {
    ratio_limit: u64,
    size_limit: u64,
    stack: Vec<Frame>,
    /// Every completed anchored node.
    memo: HashMap<usize, Info>,
    /// An alias or a merge key has been seen (D-22 applies).
    subject_to_size: bool,
    limit_error: Option<DocumentError>,
    shape_error: Option<ParseError>,
}

fn sub1(x: u64) -> u64 {
    x.saturating_sub(1)
}

fn is_merge_key(text: &str, style: TScalarStyle, tagged: bool) -> bool {
    text == "<<" && style == TScalarStyle::Plain && !tagged
}

impl AliasCheck {
    pub(super) fn new(ratio_limit: u64, size_limit: u64) -> Self {
        AliasCheck {
            ratio_limit,
            size_limit,
            stack: Vec::new(),
            memo: HashMap::new(),
            subject_to_size: false,
            limit_error: None,
            shape_error: None,
        }
    }

    /// Current container nesting depth.
    pub(super) fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Whether anything has been found wrong so far (the reader stops
    /// buffering events once it has).
    pub(super) fn failed(&self) -> bool {
        self.limit_error.is_some() || self.shape_error.is_some()
    }

    /// The error to report, if any: a malformed merge wins over a limit.
    pub(super) fn into_error(self) -> Option<OmnistError> {
        match (self.shape_error, self.limit_error) {
            (Some(e), _) => Some(e.into()),
            (None, Some(e)) => Some(e.into()),
            (None, None) => None,
        }
    }

    fn limit(&mut self, code: &str, message: &str) {
        if self.limit_error.is_none() {
            self.limit_error = Some(DocumentError::with_code("$", code, message));
        }
    }

    fn shape(&mut self, pos: (usize, usize), what: &str) {
        if self.shape_error.is_none() {
            self.shape_error = Some(ParseError::codec_syntax(
                pos.0,
                pos.1,
                format!(
                    "invalid YAML: a merge key's value must be a mapping or a sequence of \
                     mappings, not {what}"
                ),
            ));
        }
    }

    /// Where the next child sits in its parent.
    fn pos(&self) -> Pos {
        match self.stack.last() {
            None => Pos::Root,
            Some(p) if !p.is_map => {
                if p.carrier {
                    Pos::CarrierMember
                } else {
                    Pos::SeqItem
                }
            }
            Some(p) if p.expect_key => Pos::Key,
            Some(p) if p.merge_value => Pos::MergeValue,
            Some(_) => Pos::Value,
        }
    }

    /// A child finished: a mapping parent moves from key to value or back.
    fn advance_parent(&mut self) {
        if let Some(p) = self.stack.last_mut()
            && p.is_map
        {
            if p.expect_key {
                p.expect_key = false;
            } else {
                p.expect_key = true;
                p.merge_value = false;
            }
        }
    }

    /// Folds a completed child (`alias`: it is a reference, not a definition)
    /// into its parent according to where it sits.
    fn fold(&mut self, pos: Pos, c: &Info, alias: bool, at: (usize, usize)) {
        let mut bad = None;
        if let Some(p) = self.stack.last_mut() {
            match pos {
                Pos::Root | Pos::Key => {}
                Pos::Value => {
                    p.w = p.w.saturating_add(c.w);
                    p.s = p.s.saturating_add(if alias { 1 } else { c.s });
                }
                Pos::MergeValue => match c.kind {
                    Kind::Map => {
                        p.w = p.w.saturating_add(sub1(c.w));
                        p.s = p.s.saturating_add(if alias { 0 } else { sub1(c.s) });
                    }
                    Kind::Seq if c.all_map => {
                        p.w = p.w.saturating_add(c.merge_w);
                        p.s = p.s.saturating_add(if alias { 0 } else { c.inline_s });
                    }
                    Kind::Seq => bad = Some("a sequence with a member that is not a mapping"),
                    Kind::Scalar => bad = Some("a scalar"),
                },
                Pos::SeqItem => {
                    p.w = p.w.saturating_add(c.w);
                    p.s = p.s.saturating_add(if alias { 1 } else { c.s });
                    p.full_w = p.full_w.saturating_add(c.w);
                    if c.kind == Kind::Map {
                        p.merge_w = p.merge_w.saturating_add(sub1(c.w));
                    } else {
                        p.all_map = false;
                    }
                }
                Pos::CarrierMember => {
                    if c.kind == Kind::Map {
                        p.w = p.w.saturating_add(sub1(c.w));
                        p.s = p.s.saturating_add(if alias { 0 } else { sub1(c.s) });
                        p.full_w = p.full_w.saturating_add(c.w);
                    } else {
                        bad = Some("a sequence member that is not a mapping");
                    }
                }
            }
        }
        if let Some(what) = bad {
            self.shape(at, what);
        }
    }

    fn start(&mut self, aid: usize, is_map: bool, at: (usize, usize)) {
        let pos = self.pos();
        let carrier = !is_map && pos == Pos::MergeValue;
        if !is_map && pos == Pos::CarrierMember {
            self.shape(at, "a sequence member that is a sequence");
        }
        let slot = u64::from(!carrier);
        self.stack.push(Frame {
            aid,
            is_map,
            pos,
            carrier,
            w: slot,
            s: slot,
            full_w: 1,
            merge_w: 0,
            all_map: true,
            expect_key: true,
            merge_value: false,
        });
    }

    fn end(&mut self, at: (usize, usize)) {
        let f = self
            .stack
            .pop()
            .expect("an end event matches a start event");
        let info = Info {
            kind: if f.is_map { Kind::Map } else { Kind::Seq },
            w: if f.carrier { f.full_w } else { f.w },
            s: f.s,
            merge_w: if f.carrier { f.w } else { f.merge_w },
            inline_s: f.s,
            all_map: f.all_map,
        };
        if f.aid > 0 {
            self.memo.insert(f.aid, info);
        }
        // A saturated W (true count at or past 2^64) is over the limit
        // whatever the limit is: max*S saturates too, and equal saturated
        // values must not compare as "within".
        if !f.carrier && (f.w == u64::MAX || f.w > self.ratio_limit.saturating_mul(f.s)) {
            self.limit(
                CODE,
                "a mapping's or sequence's alias expansion factor exceeds the configured maximum",
            );
        }
        // D-22 at the root, after every candidate has had its ratio check
        // (a ratio violation already recorded is the one reported). No
        // special case for a saturated W: the cap is a u32 value, below it.
        if f.pos == Pos::Root && self.subject_to_size && f.w > self.size_limit {
            self.limit(
                SIZE_CODE,
                "an input with aliases or merge keys expands to more value slots than the \
                 configured maximum expanded size",
            );
        }
        self.fold(f.pos, &info, false, at);
        self.advance_parent();
    }

    /// Consumes one parser event; `at` is its 1-based `(line, column)`.
    pub(super) fn event(&mut self, ev: &Event, at: (usize, usize)) {
        match ev {
            Event::SequenceStart(aid, _) => self.start(*aid, false, at),
            Event::MappingStart(aid, _) => self.start(*aid, true, at),
            Event::SequenceEnd | Event::MappingEnd => self.end(at),
            Event::Scalar(text, style, aid, tag) => {
                let pos = self.pos();
                if pos == Pos::Key && is_merge_key(text, *style, tag.is_some()) {
                    // The `<<` entry is one written slot, whatever it holds.
                    let p = self.stack.last_mut().expect("a key has a parent mapping");
                    p.s = p.s.saturating_add(1);
                    p.merge_value = true;
                    self.subject_to_size = true;
                }
                if *aid > 0 {
                    self.memo.insert(*aid, SCALAR);
                }
                self.fold(pos, &SCALAR, false, at);
                self.advance_parent();
            }
            Event::Alias(id) => {
                self.subject_to_size = true;
                let c = match self.memo.get(id).copied() {
                    Some(c) => c,
                    None => {
                        self.limit(
                            CODE,
                            "an anchor refers to itself, directly or through other anchors \
                             (unbounded expansion)",
                        );
                        // Count it as an empty mapping from here on.
                        Info {
                            kind: Kind::Map,
                            w: 1,
                            s: 1,
                            merge_w: 0,
                            inline_s: 0,
                            all_map: true,
                        }
                    }
                };
                let pos = self.pos();
                self.fold(pos, &c, true, at);
                self.advance_parent();
            }
            Event::Nothing
            | Event::StreamStart
            | Event::StreamEnd
            | Event::DocumentStart
            | Event::DocumentEnd => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: (usize, usize) = (1, 1);

    fn feed(check: &mut AliasCheck, events: &[Event]) {
        events.iter().for_each(|e| check.event(e, AT));
    }

    fn scalar(text: &str) -> Event {
        Event::Scalar(text.to_string(), TScalarStyle::Plain, 0, None)
    }

    /// The recorded error, `Debug`-printed (it carries its code).
    fn limit_code(check: AliasCheck) -> String {
        format!("{:?}", check.into_error().expect("an error was recorded"))
    }

    /// A `W` past `u64::MAX` saturates and is rejected whatever the limit is,
    /// even the largest one; wrapping would compare under it (D-19).
    #[test]
    fn saturated_w_is_rejected_even_at_the_largest_limit() {
        let mut check = AliasCheck::new(u64::MAX, u64::MAX);
        // Two completed definitions whose W is already close to the top.
        let near_top = Info {
            kind: Kind::Map,
            w: u64::MAX - 1,
            s: 1,
            merge_w: 0,
            inline_s: 0,
            all_map: true,
        };
        check.memo.insert(1, near_top);
        check.memo.insert(2, near_top);
        let events = [
            Event::MappingStart(0, None),
            scalar("x"),
            Event::Alias(1),
            scalar("y"),
            Event::Alias(2),
            Event::MappingEnd,
        ];
        feed(&mut check, &events);
        assert!(limit_code(check).contains(CODE));
    }

    /// The same saturation through the merge path: every `W - 1` and the sum
    /// of members saturate rather than wrap.
    #[test]
    fn saturated_merge_contributions_do_not_wrap() {
        let mut check = AliasCheck::new(u64::MAX, u64::MAX);
        let near_top = Info {
            kind: Kind::Map,
            w: u64::MAX,
            s: 1,
            merge_w: 0,
            inline_s: 0,
            all_map: true,
        };
        check.memo.insert(1, near_top);
        let events = [
            Event::MappingStart(0, None),
            scalar("<<"),
            Event::SequenceStart(0, None),
            Event::Alias(1),
            Event::Alias(1),
            Event::Alias(1),
            Event::SequenceEnd,
            Event::MappingEnd,
        ];
        feed(&mut check, &events);
        assert!(limit_code(check).contains(CODE));
    }

    /// An exact W equal to the maximum times S is within; one past is over.
    #[test]
    fn equal_is_within_and_one_past_is_over() {
        for (limit, ok) in [(3u64, true), (2, false)] {
            let mut check = AliasCheck::new(limit, u64::from(u32::MAX));
            check.memo.insert(
                1,
                Info {
                    kind: Kind::Map,
                    w: 4,
                    s: 4,
                    merge_w: 0,
                    inline_s: 0,
                    all_map: true,
                },
            );
            // {a: *1}: W = 1 + 4, S = 2, E = 2.5: within 3, over 2.
            let events = [
                Event::MappingStart(0, None),
                scalar("a"),
                Event::Alias(1),
                Event::MappingEnd,
            ];
            feed(&mut check, &events);
            assert_eq!(check.failed(), !ok);
        }
    }

    /// The size cap: `W(root)` equal to it is within, one past is over, and
    /// only a document with an alias or a merge key is subject to it.
    #[test]
    fn size_cap_equal_is_within_one_past_is_over_and_alias_free_is_exempt() {
        // {a: *x} with x a scalar anchor: W(root) = 1 + 1.
        for (cap, ok) in [(2u64, true), (1, false)] {
            let mut check = AliasCheck::new(50, cap);
            check.memo.insert(1, SCALAR);
            let events = [
                Event::MappingStart(0, None),
                scalar("a"),
                Event::Alias(1),
                Event::MappingEnd,
            ];
            feed(&mut check, &events);
            assert_eq!(check.failed(), !ok, "cap {cap}");
        }
        let mut check = AliasCheck::new(50, 1);
        feed(
            &mut check,
            &[
                Event::MappingStart(0, None),
                scalar("a"),
                scalar("b"),
                Event::MappingEnd,
            ],
        );
        assert!(!check.failed());
    }

    #[test]
    fn an_alias_to_an_unfinished_or_unknown_anchor_is_a_cycle() {
        let mut check = AliasCheck::new(50, 1_000);
        check.event(&Event::Alias(7), AT);
        assert!(limit_code(check).contains(CODE));
    }

    #[test]
    fn a_malformed_merge_replaces_an_earlier_limit_error() {
        let mut check = AliasCheck::new(50, 1_000);
        check.event(&Event::Alias(7), AT);
        assert!(check.failed());
        feed(
            &mut check,
            &[Event::MappingStart(0, None), scalar("<<"), scalar("x")],
        );
        assert!(matches!(check.into_error(), Some(OmnistError::Parse(_))));
    }

    #[test]
    fn depth_tracks_open_containers() {
        let mut check = AliasCheck::new(50, 1_000);
        assert_eq!(check.depth(), 0);
        check.event(&Event::SequenceStart(0, None), AT);
        check.event(&Event::MappingStart(0, None), AT);
        assert_eq!(check.depth(), 2);
        check.event(&Event::MappingEnd, AT);
        check.event(&Event::SequenceEnd, AT);
        assert_eq!(check.depth(), 0);
        assert!(!check.failed());
        assert!(check.into_error().is_none());
    }
}
