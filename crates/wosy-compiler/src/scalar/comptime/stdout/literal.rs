//! Canonical producer for already ordinary-checked closed literal values.
use super::Shape;
use crate::{ScalarExpression, UnaryOperator};
use std::{collections::HashMap, io::{self, Write}};

#[derive(Debug)]
pub(super) enum LiteralStdout {
    Bool(bool),
    I32(i32),
    U32(u32),
    Utf8(Vec<u8>),
    Collection(Vec<LiteralStdout>),
}

impl LiteralStdout {
    /// A producer for literals, never a second evaluator. The ordinary
    /// compiler already checked the exact expected type and all fields.
    pub(super) fn from_checked(shape: &Shape, expr: &ScalarExpression) -> Option<Self> {
        use ScalarExpression as Expr;
        match (shape, expr) {
            (Shape::Bool, Expr::Boolean { value, .. }) => Some(Self::Bool(*value)),
            (Shape::I32, Expr::Integer { value, .. }) => i32::try_from(value).ok().map(Self::I32),
            (Shape::I32, Expr::Unary { operator: UnaryOperator::Negate, operand, .. }) => {
                // A negative integer spelling, not arithmetic evaluation.
                let Expr::Integer { value, .. } = operand.as_ref() else { return None; };
                i32::try_from(-value).ok().map(Self::I32)
            }
            (Shape::U32, Expr::Integer { value, .. }) => u32::try_from(value).ok().map(Self::U32),
            (Shape::Utf8, Expr::Utf8 { value, .. }) => Some(Self::Utf8(value.clone())),
            (Shape::Array { element, .. }, Expr::ArrayLiteral { elements, .. }) => elements.iter()
                .map(|value| Self::from_checked(element, value)).collect::<Option<_>>().map(Self::Collection),
            (Shape::Struct(expected), Expr::StructLiteral { fields, .. }) => {
                // The source may designate fields in any order. The output
                // always uses the already resolved declaration order.
                let values: HashMap<_, _> = fields.iter().map(|field| (field.name.as_str(), &field.value)).collect();
                expected.iter().map(|(name, ty)| Self::from_checked(ty, values.get(name.as_str())?))
                    .collect::<Option<_>>().map(Self::Collection)
            }
            _ => None,
        }
    }

    pub(super) fn write_line(&self, out: &mut impl Write, limit: usize) -> io::Result<usize> {
        let mut writer = BudgetedWriter { out, remaining: limit };
        self.write_value(&mut writer)?;
        writer.write_all(b"\n")?;
        Ok(limit - writer.remaining)
    }

    fn write_value(&self, out: &mut impl Write) -> io::Result<()> {
        match self {
            Self::Bool(true) => out.write_all(b"true"),
            Self::Bool(false) => out.write_all(b"false"),
            Self::I32(value) => write!(out, "{value}"),
            Self::U32(value) => write!(out, "{value}"),
            Self::Utf8(text) => write_text(text, out),
            Self::Collection(values) => {
                out.write_all(b"[")?;
                for (index, value) in values.iter().enumerate() {
                    if index != 0 { out.write_all(b",")?; }
                    value.write_value(out)?;
                }
                out.write_all(b"]")
            }
        }
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
