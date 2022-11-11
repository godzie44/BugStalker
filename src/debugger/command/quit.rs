pub struct Quit {}

impl Quit {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {}
    }

    pub fn run(&self) -> ! {
        std::process::exit(0)
    }
}
