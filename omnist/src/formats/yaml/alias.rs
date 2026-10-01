//! The YAML alias expansion limit (spec §2.4.1, D-18, D-19, D-20).
//!
//! The checker consumes the parser's event stream and never builds a tree, so
//! an over-limit input is refused before anything is materialized (D-19): the
//! reader buffers the events and only replays them into the tree builder once
//! every candidate node has passed.
//!
//! For every candidate node `a` (every anchored node, and every mapping and
//! sequence whether anchored or not, the document root and an inline merge
//! source included; a scalar is never checked, `E = 1.00` trivially) it
//! computes, from the reference graph alone:
//!
//! * `W(a)`, the value slots materialized when `a` is expanded;
//! * `S(a)`, the value slots written in `a`'s own definition;
//! * `E(a) = W(a) / S(a)`, rejected when it exceeds the maximum.
//!
//! A container and a scalar each count one slot; mapping keys are not value
//! slots. Inside a definition:
//!
//! * a plain alias `*b` counts one slot in `S` and contributes `W(b)` to `W`;
//! * a merge-key entry `<<: *b` counts one slot in `S` however many aliases it
//!   holds, and each alias contributes `W(b) - 1` to `W` (the container of `b`
//!   is flattened into the referring mapping, not reproduced);
//! * a merge value that is an unanchored sequence (`<<: [*p, *q]`) is a
//!   syntactic carrier: it adds no slot of its own to `W` or `S`;
//! * an inline mapping merged in place (`<<: {k: 1}`): the `<<` entry is the
//!   one slot in the referrer's `S`, the inline mapping's own written values
//!   add theirs (`S - 1`, its container excluded) and its container is
//!   flattened away in `W` (contribution `W - 1`). It is also a candidate on
//!   its own subtree.
//!
//! Known edge (a spec ambiguity; same reading as the Go port): an *anchored*
//! literal merge sequence `<<: &s [*p, *q]` is treated as an ordinary merge
//! value (a container that is flattened, `W - 1`), not as the syntactic
//! carrier of the unanchored form.
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

use crate::error::DocumentError;

/// The code every D-18/D-20 rejection carries.
pub(super) const CODE: &str = "document.limit.alias-expansion";

/// How a completed child's `(w, s)` folds into its parent.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fold {
    /// A mapping key: keys are not value slots.
    Ignore,
    /// A value slot: `w` and `s` are added in full.
    Plain,
    /// A merged mapping (alias or inline): `w - 1` goes to `W`, and `s - 1`
    /// to `S` unless it is an alias (an alias counts in `S` through the
    /// single `<<` entry).
    Merge,
    /// An unanchored literal merge sequence, whose `w` and `s` are already net
    /// of its members' flattening and are added as they stand.
    Carrier,
}

/// One container being walked.
struct Frame {
    aid: usize,
    is_map: bool,
    w: u64,
    s: u64,
    carrier: bool,
    fold: Fold,
    /// Mapping only: the next child is a key.
    expect_key: bool,
    /// Mapping only: the key just read was `<<`, so the next child is a merge
    /// value.
    merge_value: bool,
}

/// The streaming checker. Feed it every event in order; it returns the
/// rejection as soon as one is found.
pub(super) struct AliasCheck {
    limit: u64,
    stack: Vec<Frame>,
    /// `W` of every completed anchored node (a reference counts one slot in
    /// `S`, so `S` of its target is never needed).
    memo: HashMap<usize, u64>,
}

fn sub1(x: u64) -> u64 {
    x.saturating_sub(1)
}

fn is_merge_key(text: &str, style: TScalarStyle, tagged: bool) -> bool {
    text == "<<" && style == TScalarStyle::Plain && !tagged
}

fn rejection(message: &str) -> DocumentError {
    DocumentError::with_code("$", CODE, message)
}

impl AliasCheck {
    pub(super) fn new(limit: u64) -> Self {
        AliasCheck {
            limit,
            stack: Vec::new(),
            memo: HashMap::new(),
        }
    }

    /// Current container nesting depth.
    pub(super) fn depth(&self) -> usize {
        self.stack.len()
    }

