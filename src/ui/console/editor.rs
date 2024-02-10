use crate::ui::command::parser::{
    ARG_ALL_KEY, ARG_COMMAND, BACKTRACE_COMMAND, BACKTRACE_COMMAND_SHORT, BREAK_COMMAND,
    BREAK_COMMAND_SHORT, CONTINUE_COMMAND, CONTINUE_COMMAND_SHORT, FRAME_COMMAND,
    FRAME_COMMAND_INFO_SUBCOMMAND, FRAME_COMMAND_SWITCH_SUBCOMMAND, HELP_COMMAND,
    HELP_COMMAND_SHORT, MEMORY_COMMAND, MEMORY_COMMAND_READ_SUBCOMMAND, MEMORY_COMMAND_SHORT,
    MEMORY_COMMAND_WRITE_SUBCOMMAND, ORACLE_COMMAND, REGISTER_COMMAND,
    REGISTER_COMMAND_INFO_SUBCOMMAND, REGISTER_COMMAND_READ_SUBCOMMAND, REGISTER_COMMAND_SHORT,
    REGISTER_COMMAND_WRITE_SUBCOMMAND, RUN_COMMAND, RUN_COMMAND_SHORT, SHARED_LIB_COMMAND,
    SHARED_LIB_COMMAND_INFO_SUBCOMMAND, SOURCE_COMMAND, SOURCE_COMMAND_DISASM_SUBCOMMAND,
    SOURCE_COMMAND_FUNCTION_SUBCOMMAND, STEP_INSTRUCTION_COMMAND, STEP_INTO_COMMAND,
    STEP_INTO_COMMAND_SHORT, STEP_OUT_COMMAND, STEP_OUT_COMMAND_SHORT, STEP_OVER_COMMAND,
    STEP_OVER_COMMAND_SHORT, SYMBOL_COMMAND, THREAD_COMMAND, THREAD_COMMAND_CURRENT_SUBCOMMAND,
    THREAD_COMMAND_INFO_SUBCOMMAND, THREAD_COMMAND_SWITCH_SUBCOMMAND, VAR_COMMAND, VAR_LOCAL_KEY,
};
use crossterm::style::{Color, Stylize};
use nom::branch::alt;
use nom::character::complete::{alpha1, multispace1, not_line_ending};
use nom::combinator::{map, opt};
use nom::sequence::{preceded, separated_pair};
use nom_supreme::error::ErrorTree;
use nom_supreme::final_parser::Location;
use nom_supreme::tag::complete::tag;
use rustyline::completion::{Completer, Pair};
use rustyline::highlight::{Highlighter, MatchingBracketHighlighter};
use rustyline::hint::HistoryHinter;
use rustyline::history::MemHistory;
use rustyline::line_buffer::LineBuffer;
use rustyline::validate::MatchingBracketValidator;
use rustyline::{Changeset, CompletionType, Config, Context, Editor};
use rustyline_derive::{Helper, Hinter, Validator};
use std::borrow::Cow;
use std::borrow::Cow::{Borrowed, Owned};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use trie_rs::{Trie, TrieBuilder};

struct CommandHint {
    short: Option<String>,
    long: String,
    subcommands: Vec<String>,
}

impl CommandHint {
    fn long(&self) -> String {
        self.long.clone()
    }

    fn display_with_short(&self) -> String {
        if let Some(ref short) = self.short {
            if self.long.starts_with(short) {
                format!(
                    "{}{}",
                    short.clone().bold().underlined(),
                    &self.long[short.len()..]
                )
            } else {
                format!("{}|{}", &self.long, short.clone().bold().underlined())
            }
        } else {
            self.long()
        }
    }
}

impl From<&str> for CommandHint {
    fn from(value: &str) -> Self {
        CommandHint {
            short: None,
            long: value.to_string(),
            subcommands: vec![],
        }
    }
}

impl From<(&str, &str)> for CommandHint {
    fn from((short, long): (&str, &str)) -> Self {
        CommandHint {
            short: Some(short.to_string()),
            long: long.to_string(),
            subcommands: vec![],
        }
    }
}

