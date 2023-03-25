mod arguments;
mod backtrace;
mod r#break;
mod r#continue;
pub mod expression;
mod frame;
mod memory;
mod register;
mod run;
mod step_instruction;
mod step_into;
mod step_out;
mod step_over;
mod symbol;
mod trace;
pub mod variables;

pub use arguments::Arguments;
pub use backtrace::Backtrace;
pub use frame::Frame;
pub use memory::Memory;
pub use r#break::Break;
pub use r#break::Breakpoint as BreakpointType;
pub use r#continue::Continue;
pub use register::Register;
pub use run::Run;
pub use step_instruction::StepI;
pub use step_into::StepInto;
pub use step_out::StepOut;
pub use step_over::StepOver;
pub use symbol::Symbol;
pub use trace::Trace;
pub use variables::Variables;

use crate::debugger::variable::select::{Expression, VariableSelector};
use anyhow::anyhow;
use nix::libc::uintptr_t;
use nom::branch::alt;
use nom::bytes::complete::is_not;
use nom::character::complete::{
    alpha1, alphanumeric1, char, digit1, multispace1, not_line_ending, one_of,
};
use nom::combinator::{cut, eof, map, map_res, not, recognize};
use nom::error::context;
use nom::multi::{many0, many0_count, many1};
use nom::sequence::{delimited, pair, preceded, separated_pair, terminated};
use nom::{IResult, Parser};
use nom_supreme::error::ErrorTree;
use nom_supreme::final_parser::Location;
use nom_supreme::tag::complete::tag;
use std::num::ParseIntError;
use std::str::FromStr;

fn hexadecimal(input: &str) -> IResult<&str, &str, ErrorTree<&str>> {
    preceded(
        alt((tag("0x"), tag("0X"))),
        recognize(many1(terminated(
            one_of("0123456789abcdefABCDEF"),
            many0(char('_')),
        ))),
    )(input)
}

pub fn rust_identifier(input: &str) -> IResult<&str, &str, ErrorTree<&str>> {
    recognize(pair(
        alt((alpha1, tag("_"))),
        many0_count(alt((alphanumeric1, tag("_")))),
    ))(input)
}

