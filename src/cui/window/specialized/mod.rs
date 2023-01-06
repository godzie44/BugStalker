use tui::widgets::ListState;

pub(super) mod breakpoint;
pub(super) mod debugee_out;
pub(super) mod debugee_view;
pub(super) mod logs;
pub(super) mod trace;
pub(super) mod variable;

struct PersistentList<T> {
    items: Vec<T>,
    state: ListState,
}

impl<T> Default for PersistentList<T> {
    fn default() -> Self {
        Self {
            items: vec![],
            state: ListState::default(),
        }
    }
}

impl<T> PersistentList<T> {
    fn update_items(&mut self, new_items: Vec<T>) {
        self.items = new_items;
    }

    fn items(&mut self) -> &mut Vec<T> {
        &mut self.items
    }

    fn next(&mut self) {
        if self.items.is_empty() {
            self.state.select(None);
            return;
        }

        let i = match self.state.selected() {
            Some(i) => {
                if i >= self.items.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn previous(&mut self) {
        if self.items.is_empty() {
            self.state.select(None);
            return;
        }

        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.items.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn remove_selected(&mut self) {
        let i = match self.state.selected() {
            Some(i) => i,
            None => return,
        };
        self.items.remove(i);
        self.previous();
    }
}
