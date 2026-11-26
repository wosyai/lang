//! One bounded expansion of content-addressed CST records in authored order.
use super::{Kind, Node, NodeId};
use wosy_syntax::SyntaxKind;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, Error>;

pub fn syntax_kind(node: &Node) -> Result<SyntaxKind> {
    let raw = node.data.first_chunk::<2>().ok_or("missing syntax kind")?;
    SyntaxKind::try_from_raw(u16::from_be_bytes(*raw)).ok_or_else(|| "unknown syntax kind".into())
}

pub fn token_text(node: &Node) -> Result<&str> {
    Ok(std::str::from_utf8(node.data.get(2..).ok_or("missing token kind")?)?)
}

pub enum CstEvent<'a> {
    Start { kind: SyntaxKind, depth: usize, children: usize },
    Token { kind: SyntaxKind, text: &'a str },
    Finish,
}

/// The caller verifies and charges loaded bytes. The visitor can refuse an
/// expanded node before its children are scheduled. Ordered repetitions of
/// the same interned node are not removed.
pub fn walk_cst<S>(
    root: NodeId, state: &mut S,
    mut read: impl FnMut(&mut S, NodeId) -> Result<Node>,
    mut visit: impl FnMut(&mut S, CstEvent<'_>) -> Result<()>,
) -> Result<()> {
    let mut stack = vec![Some((root, 1))];
    while let Some(id) = stack.pop() {
        let Some((id, depth)) = id else {
            visit(state, CstEvent::Finish)?;
            continue;
        };
        let node = read(state, id)?;
        let kind = syntax_kind(&node)?;
        match node.kind {
            Kind::Syntax => {
                visit(state, CstEvent::Start { kind, depth, children: node.children.len() })?;
                stack.push(None);
                stack.extend(node.children.into_iter().rev().map(|id| Some((id, depth + 1))));
            }
            Kind::Token if depth > 1 => visit(state, CstEvent::Token { kind, text: token_text(&node)? })?,
            _ => return Err("invalid CST child".into()),
        }
    }
    Ok(())
}

pub struct CstExpansionBudget {
    expanded_record_bytes: usize,
    occurrences: usize,
    max_depth: usize,
    authored_bytes: usize,
}

impl CstExpansionBudget {
    pub fn new(expanded_record_bytes: usize, occurrences: usize, max_depth: usize, authored_bytes: usize) -> Self {
        Self { expanded_record_bytes, occurrences, max_depth, authored_bytes }
    }

    pub fn read(&mut self, bytes: usize) -> Result<()> {
        take(&mut self.expanded_record_bytes, bytes, "base CST validation exceeds the expanded record read budget")
    }

    pub fn file(&mut self, bytes: usize) -> Result<()> {
        take(&mut self.authored_bytes, bytes, "base CST validation exceeds the authored byte budget")?;
        self.start_tree()
    }

    pub fn start_tree(&mut self) -> Result<()> {
        take(&mut self.occurrences, 1, "base CST validation exceeds the occurrence budget")
    }

    pub fn children(&mut self, depth: usize, count: usize) -> Result<()> {
        if depth > self.max_depth || (depth == self.max_depth && count > 0) {
            return Err("base CST validation exceeds the syntax depth budget".into());
        }
        take(&mut self.occurrences, count, "base CST validation exceeds the occurrence budget")
    }
}

fn take(remaining: &mut usize, amount: usize, reason: &'static str) -> Result<()> {
    *remaining = remaining.checked_sub(amount).ok_or(reason)?;
    Ok(())
}
