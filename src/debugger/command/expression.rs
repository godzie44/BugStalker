use nom::branch::alt;
use nom::bytes::complete::tag;
use nom::character::complete::{alpha1, alphanumeric1, char, digit1};
use nom::combinator::{cut, eof, map, map_res, peek};
use nom::error::{context, Error};
use nom::multi::{many0, many0_count};
use nom::sequence::{delimited, pair, preceded, terminated};
use nom::{combinator::recognize, Finish, IResult, Parser};
use std::cmp::Ordering;
use std::collections::VecDeque;
use std::num::ParseIntError;

#[derive(Debug, PartialEq)]
pub enum Token {
    Deref,
    OpenBracket,
    ClosedBracket,
    GetField(String),
    Root(String),
    Slice(usize),
    GetByIndex(usize),
    EOF,
}

#[derive(Debug, PartialEq)]
pub struct RustIdentify(String);

struct Tokenizer<'a> {
    string: &'a str,
}

impl<'a> Tokenizer<'a> {
    fn new(string: &'a str) -> Self {
        Self {
            string: string.trim(),
        }
    }

    fn tokenize(&self) -> Result<(&str, Vec<Token>), Error<String>> {
        fn rust_identifier(input: &str) -> IResult<&str, &str> {
            recognize(pair(
                alt((alpha1, tag("_"))),
                many0_count(alt((alphanumeric1, tag("_")))),
            ))(input)
        }

        fn left_op<'a, O1, F>(
            ctx: &'static str,
            inner: F,
        ) -> impl FnMut(&'a str) -> IResult<&'a str, O1, Error<&'a str>>
        where
            F: Parser<&'a str, O1, Error<&'a str>>,
        {
            context(
                ctx,
                terminated(inner, cut(peek(alt((open_bracket, deref, root))))),
            )
        }

        fn right_op<'a, O1, F>(
            ctx: &'static str,
            inner: F,
        ) -> impl FnMut(&'a str) -> IResult<&'a str, O1, Error<&'a str>>
        where
            F: Parser<&'a str, O1, Error<&'a str>>,
        {
            context(
                ctx,
                terminated(
                    inner,
                    cut(peek(alt((
                        close_bracket,
                        field,
                        slice,
                        index,
                        map(eof, |_| Token::EOF),
                    )))),
                ),
            )
        }

        fn deref(input: &str) -> IResult<&str, Token> {
            map(left_op("deref", tag("*")), |_| Token::Deref)(input)
        }
        fn open_bracket(input: &str) -> IResult<&str, Token> {
            map(left_op("opened bracket", tag("(")), |_| Token::OpenBracket)(input)
        }
        fn root(input: &str) -> IResult<&str, Token> {
            map(right_op("root", rust_identifier), |s: &str| {
                Token::Root(s.into())
            })(input)
        }
        fn close_bracket(input: &str) -> IResult<&str, Token> {
            map(right_op("closed bracket", tag(")")), |_| {
                Token::ClosedBracket
            })(input)
        }
        fn field(input: &str) -> IResult<&str, Token> {
            map(
                right_op("field", preceded(tag("."), cut(rust_identifier))),
                |s| Token::GetField(s.into()),
            )(input)
        }
        fn slice(input: &str) -> IResult<&str, Token> {
            map_res(
                right_op("slice", delimited(tag("[.."), cut(digit1), char(']'))),
                |digits: &str| -> Result<Token, ParseIntError> {
                    Ok(Token::Slice(digits.parse()?))
                },
            )(input)
        }
        fn index(input: &str) -> IResult<&str, Token> {
            map_res(
                right_op("index", delimited(tag("["), cut(digit1), char(']'))),
                |digits: &str| -> Result<Token, ParseIntError> {
                    Ok(Token::GetByIndex(digits.parse()?))
                },
            )(input)
        }

        many0(alt((
            deref,
            open_bracket,
            close_bracket,
            field,
            slice,
            index,
            root,
        )))(self.string)
        .map_err(|e| e.to_owned())
        .finish()
    }
}

/// `ExprPlan` item.
#[derive(Debug, Clone, PartialEq)]
pub enum Operation {
    Deref,
    Root(String),
    Index(usize),
    Field(String),
    Slice(usize),
}

