use std::cmp::Ordering;
use std::collections::VecDeque;
use std::mem;

#[derive(Debug, PartialEq, Clone)]
pub enum Token {
    Deref,
    OpenBracket,
    ClosedBracket,
    Dot,
    DoubleDot,
    OpenSquareBracket,
    ClosedSquareBracket,
    Text(String),
    End,
}

impl Token {
    fn next_valid(&self) -> fn(tok: &Token) -> bool {
        match self {
            Token::Deref => |tok| matches!(tok, Token::Deref | Token::OpenBracket | Token::Text(_)),
            Token::OpenBracket => {
                |tok| matches!(tok, Token::Deref | Token::OpenBracket | Token::Text(_))
            }
            Token::ClosedBracket => |tok| {
                matches!(
                    tok,
                    Token::ClosedBracket | Token::Dot | Token::OpenSquareBracket | Token::End
                )
            },
            Token::Dot => |tok| matches!(tok, Token::Text(_)),
            Token::OpenSquareBracket => |tok| matches!(tok, Token::Text(_) | Token::DoubleDot),
            Token::ClosedSquareBracket => |tok| {
                matches!(
                    tok,
                    Token::ClosedBracket | Token::Dot | Token::OpenSquareBracket | Token::End
                )
            },
            Token::Text(_) => |tok| {
                matches!(
                    tok,
                    Token::ClosedBracket
                        | Token::Dot
                        | Token::OpenSquareBracket
                        | Token::ClosedSquareBracket
                        | Token::End
                )
            },
            Token::DoubleDot => |tok| matches!(tok, Token::Text(_)),
            Token::End => |_| true,
        }
    }
}

struct Tokenizer<'a> {
    string: &'a str,
    accum: String,
    tokens: Vec<Token>,
}

impl<'a> Tokenizer<'a> {
    fn new(string: &'a str) -> Self {
        Self {
            string,
            accum: String::default(),
            tokens: vec![],
        }
    }

    fn tokenize(mut self) -> Vec<Token> {
        let mut skip = false;
        for (i, char) in self.string.chars().enumerate() {
            if skip {
                skip = false;
                continue;
            }
            match char {
                '*' => self.push_text_and_token(Token::Deref),
                '(' => self.push_text_and_token(Token::OpenBracket),
                ')' => self.push_text_and_token(Token::ClosedBracket),
                '.' => {
                    if self.string.chars().nth(i + 1) == Some('.') {
                        self.push_text_and_token(Token::DoubleDot);
                        skip = true;
                    } else {
                        self.push_text_and_token(Token::Dot)
                    }
                }
                '[' => self.push_text_and_token(Token::OpenSquareBracket),
                ']' => self.push_text_and_token(Token::ClosedSquareBracket),
                _ => self.accum.push(char),
            }
        }
        self.push_text();
        self.tokens.push(Token::End);

        self.tokens
    }

    fn push_text(&mut self) {
        let text = mem::take(&mut self.accum);
        if !text.is_empty() {
            self.tokens.push(Token::Text(text))
        }
    }

