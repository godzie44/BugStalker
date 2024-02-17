pub mod expression;

use super::r#break::BreakpointIdentity;
use super::{frame, memory, register, source_code, thread, Command, CommandError};
use super::{r#break, CommandResult};
use crate::debugger::variable::select::{Expression, VariableSelector};
use anyhow::anyhow;
use std::u64;

pub const VAR_COMMAND: &str = "var";
pub const VAR_LOCAL_KEY: &str = "locals";
pub const ARG_COMMAND: &str = "arg";
pub const ARG_ALL_KEY: &str = "all";
pub const BACKTRACE_COMMAND: &str = "backtrace";
pub const BACKTRACE_COMMAND_SHORT: &str = "bt";
pub const CONTINUE_COMMAND: &str = "continue";
pub const CONTINUE_COMMAND_SHORT: &str = "c";
pub const FRAME_COMMAND: &str = "frame";
pub const FRAME_COMMAND_SHORT: &str = "f";
pub const FRAME_COMMAND_INFO_SUBCOMMAND: &str = "info";
pub const FRAME_COMMAND_SWITCH_SUBCOMMAND: &str = "switch";
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
pub const MEMORY_COMMAND_READ_SUBCOMMAND: &str = "read";
pub const MEMORY_COMMAND_WRITE_SUBCOMMAND: &str = "write";
pub const REGISTER_COMMAND: &str = "register";
pub const REGISTER_COMMAND_SHORT: &str = "reg";
pub const REGISTER_COMMAND_READ_SUBCOMMAND: &str = "read";
pub const REGISTER_COMMAND_WRITE_SUBCOMMAND: &str = "write";
pub const REGISTER_COMMAND_INFO_SUBCOMMAND: &str = "info";
pub const THREAD_COMMAND: &str = "thread";
pub const THREAD_COMMAND_INFO_SUBCOMMAND: &str = "info";
pub const THREAD_COMMAND_SWITCH_SUBCOMMAND: &str = "switch";
pub const THREAD_COMMAND_CURRENT_SUBCOMMAND: &str = "current";
pub const SHARED_LIB_COMMAND: &str = "sharedlib";
pub const SHARED_LIB_COMMAND_INFO_SUBCOMMAND: &str = "info";
pub const SOURCE_COMMAND: &str = "source";
pub const SOURCE_COMMAND_DISASM_SUBCOMMAND: &str = "asm";
pub const SOURCE_COMMAND_FUNCTION_SUBCOMMAND: &str = "fn";
pub const ORACLE_COMMAND: &str = "oracle";
pub const HELP_COMMAND: &str = "help";
pub const HELP_COMMAND_SHORT: &str = "h";

use chumsky::error::Rich;
use chumsky::prelude::{any, choice, end, just};
use chumsky::text::Char;
use chumsky::{extra, text, Boxed, Parser};

type Err<'a> = extra::Err<Rich<'a, char>>;

pub fn hex<'a>() -> impl chumsky::Parser<'a, &'a str, usize, Err<'a>> + Clone {
    let prefix = just("0x").or(just("0X"));
    prefix
        .ignore_then(
            text::digits(16)
                .at_least(1)
                .to_slice()
                .map(|s: &str| usize::from_str_radix(s, 16).unwrap()),
        )
        .padded()
        .labelled("hexidecimal number")
}

pub fn rust_identifier<'a>() -> impl chumsky::Parser<'a, &'a str, &'a str, Err<'a>> + Clone {
    text::ascii::ident()
        .separated_by(just("::"))
        .allow_leading()
        .at_least(1)
        .to_slice()
        .padded()
        .labelled("rust identifier")
}

pub fn brkpt_at_addr_parser<'a>() -> impl chumsky::Parser<'a, &'a str, BreakpointIdentity, Err<'a>>
{
    hex().map(BreakpointIdentity::Address)
}

pub fn brkpt_at_line_parser<'a>() -> impl chumsky::Parser<'a, &'a str, BreakpointIdentity, Err<'a>>
{
    any()
        .filter(|c: &char| c.to_char() != ':')
        .repeated()
        .to_slice()
        .then_ignore(just(':'))
        .then(text::int(10).from_str().unwrapped())
        .map(|(file, line): (&str, u64)| BreakpointIdentity::Line(file.trim().to_string(), line))
        .padded()
}