impl Operation {
    fn left_associative(&self) -> bool {
        match self {
            Operation::Deref => false,
            Operation::Field(_) => true,
            Operation::Index(_) => true,
            Operation::Slice(_) => true,
            Operation::Root(_) => true,
        }
    }
}

impl PartialOrd for Operation {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match self {
            Operation::Deref => match other {
                Operation::Deref => Some(Ordering::Equal),
                Operation::Field(_) => Some(Ordering::Less),
                Operation::Index(_) => Some(Ordering::Less),
                Operation::Slice(_) => Some(Ordering::Less),
                Operation::Root(_) => Some(Ordering::Less),
            },
            Operation::Field(_) => match other {
                Operation::Deref => Some(Ordering::Greater),
                Operation::Field(_) => Some(Ordering::Equal),
                Operation::Index(_) => Some(Ordering::Equal),
                Operation::Slice(_) => Some(Ordering::Equal),
                Operation::Root(_) => Some(Ordering::Less),
            },
            Operation::Index(_) => match other {
                Operation::Deref => Some(Ordering::Greater),
                Operation::Field(_) => Some(Ordering::Equal),
                Operation::Index(_) => Some(Ordering::Equal),
                Operation::Slice(_) => Some(Ordering::Equal),
                Operation::Root(_) => Some(Ordering::Less),
            },
            Operation::Slice(_) => match other {
                Operation::Deref => Some(Ordering::Greater),
                Operation::Field(_) => Some(Ordering::Equal),
                Operation::Index(_) => Some(Ordering::Equal),
                Operation::Slice(_) => Some(Ordering::Equal),
                Operation::Root(_) => Some(Ordering::Less),
            },
            Operation::Root(_) => match other {
                Operation::Deref => Some(Ordering::Greater),
                Operation::Field(_) => Some(Ordering::Greater),
                Operation::Index(_) => Some(Ordering::Greater),
                Operation::Slice(_) => Some(Ordering::Greater),
                Operation::Root(_) => Some(Ordering::Equal),
            },
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ParseError {
    #[error(transparent)]
    ParsingError(#[from] Error<String>),
    #[error("expression miss open bracket")]
    MissOpenBracket,
    #[error("expression miss closed bracket")]
    MissClosedBracket,
}

/// List of operations for further execution.
/// `ExprPlan` can be generated from an input string of the form "{operator}{open bracket}variable{operator}{field}{index}{closed bracket}"
/// Supported operators are: dereference, get element by index, get field by name, make slice from pointer.
#[derive(Debug, PartialEq, Default)]
pub struct ExprPlan {
    pub source: String,
    pub plan: VecDeque<Operation>,
}

impl ExprPlan {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn select_variable(var_name: &str) -> Self {
        Self {
            source: var_name.to_string(),
            plan: VecDeque::from(vec![Operation::Root(var_name.to_string())]),
        }
    }

    pub(crate) fn base_variable_name(&self) -> Option<&str> {
        self.plan.front().and_then(|item| match item {
            Operation::Root(var) => Some(var.as_str()),
            _ => None,
        })
    }
}

/// Parse `ExprPlan` from input string. Using shunting yard algorithm.
pub struct ExprPlanParser<'a> {
    string: &'a str,
}

impl<'a> ExprPlanParser<'a> {
    /// Create new `ExprPlanParser`.
    pub fn new(string: &'a str) -> Self {
        Self { string }
    }

