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
pub mod variables;

pub use arguments::Arguments;
pub use backtrace::Backtrace;
pub use backtrace::Command as BacktraceCommand;
pub use frame::Frame;
pub use memory::Memory;
pub use r#break::Break;
pub use r#break::Breakpoint;
pub use r#break::Command as BreakpointCommand;
pub use r#break::HandlingResult as BreakpointHandlingResult;
pub use r#continue::Continue;
pub use register::Register;
pub use run::Run;
pub use step_instruction::StepI;
pub use step_into::StepInto;
pub use step_out::StepOut;
pub use step_over::StepOver;
pub use symbol::Symbol;
pub use variables::Variables;

use crate::debugger::variable::select::{Expression, VariableSelector};
use anyhow::anyhow;
use nix::libc::uintptr_t;
use nom::branch::alt;
use nom::bytes::complete::is_not;
use nom::character::complete::{
    alpha1, alphanumeric1, char, digit1, multispace1, not_line_ending, one_of,
};
use nom::combinator::{cut, eof, map, map_res, not, opt, recognize};
use nom::error::context;
use nom::multi::{many0, many0_count, many1};
use nom::sequence::{delimited, pair, preceded, separated_pair, terminated};
use nom::{IResult, Parser};
use nom_supreme::error::ErrorTree;
use nom_supreme::final_parser::Location;
use nom_supreme::tag::complete::tag;
use std::num::ParseIntError;
use std::str::FromStr;

