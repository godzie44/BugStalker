use crate::cui::window::{Action, CuiComponent};
use crate::cui::AppContext;
use crossterm::event::KeyEvent;
use std::collections::{HashMap, HashSet};
use std::io::StdoutLock;
use tui::backend::CrosstermBackend;
use tui::layout::Rect;
use tui::Frame;

pub(super) struct ComplexComponent {
    name: &'static str,
    active_components: HashSet<&'static str>,
    visible_components: HashSet<&'static str>,
    components: HashMap<&'static str, Box<dyn CuiComponent>>,
    layout: fn(Rect) -> HashMap<&'static str, Rect>,
}

impl ComplexComponent {
    pub(super) fn new(
        name: &'static str,
        layout: fn(Rect) -> HashMap<&'static str, Rect>,
        components: Vec<Box<dyn CuiComponent>>,
        active_components: Vec<&'static str>,
        visible_components: Vec<&'static str>,
    ) -> Self {
        Self {
            name,
            active_components: active_components.into_iter().collect(),
            visible_components: visible_components.into_iter().collect(),
            components: components.into_iter().map(|c| (c.name(), c)).collect(),
            layout,
        }
    }
}

impl CuiComponent for ComplexComponent {
    fn render(&self, ctx: AppContext, frame: &mut Frame<CrosstermBackend<StdoutLock>>, rect: Rect) {
        let mut rects = (self.layout)(rect);
        self.visible_components.iter().for_each(|c_name| {
            if self.components.get(c_name).is_none() {
                return;
            }

            if rects.get(c_name).is_none() {
                return;
            }

            self.components[c_name].render(ctx.clone(), frame, rects.remove(c_name).unwrap())
        });
    }

    fn handle_user_event(&mut self, ctx: AppContext, e: KeyEvent) -> Vec<Action> {
        self.active_components
            .iter()
            .flat_map(|idx| {
                let b = self
                    .components
                    .get_mut(idx)
                    .unwrap()
                    .handle_user_event(ctx.clone(), e);
                b
            })
            .collect()
    }

    fn apply_app_action(&mut self, ctx: AppContext, actions: &[Action]) {
        for action in actions {
            let applicable = action
                .target()
                .map(|target| self.components.get(target).is_some())
                .unwrap_or(false);
            if !applicable {
                continue;
            }

            match action {
                Action::ActivateComponent(component) => {
                    self.active_components.insert(component);
                }
                Action::DeActivateComponent(component) => {
                    self.active_components.remove(component);
                }
                Action::ShowComponent(component) => {
                    self.visible_components.insert(component);
                }
                Action::HideComponent(component) => {
                    self.visible_components.remove(component);
                }
                Action::ActivateUserInput(_) => {
                    self.active_components.clear();
                    self.active_components.insert(action.target().unwrap());
                }
                Action::CancelUserInput => {
                    self.active_components.remove(action.target().unwrap());
                    self.active_components.insert("main");
                }
                _ => {}
            };
        }

        self.components
            .iter_mut()
            .for_each(|(_, component)| component.apply_app_action(ctx.clone(), actions));
    }

    fn name(&self) -> &'static str {
        self.name
    }
}

// enum AppMode {
//     Default,
//     UserInput,
// }
//
// pub(super) struct AppWindow {
//     main_window: ComplexComponent,
//     mode: AppMode,
// }
//
// impl AppWindow {
//     pub fn new(window: ComplexComponent) -> Self {
//         Self {
//             mode: AppMode::Default,
//             main_window: window,
//         }
//     }
//
//     pub fn activate_user_input(&self) {}
//
//     pub fn deactivate_user_input(&self) {}
// }
//
// impl CuiComponent for AppWindow {
//     fn render(
//         &self,
//         ctx: RenderContext,
//         frame: &mut Frame<CrosstermBackend<StdoutLock>>,
//         rect: Rect,
//     ) {
//         self.main_window.render(ctx, frame, rect)
//     }
//
//     fn handle_user_event(&mut self, e: KeyEvent) -> Vec<Action> {
//         self.main_window.handle_user_event(e)
//     }
//
//     fn apply_app_action(&mut self, behaviour: &[Action]) {
//         self.main_window.apply_app_action(behaviour)
//     }
//
//     fn name(&self) -> &'static str {
//         "app"
//     }
// }
