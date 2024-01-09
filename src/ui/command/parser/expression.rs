//! data query expressions parser.

use super::rust_identifier;
use crate::debugger::variable::select::{Expression, VariableSelector};
use nom::character::complete::{digit1, multispace0};
use nom::combinator::{cut, eof, peek};
use nom::error::context;
use nom::multi::many_till;
use nom::sequence::terminated;
use nom::{
    branch::alt,
    combinator::map,
    sequence::{delimited, preceded},
    IResult,
};
use nom_supreme::error::ErrorTree;
use nom_supreme::tag::complete::tag;
use std::fmt::Debug;

#[derive(Debug)]
pub enum Operation {
    Field,
    Index,
    Slice,
}

fn parens(i: &str) -> IResult<&str, Expression, ErrorTree<&str>> {
    delimited(
        multispace0,
        delimited(
            tag("("),
            map(expr, |e| Expression::Parentheses(Box::new(e))),
            cut(tag(")")),
        ),
        multispace0,
    )(i)
}

fn variable(i: &str) -> IResult<&str, Expression, ErrorTree<&str>> {
    map(delimited(multispace0, rust_identifier, multispace0), |id| {
        Expression::Variable(VariableSelector::Name {
            var_name: id.to_string(),
            local: false,
        })
    })(i)
}

fn fold_expressions(initial: Expression, remainder: Vec<(Operation, &str)>) -> Expression {
    remainder.into_iter().fold(initial, |acc, pair| {
        let (operation, expr) = pair;
        match operation {
            Operation::Field => Expression::Field(Box::new(acc), expr.to_string()),
            Operation::Index => Expression::Index(Box::new(acc), expr.parse().unwrap()),
            Operation::Slice => Expression::Slice(Box::new(acc), expr.parse().unwrap()),
        }
    })
}

fn r_op(i: &str) -> IResult<&str, Expression, ErrorTree<&str>> {
    let (i, initial) = alt((variable, parens))(i)?;
    let (i, (remainder, _)) = many_till(
        alt((
            context("field lookup", |i| {
                let (i, field) = preceded(tag("."), cut(alt((rust_identifier, digit1))))(i)?;
                Ok((i, (Operation::Field, field)))
            }),
            context("slice operator", |i| {
                let (i, len) = preceded(tag("[.."), cut(terminated(digit1, tag("]"))))(i)?;
                Ok((i, (Operation::Slice, len)))
            }),
            context("index operator", |i| {
                let (i, index) = preceded(tag("["), cut(terminated(digit1, tag("]"))))(i)?;
                Ok((i, (Operation::Index, index)))
            }),
        )),
        alt((eof, peek(tag(")")))),
    )(i)?;

    Ok((i, fold_expressions(initial, remainder)))
}

/// Parser for [`Expression`].
pub fn expr(input: &str) -> IResult<&str, Expression, ErrorTree<&str>> {
    alt((
        map(preceded(tag("*"), expr), |expr| {
            Expression::Deref(Box::new(expr))
        }),
        cut(r_op),
    ))(input)
}

#[cfg(test)]
mod test {
    use super::*;