pub const VAR_COMMAND: &str = "var";
pub const VAR_LOCAL_KEY: &str = "locals";
pub const ARG_COMMAND: &str = "arg";
pub const ARG_ALL_KEY: &str = "all";
pub const BACKTRACE_COMMAND: &str = "backtrace";
pub const BACKTRACE_COMMAND_SHORT: &str = "bt";
pub const CONTINUE_COMMAND: &str = "continue";
pub const CONTINUE_COMMAND_SHORT: &str = "c";
pub const FRAME_COMMAND: &str = "frame";
pub const RUN_COMMAND: &str = "run";
pub const RUN_COMMAND_SHORT: &str = "r";
pub const STEP_INSTRUCTION_COMMAND: &str = "stepi";
pub const STEP_INTO_COMMAND: &str = "stepinto";
pub const STEP_INTO_COMMAND_SHORT: &str = "step";
pub const STEP_OUT_COMMAND: &str = "stepout";
pub const STEP_OUT_COMMAND_SHORT: &str = "finish";
pub const STEP_OVER_COMMAND: &str = "stepover";
pub const STEP_OVER_COMMAND_SHORT: &str = "next";
pub const SYMBOL_COMMAND: &str = "symbol";
pub const BREAK_COMMAND: &str = "break";
pub const BREAK_COMMAND_SHORT: &str = "b";
pub const MEMORY_COMMAND: &str = "memory";
pub const MEMORY_COMMAND_SHORT: &str = "mem";
pub const REGISTER_COMMAND: &str = "register";
pub const REGISTER_COMMAND_SHORT: &str = "reg";
pub const HELP_COMMAND: &str = "help";
pub const HELP_COMMAND_SHORT: &str = "h";

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
    PrintBacktrace(backtrace::Command),
    Continue,
    PrintFrame,
    Run,
    StepInstruction,
    StepInto,
    StepOut,
    StepOver,
    PrintSymbol(String),
    Breakpoint(r#break::Command),
    Memory(memory::Command),
    Register(register::Command),
    Help {
        command: Option<String>,
        reason: Option<String>,
    },
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
                    preceded(tag(VAR_COMMAND), preceded(multispace1, tag(VAR_LOCAL_KEY))),
                    |_| Command::PrintVariables(Expression::Variable(VariableSelector::Any)),
                ),
                map(
                    preceded(
                        tag(VAR_COMMAND),
                        preceded(multispace1, cut(expression::expr)),
                    ),
                    Command::PrintVariables,
                ),
            ))(input)
        }

        fn print_argument_parser(input: &str) -> IResult<&str, Command, ErrorTree<&str>> {
            alt((
                map(
                    preceded(tag(ARG_COMMAND), preceded(multispace1, tag(ARG_ALL_KEY))),
                    |_| Command::PrintArguments(Expression::Variable(VariableSelector::Any)),
                ),
                map(
                    preceded(
                        tag(ARG_COMMAND),
                        preceded(multispace1, cut(expression::expr)),
                    ),
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

        let continue_parser =
            parser2_no_args!(CONTINUE_COMMAND_SHORT, CONTINUE_COMMAND, Command::Continue);
        let frame_parser = parser1_no_args!(FRAME_COMMAND, Command::PrintFrame);
        let run_parser = parser2_no_args!(RUN_COMMAND_SHORT, RUN_COMMAND, Command::Run);
        let stepi_parser = parser1_no_args!(STEP_INSTRUCTION_COMMAND, Command::StepInstruction);
        let step_into_parser = parser2_no_args!(
            STEP_INTO_COMMAND_SHORT,
            STEP_INTO_COMMAND,
            Command::StepInto
        );
        let step_out_parser =
            parser2_no_args!(STEP_OUT_COMMAND_SHORT, STEP_OUT_COMMAND, Command::StepOut);
        let step_over_parser = parser2_no_args!(
            STEP_OVER_COMMAND_SHORT,
            STEP_OVER_COMMAND,
            Command::StepOver
        );

        fn help_parser(input: &str) -> IResult<&str, Command, ErrorTree<&str>> {
            map(
                preceded(
                    alt((tag(HELP_COMMAND), tag(HELP_COMMAND_SHORT))),
                    opt(preceded(multispace1, not_line_ending)),
                ),
                |s: Option<&str>| Command::Help {
                    command: s.map(ToOwned::to_owned),
                    reason: None,
                },
            )(input)
        }

        fn backtrace_parser(input: &str) -> IResult<&str, Command, ErrorTree<&str>> {
            alt((
                map(
                    preceded(
                        alt((tag(BACKTRACE_COMMAND_SHORT), tag(BACKTRACE_COMMAND))),
                        preceded(multispace1, tag("all")),
                    ),
                    |_| Command::PrintBacktrace(backtrace::Command::All),
                ),
                map(
                    preceded(
                        alt((tag(BACKTRACE_COMMAND_SHORT), tag(BACKTRACE_COMMAND))),
                        cut(not(alphanumeric1)),
                    ),
                    |_| Command::PrintBacktrace(backtrace::Command::CurrentThread),
                ),
            ))(input)
        }

        fn symbol_parser(input: &str) -> IResult<&str, Command, ErrorTree<&str>> {
            map(
                preceded(tag(SYMBOL_COMMAND), preceded(multispace1, not_line_ending)),
                |s: &str| Command::PrintSymbol(s.trim().to_string()),
            )(input)
        }

        fn break_parser(input: &str) -> IResult<&str, Command, ErrorTree<&str>> {
            fn breakpoint_arg_parser<'a>(
            ) -> impl FnMut(&'a str) -> IResult<&'a str, Breakpoint, ErrorTree<&str>> {
                alt((
                    map_res(hexadecimal, |hex| -> Result<Breakpoint, ParseIntError> {
                        let addr = usize::from_str_radix(hex, 16)?;
                        Ok(Breakpoint::Address(addr))
                    }),
                    map_res(
                        separated_pair(is_not(":"), tag(":"), digit1),
                        |(file, line): (&str, &str)| -> Result<Breakpoint, ParseIntError> {
                            Ok(Breakpoint::Line(
                                file.trim().to_string(),
                                u64::from_str(line.trim())?,
                            ))
                        },
                    ),
                    map_res(
                        rust_identifier,
                        |fn_name: &str| -> Result<Breakpoint, ParseIntError> {
                            Ok(Breakpoint::Function(fn_name.to_string()))
                        },
                    ),
                ))
            }

            preceded(
                alt((
                    pair(tag(BREAK_COMMAND_SHORT), multispace1),
                    pair(tag(BREAK_COMMAND), multispace1),
                )),
                cut(alt((
                    preceded(
                        alt((
                            pair(tag("r"), multispace1),
                            pair(tag("remove"), multispace1),
                        )),
                        map(breakpoint_arg_parser(), |brkpt| {
                            Command::Breakpoint(BreakpointCommand::Remove(brkpt))
                        }),
                    ),
                    map(tag("dump"), |_| {
                        Command::Breakpoint(BreakpointCommand::Dump)
                    }),
                    map(breakpoint_arg_parser(), |brkpt| {
                        Command::Breakpoint(BreakpointCommand::Add(brkpt))
                    }),
                ))),
            )(input)
        }

        fn memory_parser(input: &str) -> IResult<&str, Command, ErrorTree<&str>> {
            preceded(
                alt((
                    pair(tag(MEMORY_COMMAND_SHORT), multispace1),
                    pair(tag(MEMORY_COMMAND), multispace1),
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
                    pair(tag(REGISTER_COMMAND_SHORT), multispace1),
                    pair(tag(REGISTER_COMMAND), multispace1),
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
            command(VAR_COMMAND, print_var_parser),
            command(ARG_COMMAND, print_argument_parser),
            command(BACKTRACE_COMMAND, backtrace_parser),
            command(CONTINUE_COMMAND, continue_parser),
            command(FRAME_COMMAND, frame_parser),
            command(RUN_COMMAND, run_parser),
            command(STEP_INSTRUCTION_COMMAND, stepi_parser),
            command(STEP_INTO_COMMAND, step_into_parser),
            command(STEP_OUT_COMMAND, step_out_parser),
            command(STEP_OVER_COMMAND, step_over_parser),
            command(SYMBOL_COMMAND, symbol_parser),
            command(BREAK_COMMAND, break_parser),
            command(MEMORY_COMMAND, memory_parser),
            command(REGISTER_COMMAND, register_parser),
            command(HELP_COMMAND, help_parser),
            cut(map(not_line_ending, |_| Command::Help {
                command: None,
                reason: Some("undefined command".to_string()),
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
                inputs: vec!["var locals"],
                command_matcher: |result| {
                    assert!(matches!(
                        result.unwrap(),
                        Command::PrintVariables(Expression::Variable(VariableSelector::Any))
                    ));
                },
            },
            TestCase {
                inputs: vec!["var **var1"],
                command_matcher: |result| {
                    assert!(matches!(
                        result.unwrap(),
                        Command::PrintVariables(Expression::Deref(_))
                    ));
                },
            },
            TestCase {
                inputs: vec!["var ("],
                command_matcher: |result| assert!(result.is_err()),
            },
            TestCase {
                inputs: vec!["arg all"],
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
                    assert!(matches!(
                        cmd,
                        Command::PrintBacktrace(backtrace::Command::CurrentThread)
                    ));
                },
            },
            TestCase {
                inputs: vec!["bt all", "backtrace  all  "],
                command_matcher: |result| {
                    let cmd = result.unwrap();
                    assert!(matches!(
                        cmd,
                        Command::PrintBacktrace(backtrace::Command::All)
                    ));
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
                        Command::Breakpoint(r#break::Command::Add(Breakpoint::Function(f))) if f == "some_func"
                    ));
                },
            },
            TestCase {
                inputs: vec!["b file:123", "break file:123", "   break file:123   "],
                command_matcher: |result| {
                    assert!(matches!(
                        result.unwrap(),
                        Command::Breakpoint(r#break::Command::Add(Breakpoint::Line(f, n))) if f == "file" && n == 123
                    ));
                },
            },
            TestCase {
                inputs: vec!["b 0x123", "break 0x123", "   break 0x123   "],
                command_matcher: |result| {
                    assert!(matches!(
                        result.unwrap(),
                        Command::Breakpoint(r#break::Command::Add(Breakpoint::Address(a))) if a == 0x123
                    ));
                },
            },
            TestCase {
                inputs: vec![
                    "b r some_func",
                    "break r some_func",
                    "   break r  some_func   ",
                ],
                command_matcher: |result| {
                    assert!(matches!(
                        result.unwrap(),
                        Command::Breakpoint(r#break::Command::Remove(Breakpoint::Function(f))) if f == "some_func"
                    ));
                },
            },
            TestCase {
                inputs: vec![
                    "b remove file:123",
                    "break r file:123",
                    "   break  remove file:123   ",
                ],
                command_matcher: |result| {
                    assert!(matches!(
                        result.unwrap(),
                        Command::Breakpoint(r#break::Command::Remove(Breakpoint::Line(f, n))) if f == "file" && n == 123
                    ));
                },
            },
            TestCase {
                inputs: vec!["b remove 0x123", "break r 0x123", "   break r 0x123   "],
                command_matcher: |result| {
                    assert!(matches!(
                        result.unwrap(),
                        Command::Breakpoint(r#break::Command::Remove(Breakpoint::Address(a))) if a == 0x123
                    ));
                },
            },
            TestCase {
                inputs: vec!["b dump", "break dump ", "   break   dump   "],
                command_matcher: |result| {
                    assert!(matches!(
                        result.unwrap(),
                        Command::Breakpoint(r#break::Command::Dump)
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
