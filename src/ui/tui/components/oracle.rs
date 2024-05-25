use crate::oracle::Oracle;
use crate::ui;
use crate::ui::tui::utils::tab::TabWindow;
use std::sync::Arc;

pub fn make_oracle_tab_window(oracles: &[Arc<dyn Oracle>]) -> TabWindow {
    let ora_names: Vec<_> = oracles.iter().map(|oracle| oracle.name()).collect();
    let keymap = &ui::config::current().tui_keymap;
    let windows: Vec<_> = oracles
        .iter()
        .map(|o| o.clone().make_tui_component(keymap))
        .collect();

    TabWindow::new("Choose your oracle", &ora_names, windows, None)
}
