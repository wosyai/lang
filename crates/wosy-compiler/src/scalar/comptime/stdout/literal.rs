//! Canonical producer for already ordinary-checked closed literal values.
use super::{Shape, ShapePlan};
use crate::{ScalarExpression, UnaryOperator};
use std::{collections::HashMap, io::{self, Write}};

#[derive(Debug)]
pub(super) struct LiteralStdout(Vec<LiteralItem>);

#[derive(Debug)]
enum LiteralItem {
    Bool(bool),
    I32(i32),
    U32(u32),
    Utf8(Vec<u8>),
    Delimiter(u8),
}

impl LiteralStdout {
    /// A producer for literals, never a second evaluator. The ordinary
    /// compiler already checked the exact expected type and all fields.
    pub(super) fn from_checked(plan: &ShapePlan, index: usize, expr: &ScalarExpression) -> Option<Self> {
        use ScalarExpression as Expr;
        enum Task<'a> { Value(usize, &'a Expr), Delimiter(u8) }
        let mut tasks = vec![Task::Value(index, expr)];
        let mut output = Vec::new();
        while let Some(task) = tasks.pop() {
            let (index, expr) = match task {
                Task::Delimiter(byte) => { output.push(LiteralItem::Delimiter(byte)); continue; }
                Task::Value(index, expr) => (index, expr),
            };
            let item = match (&plan.nodes[index], expr) {
                (Shape::Bool, Expr::Boolean { value, .. }) => LiteralItem::Bool(*value),
                (Shape::I32, Expr::Integer { value, .. }) => LiteralItem::I32(i32::try_from(value).ok()?),
                (Shape::I32, Expr::Unary { operator: UnaryOperator::Negate, operand, .. }) => {
                    let Expr::Integer { value, .. } = operand.as_ref() else { return None; };
                    LiteralItem::I32(i32::try_from(-value).ok()?)
                }
                (Shape::U32, Expr::Integer { value, .. }) => LiteralItem::U32(u32::try_from(value).ok()?),
                (Shape::Utf8, Expr::Utf8 { value, .. }) => LiteralItem::Utf8(value.clone()),
                (Shape::Array { element, .. }, Expr::ArrayLiteral { elements, .. }) => {
                    tasks.push(Task::Delimiter(b']'));
                    for (index, value) in elements.iter().enumerate().rev() {
                        tasks.push(Task::Value(*element, value));
                        if index != 0 { tasks.push(Task::Delimiter(b',')); }
                    }
                    LiteralItem::Delimiter(b'[')
                }
                (Shape::Struct(expected), Expr::StructLiteral { fields, .. }) => {
                    // Designators can be written in any order.
                    let values: HashMap<_, _> = fields.iter().map(|field| (field.name.as_str(), &field.value)).collect();
                    tasks.push(Task::Delimiter(b']'));
                    for (index, (name, ty)) in expected.iter().enumerate().rev() {
                        tasks.push(Task::Value(*ty, values.get(name.as_str())?));
                        if index != 0 { tasks.push(Task::Delimiter(b',')); }
                    }
                    LiteralItem::Delimiter(b'[')
                }
                _ => return None,
            };
            output.push(item);
        }
        Some(Self(output))
    }

    pub(super) fn write_line(&self, out: &mut impl Write, limit: usize) -> io::Result<usize> {
        let mut writer = BudgetedWriter { out, remaining: limit };
        for item in &self.0 {
            match item {
                LiteralItem::Bool(true) => writer.write_all(b"true")?,
                LiteralItem::Bool(false) => writer.write_all(b"false")?,
                LiteralItem::I32(value) => write!(writer, "{value}")?,
                LiteralItem::U32(value) => write!(writer, "{value}")?,
                LiteralItem::Utf8(text) => write_text(text, &mut writer)?,
                LiteralItem::Delimiter(byte) => writer.write_all(&[*byte])?,
            }
        }
        writer.write_all(b"\n")?;
        Ok(limit - writer.remaining)
    }
}

fn write_text(bytes: &[u8], out: &mut impl Write) -> io::Result<()> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.write_all(b"\"")?;
    let mut start = 0;
    for (index, &byte) in bytes.iter().enumerate() {
        let mut control;
        let escape: &[u8] = match byte {
            b'"' => b"\\\"",
            b'\\' => b"\\\\",
            b'\n' => b"\\n",
            b'\r' => b"\\r",
            b'\t' => b"\\t",
            0..=31 => {
                control = *b"\\u0000";
                control[4] = HEX[(byte >> 4) as usize];
                control[5] = HEX[(byte & 15) as usize];
                &control
            }
            _ => continue,
        };
        out.write_all(&bytes[start..index])?;
        out.write_all(escape)?;
        start = index + 1;
    }
    out.write_all(&bytes[start..])?;
    out.write_all(b"\"")
}

struct BudgetedWriter<'a, W> {
    out: &'a mut W,
    remaining: usize,
}

impl<W: Write> Write for BudgetedWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.remaining {
            return Err(io::Error::other("comptime stdout exceeds its byte budget"));
        }
        let count = self.out.write(bytes)?;
        self.remaining -= count;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> { self.out.flush() }
}
