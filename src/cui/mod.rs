use crate::cui::file_view::FileView;
use crate::cui::hook::CuiHook;
use crate::debugger::Debugger;
use crossterm::cursor::Show;
use crossterm::event;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use nix::unistd::Pid;
use std::cell::{Cell, RefCell};
use std::io::{BufRead, BufReader, Read};
use std::ops::Deref;
use std::process::{ChildStderr, ChildStdout};
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use std::{io, thread};
use tui::backend::CrosstermBackend;
use tui::style::{Color, Style};
use tui::text::{Span, Spans, Text};
use tui::Terminal;

pub mod file_view;
pub mod hook;
pub mod window;

pub(super) enum Event<I> {
    Input(I),
    Tick,
}

pub struct AppBuilder {
    file_view: Rc<FileView>,
    debugee_out: ChildStdout,
    debugee_err: ChildStderr,
}

impl AppBuilder {
    pub fn new(debugee_out: ChildStdout, debugee_err: ChildStderr) -> Self {
        Self {
            debugee_out,
            debugee_err,
            file_view: Rc::new(FileView::new()),
        }
    }

    pub fn build(self, program: impl Into<String>, pid: Pid) -> CuiApplication {
        let ctx = AppContext(Rc::new(Context {
            data: RenderData::start_screen(),
            state: Cell::new(AppState::Initial),
        }));
        let hook = CuiHook::new(ctx.clone(), self.file_view);
        let debugger = Debugger::new(program, pid, hook);
        CuiApplication::new(debugger, ctx, self.debugee_out, self.debugee_err)
    }
}

pub struct RenderData {
    debugee_file_name: RefCell<String>,
    debugee_text: RefCell<Text<'static>>,
    debugee_text_pos: Cell<u64>,
}

impl RenderData {
    fn start_screen() -> Self {
        Self {
            debugee_file_name: RefCell::default(),
            debugee_text: RefCell::new(vec![
                Spans::from(vec![Span::raw("")]),
                Spans::from(vec![Span::raw("Welcome")]),
                Spans::from(vec![Span::raw("")]),
                Spans::from(vec![Span::raw("to")]),
                Spans::from(vec![Span::raw("")]),
                Spans::from(vec![Span::styled(
                    "pet-CLI",
                    Style::default().fg(Color::LightBlue),
                )]),
                Spans::from(vec![Span::raw("")]),
                Spans::from(vec![Span::raw("Press 'p' to access pets, 'a' to add random new pets and 'd' to delete the currently selected pet.")]),
            ].into()),
            debugee_text_pos: Cell::new(0),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum AppState {
    Initial,
    DebugeeRun,
    DebugeeBreak,
    UserInput,
}

pub struct Context {
    pub(super) data: RenderData,
    pub(super) state: Cell<AppState>,
}

#[derive(Clone)]
pub struct AppContext(Rc<Context>);

impl Deref for AppContext {
    type Target = Rc<Context>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AppContext {
    pub(super) fn change_state(&self, state: AppState) {
        self.state.set(state)
    }

    pub(super) fn assert_state(&self, state: AppState) -> bool {
        self.state.get() == state
    }
}

#[derive(Default, Clone)]
struct DebugeeStreamBuffer {
    data: Arc<Mutex<Vec<StreamLine>>>,
}

enum StreamLine {
    Out(String),
    Err(String),
}

pub struct CuiApplication {
    debugger: Debugger<CuiHook>,
    ctx: AppContext,
    debugee_out: ChildStdout,
    debugee_err: ChildStderr,
}

impl CuiApplication {
    pub fn new(
        debugger: Debugger<CuiHook>,
        ctx: AppContext,
        debugee_out: ChildStdout,
        debugee_err: ChildStderr,
    ) -> Self {
        Self {
            debugger,
            ctx,
            debugee_out,
            debugee_err,
        }
    }

    pub fn run(self) -> anyhow::Result<()> {
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
                let size = stream.read_line(&mut line).unwrap();
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

        self.debugger.on_debugee_start()?;
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

        window::run(self.ctx, terminal, Rc::new(self.debugger), rx, stream_buff)
    }
}