    /// How a child about to start (or complete) folds into its parent, and
    /// whether it is a carrier sequence.
    fn classify(&self, child_is_unanchored_seq: bool) -> (Fold, bool) {
        match self.stack.last() {
            None => (Fold::Plain, false),
            Some(p) if !p.is_map => (if p.carrier { Fold::Merge } else { Fold::Plain }, false),
            Some(p) if p.expect_key => (Fold::Ignore, false),
            Some(p) if p.merge_value && child_is_unanchored_seq => (Fold::Carrier, true),
            Some(p) if p.merge_value => (Fold::Merge, false),
            Some(_) => (Fold::Plain, false),
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

    fn fold_into(&mut self, kind: Fold, w: u64, s: u64, is_alias: bool) {
        let Some(p) = self.stack.last_mut() else {
            return;
        };
        match kind {
            Fold::Ignore => {}
            Fold::Plain | Fold::Carrier => {
                p.w = p.w.saturating_add(w);
                p.s = p.s.saturating_add(s);
            }
            Fold::Merge => {
                p.w = p.w.saturating_add(sub1(w));
                if !is_alias {
                    p.s = p.s.saturating_add(sub1(s));
                }
            }
        }
    }

    fn start(&mut self, aid: usize, is_map: bool) {
        let (fold, carrier) = self.classify(!is_map && aid == 0);
        let (w, s) = if carrier { (0, 0) } else { (1, 1) };
        self.stack.push(Frame {
            aid,
            is_map,
            w,
            s,
            carrier,
            fold,
            expect_key: true,
            merge_value: false,
        });
    }

    fn end(&mut self) -> Result<(), DocumentError> {
        let f = self
            .stack
            .pop()
            .expect("an end event matches a start event");
        if !f.carrier {
            if f.aid > 0 {
                self.memo.insert(f.aid, f.w);
            }
            // A saturated W (true count at or past 2^64) is over the limit
            // whatever the limit is: max*S saturates too, and equal saturated
            // values must not compare as "within".
            if f.w == u64::MAX || f.w > self.limit.saturating_mul(f.s) {
                return Err(rejection(
                    "a mapping's or sequence's alias expansion factor exceeds the \
                     configured maximum",
                ));
            }
        }
        self.fold_into(f.fold, f.w, f.s, false);
        self.advance_parent();
        Ok(())
    }

    /// Consumes one parser event.
    pub(super) fn event(&mut self, ev: &Event) -> Result<(), DocumentError> {
        match ev {
            Event::SequenceStart(aid, _) => self.start(*aid, false),
            Event::MappingStart(aid, _) => self.start(*aid, true),
            Event::SequenceEnd | Event::MappingEnd => return self.end(),
            Event::Scalar(text, style, aid, tag) => {
                let (fold, _) = self.classify(false);
                if fold == Fold::Ignore && is_merge_key(text, *style, tag.is_some()) {
                    // The `<<` entry is one written slot, whatever it holds.
                    let p = self.stack.last_mut().expect("a key has a parent mapping");
                    p.s = p.s.saturating_add(1);
                    p.merge_value = true;
                }
                if *aid > 0 {
                    self.memo.insert(*aid, 1);
                }
                self.fold_into(fold, 1, 1, false);
                self.advance_parent();
            }
            Event::Alias(id) => {
                let Some(target_w) = self.memo.get(id).copied() else {
                    return Err(rejection(
                        "an anchor refers to itself, directly or through other anchors \
                         (unbounded expansion)",
                    ));
                };
                let (fold, _) = self.classify(false);
                self.fold_into(fold, target_w, 1, true);
                self.advance_parent();
            }
            Event::Nothing
            | Event::StreamStart
            | Event::StreamEnd
            | Event::DocumentStart
            | Event::DocumentEnd => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(check: &mut AliasCheck, events: &[Event]) -> Result<(), DocumentError> {
        events.iter().try_for_each(|e| check.event(e))
    }

    fn scalar(text: &str) -> Event {
        Event::Scalar(text.to_string(), TScalarStyle::Plain, 0, None)
    }

    /// A `W` past `u64::MAX` saturates and is rejected whatever the limit is,
    /// even the largest one; wrapping would compare under it (D-19).
    #[test]
    fn saturated_w_is_rejected_even_at_the_largest_limit() {
        let mut check = AliasCheck::new(u64::MAX);
        // Two completed definitions whose W is already close to the top.
        check.memo.insert(1, u64::MAX - 1);
        check.memo.insert(2, u64::MAX - 1);
        let events = [
            Event::MappingStart(0, None),
            scalar("x"),
            Event::Alias(1),
            scalar("y"),
            Event::Alias(2),
        ];
        feed(&mut check, &events).unwrap();
        let err = check.event(&Event::MappingEnd).unwrap_err();
        assert_eq!(err.code.as_deref(), Some(CODE));
        assert_eq!(err.path, "$");
    }

    /// An exact W equal to the maximum times S is within; one past is over.
    #[test]
    fn equal_is_within_and_one_past_is_over() {
        for (limit, ok) in [(3u64, true), (2, false)] {
            let mut check = AliasCheck::new(limit);
            check.memo.insert(1, 4);
            // {a: *1}: W = 1 + 4, S = 2, E = 2.5: within 3, over 2.
            let events = [Event::MappingStart(0, None), scalar("a"), Event::Alias(1)];
            feed(&mut check, &events).unwrap();
            assert_eq!(check.event(&Event::MappingEnd).is_ok(), ok);
        }
    }

    #[test]
    fn an_alias_to_an_unfinished_or_unknown_anchor_is_a_cycle() {
        let mut check = AliasCheck::new(50);
        let err = check.event(&Event::Alias(7)).unwrap_err();
        assert_eq!(err.code.as_deref(), Some(CODE));
    }

    #[test]
    fn depth_tracks_open_containers() {
        let mut check = AliasCheck::new(50);
        assert_eq!(check.depth(), 0);
        check.event(&Event::SequenceStart(0, None)).unwrap();
        check.event(&Event::MappingStart(0, None)).unwrap();
        assert_eq!(check.depth(), 2);
        check.event(&Event::MappingEnd).unwrap();
        check.event(&Event::SequenceEnd).unwrap();
        assert_eq!(check.depth(), 0);
    }
}
