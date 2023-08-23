use crate::debugger::command::{
    ARG_ALL_KEY, ARG_COMMAND, BACKTRACE_COMMAND, BACKTRACE_COMMAND_SHORT, BREAK_COMMAND,
    BREAK_COMMAND_SHORT, CONTINUE_COMMAND, CONTINUE_COMMAND_SHORT, FRAME_COMMAND, HELP_COMMAND,
    HELP_COMMAND_SHORT, MEMORY_COMMAND, MEMORY_COMMAND_SHORT, REGISTER_COMMAND,
    REGISTER_COMMAND_SHORT, RUN_COMMAND, RUN_COMMAND_SHORT, SHARED_LIB_COMMAND,
    STEP_INSTRUCTION_COMMAND, STEP_INTO_COMMAND, STEP_INTO_COMMAND_SHORT, STEP_OUT_COMMAND,
    STEP_OUT_COMMAND_SHORT, STEP_OVER_COMMAND, STEP_OVER_COMMAND_SHORT, SYMBOL_COMMAND,
    THREAD_COMMAND, VAR_COMMAND, VAR_LOCAL_KEY,
};
use crossterm::style::{Color, Stylize};
use nom::branch::alt;
use nom::character::complete::{multispace1, not_line_ending};
use nom::combinator::map;
use nom::sequence::preceded;
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
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use trie_rs::{Trie, TrieBuilder};

struct CommandView {
    short: Option<String>,
    long: String,
}

impl CommandView {
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

impl From<&str> for CommandView {
    fn from(value: &str) -> Self {
        CommandView {
            short: None,
            long: value.to_string(),
        }
    }
}

impl From<(&str, &str)> for CommandView {
    fn from((short, long): (&str, &str)) -> Self {
        CommandView {
            short: Some(short.to_string()),
            long: long.to_string(),
        }
    }
}

pub struct CommandCompleter {
    commands: Vec<CommandView>,
    file_hints: Trie<u8>,
    var_hints: Trie<u8>,
    vars: Vec<String>,
    arg_hints: Trie<u8>,
    args: Vec<String>,
}

impl CommandCompleter {
    fn new(commands: impl IntoIterator<Item = CommandView>) -> Self {
        Self {
            commands: commands.into_iter().collect(),
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

enum CompletableCommand {
    Breakpoint(String),
    PrintVariables(String),
    PrintArguments(String),
}

impl CompletableCommand {
    fn recognize(line: &str) -> anyhow::Result<CompletableCommand> {
        let bp_parser = map(
            preceded(
                alt((tag(BREAK_COMMAND), tag(BREAK_COMMAND_SHORT))),
                preceded(multispace1, not_line_ending),
            ),
            |s: &str| CompletableCommand::Breakpoint(s.to_string()),
        );

        let var_parser = map(
            preceded(tag(VAR_COMMAND), preceded(multispace1, not_line_ending)),
            |s: &str| CompletableCommand::PrintVariables(s.to_string()),
        );
        let arg_parser = map(
            preceded(tag(ARG_COMMAND), preceded(multispace1, not_line_ending)),
            |s: &str| CompletableCommand::PrintArguments(s.to_string()),
        );

        Ok(nom_supreme::final_parser::final_parser::<
            _,
            _,
            ErrorTree<&str>,
            ErrorTree<Location>,
        >(alt((bp_parser, var_parser, arg_parser)))(line)?)
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

                let variants = self.file_hints.predictive_search(&maybe_file);
                if !variants.is_empty() {
                    let variants_iter = variants.iter().map(|var| {
                        std::str::from_utf8(var.as_slice()).expect("invalid utf-8 string")
                    });
                    return Ok(pairs_from_variants(variants_iter, line, &maybe_file, ":"));
                }
            }
            Ok(CompletableCommand::PrintVariables(maybe_var)) => {
                if maybe_var.trim().is_empty() {
                    return Ok(pairs_from_variants(self.vars.iter(), line, &maybe_var, ""));
                }

                let variants = self.var_hints.predictive_search(&maybe_var);
                if !variants.is_empty() {
                    let variants_iter = variants.iter().map(|var| {
                        std::str::from_utf8(var.as_slice()).expect("invalid utf-8 string")
                    });
                    return Ok(pairs_from_variants(variants_iter, line, &maybe_var, ""));
                }
            }
            Ok(CompletableCommand::PrintArguments(maybe_arg)) => {
                if maybe_arg.trim().is_empty() {
                    return Ok(pairs_from_variants(self.args.iter(), line, &maybe_arg, ""));
                }

                let variants = self.arg_hints.predictive_search(&maybe_arg);
                if !variants.is_empty() {
                    let variants_iter = variants.iter().map(|var| {
                        std::str::from_utf8(var.as_slice()).expect("invalid utf-8 string")
                    });
                    return Ok(pairs_from_variants(variants_iter, line, &maybe_arg, ""));
                }
            }
            _ => {}
        }

        let pairs = self
            .commands
            .iter()
            .filter_map(|cmd| {
                cmd.long.starts_with(line).then(|| Pair {
                    display: cmd.display_with_short(),
                    replacement: cmd.long(),
                })
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

pub fn create_editor(promt: &str) -> anyhow::Result<Editor<RLHelper, MemHistory>> {
    let config = Config::builder()
        .history_ignore_space(true)
        .completion_type(CompletionType::List)
        .build();

    let commands = [
        VAR_COMMAND.into(),
        ARG_COMMAND.into(),
        (CONTINUE_COMMAND_SHORT, CONTINUE_COMMAND).into(),
        FRAME_COMMAND.into(),
        (RUN_COMMAND_SHORT, RUN_COMMAND).into(),
        STEP_INSTRUCTION_COMMAND.into(),
        (STEP_INTO_COMMAND_SHORT, STEP_INTO_COMMAND).into(),
        (STEP_OUT_COMMAND_SHORT, STEP_OUT_COMMAND).into(),
        (STEP_OVER_COMMAND_SHORT, STEP_OVER_COMMAND).into(),
        SYMBOL_COMMAND.into(),
        (BREAK_COMMAND_SHORT, BREAK_COMMAND).into(),
        (BACKTRACE_COMMAND_SHORT, BACKTRACE_COMMAND).into(),
        (MEMORY_COMMAND_SHORT, MEMORY_COMMAND).into(),
        (REGISTER_COMMAND_SHORT, REGISTER_COMMAND).into(),
        (HELP_COMMAND_SHORT, HELP_COMMAND).into(),
        (THREAD_COMMAND).into(),
        (SHARED_LIB_COMMAND).into(),
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