    /// Parse `ExprPlan` from input string.
    pub fn parse(&self) -> Result<ExprPlan, ParseError> {
        let tokenizer = Tokenizer::new(self.string);

        let mut plan = VecDeque::new();

        enum StackItem {
            Operator(Operation),
            OpenBracket,
        }
        let mut stack: Vec<StackItem> = vec![];

        fn pop_high_prio(
            stack: &mut Vec<StackItem>,
            output: &mut VecDeque<Operation>,
            op1: Operation,
        ) {
            while let Some(StackItem::Operator(op2)) = stack.last() {
                match op2.partial_cmp(&op1) {
                    Some(Ordering::Greater) | Some(Ordering::Equal) if op1.left_associative() => {
                        output.push_back(op2.clone());
                        stack.pop();
                    }
                    _ => break,
                }
            }
        }

        let (_, tokens) = tokenizer.tokenize()?;

        for token in tokens {
            match &token {
                Token::Root(name) => {
                    let op1 = Operation::Root(name.clone());
                    pop_high_prio(&mut stack, &mut plan, op1.clone());
                    stack.push(StackItem::Operator(op1));
                }
                Token::Deref => {
                    let op1 = Operation::Deref;
                    pop_high_prio(&mut stack, &mut plan, op1.clone());
                    stack.push(StackItem::Operator(op1));
                }
                Token::OpenBracket => {
                    stack.push(StackItem::OpenBracket);
                }
                Token::ClosedBracket => loop {
                    match stack.last() {
                        Some(StackItem::OpenBracket) => {
                            stack.pop();
                            break;
                        }
                        Some(StackItem::Operator(op)) => {
                            plan.push_back(op.clone());
                            stack.pop();
                        }
                        None => return Err(ParseError::MissOpenBracket),
                    }
                },
                Token::GetField(field) => {
                    let op1 = Operation::Field(field.clone());
                    pop_high_prio(&mut stack, &mut plan, op1.clone());
                    stack.push(StackItem::Operator(op1));
                }
                Token::GetByIndex(idx) => {
                    let op1 = Operation::Index(*idx);
                    pop_high_prio(&mut stack, &mut plan, op1.clone());
                    stack.push(StackItem::Operator(op1));
                }
                Token::Slice(len) => {
                    let op1 = Operation::Slice(*len);
                    pop_high_prio(&mut stack, &mut plan, op1.clone());
                    stack.push(StackItem::Operator(op1));
                }
                Token::EOF => {}
            }
        }

        while let Some(it) = stack.pop() {
            match it {
                StackItem::Operator(op) => {
                    plan.push_back(op);
                }
                StackItem::OpenBracket => return Err(ParseError::MissClosedBracket),
            }
        }

        Ok(ExprPlan {
            source: self.string.to_string(),
            plan,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use nom::error::ErrorKind;

    #[test]
    fn test_tokenizer() {
        struct TestCase {
            string: &'static str,
            tokens: Vec<Token>,
        }
        let test_cases = vec![
            TestCase {
                string: "var1",
                tokens: vec![Token::Root("var1".into())],
            },
            TestCase {
                string: "*var1",
                tokens: vec![Token::Deref, Token::Root("var1".into())],
            },
            TestCase {
                string: "**var1",
                tokens: vec![Token::Deref, Token::Deref, Token::Root("var1".into())],
            },
            TestCase {
                string: "**var1.field1.field2",
                tokens: vec![
                    Token::Deref,
                    Token::Deref,
                    Token::Root("var1".into()),
                    Token::GetField("field1".into()),
                    Token::GetField("field2".into()),
                ],
            },
            TestCase {
                string: "**(var1.field1.field2)",
                tokens: vec![
                    Token::Deref,
                    Token::Deref,
                    Token::OpenBracket,
                    Token::Root("var1".into()),
                    Token::GetField("field1".into()),
                    Token::GetField("field2".into()),
                    Token::ClosedBracket,
                ],
            },
            TestCase {
                string: "*(*(var1.field1.field2[1][2]))",
                tokens: vec![
                    Token::Deref,
                    Token::OpenBracket,
                    Token::Deref,
                    Token::OpenBracket,
                    Token::Root("var1".into()),
                    Token::GetField("field1".into()),
                    Token::GetField("field2".into()),
                    Token::GetByIndex(1),
                    Token::GetByIndex(2),
                    Token::ClosedBracket,
                    Token::ClosedBracket,
                ],
            },
            TestCase {
                string: "var1.field1[..5]",
                tokens: vec![
                    Token::Root("var1".into()),
                    Token::GetField("field1".into()),
                    Token::Slice(5),
                ],
            },
        ];

        for tc in test_cases {
            let tokenizer = Tokenizer::new(tc.string);
            let (_, tokens) = tokenizer.tokenize().unwrap();
            assert_eq!(tokens, tc.tokens);
        }
    }

    #[test]
    fn test_tokenizer_err() {
        struct TestCase {
            string: &'static str,
            error: &'static str,
        }
        let test_cases = vec![
            TestCase {
                string: "var1 var2",
                error: "error Eof at:  var2",
            },
            TestCase {
                string: "var1..",
                error: "error Tag at: .",
            },
            TestCase {
                string: "var1[]",
                error: "error Digit at: ]",
            },
        ];

        for tc in test_cases {
            let tokenizer = Tokenizer::new(tc.string);
            let err = tokenizer.tokenize().err().unwrap();
            assert_eq!(err.to_string(), tc.error);
        }
    }

    #[test]
    fn test_parser() {
        struct TestCase {
            string: &'static str,
            out: Result<ExprPlan, ParseError>,
        }
        let test_cases = vec![
            TestCase {
                string: "**var1.field1",
                out: Ok(ExprPlan {
                    source: "**var1.field1".to_string(),
                    plan: VecDeque::from(vec![
                        Operation::Root("var1".to_string()),
                        Operation::Field("field1".to_string()),
                        Operation::Deref,
                        Operation::Deref,
                    ]),
                }),
            },
            TestCase {
                string: "var1.field1.field2[1][2].field3",
                out: Ok(ExprPlan {
                    source: "var1.field1.field2[1][2].field3".to_string(),
                    plan: VecDeque::from(vec![
                        Operation::Root("var1".to_string()),
                        Operation::Field("field1".to_string()),
                        Operation::Field("field2".to_string()),
                        Operation::Index(1),
                        Operation::Index(2),
                        Operation::Field("field3".to_string()),
                    ]),
                }),
            },
            TestCase {
                string: "*(*var1.field1).field2",
                out: Ok(ExprPlan {
                    source: "*(*var1.field1).field2".to_string(),
                    plan: VecDeque::from(vec![
                        Operation::Root("var1".to_string()),
                        Operation::Field("field1".to_string()),
                        Operation::Deref,
                        Operation::Field("field2".to_string()),
                        Operation::Deref,
                    ]),
                }),
            },
            TestCase {
                string: "*(*var1.field1[0][1]).field2",
                out: Ok(ExprPlan {
                    source: "*(*var1.field1[0][1]).field2".to_string(),
                    plan: VecDeque::from(vec![
                        Operation::Root("var1".to_string()),
                        Operation::Field("field1".to_string()),
                        Operation::Index(0),
                        Operation::Index(1),
                        Operation::Deref,
                        Operation::Field("field2".to_string()),
                        Operation::Deref,
                    ]),
                }),
            },
            TestCase {
                string: "(var1.field1)",
                out: Ok(ExprPlan {
                    source: "(var1.field1)".to_string(),
                    plan: VecDeque::from(vec![
                        Operation::Root("var1".to_string()),
                        Operation::Field("field1".to_string()),
                    ]),
                }),
            },
            TestCase {
                string: "(var1).field1",
                out: Ok(ExprPlan {
                    source: "(var1).field1".to_string(),
                    plan: VecDeque::from(vec![
                        Operation::Root("var1".to_string()),
                        Operation::Field("field1".to_string()),
                    ]),
                }),
            },
            TestCase {
                string: "var1[..5]",
                out: Ok(ExprPlan {
                    source: "var1[..5]".to_string(),
                    plan: VecDeque::from(vec![
                        Operation::Root("var1".to_string()),
                        Operation::Slice(5),
                    ]),
                }),
            },
            TestCase {
                string: "(var1.)field1",
                out: Err(ParseError::ParsingError(nom::error::Error::new(
                    ")field1".to_string(),
                    ErrorKind::Tag,
                ))),
            },
            TestCase {
                string: "var1.",
                out: Err(ParseError::ParsingError(nom::error::Error::new(
                    "".to_string(),
                    ErrorKind::Tag,
                ))),
            },
            TestCase {
                string: "((var1)",
                out: Err(ParseError::MissClosedBracket),
            },
            TestCase {
                string: "(var1))",
                out: Err(ParseError::MissOpenBracket),
            },
        ];

        for tc in test_cases {
            let parser = ExprPlanParser::new(tc.string);
            let result = parser.parse();

            assert_eq!(result, tc.out);
        }
    }
}