fn command<'a, F>(
    ctx: &'static str,
    inner: F,
) -> impl FnMut(&'a str) -> IResult<&'a str, Command, ErrorTree<&'a str>>
where
    F: Parser<&'a str, Command, ErrorTree<&'a str>>,
{
    context(
        ctx,
        delimited(
            many0(one_of(" \t\r\n")),
            inner,
            cut(preceded(many0(one_of(" \t\r\n")), eof)),
        ),
    )
}

/// External commands that can be processed by the debugger.
#[derive(Debug)]
pub enum Command {
    PrintVariables(Expression),
    PrintArguments(Expression),
    PrintBacktrace,
    Continue,
    PrintFrame,
    Run,
    StepInstruction,
    StepInto,
    StepOut,
    StepOver,
    PrintTrace,
    PrintSymbol(String),
    Breakpoint(BreakpointType),
    Memory(memory::Command),
    Register(register::Command),
    Help(Option<String>),
}

#[derive(thiserror::Error, Debug)]
pub enum HandlingError {
    #[error("malformed command (try `help command`):\n{0}")]
    Parser(anyhow::Error),
    #[error(transparent)]
    Debugger(#[from] anyhow::Error),
}

pub type HandleResult<T> = std::result::Result<T, HandlingError>;

impl Command {
    /// Parse input string into command.
    pub fn parse(input: &str) -> HandleResult<Command> {
        nom_supreme::final_parser::final_parser::<_, _, _, ErrorTree<Location>>(Self::parse_inner)(
            input,
        )
        .map_err(|e| HandlingError::Parser(anyhow!("{}", e)))
    }

    fn parse_inner(input: &str) -> IResult<&str, Command, ErrorTree<&str>> {
        fn print_var_parser(input: &str) -> IResult<&str, Command, ErrorTree<&str>> {
            alt((
                map(
                    preceded(tag("vars"), preceded(multispace1, tag("locals"))),
                    |_| Command::PrintVariables(Expression::Variable(VariableSelector::Any)),
                ),
                map(
                    preceded(tag("vars"), preceded(multispace1, cut(expression::expr))),
                    Command::PrintVariables,
                ),
            ))(input)
        }

        fn print_argument_parser(input: &str) -> IResult<&str, Command, ErrorTree<&str>> {
            alt((
                map(
                    preceded(tag("args"), preceded(multispace1, tag("all"))),
                    |_| Command::PrintArguments(Expression::Variable(VariableSelector::Any)),
                ),
                map(
                    preceded(tag("args"), preceded(multispace1, cut(expression::expr))),
                    Command::PrintArguments,
                ),
            ))(input)
        }

        macro_rules! parser1_no_args {
            ($tag: expr, $command: expr) => {
                map(preceded(tag($tag), not(alphanumeric1)), |_| $command)
            };
        }

        macro_rules! parser2_no_args {
            ($tag1: expr, $tag2: expr, $command: expr) => {
                map(
                    alt((
                        preceded(tag($tag1), not(alphanumeric1)),
                        preceded(tag($tag2), cut(not(alphanumeric1))),
                    )),
                    |_| $command,
                )
            };
        }

        let backtrace_parser = parser2_no_args!("bt", "backtrace", Command::PrintBacktrace);
        let continue_parser = parser2_no_args!("c", "continue", Command::Continue);
        let frame_parser = parser1_no_args!("frame", Command::PrintFrame);
        let run_parser = parser2_no_args!("r", "run", Command::Run);
        let stepi_parser = parser1_no_args!("stepi", Command::StepInstruction);
        let step_into_parser = parser2_no_args!("step", "stepinto", Command::StepInto);
        let step_out_parser = parser2_no_args!("finish", "stepout", Command::StepOut);
        let step_over_parser = parser2_no_args!("next", "stepover", Command::StepOver);
        let trace_parser = parser1_no_args!("trace", Command::PrintTrace);
        let help_parser = parser1_no_args!("help", Command::Help(None));

        fn symbol_parser(input: &str) -> IResult<&str, Command, ErrorTree<&str>> {
            map(
                preceded(tag("symbol"), preceded(multispace1, not_line_ending)),
                |s: &str| Command::PrintSymbol(s.trim().to_string()),
            )(input)
        }

        fn break_parser(input: &str) -> IResult<&str, Command, ErrorTree<&str>> {
            preceded(
                alt((pair(tag("b"), multispace1), pair(tag("break"), multispace1))),
                cut(alt((
                    map_res(hexadecimal, |hex| -> Result<Command, ParseIntError> {
                        let addr = usize::from_str_radix(hex, 16)?;
                        Ok(Command::Breakpoint(BreakpointType::Address(addr)))
                    }),
                    map_res(
                        separated_pair(is_not(":"), tag(":"), digit1),
                        |(file, line): (&str, &str)| -> Result<Command, ParseIntError> {
                            Ok(Command::Breakpoint(BreakpointType::Line(
                                file.trim().to_string(),
                                u64::from_str(line.trim())?,
                            )))
                        },
                    ),
                    map_res(
                        rust_identifier,
                        |fn_name: &str| -> Result<Command, ParseIntError> {
                            Ok(Command::Breakpoint(BreakpointType::Function(
                                fn_name.to_string(),
                            )))
                        },
                    ),
                ))),
            )(input)
        }

        fn memory_parser(input: &str) -> IResult<&str, Command, ErrorTree<&str>> {
            preceded(
                alt((
                    pair(tag("mem"), multispace1),
                    pair(tag("memory"), multispace1),
                )),
                cut(alt((
                    map_res(
                        preceded(tag("read"), preceded(multispace1, hexadecimal)),
                        |hex| -> Result<Command, ParseIntError> {
                            let addr = usize::from_str_radix(hex, 16)?;
                            Ok(Command::Memory(memory::Command::Read(addr)))
                        },
                    ),
                    map_res(
                        preceded(
                            tag("write"),
                            preceded(
                                multispace1,
                                separated_pair(hexadecimal, multispace1, hexadecimal),
                            ),
                        ),
                        |(addr, val): (&str, &str)| -> Result<Command, ParseIntError> {
                            Ok(Command::Memory(memory::Command::Write(
                                usize::from_str_radix(addr, 16)?,
                                uintptr_t::from_str_radix(val, 16)?,
                            )))
                        },
                    ),
                ))),
            )(input)
        }

        fn register_parser(input: &str) -> IResult<&str, Command, ErrorTree<&str>> {
            preceded(
                alt((
                    pair(tag("reg"), multispace1),
                    pair(tag("register"), multispace1),
                )),
                cut(alt((
                    map(preceded(tag("dump"), cut(not(alphanumeric1))), |_| {
                        Command::Register(register::Command::Dump)
                    }),
                    map(
                        preceded(tag("read"), preceded(multispace1, alphanumeric1)),
                        |reg_name: &str| {
                            Command::Register(register::Command::Read(reg_name.to_string()))
                        },
                    ),
                    map_res(
                        preceded(
                            tag("write"),
                            preceded(
                                multispace1,
                                separated_pair(alphanumeric1, multispace1, hexadecimal),
                            ),
                        ),
                        |(reg_name, val): (&str, &str)| -> Result<Command, ParseIntError> {
                            Ok(Command::Register(register::Command::Write(
                                reg_name.to_string(),
                                u64::from_str_radix(val, 16)?,
                            )))
                        },
                    ),
                ))),
            )(input)
        }

        alt((
            command("vars", print_var_parser),
            command("args", print_argument_parser),
            command("backtrace", backtrace_parser),
            command("continue", continue_parser),
            command("frame", frame_parser),
            command("run", run_parser),
            command("stepi", stepi_parser),
            command("stepinto", step_into_parser),
            command("stepout", step_out_parser),
            command("stepover", step_over_parser),
            command("trace", trace_parser),
            command("symbol", symbol_parser),
            command("break", break_parser),
            command("memory", memory_parser),
            command("register", register_parser),
            command("help", help_parser),
            cut(map(not_line_ending, |_| {
                Command::Help(Some("undefined command".to_string()))
            })),
        ))(input)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parser() {
        struct TestCase {
            inputs: Vec<&'static str>,
            command_matcher: fn(result: Result<Command, HandlingError>),
        }
        let cases = vec![
            TestCase {
                inputs: vec!["vars locals"],
                command_matcher: |result| {
                    assert!(matches!(
                        result.unwrap(),
                        Command::PrintVariables(Expression::Variable(VariableSelector::Any))
                    ));
                },
            },
            TestCase {
                inputs: vec!["vars **var1"],
                command_matcher: |result| {
                    assert!(matches!(
                        result.unwrap(),
                        Command::PrintVariables(Expression::Deref(_))
                    ));
                },
            },
            TestCase {
                inputs: vec!["vars ("],
                command_matcher: |result| assert!(result.is_err()),
            },
            TestCase {
                inputs: vec!["args all"],
                command_matcher: |result| {
                    assert!(matches!(
                        result.unwrap(),
                        Command::PrintArguments(Expression::Variable(VariableSelector::Any))
                    ));
                },
            },
            TestCase {
                inputs: vec!["bt", "backtrace"],
                command_matcher: |result| {
                    let cmd = result.unwrap();
                    assert!(matches!(cmd, Command::PrintBacktrace));
                },
            },
            TestCase {
                inputs: vec!["trace"],
                command_matcher: |result| {
                    let cmd = result.unwrap();
                    assert!(matches!(cmd, Command::PrintTrace));
                },
            },
            TestCase {
                inputs: vec!["c", "continue"],
                command_matcher: |result| {
                    assert!(matches!(result.unwrap(), Command::Continue));
                },
            },
            TestCase {
                inputs: vec!["frame"],
                command_matcher: |result| {
                    assert!(matches!(result.unwrap(), Command::PrintFrame));
                },
            },
            TestCase {
                inputs: vec!["r", "run"],
                command_matcher: |result| {
                    assert!(matches!(result.unwrap(), Command::Run));
                },
            },
            TestCase {
                inputs: vec!["symbol main", " symbol  main "],
                command_matcher: |result| {
                    assert!(matches!(result.unwrap(), Command::PrintSymbol(s) if s == "main"));
                },
            },
            TestCase {
                inputs: vec!["  stepi"],
                command_matcher: |result| {
                    assert!(matches!(result.unwrap(), Command::StepInstruction));
                },
            },
            TestCase {
                inputs: vec!["step", "stepinto"],
                command_matcher: |result| {
                    assert!(matches!(result.unwrap(), Command::StepInto));
                },
            },
            TestCase {
                inputs: vec!["finish", "stepout"],
                command_matcher: |result| {
                    assert!(matches!(result.unwrap(), Command::StepOut));
                },
            },
            TestCase {
                inputs: vec!["next", "stepover"],
                command_matcher: |result| {
                    assert!(matches!(result.unwrap(), Command::StepOver));
                },
            },
            TestCase {
                inputs: vec!["b some_func", "break some_func", "   break some_func   "],
                command_matcher: |result| {
                    assert!(matches!(
                        result.unwrap(),
                        Command::Breakpoint(BreakpointType::Function(f)) if f == "some_func"
                    ));
                },
            },
            TestCase {
                inputs: vec!["b file:123", "break file:123", "   break file:123   "],
                command_matcher: |result| {
                    assert!(matches!(
                        result.unwrap(),
                        Command::Breakpoint(BreakpointType::Line(f, n)) if f == "file" && n == 123
                    ));
                },
            },
            TestCase {
                inputs: vec!["b 0x123", "break 0x123", "   break 0x123   "],
                command_matcher: |result| {
                    assert!(matches!(
                        result.unwrap(),
                        Command::Breakpoint(BreakpointType::Address(a)) if a == 0x123
                    ));
                },
            },
            TestCase {
                inputs: vec![
                    "mem read 0x123",
                    "memory read 0x123",
                    "   mem read   0x123   ",
                ],
                command_matcher: |result| {
                    assert!(matches!(
                        result.unwrap(),
                        Command::Memory(memory::Command::Read(a)) if a == 0x123
                    ));
                },
            },
            TestCase {
                inputs: vec![
                    "mem write 0x123 0x321",
                    "memory write 0x123 0x321",
                    "   mem write   0x123  0x321 ",
                ],
                command_matcher: |result| {
                    assert!(matches!(
                        result.unwrap(),
                        Command::Memory(memory::Command::Write(a, v)) if a == 0x123 && v == 0x321
                    ));
                },
            },
            TestCase {
                inputs: vec!["reg dump", "register dump", "   reg  dump "],
                command_matcher: |result| {
                    assert!(matches!(
                        result.unwrap(),
                        Command::Register(register::Command::Dump)
                    ));
                },
            },
            TestCase {
                inputs: vec!["reg read rip", "register read rip", "   reg  read   rip "],
                command_matcher: |result| {
                    assert!(matches!(
                        result.unwrap(),
                        Command::Register(register::Command::Read(r)) if r == "rip"
                    ));
                },
            },
            TestCase {
                inputs: vec![
                    "reg write rip 0x123",
                    "register write rip 0x123",
                    "   reg  write  rip  0x123 ",
                ],
                command_matcher: |result| {
                    assert!(matches!(
                        result.unwrap(),
                        Command::Register(register::Command::Write(r, v)) if r == "rip" && v == 0x123
                    ));
                },
            },
        ];

        for case in cases {
            for input in case.inputs {
                let result = Command::parse(input);
                (case.command_matcher)(result);
            }
        }
    }
}