pub struct CommandCompleter {
    commands: Vec<CommandHint>,
    subcommand_hints: HashMap<String, Vec<String>>,
    file_hints: Trie<u8>,
    var_hints: Trie<u8>,
    vars: Vec<String>,
    arg_hints: Trie<u8>,
    args: Vec<String>,
}

impl CommandCompleter {
    fn new(commands: impl IntoIterator<Item = CommandHint>) -> Self {
        let commands: Vec<CommandHint> = commands.into_iter().collect();
        let subcommand_hints = commands
            .iter()
            .flat_map(|cmd| {
                let mut hints = vec![(cmd.long.clone(), cmd.subcommands.clone())];
                if let Some(ref short) = cmd.short {
                    hints.push((short.clone(), cmd.subcommands.clone()));
                }
                hints
            })
            .collect::<HashMap<String, Vec<String>>>();

        Self {
            commands,
            subcommand_hints,
            file_hints: TrieBuilder::new().build(),
            var_hints: TrieBuilder::new().build(),
            arg_hints: TrieBuilder::new().build(),
            args: vec![],
            vars: vec![],
        }
    }

    pub fn replace_file_hints(&mut self, files: impl IntoIterator<Item = PathBuf>) {
        let mut builder = TrieBuilder::new();
        files.into_iter().for_each(|path: PathBuf| {
            let file_name = path
                .file_name()
                .and_then(|oss| oss.to_str())
                .unwrap_or_default()
                .to_owned();
            builder.push(file_name);
        });
        self.file_hints = builder.build();
    }

    pub fn replace_local_var_hints(&mut self, variables: impl IntoIterator<Item = String>) {
        let mut builder = TrieBuilder::new();
        self.vars = variables.into_iter().collect();
        self.vars.push(VAR_LOCAL_KEY.underlined().to_string());
        self.vars.iter().for_each(|var| {
            builder.push(var);
        });
        self.var_hints = builder.build();
    }

    pub fn replace_arg_hints(&mut self, args: impl IntoIterator<Item = String>) {
        let mut builder = TrieBuilder::new();
        self.args = args.into_iter().collect();
        self.vars.push(ARG_ALL_KEY.underlined().to_string());
        self.args.iter().for_each(|arg| {
            builder.push(arg);
        });
        self.arg_hints = builder.build();
    }
}