pub fn brkpt_number<'a>() -> impl chumsky::Parser<'a, &'a str, BreakpointIdentity, Err<'a>> {
    text::int(10)
        .from_str()
        .unwrapped()
        .map(|number: u32| BreakpointIdentity::Number(number))
        .padded()
}

pub fn brkpt_at_fn<'a>() -> impl chumsky::Parser<'a, &'a str, BreakpointIdentity, Err<'a>> {
    any()
        .repeated()
        .to_slice()
        .map(|fn_name: &str| BreakpointIdentity::Function(fn_name.trim().to_string()))
}

fn command<'a, I>(ctx: &'static str, inner: I) -> Boxed<'a, 'a, &'a str, Command, Err<'a>>
where
    I: chumsky::Parser<'a, &'a str, Command, Err<'a>> + 'a,
{
    inner.then_ignore(end()).labelled(ctx).boxed()
}

impl Command {
    /// Parse input string into command.
    pub fn parse(input: &str) -> CommandResult<Command> {
        Self::parser()
            .parse(input)
            .into_result()
            .map_err(|e| CommandError::Parsing(anyhow!("{}", e[0])))
    }

    fn parser<'a>() -> impl chumsky::Parser<'a, &'a str, Command, Err<'a>> {
        let op = |sym| just(sym).padded();

        let print_local_vars = op(VAR_COMMAND)
            .then(op(VAR_LOCAL_KEY))
            .map(|_| Command::PrintVariables(Expression::Variable(VariableSelector::Any)));
        let print_var = op(VAR_COMMAND)
            .ignore_then(expression::parser())
            .map(Command::PrintVariables);

        let print_variables = choice((print_local_vars, print_var)).boxed();

        let print_all_args = op(ARG_COMMAND)
            .then(op(ARG_ALL_KEY))
            .map(|_| Command::PrintArguments(Expression::Variable(VariableSelector::Any)));
        let print_arg = op(ARG_COMMAND)
            .ignore_then(expression::parser())
            .map(Command::PrintArguments);

        let print_arguments = choice((print_all_args, print_arg)).boxed();

        let op2 = |full, short| op(full).or(op(short));

        let r#continue = op2(CONTINUE_COMMAND, CONTINUE_COMMAND_SHORT).to(Command::Continue);
        let run = op2(RUN_COMMAND, RUN_COMMAND_SHORT).to(Command::Run);
        let stepi = op(STEP_INSTRUCTION_COMMAND).to(Command::StepInstruction);
        let step_into = op2(STEP_INTO_COMMAND, STEP_INTO_COMMAND_SHORT).to(Command::StepInto);
        let step_out = op2(STEP_OUT_COMMAND, STEP_OUT_COMMAND_SHORT).to(Command::StepOut);
        let step_over = op2(STEP_OVER_COMMAND, STEP_OVER_COMMAND_SHORT).to(Command::StepOver);

        let source_code = op(SOURCE_COMMAND)
            .ignore_then(choice((
                op(SOURCE_COMMAND_DISASM_SUBCOMMAND)
                    .to(Command::SourceCode(source_code::Command::Asm)),
                op(SOURCE_COMMAND_FUNCTION_SUBCOMMAND)
                    .to(Command::SourceCode(source_code::Command::Function)),
                text::int(10)
                    .from_str()
                    .unwrapped()
                    .map(|num| Command::SourceCode(source_code::Command::Range(num)))
                    .padded(),
            )))
            .boxed();

        let help = op2(HELP_COMMAND, HELP_COMMAND_SHORT)
            .ignore_then(text::ident().or_not())
            .map(|s| Command::Help {
                command: s.map(ToOwned::to_owned),
                reason: None,
            })
            .padded()
            .boxed();

        let backtrace = op2(BACKTRACE_COMMAND, BACKTRACE_COMMAND_SHORT)
            .ignore_then(op("all").or_not())
            .map(|all| {
                if all.is_some() {
                    Command::PrintBacktrace(super::backtrace::Command::All)
                } else {
                    Command::PrintBacktrace(super::backtrace::Command::CurrentThread)
                }
            })
            .boxed();

        let symbol = op(SYMBOL_COMMAND)
            .ignore_then(any().repeated().padded().to_slice())
            .map(|s| Command::PrintSymbol(s.trim().to_string()))
            .boxed();

        let r#break = op2(BREAK_COMMAND, BREAK_COMMAND_SHORT)
            .ignore_then(choice((
                op2("remove", "r")
                    .ignore_then(choice((
                        brkpt_at_addr_parser(),
                        brkpt_at_line_parser(),
                        brkpt_number(),
                        brkpt_at_fn(),
                    )))
                    .map(|brkpt| Command::Breakpoint(r#break::Command::Remove(brkpt))),
                op("info").to(Command::Breakpoint(r#break::Command::Info)),
                choice((
                    brkpt_at_addr_parser(),
                    brkpt_at_line_parser(),
                    brkpt_at_fn(),
                ))
                .map(|brkpt| Command::Breakpoint(r#break::Command::Add(brkpt))),
            )))
            .boxed();

        let memory = op2(MEMORY_COMMAND, MEMORY_COMMAND_SHORT)
            .ignore_then(choice((
                op(MEMORY_COMMAND_READ_SUBCOMMAND)
                    .ignore_then(hex())
                    .map(|addr| Command::Memory(memory::Command::Read(addr))),
                op(MEMORY_COMMAND_WRITE_SUBCOMMAND)
                    .ignore_then(hex().then(hex()))
                    .map(|(addr, val)| Command::Memory(memory::Command::Write(addr, val))),
            )))
            .boxed();

        let register = op2(REGISTER_COMMAND, REGISTER_COMMAND_SHORT)
            .ignore_then(choice((
                op(REGISTER_COMMAND_INFO_SUBCOMMAND).to(Command::Register(register::Command::Info)),
                op(REGISTER_COMMAND_READ_SUBCOMMAND)
                    .ignore_then(text::ident())
                    .map(|reg_name| {
                        Command::Register(register::Command::Read(reg_name.to_string()))
                    })
                    .padded(),
                op(REGISTER_COMMAND_WRITE_SUBCOMMAND)
                    .ignore_then(text::ident().then(hex()))
                    .map(|(reg_name, val)| {
                        Command::Register(register::Command::Write(
                            reg_name.to_string(),
                            val as u64,
                        ))
                    })
                    .padded(),
            )))
            .boxed();

        let thread = op(THREAD_COMMAND)
            .ignore_then(choice((
                op(THREAD_COMMAND_INFO_SUBCOMMAND).to(Command::Thread(thread::Command::Info)),
                op(THREAD_COMMAND_CURRENT_SUBCOMMAND).to(Command::Thread(thread::Command::Current)),
                op(THREAD_COMMAND_SWITCH_SUBCOMMAND)
                    .ignore_then(text::int(10))
                    .from_str()
                    .unwrapped()
                    .map(|num| Command::Thread(thread::Command::Switch(num)))
                    .padded(),
            )))
            .boxed();

        let frame = op2(FRAME_COMMAND, FRAME_COMMAND_SHORT)
            .ignore_then(choice((
                op(FRAME_COMMAND_INFO_SUBCOMMAND).to(Command::Frame(frame::Command::Info)),
                op("switch")
                    .ignore_then(text::int(10).from_str().unwrapped())
                    .map(|num| Command::Frame(frame::Command::Switch(num)))
                    .padded(),
            )))
            .boxed();

        let shared_lib = op(SHARED_LIB_COMMAND)
            .then(op(SHARED_LIB_COMMAND_INFO_SUBCOMMAND))
            .to(Command::SharedLib)
            .boxed();

        let oracle = op(ORACLE_COMMAND)
            .ignore_then(text::ident().padded().then(text::ident().or_not()))
            .map(|(name, subcmd)| {
                Command::Oracle(name.trim().to_string(), subcmd.map(ToString::to_string))
            })
            .padded()
            .boxed();

        choice((
            command(VAR_COMMAND, print_variables),
            command(ARG_COMMAND, print_arguments),
            command(CONTINUE_COMMAND, r#continue),
            command(RUN_COMMAND, run),
            command(STEP_INSTRUCTION_COMMAND, stepi),
            command(STEP_INTO_COMMAND, step_into),
            command(STEP_OUT_COMMAND, step_out),
            command(STEP_OVER_COMMAND, step_over),
            command(SOURCE_COMMAND, source_code),
            command(HELP_COMMAND, help),
            command(BACKTRACE_COMMAND, backtrace),
            command(SYMBOL_COMMAND, symbol),
            command(BREAK_COMMAND, r#break),
            command(MEMORY_COMMAND, memory),
            command(REGISTER_COMMAND, register),
            command(THREAD_COMMAND, thread),
            command(FRAME_COMMAND, frame),
            command(SHARED_LIB_COMMAND, shared_lib),
            command(ORACLE_COMMAND, oracle),
        ))
        .map_err(|e| {
            let span = e.span();
            if span.start == 0 && span.end == 0 {
                Rich::custom(*e.span(), "type help for list of commands")
            } else {
                e
            }
        })
    }

    // fn parse_inner(input: &str) -> IResult<&str, Command, ErrorTree<&str>> {
    //
    //
    //
    //
    //
    //
    //

    //
    //         alt((
    //             command(VAR_COMMAND, print_var_parser),
    //             command(ARG_COMMAND, print_argument_parser),
    //             command(BACKTRACE_COMMAND, backtrace_parser),
    //             command(CONTINUE_COMMAND, continue_parser),
    //             command(FRAME_COMMAND, frame_parser),
    //             command(RUN_COMMAND, run_parser),
    //             command(STEP_INSTRUCTION_COMMAND, stepi_parser),
    //             command(STEP_INTO_COMMAND, step_into_parser),
    //             command(STEP_OUT_COMMAND, step_out_parser),
    //             command(STEP_OVER_COMMAND, step_over_parser),
    //             command(SYMBOL_COMMAND, symbol_parser),
    //             command(BREAK_COMMAND, break_parser),
    //             command(MEMORY_COMMAND, memory_parser),
    //             command(REGISTER_COMMAND, register_parser),
    //             command(HELP_COMMAND, help_parser),
    //             command(THREAD_COMMAND, thread_parser),
    //             command(SHARED_LIB_COMMAND, shared_lib_parser),
    //             command(SOURCE_COMMAND, source_code_parser),
    //             command(ORACLE_COMMAND, oracle_parser),
    //             cut(map(not_line_ending, |cmd: &str| {
    //                 if cmd.is_empty() {
    //                     Command::SkipInput
    //                 } else {
    //                     Command::Help {
    //                         command: None,
    //                         reason: Some("unknown command".to_string()),
    //                     }
    //                 }
    //             })),
    //         ))(input)
    //     }
}

#[test]
fn test_hex_parser() {
    struct TestCase {
        string: &'static str,
        result: Result<usize, ()>,
    }
    let cases = vec![
        TestCase {
            string: "0x123AA",
            result: Ok(0x123aa_usize),
        },
        TestCase {
            string: "  0X123AA ",
            result: Ok(0x123aa_usize),
        },
        TestCase {
            string: "  0x 123AA ",
            result: Err(()),
        },
        TestCase {
            string: "  123AA ",
            result: Err(()),
        },
    ];

    for tc in cases {
        let expr = hex().parse(tc.string).into_result();
        assert_eq!(expr.map_err(|_| ()), tc.result);
    }
}

#[test]
fn test_rust_identifier_parser() {
    struct TestCase {
        string: &'static str,
        result: Result<&'static str, ()>,
    }
    let cases = vec![
        TestCase {
            string: "some_var",
            result: Ok("some_var"),
        },
        TestCase {
            string: "_some_var",
            result: Ok("_some_var"),
        },
        TestCase {
            string: "  _some_var ",
            result: Ok("_some_var"),
        },
        TestCase {
            string: "::aa::BB::_CC1",
            result: Ok("::aa::BB::_CC1"),
        },
        TestCase {
            string: "1a",
            result: Err(()),
        },
        TestCase {
            string: "aa::",
            result: Err(()),
        },
    ];

    for tc in cases {
        let expr = rust_identifier().parse(tc.string).into_result();
        assert_eq!(expr.map_err(|_| ()), tc.result);
    }
}

#[test]
fn test_parser() {
    struct TestCase {
        inputs: Vec<&'static str>,
        command_matcher: fn(result: Result<Command, CommandError>),
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
            inputs: vec!["das"],
            command_matcher: |result| assert!(result.is_err()),
        },
        TestCase {
            inputs: vec!["voo"],
            command_matcher: |result| assert!(result.is_err()),
        },
        TestCase {
            inputs: vec!["arg 11"],
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
                    Command::PrintBacktrace(super::backtrace::Command::CurrentThread)
                ));
            },
        },
        TestCase {
            inputs: vec!["bt all", "backtrace  all  "],
            command_matcher: |result| {
                let cmd = result.unwrap();
                assert!(matches!(
                    cmd,
                    Command::PrintBacktrace(super::backtrace::Command::All)
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
            inputs: vec!["frame info ", "  frame  info"],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Frame(frame::Command::Info)
                ));
            },
        },
        TestCase {
            inputs: vec!["f info ", "  f  info"],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Frame(frame::Command::Info)
                ));
            },
        },
        TestCase {
            inputs: vec!["frame switch 1", "  frame  switch   1 "],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Frame(frame::Command::Switch(1))
                ));
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
                    Command::Breakpoint(r#break::Command::Add(BreakpointIdentity::Function(f))) if f == "some_func"
                ));
            },
        },
        TestCase {
            inputs: vec!["b some_func<T1,T2>"],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Breakpoint(r#break::Command::Add(BreakpointIdentity::Function(f))) if f == "some_func<T1,T2>"
                ));
            },
        },
        TestCase {
            inputs: vec!["b some_struct<T1,T2>::some_func<T3,T4>"],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Breakpoint(r#break::Command::Add(BreakpointIdentity::Function(f))) if f == "some_struct<T1,T2>::some_func<T3,T4>"
                ));
            },
        },
        TestCase {
            inputs: vec!["b ns1::some_func", "break ns1::ns2::some_func"],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Breakpoint(r#break::Command::Add(BreakpointIdentity::Function(f))) if f == "ns1::some_func" || f == "ns1::ns2::some_func"
                ));
            },
        },
        TestCase {
            inputs: vec!["b file:123", "break file:123", "   break file:123   "],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Breakpoint(r#break::Command::Add(BreakpointIdentity::Line(f, n))) if f == "file" && n == 123
                ));
            },
        },
        TestCase {
            inputs: vec!["b 0x123", "break 0x123", "   break 0x0123   "],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Breakpoint(r#break::Command::Add(BreakpointIdentity::Address(a))) if a == 0x123
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
                    Command::Breakpoint(r#break::Command::Remove(BreakpointIdentity::Function(f))) if f == "some_func"
                ));
            },
        },
        TestCase {
            inputs: vec![
                "b r ns1::some_func",
                "break r ns1::some_func",
                "   break r  ns1::some_func   ",
            ],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Breakpoint(r#break::Command::Remove(BreakpointIdentity::Function(f))) if f == "ns1::some_func"
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
                    Command::Breakpoint(r#break::Command::Remove(BreakpointIdentity::Line(f, n))) if f == "file" && n == 123
                ));
            },
        },
        TestCase {
            inputs: vec!["b remove 0x123", "break r 0x123", "   break r 0x123   "],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Breakpoint(r#break::Command::Remove(BreakpointIdentity::Address(a))) if a == 0x123
                ));
            },
        },
        TestCase {
            inputs: vec!["b info", "break info ", "   break   info   "],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Breakpoint(r#break::Command::Info)
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
            inputs: vec!["reg info", "register info", "   reg  info "],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Register(register::Command::Info)
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
        TestCase {
            inputs: vec!["thread info", "thread    info  "],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Thread(thread::Command::Info)
                ));
            },
        },
        TestCase {
            inputs: vec!["thread current", "thread    current  "],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Thread(thread::Command::Current)
                ));
            },
        },
        TestCase {
            inputs: vec!["thread switch 1", " thread  switch 1  "],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Thread(thread::Command::Switch(1))
                ));
            },
        },
        TestCase {
            inputs: vec!["sharedlib info", " sharedlib     info  "],
            command_matcher: |result| {
                assert!(matches!(result.unwrap(), Command::SharedLib));
            },
        },
        TestCase {
            inputs: vec!["source asm", " source   asm  "],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::SourceCode(source_code::Command::Asm)
                ));
            },
        },
        TestCase {
            inputs: vec!["source fn", " source   fn  "],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::SourceCode(source_code::Command::Function)
                ));
            },
        },
        TestCase {
            inputs: vec!["source 12", " source   12  "],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::SourceCode(source_code::Command::Range(r)) if r == 12
                ));
            },
        },
        TestCase {
            inputs: vec!["oracle tokio", " oracle  tokio   "],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Oracle(name, None) if name == "tokio"
                ));
            },
        },
        TestCase {
            inputs: vec!["oracle tokio all ", " oracle  tokio   all"],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Oracle(name, Some(subcmd)) if name == "tokio" && subcmd == "all"
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
