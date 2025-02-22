//! data query expressions parser.
use crate::debugger::variable::select::{DQE, Literal, LiteralOrWildcard, VariableSelector};
use crate::ui::command::parser::{hex, rust_identifier};
use chumsky::Parser;
use chumsky::prelude::*;
use std::collections::HashMap;

type Err<'a> = extra::Err<Rich<'a, char>>;

fn ptr_cast<'a>() -> impl Parser<'a, &'a str, DQE, Err<'a>> + Clone {
    let op = |c| just(c).padded();

    // try to interp any string between brackets as a type
    let any = any::<_, Err>()
        .filter(|c| {
            // this is a filter rule for a type identifier
            // may be it is good enough,
            // if it's not - something like `syn::parse_str` may be used
            char::is_ascii_alphanumeric(c)
                || *c == ':'
                || *c == '<'
                || *c == '>'
                || *c == ' '
                || *c == '*'
                || *c == '&'
                || *c == '_'
                || *c == ','
                || *c == '\''
        })
        .repeated()
        .at_least(1)
        .to_slice();
    let type_p = any.delimited_by(op('('), op(')'));
    type_p
        .then(hex())
        .map(|(r#type, ptr)| DQE::PtrCast(ptr, r#type.trim().to_string()))
        .labelled("pointer cast")
}

fn literal<'a>() -> impl Parser<'a, &'a str, Literal, Err<'a>> + Clone {
    let op = |c| just(c).padded();

    let literal = recursive(|literal| {
        let int = just("-")
            .or_not()
            .then(text::int(10).from_str::<u64>().unwrapped())
            .map(|(sign, val)| {
                Literal::Int(if sign.is_some() {
                    -(val as i64)
                } else {
                    val as i64
                })
            });

        let float = just("-")
            .or_not()
            .then(text::int(10).then_ignore(just(".")).then(text::int(10)))
            .map(|(sign, (i, f))| {
                let sign = sign.unwrap_or_default();
                Literal::Float(format!("{sign}{i}.{f}").parse::<f64>().expect("infallible"))
            });

        fn make_string<'a, 's: 'a>(
            q: &'a str,
        ) -> impl Parser<'a, &'a str, Literal, Err<'a>> + Clone {
            one_of::<_, _, Err<'a>>(q)
                .ignore_then(none_of(q).repeated().collect::<String>())
                .then_ignore(one_of(q))
                .map(Literal::String)
        }
        let string1 = make_string("\"");
        let string2 = make_string("'");

        let bool = op("true")
            .to(Literal::Bool(true))
            .or(op("false").to(Literal::Bool(false)));

        let enum_variant = rust_identifier()
            .then(literal.clone().delimited_by(op("("), op(")")).or_not())
            .map(|(ident, lit)| Literal::EnumVariant(ident.to_string(), lit.map(Box::new)));

        let wildcard = op("*");
        let literal_or_wildcard = literal
            .clone()
            .map(LiteralOrWildcard::Literal)
            .or(wildcard.to(LiteralOrWildcard::Wildcard));

        let array = op("{")
            .ignore_then(
                literal_or_wildcard
                    .clone()
                    .separated_by(op(","))
                    .collect::<Vec<_>>()
                    .map(|literals: Vec<LiteralOrWildcard>| {
                        Literal::Array(literals.into_boxed_slice())
                    }),
            )
            .then_ignore(op("}"));

        let kv = rust_identifier()
            .then_ignore(op(":"))
            .then(literal_or_wildcard)
            .map(|(k, v)| (k.to_string(), v));
        let assoc_array = op("{")
            .ignore_then(
                kv.separated_by(op(","))
                    .collect::<HashMap<_, _>>()
                    .map(Literal::AssocArray),
            )
            .then_ignore(op("}"));

        float
            .or(bool)
            .or(hex().map(Literal::Address))
            .or(int)
            .or(enum_variant)
            .or(string1)
            .or(string2)
            .or(array)
            .or(assoc_array)
    });

    literal
}