enum CompletableCommand<'a> {
    Breakpoint(&'a str),
    PrintVariables(&'a str),
    PrintArguments(&'a str),
    Unrecognized(&'a str, Option<&'a str>),
}

impl<'a> CompletableCommand<'a> {
    fn recognize(line: &'a str) -> anyhow::Result<CompletableCommand> {
        let bp_parser = map(
            preceded(
                alt((tag(BREAK_COMMAND), tag(BREAK_COMMAND_SHORT))),
                preceded(multispace1, not_line_ending),
            ),
            CompletableCommand::Breakpoint,
        );

        let var_parser = map(
            preceded(tag(VAR_COMMAND), preceded(multispace1, not_line_ending)),
            CompletableCommand::PrintVariables,
        );

        let arg_parser = map(
            preceded(tag(ARG_COMMAND), preceded(multispace1, not_line_ending)),
            CompletableCommand::PrintArguments,
        );

        let other_parser = map(
            separated_pair(alpha1, multispace1, opt(alpha1)),
            |(s1, s2)| CompletableCommand::Unrecognized(s1, s2),
        );

        Ok(nom_supreme::final_parser::final_parser::<
            _,
            _,
            ErrorTree<&str>,
            ErrorTree<Location>,
        >(alt((
            bp_parser,
            var_parser,
            arg_parser,
            other_parser,
        )))(line)?)
    }
}

impl Completer for CommandCompleter {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        _pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {
        fn pairs_from_variants(
            variants: impl Iterator<Item = impl ToString>,
            line: &str,
            tpl: &str,
            replacement_suffix: &str,
        ) -> (usize, Vec<Pair>) {
            let pos = line.len() - tpl.len();
            let pairs = variants.map(|v| Pair {
                display: v.to_string(),
                replacement: v.to_string() + replacement_suffix,
            });
            (pos, pairs.collect())
        }

        match CompletableCommand::recognize(line) {
            Ok(CompletableCommand::Breakpoint(maybe_file)) => {
                if maybe_file.trim().is_empty() {
                    return Ok((0, vec![]));
                }

                let variants = self.file_hints.predictive_search(maybe_file);
                if !variants.is_empty() {
                    let variants_iter = variants.iter().map(|var| {
                        std::str::from_utf8(var.as_slice()).expect("invalid utf-8 string")
                    });
                    return Ok(pairs_from_variants(variants_iter, line, maybe_file, ":"));
                }
            }
            Ok(CompletableCommand::PrintVariables(maybe_var)) => {
                if maybe_var.trim().is_empty() {
                    return Ok(pairs_from_variants(self.vars.iter(), line, maybe_var, ""));
                }

                let variants = self.var_hints.predictive_search(maybe_var);
                if !variants.is_empty() {
                    let variants_iter = variants.iter().map(|var| {
                        std::str::from_utf8(var.as_slice()).expect("invalid utf-8 string")
                    });
                    return Ok(pairs_from_variants(variants_iter, line, maybe_var, ""));
                }
            }
            Ok(CompletableCommand::PrintArguments(maybe_arg)) => {
                if maybe_arg.trim().is_empty() {
                    return Ok(pairs_from_variants(self.args.iter(), line, maybe_arg, ""));
                }

                let variants = self.arg_hints.predictive_search(maybe_arg);
                if !variants.is_empty() {
                    let variants_iter = variants.iter().map(|var| {
                        std::str::from_utf8(var.as_slice()).expect("invalid utf-8 string")
                    });
                    return Ok(pairs_from_variants(variants_iter, line, maybe_arg, ""));
                }
            }
            Ok(CompletableCommand::Unrecognized(cmd, mb_subcmd_part)) => {
                if let Some(subcommands) = self.subcommand_hints.get(cmd) {
                    let pos = cmd.len() + 1;
                    let subcmd_part = mb_subcmd_part.unwrap_or_default();
                    let subcommands = subcommands
                        .iter()
                        .filter(|&subcmd| subcmd.starts_with(subcmd_part))
                        .map(|subcmd| Pair {
                            display: subcmd.to_string(),
                            replacement: subcmd.to_string(),
                        })
                        .collect();

                    return Ok((pos, subcommands));
                }
            }
            _ => {}
        }

        let pairs = self
            .commands
            .iter()
            .filter(|&cmd| cmd.long.starts_with(line))
            .map(|cmd| Pair {
                display: cmd.display_with_short(),
                replacement: cmd.long(),
            })
            .collect();
        Ok((0, pairs))
    }
}

#[derive(Helper, Hinter, Validator)]
pub struct RLHelper {
    pub completer: Arc<Mutex<CommandCompleter>>,
    highlighter: MatchingBracketHighlighter,
    #[rustyline(Validator)]
    validator: MatchingBracketValidator,
    #[rustyline(Hinter)]
    hinter: HistoryHinter,
    pub colored_prompt: String,
}

impl Completer for RLHelper {
    type Candidate = <CommandCompleter as Completer>::Candidate;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {
        self.completer.lock().unwrap().complete(line, pos, ctx)
    }

    fn update(&self, line: &mut LineBuffer, start: usize, elected: &str, cl: &mut Changeset) {
        self.completer
            .lock()
            .unwrap()
            .update(line, start, elected, cl)
    }
}

impl Highlighter for RLHelper {
    fn highlight<'l>(&self, line: &'l str, pos: usize) -> Cow<'l, str> {
        self.highlighter.highlight(line, pos)
    }

    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        default: bool,
    ) -> Cow<'b, str> {
        if default {
            Borrowed(&self.colored_prompt)
        } else {
            Borrowed(prompt)
        }
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Owned("\x1b[1m".to_owned() + hint + "\x1b[m")
    }

    fn highlight_char(&self, line: &str, pos: usize) -> bool {
        self.highlighter.highlight_char(line, pos)
    }
}