    fn parse_expr(
        input: &str,
    ) -> Result<Expression, ErrorTree<nom_supreme::final_parser::Location>> {
        nom_supreme::final_parser::final_parser::<
            _,
            _,
            _,
            ErrorTree<nom_supreme::final_parser::Location>,
        >(expr)(input)
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
                    local: false,
                }),
            },
            TestCase {
                string: "*var1",
                expr: Expression::Deref(Box::new(Expression::Variable(VariableSelector::Name {
                    var_name: "var1".to_string(),
                    local: false,
                }))),
            },
            TestCase {
                string: "**var1",
                expr: Expression::Deref(Box::new(Expression::Deref(Box::new(
                    Expression::Variable(VariableSelector::Name {
                        var_name: "var1".to_string(),
                        local: false,
                    }),
                )))),
            },
            TestCase {
                string: "**var1.field1.field2",
                expr: Expression::Deref(Box::new(Expression::Deref(Box::new(Expression::Field(
                    Box::new(Expression::Field(
                        Box::new(Expression::Variable(VariableSelector::Name {
                            var_name: "var1".to_string(),
                            local: false,
                        })),
                        "field1".to_string(),
                    )),
                    "field2".to_string(),
                ))))),
            },
            TestCase {
                string: "**(var1.field1.field2)",
                expr: Expression::Deref(Box::new(Expression::Deref(Box::new(
                    Expression::Parentheses(Box::new(Expression::Field(
                        Box::new(Expression::Field(
                            Box::new(Expression::Variable(VariableSelector::Name {
                                var_name: "var1".to_string(),
                                local: false,
                            })),
                            "field1".to_string(),
                        )),
                        "field2".to_string(),
                    ))),
                )))),
            },
            TestCase {
                string: "(**var1).field1.field2",
                expr: Expression::Field(
                    Box::new(Expression::Field(
                        Box::new(Expression::Parentheses(Box::new(Expression::Deref(
                            Box::new(Expression::Deref(Box::new(Expression::Variable(
                                VariableSelector::Name {
                                    var_name: "var1".to_string(),
                                    local: false,
                                },
                            )))),
                        )))),
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
                            Box::new(Expression::Parentheses(Box::new(Expression::Deref(
                                Box::new(Expression::Parentheses(Box::new(Expression::Field(
                                    Box::new(Expression::Variable(VariableSelector::Name {
                                        var_name: "var1".to_string(),
                                        local: false,
                                    })),
                                    "field1".to_string(),
                                )))),
                            )))),
                            "field2".to_string(),
                        )),
                        1,
                    )),
                    2,
                ))),
            },
            TestCase {
                string: "var1.field1[..5]",
                expr: Expression::Slice(
                    Box::new(Expression::Field(
                        Box::new(Expression::Variable(VariableSelector::Name {
                            var_name: "var1".to_string(),
                            local: false,
                        })),
                        "field1".to_string(),
                    )),
                    5,
                ),
            },
            TestCase {
                string: "enum1.0.a",
                expr: Expression::Field(
                    Box::new(Expression::Field(
                        Box::new(Expression::Variable(VariableSelector::Name {
                            var_name: "enum1".to_string(),
                            local: false,
                        })),
                        "0".to_string(),
                    )),
                    "a".to_string(),
                ),
            },
        ];

        for tc in test_cases {
            let expr = parse_expr(tc.string).unwrap();
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
                err_text: r#"while parsing ManyTill at line 1, column 6,
one of:
  in section "field lookup" at line 1, column 6,
  expected "." at line 1, column 6, or
  in section "slice operator" at line 1, column 6,
  expected "[.." at line 1, column 6, or
  in section "index operator" at line 1, column 6,
  expected "[" at line 1, column 6"#,
            },
            TestCase {
                string: "var1..",
                err_text: r#"in section "field lookup" at line 1, column 5,
one of:
  expected an ascii letter at line 1, column 6, or
  expected "_" at line 1, column 6"#,
            },
            TestCase {
                string: "var1[]",
                err_text: r#"in section "index operator" at line 1, column 5,
expected an ascii digit at line 1, column 6"#,
            },
            TestCase {
                string: "(var1.)field1",
                err_text: r#"in section "field lookup" at line 1, column 6,
one of:
  expected an ascii letter at line 1, column 7, or
  expected "_" at line 1, column 7"#,
            },
            TestCase {
                string: "((var1)",
                err_text: r#"expected ")" at line 1, column 8"#,
            },
            TestCase {
                string: "(var1))",
                err_text: r#"expected eof at line 1, column 7"#,
            },
            TestCase {
                string: "*",
                err_text: r#"one of:
  expected an ascii letter at line 1, column 2, or
  expected "_" at line 1, column 2, or
  expected "(" at line 1, column 2"#,
            },
        ];

        for tc in test_cases {
            let err = parse_expr(tc.string).unwrap_err();
            assert!(err.to_string().contains(tc.err_text));
        }
    }
}
