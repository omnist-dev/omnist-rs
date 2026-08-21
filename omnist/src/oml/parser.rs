//! OML parser: recursive-descent consumer of [`super::scanner::Scanner`]
//! tokens, producing a [`RawNode`].
//!
//! See the parent module's doc comment for the overall architecture
//! rationale (issue #10).

use super::scanner::{Scanner, TemporalKind, TokKind};
use crate::document::{self, RawNode, Scalar};
use crate::error::ParseError;

pub(super) const RESERVED: [&str; 3] = ["null", "true", "false"];

pub(super) struct Parser<'a> {
    sc: Scanner<'a>,
    kind: TokKind,
    start: usize,
    end: usize,
    node_count: usize,
}

impl<'a> Parser<'a> {
    pub(super) fn new(mut sc: Scanner<'a>) -> Result<Self, ParseError> {
        let (kind, start, end) = sc.next()?;
        Ok(Parser {
            sc,
            kind,
            start,
            end,
            node_count: 0,
        })
    }

    fn charge_node(&mut self, pos: usize) -> Result<(), ParseError> {
        self.node_count += 1;
        if self.node_count > document::MAX_NODES {
            return Err(self.sc.error_at(
                pos,
                format!(
                    "document exceeds the maximum node count ({})",
                    document::MAX_NODES
                ),
            ));
        }
        Ok(())
    }

    fn advance(&mut self) -> Result<(TokKind, usize, usize), ParseError> {
        let (nk, ns, ne) = self.sc.next()?;
        let cur = (std::mem::replace(&mut self.kind, nk), self.start, self.end);
        self.start = ns;
        self.end = ne;
        Ok(cur)
    }

    fn skip_sep(&mut self) -> Result<(), ParseError> {
        while matches!(self.kind, TokKind::Sep) {
            self.advance()?;
        }
        Ok(())
    }

    fn tok_display(kind: &TokKind, text: &str) -> String {
        match kind {
            TokKind::Str(s) => format!("{s:?}"),
            _ => format!("{text:?}"),
        }
    }

    pub(super) fn parse_document(&mut self) -> Result<RawNode, ParseError> {
        self.skip_sep()?;
        let node = if matches!(self.kind, TokKind::Eof) {
            self.charge_node(self.start)?;
            RawNode::Edges(vec![])
        } else if matches!(self.kind, TokKind::LBrace) {
            self.parse_brace_value(0)?
        } else if self.looks_like_edge() {
            self.charge_node(self.start)?;
            RawNode::Edges(self.parse_node_edges(0)?)
        } else {
            self.parse_scalar()?
        };
        self.skip_sep()?;
        if !matches!(self.kind, TokKind::Eof) {
            let text: String = self.sc.text[self.start..self.end].to_string();
            return Err(self.sc.error_at(
                self.start,
                format!(
                    "unexpected trailing content after the document body (token {})",
                    Self::tok_display(&self.kind, &text)
                ),
            ));
        }
        Ok(node)
    }

    fn looks_like_edge(&mut self) -> bool {
        match &self.kind {
            TokKind::Str(_) => self.peek_is_colon(),
            TokKind::Ident(text) => {
                if RESERVED.contains(&text.as_str()) {
                    false
                } else {
                    self.peek_is_colon()
                }
            }
            _ => false,
        }
    }

    fn peek_is_colon(&mut self) -> bool {
        let saved_pos = self.sc.pos;
        let result = self.sc.next();
        self.sc.pos = saved_pos;
        matches!(result, Ok((TokKind::Colon, _, _)))
    }

    fn parse_node_edges(&mut self, depth: usize) -> Result<Vec<(String, RawNode)>, ParseError> {
        let mut edges = Vec::new();
        self.skip_sep()?;
        while !matches!(self.kind, TokKind::RBrace | TokKind::Eof) {
            let label = self.parse_label()?;
            let (colon_kind, colon_start, colon_end) = self.advance()?;
            if !matches!(colon_kind, TokKind::Colon) {
                let text: String = self.sc.text[colon_start..colon_end].to_string();
                return Err(self.sc.error_at(
                    colon_start,
                    format!(
                        "expected ':' after label {label:?}, got {}",
                        Self::tok_display(&colon_kind, &text)
                    ),
                ));
            }
            let child_depth = depth + 1;
            if matches!(self.kind, TokKind::LBracket) {
                for element in self.parse_array(child_depth)? {
                    edges.push((label.clone(), element));
                }
            } else {
                edges.push((label, self.parse_value(child_depth)?));
            }
            if matches!(self.kind, TokKind::RBrace | TokKind::Eof) {
                break;
            }
            if !matches!(self.kind, TokKind::Sep) {
                let text: String = self.sc.text[self.start..self.end].to_string();
                return Err(self.sc.error_at(
                    self.start,
                    format!(
                        "expected a separator (newline or ';') or '}}', got {}",
                        Self::tok_display(&self.kind, &text)
                    ),
                ));
            }
            self.skip_sep()?;
        }
        Ok(edges)
    }