pub fn create_editor(
    promt: &str,
    oracles: &[&str],
) -> anyhow::Result<Editor<RLHelper, MemHistory>> {
    let config = Config::builder()
        .history_ignore_space(true)
        .completion_type(CompletionType::List)
        .build();

    let commands = [
        VAR_COMMAND.into(),
        ARG_COMMAND.into(),
        (CONTINUE_COMMAND_SHORT, CONTINUE_COMMAND).into(),
        CommandHint {
            short: None,
            long: FRAME_COMMAND.to_string(),
            subcommands: vec![
                FRAME_COMMAND_INFO_SUBCOMMAND.to_string(),
                FRAME_COMMAND_SWITCH_SUBCOMMAND.to_string(),
            ],
        },
        (RUN_COMMAND_SHORT, RUN_COMMAND).into(),
        STEP_INSTRUCTION_COMMAND.into(),
        (STEP_INTO_COMMAND_SHORT, STEP_INTO_COMMAND).into(),
        (STEP_OUT_COMMAND_SHORT, STEP_OUT_COMMAND).into(),
        (STEP_OVER_COMMAND_SHORT, STEP_OVER_COMMAND).into(),
        SYMBOL_COMMAND.into(),
        (BREAK_COMMAND_SHORT, BREAK_COMMAND).into(),
        (BACKTRACE_COMMAND_SHORT, BACKTRACE_COMMAND).into(),
        CommandHint {
            short: Some(MEMORY_COMMAND_SHORT.to_string()),
            long: MEMORY_COMMAND.to_string(),
            subcommands: vec![
                MEMORY_COMMAND_READ_SUBCOMMAND.to_string(),
                MEMORY_COMMAND_WRITE_SUBCOMMAND.to_string(),
            ],
        },
        CommandHint {
            short: Some(REGISTER_COMMAND_SHORT.to_string()),
            long: REGISTER_COMMAND.to_string(),
            subcommands: vec![
                REGISTER_COMMAND_READ_SUBCOMMAND.to_string(),
                REGISTER_COMMAND_WRITE_SUBCOMMAND.to_string(),
                REGISTER_COMMAND_INFO_SUBCOMMAND.to_string(),
            ],
        },
        (HELP_COMMAND_SHORT, HELP_COMMAND).into(),
        CommandHint {
            short: None,
            long: THREAD_COMMAND.to_string(),
            subcommands: vec![
                THREAD_COMMAND_INFO_SUBCOMMAND.to_string(),
                THREAD_COMMAND_SWITCH_SUBCOMMAND.to_string(),
                THREAD_COMMAND_CURRENT_SUBCOMMAND.to_string(),
            ],
        },
        CommandHint {
            short: None,
            long: SHARED_LIB_COMMAND.to_string(),
            subcommands: vec![SHARED_LIB_COMMAND_INFO_SUBCOMMAND.to_string()],
        },
        CommandHint {
            short: None,
            long: SOURCE_COMMAND.to_string(),
            subcommands: vec![
                SOURCE_COMMAND_DISASM_SUBCOMMAND.to_string(),
                SOURCE_COMMAND_FUNCTION_SUBCOMMAND.to_string(),
            ],
        },
        CommandHint {
            short: None,
            long: ORACLE_COMMAND.to_string(),
            subcommands: oracles.iter().map(ToString::to_string).collect(),
        },
        ("q", "quit").into(),
    ];

    let h = RLHelper {
        completer: Arc::new(Mutex::new(CommandCompleter::new(commands))),
        highlighter: MatchingBracketHighlighter::new(),
        hinter: HistoryHinter {},
        colored_prompt: format!("{}", promt.with(Color::DarkGreen)),
        validator: MatchingBracketValidator::new(),
    };

    let mut editor = Editor::with_history(config, MemHistory::new())?;
    editor.set_helper(Some(h));
    Ok(editor)
}
