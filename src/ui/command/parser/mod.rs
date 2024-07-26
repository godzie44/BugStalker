pub mod expression;

use super::r#break::BreakpointIdentity;
use super::{frame, memory, r#async, register, source_code, thread, watch, Command, CommandError};
use super::{r#break, CommandResult};
use crate::debugger::register::debug::BreakCondition;
use crate::debugger::variable::dqe::Dqe;
use crate::debugger::variable::dqe::Selector;
use crate::ui::command::watch::WatchpointIdentity;
use ariadne::{Color, Fmt, Label, Report, ReportKind, Source};
use chumsky::error::{Rich, RichPattern, RichReason};
use chumsky::prelude::{any, choice, end, just, one_of};
use chumsky::text::{Char, whitespace};
use chumsky::{Boxed, Parser, extra, text};
use itertools::Itertools;

pub const VAR_COMMAND: &str = "var";
pub const VAR_LOCAL_KEY: &str = "locals";
pub const ARG_COMMAND: &str = "arg";
pub const ARG_ALL_KEY: &str = "all";
pub const BACKTRACE_COMMAND: &str = "backtrace";
pub const BACKTRACE_COMMAND_SHORT: &str = "bt";
pub const BACKTRACE_ALL_SUBCOMMAND: &str = "all";
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
pub const BREAK_REMOVE_SUBCOMMAND: &str = "remove";
pub const BREAK_REMOVE_SUBCOMMAND_SHORT: &str = "r";
pub const BREAK_INFO_SUBCOMMAND: &str = "info";
pub const WATCH_COMMAND: &str = "watch";
pub const WATCH_COMMAND_SHORT: &str = "w";
pub const WATCH_REMOVE_SUBCOMMAND: &str = "remove";
pub const WATCH_REMOVE_SUBCOMMAND_SHORT: &str = "r";
pub const WATCH_INFO_SUBCOMMAND: &str = "info";
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
pub const ASYNC_COMMAND: &str = "async";
pub const ASYNC_COMMAND_BACKTRACE_SUBCOMMAND: &str = "backtrace";
pub const ASYNC_COMMAND_BACKTRACE_SUBCOMMAND_SHORT: &str = "bt";
pub const ASYNC_COMMAND_TASK_SUBCOMMAND: &str = "task";
pub const HELP_COMMAND: &str = "help";
pub const HELP_COMMAND_SHORT: &str = "h";

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

pub fn watchpoint_cond<'a>() -> impl chumsky::Parser<'a, &'a str, BreakCondition, Err<'a>> {
    let ws_req = whitespace().at_least(1);
    let ws_req_or_end = ws_req.or(end());
    let op = |sym| whitespace().then(just(sym)).then(ws_req_or_end);
    op("+rw")
        .to(BreakCondition::DataReadsWrites)
        .or(op("+rw").to(BreakCondition::DataWrites))
        .or(text::whitespace().to(BreakCondition::DataWrites))
}

pub fn watchpoint_at_dqe<'a>() -> impl chumsky::Parser<'a, &'a str, WatchpointIdentity, Err<'a>> {
    let source_rewind_parser = any::<_, Err>().repeated().to_slice().rewind();
    source_rewind_parser
        .then(expression::parser().padded())
        .map(|(source, dqe)| WatchpointIdentity::DQE(source.trim().to_string(), dqe))
}

pub fn watchpoint_at_address<'a>() -> impl chumsky::Parser<'a, &'a str, WatchpointIdentity, Err<'a>>
{
    hex()
        .then(just(':').ignore_then(one_of("1248")))
        .padded()
        .map(|(addr, size)| {
            WatchpointIdentity::Address(addr, size.to_digit(10).expect("infallible") as u8)
        })
}

fn command<'a, I>(ctx: &'static str, inner: I) -> Boxed<'a, 'a, &'a str, Command, Err<'a>>
where
    I: chumsky::Parser<'a, &'a str, Command, Err<'a>> + 'a,
{
    inner.then_ignore(end()).labelled(ctx).boxed()
}

impl Command {
    pub fn render_errors(src: &str, errors: Vec<Rich<char>>) -> String {
        let mut reports = vec![];

        for e in errors {
            fn generate_reports(
                src: &str,
                reports: &mut Vec<String>,
                err: &Rich<char>,
                reason: &RichReason<char>,
            ) {
                let report = Report::build(ReportKind::Error, "<command>", err.span().start)
                    .with_help("try \"help\" command");

                let report = match reason {
                    RichReason::ExpectedFound { expected, found } => report
                        .with_message(format!(
                            "{}, expected {}",
                            if found.is_some() {
                                "unexpected token in input"
                            } else {
                                "unexpected end of input"
                            },
                            if expected.is_empty() {
                                "something else".to_string()
                            } else {
                                expected
                                    .iter()
                                    .map(|e| match e {
                                        RichPattern::Token(tok) => tok.to_string(),
                                        RichPattern::Label(label) => label.to_string(),
                                        RichPattern::EndOfInput => "end of input".to_string(),
                                    })
                                    .join(", ")
                            }
                        ))
                        .with_label(
                            Label::new(("<command>", err.span().into_range()))
                                .with_message(format!(
                                    "unexpected token {}",
                                    err.found()
                                        .map(|t| t.to_string())
                                        .unwrap_or("EOL".to_string())
                                        .fg(Color::Red)
                                ))
                                .with_color(Color::Red),
                        ),
                    RichReason::Custom(msg) => report.with_message(msg).with_label(
                        Label::new(("<command>", err.span().into_range()))
                            .with_message(format!("{}", msg.fg(Color::Red)))
                            .with_color(Color::Red),
                    ),
                    RichReason::Many(reasons) => {
                        for r in reasons {
                            generate_reports(src, reports, err, r);
                        }
                        return;
                    }
                };

                let mut buf = vec![];
                _ = report
                    .finish()
                    .write_for_stdout(("<command>", Source::from(&src)), &mut buf);
                reports.push(
                    std::str::from_utf8(&buf[..])
                        .expect("infallible")
                        .to_string(),
                );
            }

            generate_reports(src, &mut reports, &e, e.reason());
        }

        reports.join("\n")
    }

    fn parser<'a>() -> impl Parser<'a, &'a str, Command, Err<'a>> {
        let ws_req = whitespace().at_least(1);
        let ws_req_or_end = ws_req.or(end());
        let op = |sym| whitespace().then(just(sym)).then(ws_req_or_end);
        let op_w_arg = |sym| whitespace().then(just(sym)).then(ws_req);
        let sub_op = |sym| just(sym).then(ws_req_or_end);
        let sub_op_w_arg = |sym| just(sym).then(ws_req);

        let print_local_vars = op_w_arg(VAR_COMMAND)
            .then(sub_op(VAR_LOCAL_KEY))
            .map(|_| Command::PrintVariables(Dqe::Variable(Selector::Any)));
        let print_var = op_w_arg(VAR_COMMAND)
            .ignore_then(expression::parser())
            .map(Command::PrintVariables);

        let print_variables = choice((print_local_vars, print_var)).boxed();

        let print_all_args = op_w_arg(ARG_COMMAND)
            .then(sub_op(ARG_ALL_KEY))
            .map(|_| Command::PrintArguments(Dqe::Variable(Selector::Any)));
        let print_arg = op_w_arg(ARG_COMMAND)
            .ignore_then(expression::parser())
            .map(Command::PrintArguments);

        let print_arguments = choice((print_all_args, print_arg)).boxed();

        let op2 = |full, short| op(full).or(op(short));
        let op2_w_arg = |full, short| op_w_arg(full).or(op_w_arg(short));
        let sub_op2 = |full, short| sub_op(full).or(sub_op(short));
        let sub_op2_w_arg = |full, short| sub_op_w_arg(full).or(sub_op_w_arg(short));

        let r#continue = op2(CONTINUE_COMMAND, CONTINUE_COMMAND_SHORT).to(Command::Continue);
        let run = op2(RUN_COMMAND, RUN_COMMAND_SHORT).to(Command::Run);
        let stepi = op(STEP_INSTRUCTION_COMMAND).to(Command::StepInstruction);
        let step_into = op2(STEP_INTO_COMMAND, STEP_INTO_COMMAND_SHORT).to(Command::StepInto);
        let step_out = op2(STEP_OUT_COMMAND, STEP_OUT_COMMAND_SHORT).to(Command::StepOut);
        let step_over = op2(STEP_OVER_COMMAND, STEP_OVER_COMMAND_SHORT).to(Command::StepOver);

        let source_code = op_w_arg(SOURCE_COMMAND)
            .ignore_then(choice((
                sub_op(SOURCE_COMMAND_DISASM_SUBCOMMAND)
                    .to(Command::SourceCode(source_code::Command::Asm)),
                sub_op(SOURCE_COMMAND_FUNCTION_SUBCOMMAND)
                    .to(Command::SourceCode(source_code::Command::Function)),
                text::int(10)
                    .from_str()
                    .unwrapped()
                    .map(|num| Command::SourceCode(source_code::Command::Range(num)))
                    .padded(),
            )))
            .boxed();

        let help = op2(HELP_COMMAND, HELP_COMMAND_SHORT)
            .ignore_then(any().repeated().at_least(1).padded().to_slice().or_not())
            .map(|s| Command::Help {
                command: s.map(ToOwned::to_owned),
                reason: None,
            })
            .padded()
            .boxed();

        let backtrace = op2(BACKTRACE_COMMAND, BACKTRACE_COMMAND_SHORT)
            .ignore_then(sub_op(BACKTRACE_ALL_SUBCOMMAND).or_not())
            .map(|all| {
                if all.is_some() {
                    Command::PrintBacktrace(super::backtrace::Command::All)
                } else {
                    Command::PrintBacktrace(super::backtrace::Command::CurrentThread)
                }
            })
            .boxed();

        let symbol = op_w_arg(SYMBOL_COMMAND)
            .ignore_then(any().repeated().padded().to_slice())
            .map(|s| Command::PrintSymbol(s.trim().to_string()))
            .boxed();

        let r#break = op2_w_arg(BREAK_COMMAND, BREAK_COMMAND_SHORT)
            .ignore_then(choice((
                sub_op2_w_arg(BREAK_REMOVE_SUBCOMMAND, BREAK_REMOVE_SUBCOMMAND_SHORT)
                    .ignore_then(choice((
                        brkpt_at_addr_parser(),
                        brkpt_at_line_parser(),
                        brkpt_number(),
                        brkpt_at_fn(),
                    )))
                    .map(|brkpt| Command::Breakpoint(r#break::Command::Remove(brkpt))),
                sub_op(BREAK_INFO_SUBCOMMAND).to(Command::Breakpoint(r#break::Command::Info)),
                choice((
                    brkpt_at_addr_parser(),
                    brkpt_at_line_parser(),
                    brkpt_at_fn(),
                ))
                .map(|brkpt| Command::Breakpoint(r#break::Command::Add(brkpt))),
            )))
            .boxed();

        let watchpoint = op2_w_arg(WATCH_COMMAND, WATCH_COMMAND_SHORT)
            .ignore_then(choice((
                sub_op2_w_arg(WATCH_REMOVE_SUBCOMMAND, WATCH_REMOVE_SUBCOMMAND_SHORT)
                    .ignore_then(choice((
                        text::int(10)
                            .from_str()
                            .unwrapped()
                            .map(|number: u32| WatchpointIdentity::Number(number))
                            .padded(),
                        watchpoint_at_address(),
                        watchpoint_at_dqe(),
                    )))
                    .map(|ident| Command::Watchpoint(watch::Command::Remove(ident))),
                sub_op(BREAK_INFO_SUBCOMMAND).to(Command::Watchpoint(watch::Command::Info)),
                watchpoint_cond()
                    .then(watchpoint_at_address())
                    .map(|(cond, ident)| Command::Watchpoint(watch::Command::Add(ident, cond))),
                watchpoint_cond()
                    .then(watchpoint_at_dqe())
                    .map(|(cond, identity)| {
                        Command::Watchpoint(watch::Command::Add(identity, cond))
                    }),
            )))
            .boxed();

        let memory = op2_w_arg(MEMORY_COMMAND, MEMORY_COMMAND_SHORT)
            .ignore_then(choice((
                sub_op_w_arg(MEMORY_COMMAND_READ_SUBCOMMAND)
                    .ignore_then(hex())
                    .map(|addr| Command::Memory(memory::Command::Read(addr))),
                sub_op_w_arg(MEMORY_COMMAND_WRITE_SUBCOMMAND)
                    .ignore_then(hex().then(hex()))
                    .map(|(addr, val)| Command::Memory(memory::Command::Write(addr, val))),
            )))
            .boxed();

        let register = op2_w_arg(REGISTER_COMMAND, REGISTER_COMMAND_SHORT)
            .ignore_then(choice((
                sub_op(REGISTER_COMMAND_INFO_SUBCOMMAND)
                    .to(Command::Register(register::Command::Info)),
                sub_op_w_arg(REGISTER_COMMAND_READ_SUBCOMMAND)
                    .ignore_then(text::ident())
                    .map(|reg_name| {
                        Command::Register(register::Command::Read(reg_name.to_string()))
                    })
                    .padded(),
                sub_op_w_arg(REGISTER_COMMAND_WRITE_SUBCOMMAND)
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

        let thread = op_w_arg(THREAD_COMMAND)
            .ignore_then(choice((
                sub_op(THREAD_COMMAND_INFO_SUBCOMMAND).to(Command::Thread(thread::Command::Info)),
                sub_op(THREAD_COMMAND_CURRENT_SUBCOMMAND)
                    .to(Command::Thread(thread::Command::Current)),
                sub_op_w_arg(THREAD_COMMAND_SWITCH_SUBCOMMAND)
                    .ignore_then(text::int(10))
                    .from_str()
                    .unwrapped()
                    .map(|num| Command::Thread(thread::Command::Switch(num)))
                    .padded(),
            )))
            .boxed();

        let frame = op2_w_arg(FRAME_COMMAND, FRAME_COMMAND_SHORT)
            .ignore_then(choice((
                sub_op(FRAME_COMMAND_INFO_SUBCOMMAND).to(Command::Frame(frame::Command::Info)),
                sub_op(FRAME_COMMAND_SWITCH_SUBCOMMAND)
                    .ignore_then(text::int(10).from_str().unwrapped())
                    .map(|num| Command::Frame(frame::Command::Switch(num)))
                    .padded(),
            )))
            .boxed();

        let shared_lib = op_w_arg(SHARED_LIB_COMMAND)
            .then(sub_op(SHARED_LIB_COMMAND_INFO_SUBCOMMAND))
            .to(Command::SharedLib)
            .boxed();

        let r#async = op_w_arg(ASYNC_COMMAND)
            .ignore_then(choice((
                sub_op2(
                    ASYNC_COMMAND_BACKTRACE_SUBCOMMAND,
                    ASYNC_COMMAND_BACKTRACE_SUBCOMMAND_SHORT,
                )
                .ignore_then(sub_op(BACKTRACE_ALL_SUBCOMMAND).or_not())
                .map(|all| {
                    if all.is_some() {
                        Command::Async(r#async::Command::FullBacktrace)
                    } else {
                        Command::Async(r#async::Command::ShortBacktrace)
                    }
                }),
                sub_op(ASYNC_COMMAND_TASK_SUBCOMMAND)
                    .ignore_then(any().repeated().padded().to_slice())
                    .map(|s| {
                        let s = s.trim();
                        if s.is_empty() {
                            Command::Async(r#async::Command::CurrentTask(None))
                        } else {
                            Command::Async(r#async::Command::CurrentTask(Some(s.to_string())))
                        }
                    }),
            )))
            .boxed();

        let oracle = op_w_arg(ORACLE_COMMAND)
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
            command(WATCH_COMMAND, watchpoint),
            command(ASYNC_COMMAND, r#async),
        ))
    }

    /// Parse input string into command.
    pub fn parse(input: &str) -> CommandResult<Command> {
        Self::parser()
            .parse(input)
            .into_result()
            .map_err(|e| CommandError::Parsing(Self::render_errors(input, e)))
    }
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
                    Command::PrintVariables(Dqe::Variable(Selector::Any))
                ));
            },
        },
        TestCase {
            inputs: vec!["var **var1"],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::PrintVariables(Dqe::Deref(_))
                ));
            },
        },
        TestCase {
            inputs: vec!["var locals_var"],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::PrintVariables(Dqe::Variable(Selector::Name { var_name, .. })) if var_name == "locals_var"
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
            inputs: vec!["br", "arglocals"],
            command_matcher: |result| assert!(result.is_err()),
        },
        TestCase {
            inputs: vec!["arg all"],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::PrintArguments(Dqe::Variable(Selector::Any))
                ));
            },
        },
        TestCase {
            inputs: vec!["arg all_arg"],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::PrintArguments(Dqe::Variable(Selector::Name { var_name, .. })) if var_name == "all_arg"
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
            inputs: vec!["b rust_fn", "b info_rust"],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Breakpoint(r#break::Command::Add(BreakpointIdentity::Function(f))) if f == "rust_fn" || f == "info_rust"
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
            inputs: vec!["watch var1", "watch var1 ", "   w   var1   "],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Watchpoint(watch::Command::Add(
                        WatchpointIdentity::DQE(source, Dqe::Variable(Selector::Name {var_name, ..})), BreakCondition::DataWrites
                    )) if var_name == "var1" && source == "var1"
                ));
            },
        },
        TestCase {
            inputs: vec!["watch +rw var1", "watch +rw  var1 ", "   w  +rw  var1   "],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Watchpoint(watch::Command::Add(
                        WatchpointIdentity::DQE(source, Dqe::Variable(Selector::Name {var_name, ..})), BreakCondition::DataReadsWrites
                    )) if var_name == "var1" && source == "var1"
                ));
            },
        },
        TestCase {
            inputs: vec![
                "watch ns1::ns2::var1",
                "watch ns1::ns2::var1 ",
                "   w   ns1::ns2::var1   ",
            ],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Watchpoint(watch::Command::Add(
                        WatchpointIdentity::DQE(source, Dqe::Variable(Selector::Name {var_name, ..})), BreakCondition::DataWrites
                    )) if var_name == "ns1::ns2::var1" && source == "ns1::ns2::var1"
                ));
            },
        },
        TestCase {
            inputs: vec!["watch 0x123:4", "watch 0x123:4 ", "   w   0x123:4   "],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Watchpoint(watch::Command::Add(WatchpointIdentity::Address(addr, size), BreakCondition::DataWrites)) if addr == 0x123 && size == 4
                ));
            },
        },
        TestCase {
            inputs: vec!["watch info", "watch info ", "   w   info   "],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Watchpoint(watch::Command::Info)
                ));
            },
        },
        TestCase {
            inputs: vec!["watch r var1", "watch remove var1 ", "   w   r var1   "],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Watchpoint(watch::Command::Remove(WatchpointIdentity::DQE(source, Dqe::Variable(Selector::Name {var_name, ..})))) if var_name == "var1" && source == "var1"
                ));
            },
        },
        TestCase {
            inputs: vec!["watch r 2", "watch remove 2 ", "   w   r 2   "],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Watchpoint(watch::Command::Remove(WatchpointIdentity::Number(n))) if n == 2
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
            inputs: vec!["async backtrace", " async   bt  "],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Async(r#async::Command::ShortBacktrace)
                ));
            },
        },
        TestCase {
            inputs: vec!["async backtrace all", " async   bt  all "],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Async(r#async::Command::FullBacktrace)
                ));
            },
        },
        TestCase {
            inputs: vec!["async task", " async   task "],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Async(r#async::Command::CurrentTask(None))
                ));
            },
        },
        TestCase {
            inputs: vec!["async task abc.*", " async   task abc.*  "],
            command_matcher: |result| {
                assert!(matches!(
                    result.unwrap(),
                    Command::Async(r#async::Command::CurrentTask(Some(s))) if s == "abc.*"
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