    fn push_text_and_token(&mut self, tok: Token) {
        self.push_text();
        self.tokens.push(tok);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Operator {
    Deref,
    Field,
    Index,
    Slice,
}

impl Operator {
    fn left_associative(&self) -> bool {
        match self {
            Operator::Deref => false,
            Operator::Field => true,
            Operator::Index => true,
            Operator::Slice => true,
        }
    }
}

impl PartialOrd for Operator {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match self {
            Operator::Deref => match other {
                Operator::Deref => Some(Ordering::Equal),
                Operator::Field => Some(Ordering::Less),
                Operator::Index => Some(Ordering::Less),
                Operator::Slice => Some(Ordering::Less),
            },
            Operator::Field => match other {
                Operator::Deref => Some(Ordering::Greater),
                Operator::Field => Some(Ordering::Equal),
                Operator::Index => Some(Ordering::Equal),
                Operator::Slice => Some(Ordering::Equal),
            },
            Operator::Index => match other {
                Operator::Deref => Some(Ordering::Greater),
                Operator::Field => Some(Ordering::Equal),
                Operator::Index => Some(Ordering::Equal),
                Operator::Slice => Some(Ordering::Equal),
            },
            Operator::Slice => match other {
                Operator::Deref => Some(Ordering::Greater),
                Operator::Field => Some(Ordering::Equal),
                Operator::Index => Some(Ordering::Equal),
                Operator::Slice => Some(Ordering::Equal),
            },
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ParseError {
    #[error("unexpected token: {0:?}")]
    UnexpectedToken(Token),
    #[error("expression miss open bracket")]
    MissOpenBracket,
    #[error("expression miss closed bracket")]
    MissClosedBracket,
    #[error("expect operand for operation {0}")]
    OperandNotFound(&'static str),
    #[error("invalid operand {0}")]
    InvalidOperand(String),
}

/// `SelectPlan` item.
#[derive(Debug, PartialEq)]
pub enum Operation {
    Deref,
    FindVariable(String),
    GetByIndex(usize),
    GetField(String),
    Slice(usize),
}

/// List of operations for further execution.
/// `SelectPlan` can be generated from an input string of the form "{operator}{open bracket}variable{operator}{field}{index}{closed bracket}"
/// Supported operators are: dereference, get element by index, get field by name.
#[derive(Debug, PartialEq, Default)]
pub struct SelectPlan {
    pub source: String,
    pub plan: VecDeque<Operation>,
}

impl SelectPlan {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn select_variable(var_name: &str) -> Self {
        Self {
            source: var_name.to_string(),
            plan: VecDeque::from(vec![Operation::FindVariable(var_name.to_string())]),
        }
    }

    pub(crate) fn base_variable_name(&self) -> Option<&str> {
        self.plan.front().and_then(|item| match item {
            Operation::FindVariable(var) => Some(var.as_str()),
            _ => None,
        })
    }
}

/// Parse `SelectPlan` from input string. Using shunting yard algorithm.
pub struct SelectPlanParser<'a> {
    string: &'a str,
}

impl<'a> SelectPlanParser<'a> {
    /// Create new `SelectPlanParser`.
    pub fn new(string: &'a str) -> Self {
        Self { string }
    }

    /// Parse `SelectPlan` from input string.
    pub fn parse(&self) -> Result<SelectPlan, ParseError> {
        let tokenizer = Tokenizer::new(self.string);

        enum OperatorOrOperand {
            Operator(Operator),
            Operand(String),
        }
        let mut output = VecDeque::new();

        enum StackItem {
            Operator(Operator),
            OpenBracket,
        }
        let mut stack: Vec<StackItem> = vec![];

        fn pop_high_prio(
            stack: &mut Vec<StackItem>,
            output: &mut VecDeque<OperatorOrOperand>,
            op1: Operator,
        ) {
            while let Some(StackItem::Operator(op2)) = stack.last() {
                match op2.partial_cmp(&op1) {
                    Some(Ordering::Greater) | Some(Ordering::Equal) if op1.left_associative() => {
                        output.push_back(OperatorOrOperand::Operator(*op2));
                        stack.pop();
                    }
                    _ => break,
                }
            }
        }

        let mut next_valid: fn(&Token) -> bool = |_: &Token| true;
        let mut tokens = tokenizer.tokenize();

        for i in 0..tokens.len() {
            if !next_valid(&tokens[i]) {
                return Err(ParseError::UnexpectedToken(tokens.swap_remove(i)));
            }
            next_valid = tokens[i].next_valid();

            match &tokens[i] {
                Token::Text(s) => {
                    output.push_back(OperatorOrOperand::Operand(s.clone()));
                }
                Token::Deref => {
                    let op1 = Operator::Deref;
                    pop_high_prio(&mut stack, &mut output, op1);
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
                            output.push_back(OperatorOrOperand::Operator(*op));
                            stack.pop();
                        }
                        None => return Err(ParseError::MissOpenBracket),
                    }
                },
                Token::Dot => {
                    let op1 = Operator::Field;
                    pop_high_prio(&mut stack, &mut output, op1);
                    stack.push(StackItem::Operator(op1));
                }
                Token::OpenSquareBracket => {
                    let op1 = if tokens[i + 1] == Token::DoubleDot {
                        Operator::Slice
                    } else {
                        Operator::Index
                    };
                    pop_high_prio(&mut stack, &mut output, op1);
                    stack.push(StackItem::Operator(op1));
                }
                Token::ClosedSquareBracket => {}
                Token::DoubleDot => {}
                Token::End => {}
            }
        }

        while let Some(it) = stack.pop() {
            match it {
                StackItem::Operator(op) => {
                    output.push_back(OperatorOrOperand::Operator(op));
                }
                StackItem::OpenBracket => return Err(ParseError::MissClosedBracket),
            }
        }

        let mut plan = VecDeque::new();
        let mut operand_stack = vec![];
        for (idx, item) in output.into_iter().enumerate() {
            if idx == 0 {
                if let OperatorOrOperand::Operand(text) = item {
                    plan.push_back(Operation::FindVariable(text))
                } else {
                    return Err(ParseError::OperandNotFound("find variable"));
                }
                continue;
            };

            match item {
                OperatorOrOperand::Operator(op) => match op {
                    Operator::Deref => plan.push_back(Operation::Deref),
                    Operator::Field => plan.push_back(Operation::GetField(
                        operand_stack
                            .pop()
                            .ok_or(ParseError::OperandNotFound("get field"))?,
                    )),
                    Operator::Index => {
                        let operand = operand_stack
                            .pop()
                            .ok_or(ParseError::OperandNotFound("get by index"))?;
                        let index = operand
                            .parse::<usize>()
                            .map_err(|_| ParseError::InvalidOperand(operand))?;
                        plan.push_back(Operation::GetByIndex(index));
                    }
                    Operator::Slice => {
                        let operand = operand_stack
                            .pop()
                            .ok_or(ParseError::OperandNotFound("slice"))?;
                        let index = operand
                            .parse::<usize>()
                            .map_err(|_| ParseError::InvalidOperand(operand))?;
                        plan.push_back(Operation::Slice(index));
                    }
                },
                OperatorOrOperand::Operand(text) => {
                    operand_stack.push(text);
                }
            }
        }

        Ok(SelectPlan {
            source: self.string.to_string(),
            plan,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_tokenizer() {
        struct TestCase {
            string: &'static str,
            tokens: Vec<Token>,
        }
        let test_cases = vec![
            TestCase {
                string: "var1",
                tokens: vec![Token::Text("var1".into()), Token::End],
            },
            TestCase {
                string: "*var1",
                tokens: vec![Token::Deref, Token::Text("var1".into()), Token::End],
            },
            TestCase {
                string: "**var1",
                tokens: vec![
                    Token::Deref,
                    Token::Deref,
                    Token::Text("var1".into()),
                    Token::End,
                ],
            },
            TestCase {
                string: "**var1.field1.field2",
                tokens: vec![
                    Token::Deref,
                    Token::Deref,
                    Token::Text("var1".into()),
                    Token::Dot,
                    Token::Text("field1".into()),
                    Token::Dot,
                    Token::Text("field2".into()),
                    Token::End,
                ],
            },
            TestCase {
                string: "**(var1.field1.field2)",
                tokens: vec![
                    Token::Deref,
                    Token::Deref,
                    Token::OpenBracket,
                    Token::Text("var1".into()),
                    Token::Dot,
                    Token::Text("field1".into()),
                    Token::Dot,
                    Token::Text("field2".into()),
                    Token::ClosedBracket,
                    Token::End,
                ],
            },
            TestCase {
                string: "*(*(var1.field1.field2[1][2]))",
                tokens: vec![
                    Token::Deref,
                    Token::OpenBracket,
                    Token::Deref,
                    Token::OpenBracket,
                    Token::Text("var1".into()),
                    Token::Dot,
                    Token::Text("field1".into()),
                    Token::Dot,
                    Token::Text("field2".into()),
                    Token::OpenSquareBracket,
                    Token::Text("1".into()),
                    Token::ClosedSquareBracket,
                    Token::OpenSquareBracket,
                    Token::Text("2".into()),
                    Token::ClosedSquareBracket,
                    Token::ClosedBracket,
                    Token::ClosedBracket,
                    Token::End,
                ],
            },
            TestCase {
                string: "var1.field1[..5]",
                tokens: vec![
                    Token::Text("var1".into()),
                    Token::Dot,
                    Token::Text("field1".into()),
                    Token::OpenSquareBracket,
                    Token::DoubleDot,
                    Token::Text("5".into()),
                    Token::ClosedSquareBracket,
                    Token::End,
                ],
            },
        ];

        for tc in test_cases {
            let tokenizer = Tokenizer::new(tc.string);
            let tokens = tokenizer.tokenize();
            assert_eq!(tokens, tc.tokens);
        }
    }

    #[test]
    fn test_parser() {
        struct TestCase {
            string: &'static str,
            out: Result<SelectPlan, ParseError>,
        }
        let test_cases = vec![
            TestCase {
                string: "**var1.field1",
                out: Ok(SelectPlan {
                    source: "**var1.field1".to_string(),
                    plan: VecDeque::from(vec![
                        Operation::FindVariable("var1".to_string()),
                        Operation::GetField("field1".to_string()),
                        Operation::Deref,
                        Operation::Deref,
                    ]),
                }),
            },
            TestCase {
                string: "var1.field1.field2[1][2].field3",
                out: Ok(SelectPlan {
                    source: "var1.field1.field2[1][2].field3".to_string(),
                    plan: VecDeque::from(vec![
                        Operation::FindVariable("var1".to_string()),
                        Operation::GetField("field1".to_string()),
                        Operation::GetField("field2".to_string()),
                        Operation::GetByIndex(1),
                        Operation::GetByIndex(2),
                        Operation::GetField("field3".to_string()),
                    ]),
                }),
            },
            TestCase {
                string: "*(*var1.field1).field2",
                out: Ok(SelectPlan {
                    source: "*(*var1.field1).field2".to_string(),
                    plan: VecDeque::from(vec![
                        Operation::FindVariable("var1".to_string()),
                        Operation::GetField("field1".to_string()),
                        Operation::Deref,
                        Operation::GetField("field2".to_string()),
                        Operation::Deref,
                    ]),
                }),
            },
            TestCase {
                string: "*(*var1.field1[0][1]).field2",
                out: Ok(SelectPlan {
                    source: "*(*var1.field1[0][1]).field2".to_string(),
                    plan: VecDeque::from(vec![
                        Operation::FindVariable("var1".to_string()),
                        Operation::GetField("field1".to_string()),
                        Operation::GetByIndex(0),
                        Operation::GetByIndex(1),
                        Operation::Deref,
                        Operation::GetField("field2".to_string()),
                        Operation::Deref,
                    ]),
                }),
            },
            TestCase {
                string: "(var1.field1)",
                out: Ok(SelectPlan {
                    source: "(var1.field1)".to_string(),
                    plan: VecDeque::from(vec![
                        Operation::FindVariable("var1".to_string()),
                        Operation::GetField("field1".to_string()),
                    ]),
                }),
            },
            TestCase {
                string: "(var1).field1",
                out: Ok(SelectPlan {
                    source: "(var1).field1".to_string(),
                    plan: VecDeque::from(vec![
                        Operation::FindVariable("var1".to_string()),
                        Operation::GetField("field1".to_string()),
                    ]),
                }),
            },
            TestCase {
                string: "var1[..5]",
                out: Ok(SelectPlan {
                    source: "var1[..5]".to_string(),
                    plan: VecDeque::from(vec![
                        Operation::FindVariable("var1".to_string()),
                        Operation::Slice(5),
                    ]),
                }),
            },
            TestCase {
                string: "(var1.)field1",
                out: Err(ParseError::UnexpectedToken(Token::ClosedBracket)),
            },
            TestCase {
                string: "var1.",
                out: Err(ParseError::UnexpectedToken(Token::End)),
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
            let parser = SelectPlanParser::new(tc.string);
            let result = parser.parse();

            assert_eq!(result, tc.out);
        }
    }
}
