//! Главное окно программы и управление дополнительными окнами.

use std::time::Duration;

use crate::camera::CameraView;
use crate::server::AppState;
use crate::task_manager::TaskManager;

/// Главное окно-лаунчер. Логика диспетчера задач остаётся в своём модуле.
pub struct DashboardApp {
    state: AppState,
    task_manager: TaskManager,
    task_manager_open: bool,
    connections_open: bool,
    camera_view: CameraView,
    cameras_open: bool,
}

impl DashboardApp {
    pub fn new(state: AppState) -> Self {
        let camera_view = CameraView::new(state.clone());
        Self {
            task_manager: TaskManager::new(state.clone()),
            state,
            task_manager_open: false,
            connections_open: false,
            camera_view,
            cameras_open: false,
        }
    }

    /// Рисует стартовое окно с доступными инструментами.
    fn launcher_ui(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(50.0);
                ui.heading("Панель учителя");
                ui.label("Выберите инструмент — он откроется в отдельном окне.");
                ui.add_space(20.0);
                if ui
                    .add_sized([260.0, 42.0], egui::Button::new("Диспетчер задач"))
                    .clicked()
                {
                    self.task_manager_open = true;
                }
                if ui
                    .add_sized([260.0, 42.0], egui::Button::new("Подключённые компьютеры"))
                    .clicked()
                {
                    self.connections_open = true;
                }
                if ui
                    .add_sized([260.0, 42.0], egui::Button::new("Камеры"))
                    .clicked()
                {
                    self.cameras_open = true;
                }
            });
        });
    }

    /// Отдельное окно со списком подключений — вторая готовая опция главного меню.
    fn connections_window(&mut self, ctx: &egui::Context) {
        if !self.connections_open {
            return;
        }
        let mut close = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("connections_window"),
            egui::ViewportBuilder::default()
                .with_title("Подключённые компьютеры")
                .with_inner_size([420.0, 300.0]),
            |window_ctx, _| {
                close = window_ctx.input(|input| input.viewport().close_requested());
                egui::CentralPanel::default().show(window_ctx, |ui| {
                    ui.heading("Подключённые компьютеры");
                    ui.separator();
                    let mut agents: Vec<(String, usize)> = self
                        .state
                        .agents
                        .lock()
                        .unwrap()
                        .iter()
                        .map(|(name, agent)| (name.clone(), agent.processes.len()))
                        .collect();
                    agents.sort_by(|a, b| a.0.cmp(&b.0));
                    if agents.is_empty() {
                        ui.weak("Пока никто не подключился");
                    }
                    for (name, process_count) in agents {
                        ui.label(format!(
                            "🖥  {name} — процессов в последнем отчёте: {process_count}"
                        ));
                    }
                });
            },
        );
        self.connections_open = !close;
    }

    /// Открывает самостоятельное нативное окно с UI из `task_manager.rs`.
    fn task_manager_window(&mut self, ctx: &egui::Context) {
        if !self.task_manager_open {
            return;
        }
        let mut close = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("task_manager_window"),
            egui::ViewportBuilder::default()
                .with_title("Диспетчер задач")
                .with_inner_size([900.0, 600.0]),
            |window_ctx, _| {
                close = window_ctx.input(|input| input.viewport().close_requested());
                self.task_manager.ui(window_ctx);
            },
        );
        self.task_manager_open = !close;
    }

    /// Открывает окно, в котором можно выбрать и посмотреть активную камеру.
    fn cameras_window(&mut self, ctx: &egui::Context) {
        if !self.cameras_open {
            return;
        }
        let mut close = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("cameras_window"),
            egui::ViewportBuilder::default()
                .with_title("Камеры")
                .with_inner_size([900.0, 650.0]),
            |window_ctx, _| {
                close = window_ctx.input(|input| input.viewport().close_requested());
                self.camera_view.ui(window_ctx);
            },
        );
        if close {
            self.camera_view.stop_all();
        }
        self.cameras_open = !close;
    }
}

impl eframe::App for DashboardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Сервер обновляет данные асинхронно; это показывает изменения без действий пользователя.
        ctx.request_repaint_after(Duration::from_millis(500));
        self.launcher_ui(ctx);
        self.task_manager_window(ctx);
        self.connections_window(ctx);
        self.cameras_window(ctx);
    }
}