    fn parse_label(&mut self) -> Result<String, ParseError> {
        let (kind, start, end) = self.advance()?;
        match kind {
            TokKind::Str(s) => Ok(s),
            TokKind::Ident(text) => {
                if RESERVED.contains(&text.as_str()) {
                    Err(self.sc.error_at(
                        start,
                        format!(
                            "{text:?} is a reserved word and cannot be a bare label; quote it: \
                             \"{text}\""
                        ),
                    ))
                } else {
                    Ok(text)
                }
            }
            other => {
                let text: String = self.sc.text[start..end].to_string();
                Err(self.sc.error_at(
                    start,
                    format!("expected a label, got {}", Self::tok_display(&other, &text)),
                ))
            }
        }
    }

    fn check_depth(&self, depth: usize) -> Result<(), ParseError> {
        if depth > document::MAX_DEPTH {
            return Err(self.sc.error_at(
                self.start,
                format!(
                    "nesting exceeds the maximum depth ({})",
                    document::MAX_DEPTH
                ),
            ));
        }
        Ok(())
    }

    fn parse_value(&mut self, depth: usize) -> Result<RawNode, ParseError> {
        self.check_depth(depth)?;
        if matches!(self.kind, TokKind::LBrace) {
            self.parse_brace_value(depth)
        } else {
            self.parse_scalar()
        }
    }

    fn parse_brace_value(&mut self, depth: usize) -> Result<RawNode, ParseError> {
        self.check_depth(depth)?;
        self.charge_node(self.start)?;
        self.advance()?; // consume '{'
        self.skip_sep()?;
        let edges = self.parse_node_edges(depth)?;
        self.skip_sep()?;
        let (close_kind, close_start, close_end) = self.advance()?;
        if !matches!(close_kind, TokKind::RBrace) {
            let text: String = self.sc.text[close_start..close_end].to_string();
            return Err(self.sc.error_at(
                close_start,
                format!(
                    "expected '}}', got {}",
                    Self::tok_display(&close_kind, &text)
                ),
            ));
        }
        Ok(RawNode::Edges(edges))
    }

    fn parse_array(&mut self, depth: usize) -> Result<Vec<RawNode>, ParseError> {
        let open_start = self.start;
        self.advance()?; // consume '['
        self.skip_sep()?;
        if matches!(self.kind, TokKind::RBracket) {
            return Err(self
                .sc
                .error_at(open_start, "empty array is not allowed".to_string()));
        }
        let mut elements = Vec::new();
        loop {
            if matches!(self.kind, TokKind::LBracket) {
                return Err(self.sc.error_at(
                    self.start,
                    "nested array is not allowed (arrays may only contain scalars, null, or \
                     brace subtrees)"
                        .to_string(),
                ));
            }
            elements.push(self.parse_value(depth)?);
            self.skip_sep()?;
            if matches!(self.kind, TokKind::Comma) {
                self.advance()?;
                self.skip_sep()?;
                if matches!(self.kind, TokKind::RBracket) {
                    break;
                }
                continue;
            }
            break;
        }
        let (close_kind, close_start, close_end) = self.advance()?;
        if !matches!(close_kind, TokKind::RBracket) {
            let text: String = self.sc.text[close_start..close_end].to_string();
            return Err(self.sc.error_at(
                close_start,
                format!(
                    "expected ',' or ']' in array, got {}",
                    Self::tok_display(&close_kind, &text)
                ),
            ));
        }
        Ok(elements)
    }

    /// Returns a [`RawNode`] leaf, not a bare [`Scalar`] -- `TokKind::
    /// Temporal` (a genuinely bare, grammar-validated date/time/datetime
    /// literal) becomes the matching [`Scalar::Date`]/[`Scalar::Time`]/
    /// [`Scalar::Datetime`] variant (issue #105), distinct from
    /// `TokKind::Str` (an ordinary quoted string that merely holds
    /// date/time/datetime-*shaped* text), which stays a plain
    /// `Scalar::Str`.
    fn parse_scalar(&mut self) -> Result<RawNode, ParseError> {
        self.charge_node(self.start)?;
        let (kind, start, end) = self.advance()?;
        match kind {
            TokKind::Str(s) => Ok(RawNode::Leaf(Scalar::Str(s))),
            TokKind::Temporal(TemporalKind::Date, s) => Ok(RawNode::Leaf(Scalar::Date(s))),
            TokKind::Temporal(TemporalKind::Time, s) => Ok(RawNode::Leaf(Scalar::Time(s))),
            TokKind::Temporal(TemporalKind::Datetime, s) => Ok(RawNode::Leaf(Scalar::Datetime(s))),
            TokKind::Int(i) => Ok(RawNode::Leaf(Scalar::Int(i))),
            TokKind::Float(f) => Ok(RawNode::Leaf(Scalar::Float(f))),
            TokKind::Ident(text) => match text.as_str() {
                "null" => Ok(RawNode::Leaf(Scalar::Null)),
                "true" => Ok(RawNode::Leaf(Scalar::Bool(true))),
                "false" => Ok(RawNode::Leaf(Scalar::Bool(false))),
                _ => Err(self.sc.error_at(
                    start,
                    format!("bare word {text:?} is not a valid value here; strings must be quoted"),
                )),
            },
            other => {
                let text: String = self.sc.text[start..end].to_string();
                Err(self.sc.error_at(
                    start,
                    format!("expected a value, got {}", Self::tok_display(&other, &text)),
                ))
            }
        }
    }
}
