//! data query expressions parser.
use crate::debugger::variable::select::{Expression, VariableSelector};
use crate::ui::command::parser::{hex, rust_identifier};
use chumsky::prelude::*;
use chumsky::Parser;

type Err<'a> = extra::Err<Rich<'a, char>>;

fn ptr_cast<'a>() -> impl Parser<'a, &'a str, Expression, Err<'a>> + Clone {
    let op = |c| just(c).padded();

    // try to interp any string between brackets as a type
    let any = any::<_, Err>()
        .filter(|c| *c != ')')
        .repeated()
        .at_least(1)
        .to_slice();
    let type_p = any.delimited_by(op('('), op(')'));
    type_p
        .then(hex())
        .map(|(r#type, ptr)| Expression::PtrCast(ptr, r#type.trim().to_string()))
        .labelled("pointer cast")
}

pub fn parser<'a>() -> impl Parser<'a, &'a str, Expression, Err<'a>> {
    let selector = rust_identifier().padded().map(|name: &str| {
        Expression::Variable(VariableSelector::Name {
            var_name: name.to_string(),
            only_local: false,
        })
    });

    let expr = recursive(|expr| {
        let op = |c| just(c).padded();

        let atom = selector.or(expr.delimited_by(op('('), op(')'))).padded();

        let field = text::ascii::ident()
            .or(text::int(10))
            .map(|s: &str| s.to_string())
            .labelled("field name or tuple index");
        let field_expr = atom.clone().foldl(
            op('.')
                .to(Expression::Field as fn(_, _) -> _)
                .then(field)
                .repeated(),
            |lhs, (op, rhs)| op(Box::new(lhs), rhs),
        );

        let index_op = text::int(10)
            .padded()
            .map(|v: &str| v.parse::<u64>().unwrap())
            .labelled("index value")
            .delimited_by(op('['), op(']'));
        let index_expr = field_expr.clone().foldl(index_op.repeated(), |r, idx| {
            Expression::Index(Box::new(r), idx)
        });

        let mb_usize = text::int(10)
            .or_not()
            .padded()
            .map(|v: Option<&str>| v.map(|v| v.parse::<usize>().unwrap()));

        let slice_op = mb_usize
            .then_ignore(just("..").padded())
            .then(mb_usize)
            .labelled("slice range (start..end)")
            .delimited_by(op('['), op(']'));
        let slice_expr = index_expr
            .clone()
            .foldl(slice_op.repeated(), |r, (from, to)| {
                Expression::Slice(Box::new(r), from, to)
            });

        let expr = slice_expr.or(ptr_cast());

        op('*')
            .repeated()
            .foldr(expr, |_op, rhs| Expression::Deref(Box::new(rhs)))
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
            result: Result<Expression, ()>,
        }
        let cases = vec![
            TestCase {
                string: "(*SomeStruct) 0x12345",
                result: Ok(Expression::PtrCast(0x12345, "*SomeStruct".to_string())),
            },
            TestCase {
                string: " ( &u32 )0x12345",
                result: Ok(Expression::PtrCast(0x12345, "&u32".to_string())),
            },
            TestCase {
                string: "(*const abc::def::SomeType)  0x123AABCD",
                result: Ok(Expression::PtrCast(
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
                result: Ok(Expression::PtrCast(
                    0x7FFFFFFFDC94,
                    "*const i32".to_string(),
                )),
            },
        ];

        for tc in cases {
            let expr = ptr_cast().parse(tc.string).into_result();
            assert_eq!(expr.map_err(|_| ()), tc.result);
        }
    }

    #[test]
    fn test_expr_parsing() {
        struct TestCase {
            string: &'static str,
            expr: Expression,
        }
        let test_cases = vec![
            TestCase {
                string: "var1",
                expr: Expression::Variable(VariableSelector::Name {
                    var_name: "var1".to_string(),
                    only_local: false,
                }),
            },
            TestCase {
                string: "*var1",
                expr: Expression::Deref(Box::new(Expression::Variable(VariableSelector::Name {
                    var_name: "var1".to_string(),
                    only_local: false,
                }))),
            },
            TestCase {
                string: "**var1",
                expr: Expression::Deref(Box::new(Expression::Deref(Box::new(
                    Expression::Variable(VariableSelector::Name {
                        var_name: "var1".to_string(),
                        only_local: false,
                    }),
                )))),
            },
            TestCase {
                string: "**var1.field1.field2",
                expr: Expression::Deref(Box::new(Expression::Deref(Box::new(Expression::Field(
                    Box::new(Expression::Field(
                        Box::new(Expression::Variable(VariableSelector::Name {
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
                expr: Expression::Deref(Box::new(Expression::Deref(Box::new(Expression::Field(
                    Box::new(Expression::Field(
                        Box::new(Expression::Variable(VariableSelector::Name {
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
                expr: Expression::Field(
                    Box::new(Expression::Field(
                        Box::new(Expression::Deref(Box::new(Expression::Deref(Box::new(
                            Expression::Variable(VariableSelector::Name {
                                var_name: "var1".to_string(),
                                only_local: false,
                            }),
                        ))))),
                        "field1".to_string(),
                    )),
                    "field2".to_string(),
                ),
            },
            TestCase {
                string: "*(*(var1.field1)).field2[1][2]",
                expr: Expression::Deref(Box::new(Expression::Index(
                    Box::new(Expression::Index(
                        Box::new(Expression::Field(
                            Box::new(Expression::Deref(Box::new(Expression::Field(
                                Box::new(Expression::Variable(VariableSelector::Name {
                                    var_name: "var1".to_string(),
                                    only_local: false,
                                })),
                                "field1".to_string(),
                            )))),
                            "field2".to_string(),
                        )),
                        1,
                    )),
                    2,
                ))),
            },
            TestCase {
                string: "var1.field1[5..]",
                expr: Expression::Slice(
                    Box::new(Expression::Field(
                        Box::new(Expression::Variable(VariableSelector::Name {
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
                expr: Expression::Slice(
                    Box::new(Expression::Field(
                        Box::new(Expression::Variable(VariableSelector::Name {
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
                expr: Expression::Slice(
                    Box::new(Expression::Field(
                        Box::new(Expression::Variable(VariableSelector::Name {
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
                expr: Expression::Slice(
                    Box::new(Expression::Field(
                        Box::new(Expression::Variable(VariableSelector::Name {
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
                expr: Expression::Field(
                    Box::new(Expression::Field(
                        Box::new(Expression::Variable(VariableSelector::Name {
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
                expr: Expression::PtrCast(0x123AABCD, "*mut SomeType".to_string()),
            },
            TestCase {
                string: "(&abc::def::SomeType)0x123AABCD",
                expr: Expression::PtrCast(0x123AABCD, "&abc::def::SomeType".to_string()),
            },
            TestCase {
                string: "(*const abc::def::SomeType)  0x123AABCD",
                expr: Expression::PtrCast(0x123AABCD, "*const abc::def::SomeType".to_string()),
            },
            TestCase {
                string: "*((*const abc::def::SomeType) 0x123AABCD)",
                expr: Expression::Deref(
                    Expression::PtrCast(0x123AABCD, "*const abc::def::SomeType".to_string())
                        .boxed(),
                ),
            },
            TestCase {
                string: "*(*const i32)0x007FFFFFFFDC94",
                expr: Expression::Deref(
                    Expression::PtrCast(0x7FFFFFFFDC94, "*const i32".to_string()).boxed(),
                ),
            },
        ];

        for tc in test_cases {
            let expr = parser().parse(tc.string).into_result().unwrap();
            assert_eq!(expr, tc.expr);
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
                err_text: "found ')' expected field name or tuple index, or '0'",
            },
            TestCase {
                string: "((var1)",
                err_text: "found end of input expected '.', '[', ')', or '0'",
            },
            TestCase {
                string: "(var1))",
                err_text: "found ')' expected '.', '[', or end of input",
            },
            TestCase {
                string: "*",
                err_text: "found end of input expected '*', ':', or '('",
            },
        ];

        for tc in test_cases {
            let err = parser().parse(tc.string).into_result().unwrap_err();
            assert_eq!(err[0].to_string(), tc.err_text);
        }
    }
}
