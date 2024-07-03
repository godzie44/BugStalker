use itertools::Itertools;
use std::collections::HashMap;
use std::fmt::{Debug, Display, Formatter};

/// Literal object.
/// Using it for a searching element by key in key-value containers.
#[derive(PartialEq, Clone)]
pub enum Literal {
    String(String),
    Int(i64),
    Float(f64),
    Address(usize),
    Bool(bool),
    EnumVariant(String, Option<Box<Literal>>),
    Array(Box<[LiteralOrWildcard]>),
    AssocArray(HashMap<String, LiteralOrWildcard>),
}

impl Display for Literal {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Literal::String(str) => f.write_fmt(format_args!("\"{str}\"")),
            Literal::Int(i) => f.write_str(&i.to_string()),
            Literal::Float(float) => f.write_str(&float.to_string()),
            Literal::Address(addr) => f.write_fmt(format_args!("{addr:#016X}")),
            Literal::Bool(b) => f.write_fmt(format_args!("{b}")),
            Literal::EnumVariant(variant, data) => {
                if let Some(data) = data {
                    f.write_fmt(format_args!("{variant}({data})"))
                } else {
                    f.write_fmt(format_args!("{variant}"))
                }
            }
            Literal::Array(array) => {
                let body = array
                    .iter()
                    .map(|item| match item {
                        LiteralOrWildcard::Literal(lit) => lit.to_string(),
                        LiteralOrWildcard::Wildcard => "*".to_string(),
                    })
                    .join(", ");
                f.write_fmt(format_args!("{{ {body} }}"))
            }
            Literal::AssocArray(assoc_array) => {
                let body = assoc_array
                    .iter()
                    .map(|(key, value)| {
                        let value_string = match value {
                            LiteralOrWildcard::Literal(lit) => lit.to_string(),
                            LiteralOrWildcard::Wildcard => "*".to_string(),
                        };
                        format!("\"{key}\": {value_string}")
                    })
                    .join(", ");

                f.write_fmt(format_args!("{{ {body} }}"))
            }
        }
    }
}

impl Debug for Literal {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_string())
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum LiteralOrWildcard {
    Literal(Literal),
    Wildcard,
}

macro_rules! impl_equal {
    ($lhs: expr, $rhs: expr, $lit: path) => {
        if let $lit(lhs) = $lhs {
            lhs == &$rhs
        } else {
            false
        }
    };
}

impl Literal {
    pub fn equal_with_string(&self, rhs: &str) -> bool {
        impl_equal!(self, rhs, Literal::String)
    }

    pub fn equal_with_address(&self, rhs: usize) -> bool {
        impl_equal!(self, rhs, Literal::Address)
    }

    pub fn equal_with_bool(&self, rhs: bool) -> bool {
        impl_equal!(self, rhs, Literal::Bool)
    }

    pub fn equal_with_int(&self, rhs: i64) -> bool {
        impl_equal!(self, rhs, Literal::Int)
    }

    pub fn equal_with_float(&self, rhs: f64) -> bool {
        const EPS: f64 = 0.0000001f64;
        if let Literal::Float(float) = self {
            let diff = (*float - rhs).abs();
            diff < EPS
        } else {
            false
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum Selector {
    Name { var_name: String, local_only: bool },
    Any,
}

impl Selector {
    pub fn by_name(name: impl ToString, local_only: bool) -> Self {
        Self::Name {
            var_name: name.to_string(),
            local_only,
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct PointerCast {
    pub ptr: usize,
    pub ty: String,
}

impl PointerCast {
    pub fn new(ptr: usize, ty: impl ToString) -> Self {
        Self {
            ptr,
            ty: ty.to_string(),
        }
    }
}

/// Data query expression.
/// List of operations for select variables and their properties.
///
/// Expression can be parsed from an input string like `*(*variable1.field2)[1]`
/// (see [`crate::ui::command`] module)
///
/// Supported operations are: dereference, get an element by index, get field by name, make slice from a pointer.
#[derive(Debug, PartialEq, Clone)]
pub enum Dqe {
    /// Select variables or arguments from debugee state.
    Variable(Selector),
    /// Cast raw memory address to a typed pointer.
    PtrCast(PointerCast),
    /// Get structure field (or similar, for example, values from hashmap with string keys).
    Field(Box<Dqe>, String),
    /// Get an element from array (or vector, vecdeq, etc.) by its index.
    Index(Box<Dqe>, Literal),
    /// Get array (or vector, vecdeq, etc.) slice.
    Slice(Box<Dqe>, Option<usize>, Option<usize>),
    /// Dereference pointer value.
    Deref(Box<Dqe>),
    /// Get address of value.
    Address(Box<Dqe>),
    /// Get canonic value (actual for specialized value, typically return underlying structure).
    Canonic(Box<Dqe>),
}

impl Dqe {
    /// Return boxed expression.
    pub fn boxed(self) -> Box<Self> {
        Box::new(self)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_literal_display() {
        struct TestCase {
            literal: Literal,
            expect: &'static str,
        }
        let test_cases = &[
            TestCase {
                literal: Literal::String("abc".to_string()),
                expect: "\"abc\"",
            },
            TestCase {
                literal: Literal::Array(Box::new([
                    LiteralOrWildcard::Literal(Literal::Int(1)),
                    LiteralOrWildcard::Literal(Literal::Int(1)),
                    LiteralOrWildcard::Wildcard,
                ])),
                expect: "{ 1, 1, * }",
            },
            TestCase {
                literal: Literal::Address(101),
                expect: "0x00000000000065",
            },
            TestCase {
                literal: Literal::EnumVariant(
                    "Some".to_string(),
                    Some(Box::new(Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::Bool(true)),
                    ])))),
                ),
                expect: "Some({ true })",
            },
            TestCase {
                literal: Literal::AssocArray(HashMap::from([(
                    "__1".to_string(),
                    LiteralOrWildcard::Literal(Literal::String("abc".to_string())),
                )])),
                expect: "{ \"__1\": \"abc\" }",
            },
        ];

        for tc in test_cases {
            assert_eq!(tc.literal.to_string(), tc.expect);
        }
    }
}
