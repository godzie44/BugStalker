use crate::cui::hook::CuiHook;
use crate::debugger::{command, Debugger};
use crossterm::cursor::Show;
use crossterm::event;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use nix::unistd::Pid;
use os_pipe::PipeReader;
use std::cell::RefCell;
use std::io::{BufRead, BufReader, Read};
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use std::{io, thread};
use tui::backend::CrosstermBackend;
use tui::Terminal;

mod context;
pub mod hook;
pub mod window;

pub(super) enum Event<I> {
    Input(I),
    Tick,
}

pub struct AppBuilder {
    debugee_out: PipeReader,
    debugee_err: PipeReader,
}

impl AppBuilder {
    pub fn new(debugee_out: PipeReader, debugee_err: PipeReader) -> Self {
        Self {
            debugee_out,
            debugee_err,
        }
    }

    pub fn build(self, program: impl Into<String>, pid: Pid) -> anyhow::Result<CuiApplication> {
        let hook = CuiHook::new();
        let debugger = Debugger::new(program, pid, hook)?;
        Ok(CuiApplication::new(
            debugger,
            self.debugee_out,
            self.debugee_err,
        ))
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum AppState {
    Initial,
    DebugeeRun,
    DebugeeBreak,
    UserInput,
    Finish,
}

#[derive(Default, Clone)]
pub struct DebugeeStreamBuffer {
    data: Arc<Mutex<Vec<StreamLine>>>,
}

enum StreamLine {
    Out(String),
    Err(String),
}

pub struct CuiApplication {
    debugger: Debugger<CuiHook>,
    debugee_out: PipeReader,
    debugee_err: PipeReader,
}

impl CuiApplication {
    pub fn new(
        debugger: Debugger<CuiHook>,
        debugee_out: PipeReader,
        debugee_err: PipeReader,
    ) -> Self {
        Self {
            debugger,
            debugee_out,
            debugee_err,
        }
    }

    pub fn run(mut self) -> anyhow::Result<()> {
        let stream_buff = DebugeeStreamBuffer::default();
        enum StreamType {
            StdErr,
            StdOut,
        }
        fn stream_to_buffer(
            stream: impl Read,
            buffer: Arc<Mutex<Vec<StreamLine>>>,
            stream_type: StreamType,
        ) {
            let mut stream = BufReader::new(stream);
            loop {
                let mut line = String::new();
                let size = stream.read_line(&mut line).unwrap_or(0);
                if size == 0 {
                    return;
                }
                let line = match stream_type {
                    StreamType::StdErr => StreamLine::Err(line),
                    StreamType::StdOut => StreamLine::Out(line),
                };
                buffer.lock().unwrap().push(line);
            }
        }

        {
            let out_buff = stream_buff.data.clone();
            thread::spawn(move || stream_to_buffer(self.debugee_out, out_buff, StreamType::StdOut));
            let err_buff = stream_buff.data.clone();
            thread::spawn(move || stream_to_buffer(self.debugee_err, err_buff, StreamType::StdErr));
        }

        // start debugee
        command::Continue::new(&mut self.debugger).run()?;

        enable_raw_mode()?;

        let (tx, rx) = mpsc::channel();
        let tick_rate = Duration::from_millis(200);
        thread::spawn(move || {
            let mut last_tick = Instant::now();
            loop {
                let timeout = tick_rate
                    .checked_sub(last_tick.elapsed())
                    .unwrap_or_else(|| Duration::from_secs(0));

                if event::poll(timeout).expect("poll works") {
                    if let event::Event::Key(key) = event::read().expect("can read events") {
                        tx.send(Event::Input(key)).expect("can send events");
                    }
                }

                if last_tick.elapsed() >= tick_rate && tx.send(Event::Tick).is_ok() {
                    last_tick = Instant::now();
                }
            }
        });
        let stdout = io::stdout();
        let mut stdout = stdout.lock();

        crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;

        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic| {
            disable_raw_mode().unwrap();
            crossterm::execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture).unwrap();
            crossterm::execute!(io::stdout(), Show).unwrap();
            original_hook(panic);
        }));

        window::run(
            terminal,
            Rc::new(RefCell::new(self.debugger)),
            rx,
            stream_buff,
        )
    }
}
