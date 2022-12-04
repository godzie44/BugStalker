use crate::cui::file_view::FileView;
use crate::cui::hook::CuiHook;
use crate::cui::window::RenderContext;
use crate::debugger::Debugger;
use crossterm::event;
use crossterm::event::EnableMouseCapture;
use crossterm::terminal::enable_raw_mode;
use crossterm::terminal::EnterAlternateScreen;
use nix::unistd::Pid;
use std::cell::RefCell;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::mpsc;
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
}

impl AppBuilder {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            file_view: Rc::new(FileView::new()),
        }
    }

    pub fn build(self, program: impl Into<String>, pid: Pid) -> CuiApplication {
        let render_data = SharedRenderData(Rc::new(RenderData::start_screen()));
        let hook = CuiHook::new(render_data.clone(), self.file_view);
        let debugger = Debugger::new(program, pid, hook);
        CuiApplication::new(debugger, render_data)
    }
}

pub struct RenderData {
    main_text: RefCell<Text<'static>>,
}

impl RenderData {
    fn start_screen() -> Self {
        Self {
            main_text: RefCell::new(vec![
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
            ].into())
        }
    }
}

#[derive(Clone)]
pub struct SharedRenderData(Rc<RenderData>);

impl Deref for SharedRenderData {
    type Target = Rc<RenderData>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct CuiApplication {
    debugger: Debugger<CuiHook>,
    render_data: SharedRenderData,
}

impl CuiApplication {
    pub fn new(debugger: Debugger<CuiHook>, render_data: SharedRenderData) -> Self {
        Self {
            debugger,
            render_data,
        }
    }

    pub fn run(self) -> anyhow::Result<()> {
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
                    if let crossterm::event::Event::Key(key) =
                        event::read().expect("can read events")
                    {
                        tx.send(Event::Input(key)).expect("can send events");
                    } else {
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

        window::run(
            RenderContext::new(self.render_data),
            terminal,
            Rc::new(self.debugger),
            rx,
        )
    }
}