pub fn parser<'a>() -> impl Parser<'a, &'a str, DQE, Err<'a>> {
    let base_selector = rust_identifier()
        .padded()
        .map(|name: &str| {
            DQE::Variable(VariableSelector::Name {
                var_name: name.to_string(),
                only_local: false,
            })
        })
        .or(ptr_cast());

    let expr = recursive(|expr| {
        let op = |c| just(c).padded();

        let atom = base_selector
            .or(expr.delimited_by(op('('), op(')')))
            .padded();

        let field = text::ascii::ident()
            .or(text::int(10))
            .labelled("field name or tuple index");

        let field_op = op('.')
            .ignore_then(field)
            .map(|field: &str| -> Box<dyn FnOnce(DQE) -> DQE> {
                Box::new(move |r| DQE::Field(Box::new(r), field.to_string()))
            })
            .boxed();

        let index_op = literal()
            .padded()
            .labelled("index value")
            .delimited_by(op('['), op(']'))
            .map(|idx| -> Box<dyn FnOnce(DQE) -> DQE> {
                Box::new(move |r: DQE| DQE::Index(Box::new(r), idx))
            })
            .boxed();

        let mb_usize = text::int(10)
            .or_not()
            .padded()
            .map(|v: Option<&str>| v.map(|v| v.parse::<usize>().unwrap()));

        let slice_op = mb_usize
            .then_ignore(just("..").padded())
            .then(mb_usize)
            .labelled("slice range (start..end)")
            .delimited_by(op('['), op(']'))
            .map(|(from, to)| -> Box<dyn FnOnce(DQE) -> DQE> {
                Box::new(move |r: DQE| DQE::Slice(Box::new(r), from, to))
            })
            .boxed();

        let expr = atom.foldl(
            field_op.or(index_op).or(slice_op).repeated(),
            |r, expr_fn| expr_fn(r),
        );

        op('*')
            .to(DQE::Deref as fn(_) -> _)
            .or(op('&').to(DQE::Address as fn(_) -> _))
            .or(op('~').to(DQE::Canonic as fn(_) -> _))
            .repeated()
            .foldr(expr, |op, rhs| op(Box::new(rhs)))
    });

    expr.then_ignore(end())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_ptr_cast_parser() {
        struct TestCase {
            string: &'static str,
            result: Result<DQE, ()>,
        }
        let cases = vec![
            TestCase {
                string: "(*SomeStruct) 0x12345",
                result: Ok(DQE::PtrCast(0x12345, "*SomeStruct".to_string())),
            },
            TestCase {
                string: " ( &u32 )0x12345",
                result: Ok(DQE::PtrCast(0x12345, "&u32".to_string())),
            },
            TestCase {
                string: "(*const abc::def::SomeType)  0x123AABCD",
                result: Ok(DQE::PtrCast(
                    0x123AABCD,
                    "*const abc::def::SomeType".to_string(),
                )),
            },
            TestCase {
                string: " ( &u32 )12345",
                result: Err(()),
            },
            TestCase {
                string: "(*const i32)0x007FFFFFFFDC94",
                result: Ok(DQE::PtrCast(0x7FFFFFFFDC94, "*const i32".to_string())),
            },
        ];

        for tc in cases {
            let expr = ptr_cast().parse(tc.string).into_result();
            assert_eq!(expr.map_err(|_| ()), tc.result);
        }
    }

    #[test]
    fn test_literal_parser() {
        struct TestCase {
            string: &'static str,
            result: Literal,
        }
        let test_cases = vec![
            TestCase {
                string: "1",
                result: Literal::Int(1),
            },
            TestCase {
                string: "-1",
                result: Literal::Int(-1),
            },
            TestCase {
                string: "1.1",
                result: Literal::Float(1.1),
            },
            TestCase {
                string: "-1.0",
                result: Literal::Float(-1.0),
            },
            TestCase {
                string: "0x123ABC",
                result: Literal::Address(0x123ABC),
            },
            TestCase {
                string: "0X123ABC",
                result: Literal::Address(0x123ABC),
            },
            TestCase {
                string: "\"abc\"",
                result: Literal::String("abc".to_string()),
            },
            TestCase {
                string: "'\"abc\"'",
                result: Literal::String("\"abc\"".to_string()),
            },
            TestCase {
                string: "'\"ab\nc\"'",
                result: Literal::String("\"ab\nc\"".to_string()),
            },
            TestCase {
                string: "true",
                result: Literal::Bool(true),
            },
            TestCase {
                string: "false",
                result: Literal::Bool(false),
            },
            TestCase {
                string: "EnumVariantA",
                result: Literal::EnumVariant("EnumVariantA".to_string(), None),
            },
            TestCase {
                string: "Some(true)",
                result: Literal::EnumVariant(
                    "Some".to_string(),
                    Some(Box::new(Literal::Bool(true))),
                ),
            },
            TestCase {
                string: "EnumVariantA(EnumVariantB(1))",
                result: Literal::EnumVariant(
                    "EnumVariantA".to_string(),
                    Some(Box::new(Literal::EnumVariant(
                        "EnumVariantB".to_string(),
                        Some(Box::new(Literal::Int(1))),
                    ))),
                ),
            },
            TestCase {
                string: "{1, 2,*}",
                result: Literal::Array(Box::new([
                    LiteralOrWildcard::Literal(Literal::Int(1)),
                    LiteralOrWildcard::Literal(Literal::Int(2)),
                    LiteralOrWildcard::Wildcard,
                ])),
            },
            TestCase {
                string: "{{1,2}, \"str\", * , EnumVariantA}",
                result: Literal::Array(Box::new([
                    LiteralOrWildcard::Literal(Literal::Array(Box::new([
                        LiteralOrWildcard::Literal(Literal::Int(1)),
                        LiteralOrWildcard::Literal(Literal::Int(2)),
                    ]))),
                    LiteralOrWildcard::Literal(Literal::String("str".to_string())),
                    LiteralOrWildcard::Wildcard,
                    LiteralOrWildcard::Literal(Literal::EnumVariant(
                        "EnumVariantA".to_string(),
                        None,
                    )),
                ])),
            },
            TestCase {
                string: "{ field_1: \"val1\", field_2:*, field_3: 5}",
                result: Literal::AssocArray(HashMap::from([
                    (
                        "field_1".to_string(),
                        LiteralOrWildcard::Literal(Literal::String("val1".to_string())),
                    ),
                    ("field_2".to_string(), LiteralOrWildcard::Wildcard),
                    (
                        "field_3".to_string(),
                        LiteralOrWildcard::Literal(Literal::Int(5)),
                    ),
                ])),
            },
            TestCase {
                string: "{ field_1: {sub_field_1: 1}, field_2: {1, 2}, field_3: A({3, 4})}",
                result: Literal::AssocArray(HashMap::from([
                    (
                        "field_1".to_string(),
                        LiteralOrWildcard::Literal(Literal::AssocArray(HashMap::from([(
                            "sub_field_1".to_string(),
                            LiteralOrWildcard::Literal(Literal::Int(1)),
                        )]))),
                    ),
                    (
                        "field_2".to_string(),
                        LiteralOrWildcard::Literal(Literal::Array(Box::new([
                            LiteralOrWildcard::Literal(Literal::Int(1)),
                            LiteralOrWildcard::Literal(Literal::Int(2)),
                        ]))),
                    ),
                    (
                        "field_3".to_string(),
                        LiteralOrWildcard::Literal(Literal::EnumVariant(
                            "A".to_string(),
                            Some(Box::new(Literal::Array(Box::new([
                                LiteralOrWildcard::Literal(Literal::Int(3)),
                                LiteralOrWildcard::Literal(Literal::Int(4)),
                            ])))),
                        )),
                    ),
                ])),
            },
        ];

        for tc in test_cases {
            let literal = literal().parse(tc.string).into_result().unwrap();
            assert_eq!(literal, tc.result);
        }
    }

    #[test]
    fn test_expr_parsing() {
        struct TestCase {
            string: &'static str,
            expr: DQE,
        }
        let test_cases = vec![
            TestCase {
                string: "var1",
                expr: DQE::Variable(VariableSelector::Name {
                    var_name: "var1".to_string(),
                    only_local: false,
                }),
            },
            TestCase {
                string: "*var1",
                expr: DQE::Deref(Box::new(DQE::Variable(VariableSelector::Name {
                    var_name: "var1".to_string(),
                    only_local: false,
                }))),
            },
            TestCase {
                string: "~var1",
                expr: DQE::Canonic(Box::new(DQE::Variable(VariableSelector::Name {
                    var_name: "var1".to_string(),
                    only_local: false,
                }))),
            },
            TestCase {
                string: "**var1",
                expr: DQE::Deref(Box::new(DQE::Deref(Box::new(DQE::Variable(
                    VariableSelector::Name {
                        var_name: "var1".to_string(),
                        only_local: false,
                    },
                ))))),
            },
            TestCase {
                string: "~*var1",
                expr: DQE::Canonic(
                    DQE::Deref(
                        DQE::Variable(VariableSelector::Name {
                            var_name: "var1".to_string(),
                            only_local: false,
                        })
                        .boxed(),
                    )
                    .boxed(),
                ),
            },
            TestCase {
                string: "**var1.field1.field2",
                expr: DQE::Deref(Box::new(DQE::Deref(Box::new(DQE::Field(
                    Box::new(DQE::Field(
                        Box::new(DQE::Variable(VariableSelector::Name {
                            var_name: "var1".to_string(),
                            only_local: false,
                        })),
                        "field1".to_string(),
                    )),
                    "field2".to_string(),
                ))))),
            },
            TestCase {
                string: "**(var1.field1.field2)",
                expr: DQE::Deref(Box::new(DQE::Deref(Box::new(DQE::Field(
                    Box::new(DQE::Field(
                        Box::new(DQE::Variable(VariableSelector::Name {
                            var_name: "var1".to_string(),
                            only_local: false,
                        })),
                        "field1".to_string(),
                    )),
                    "field2".to_string(),
                ))))),
            },
            TestCase {
                string: "(**var1).field1.field2",
                expr: DQE::Field(
                    Box::new(DQE::Field(
                        Box::new(DQE::Deref(Box::new(DQE::Deref(Box::new(DQE::Variable(
                            VariableSelector::Name {
                                var_name: "var1".to_string(),
                                only_local: false,
                            },
                        )))))),
                        "field1".to_string(),
                    )),
                    "field2".to_string(),
                ),
            },
            TestCase {
                string: "*(*(var1.field1)).field2[1][2]",
                expr: DQE::Deref(Box::new(DQE::Index(
                    Box::new(DQE::Index(
                        Box::new(DQE::Field(
                            Box::new(DQE::Deref(Box::new(DQE::Field(
                                Box::new(DQE::Variable(VariableSelector::Name {
                                    var_name: "var1".to_string(),
                                    only_local: false,
                                })),
                                "field1".to_string(),
                            )))),
                            "field2".to_string(),
                        )),
                        Literal::Int(1),
                    )),
                    Literal::Int(2),
                ))),
            },
            TestCase {
                string: "var1.field1[5..]",
                expr: DQE::Slice(
                    Box::new(DQE::Field(
                        Box::new(DQE::Variable(VariableSelector::Name {
                            var_name: "var1".to_string(),
                            only_local: false,
                        })),
                        "field1".to_string(),
                    )),
                    Some(5),
                    None,
                ),
            },
            TestCase {
                string: "var1.field1[..5]",
                expr: DQE::Slice(
                    Box::new(DQE::Field(
                        Box::new(DQE::Variable(VariableSelector::Name {
                            var_name: "var1".to_string(),
                            only_local: false,
                        })),
                        "field1".to_string(),
                    )),
                    None,
                    Some(5),
                ),
            },
            TestCase {
                string: "var1.field1[5..5]",
                expr: DQE::Slice(
                    Box::new(DQE::Field(
                        Box::new(DQE::Variable(VariableSelector::Name {
                            var_name: "var1".to_string(),
                            only_local: false,
                        })),
                        "field1".to_string(),
                    )),
                    Some(5),
                    Some(5),
                ),
            },
            TestCase {
                string: "var1.field1[..]",
                expr: DQE::Slice(
                    Box::new(DQE::Field(
                        Box::new(DQE::Variable(VariableSelector::Name {
                            var_name: "var1".to_string(),
                            only_local: false,
                        })),
                        "field1".to_string(),
                    )),
                    None,
                    None,
                ),
            },
            TestCase {
                string: "enum1.0.a",
                expr: DQE::Field(
                    Box::new(DQE::Field(
                        Box::new(DQE::Variable(VariableSelector::Name {
                            var_name: "enum1".to_string(),
                            only_local: false,
                        })),
                        "0".to_string(),
                    )),
                    "a".to_string(),
                ),
            },
            TestCase {
                string: "(*mut SomeType)0x123AABCD",
                expr: DQE::PtrCast(0x123AABCD, "*mut SomeType".to_string()),
            },
            TestCase {
                string: "(&abc::def::SomeType)0x123AABCD",
                expr: DQE::PtrCast(0x123AABCD, "&abc::def::SomeType".to_string()),
            },
            TestCase {
                string: "(*const abc::def::SomeType)  0x123AABCD",
                expr: DQE::PtrCast(0x123AABCD, "*const abc::def::SomeType".to_string()),
            },
            TestCase {
                string: "*((*const abc::def::SomeType) 0x123AABCD)",
                expr: DQE::Deref(
                    DQE::PtrCast(0x123AABCD, "*const abc::def::SomeType".to_string()).boxed(),
                ),
            },
            TestCase {
                string: "*(*const i32)0x007FFFFFFFDC94",
                expr: DQE::Deref(DQE::PtrCast(0x7FFFFFFFDC94, "*const i32".to_string()).boxed()),
            },
            TestCase {
                string: "var.arr[0].some_val",
                expr: DQE::Field(
                    DQE::Index(
                        DQE::Field(
                            DQE::Variable(VariableSelector::Name {
                                var_name: "var".to_string(),
                                only_local: false,
                            })
                            .boxed(),
                            "arr".to_string(),
                        )
                        .boxed(),
                        Literal::Int(0),
                    )
                    .boxed(),
                    "some_val".to_string(),
                ),
            },
            TestCase {
                string: "arr[0][..][1..][0].some_val",
                expr: DQE::Field(
                    DQE::Index(
                        DQE::Slice(
                            DQE::Slice(
                                DQE::Index(
                                    DQE::Variable(VariableSelector::Name {
                                        var_name: "arr".to_string(),
                                        only_local: false,
                                    })
                                    .boxed(),
                                    Literal::Int(0),
                                )
                                .boxed(),
                                None,
                                None,
                            )
                            .boxed(),
                            Some(1),
                            None,
                        )
                        .boxed(),
                        Literal::Int(0),
                    )
                    .boxed(),
                    "some_val".to_string(),
                ),
            },
            TestCase {
                string: "map[\"key\"][-5][1.1][false][0x12]",
                expr: DQE::Index(
                    DQE::Index(
                        DQE::Index(
                            DQE::Index(
                                DQE::Index(
                                    DQE::Variable(VariableSelector::Name {
                                        var_name: "map".to_string(),
                                        only_local: false,
                                    })
                                    .boxed(),
                                    Literal::String("key".to_string()),
                                )
                                .boxed(),
                                Literal::Int(-5),
                            )
                            .boxed(),
                            Literal::Float(1.1),
                        )
                        .boxed(),
                        Literal::Bool(false),
                    )
                    .boxed(),
                    Literal::Address(0x12),
                ),
            },
            TestCase {
                string: "map[Some(true)]",
                expr: DQE::Index(
                    DQE::Variable(VariableSelector::Name {
                        var_name: "map".to_string(),
                        only_local: false,
                    })
                    .boxed(),
                    Literal::EnumVariant("Some".to_string(), Some(Box::new(Literal::Bool(true)))),
                ),
            },
            TestCase {
                string: "&a",
                expr: DQE::Address(
                    DQE::Variable(VariableSelector::Name {
                        var_name: "a".to_string(),
                        only_local: false,
                    })
                    .boxed(),
                ),
            },
            TestCase {
                string: "&*a.b",
                expr: DQE::Address(
                    DQE::Deref(
                        DQE::Field(
                            DQE::Variable(VariableSelector::Name {
                                var_name: "a".to_string(),
                                only_local: false,
                            })
                            .boxed(),
                            "b".to_string(),
                        )
                        .boxed(),
                    )
                    .boxed(),
                ),
            },
            TestCase {
                string: "&&(*i32)0x123",
                expr: DQE::Address(
                    DQE::Address(DQE::PtrCast(0x123, "*i32".to_string()).boxed()).boxed(),
                ),
            },
        ];

        for tc in test_cases {
            let expr = parser().parse(tc.string).into_result().unwrap();
            assert_eq!(expr, tc.expr, "case: {}", tc.string);
        }
    }

    #[test]
    fn test_expr_parsing_error() {
        struct TestCase {
            string: &'static str,
            err_text: &'static str,
        }
        let test_cases = vec![
            TestCase {
                string: "var1 var2",
                err_text: "found 'v' expected '.', '[', or end of input",
            },
            TestCase {
                string: "var1..",
                err_text: "found '.' expected field name or tuple index",
            },
            TestCase {
                string: "var1[]",
                err_text: "found ']' expected index value, or slice range (start..end)",
            },
            TestCase {
                string: "(var1.)field1",
                err_text: "found ')' expected field name or tuple index",
            },
            TestCase {
                string: "((var1)",
                err_text: "found end of input expected '0', '.', '[', or ')'",
            },
            TestCase {
                string: "(var1))",
                err_text: "found end of input expected '0', '.', '[', or end of input",
            },
            TestCase {
                string: "*",
                err_text: "found end of input expected '*', '&', '~', ':', or '('",
            },
        ];

        for tc in test_cases {
            let err = parser().parse(tc.string).into_result().unwrap_err();
            assert_eq!(err[0].to_string(), tc.err_text);
        }
    }
}
