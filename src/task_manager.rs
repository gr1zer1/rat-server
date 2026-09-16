use crate::protocol::ServerToAgent;
use crate::server::AppState;

/// Окно "Диспетчер задач" — показывает подключённые компьютеры класса
/// и позволяет смотреть/завершать процессы на выбранном.
pub struct TeacherApp {
    state: AppState,
    selected_agent: Option<String>,
}

impl TeacherApp {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            selected_agent: None,
        }
    }

    fn agents_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("agents_panel")
            .min_width(180.0)
            .show(ctx, |ui| {
                ui.heading("Компьютеры класса");
                ui.separator();

                let agents = self.state.agents.lock().unwrap();
                let mut names: Vec<&String> = agents.keys().collect();
                names.sort();

                if names.is_empty() {
                    ui.weak("Пока никто не подключился");
                }

                for name in names {
                    let is_selected = self.selected_agent.as_deref() == Some(name.as_str());
                    if ui
                        .selectable_label(is_selected, format!("🖥  {name}"))
                        .clicked()
                    {
                        self.selected_agent = Some(name.clone());
                    }
                }
            });
    }

    fn processes_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(selected) = self.selected_agent.clone() else {
                ui.centered_and_justified(|ui| {
                    ui.weak("Выбери компьютер слева, чтобы увидеть список процессов");
                });
                return;
            };

            ui.horizontal(|ui| {
                ui.heading(format!("Процессы: {selected}"));
                if ui.button("🔄  Обновить список").clicked() {
                    self.send_command(&selected, ServerToAgent::ListTasks);
                }
            });
            ui.separator();

            let processes = {
                let agents = self.state.agents.lock().unwrap();
                match agents.get(&selected) {
                    Some(entry) => entry.processes.clone(),
                    None => {
                        ui.weak("Этот компьютер отключился");
                        Vec::new()
                    }
                }
            };

            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("process_grid")
                    .num_columns(4)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("PID");
                        ui.strong("Процесс");
                        ui.strong("Память");
                        ui.strong("");
                        ui.end_row();

                        for p in &processes {
                            ui.label(p.pid.to_string());
                            ui.label(&p.name);
                            ui.label(format!("{} МБ", p.memory_kb / 1024));
                            if ui.button("Завершить").clicked() {
                                self.send_command(&selected, ServerToAgent::KillTask { pid: p.pid });
                            }
                            ui.end_row();
                        }
                    });
            });
        });
    }

    fn send_command(&self, agent_name: &str, cmd: ServerToAgent) {
        let agents = self.state.agents.lock().unwrap();
        if let Some(entry) = agents.get(agent_name) {
            let _ = entry.cmd_tx.send(cmd);
        }
    }
}

impl eframe::App for TeacherApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Периодически перерисовываем окно, чтобы видеть новые подключения
        // и ответы от агентов без лишних действий пользователя.
        ctx.request_repaint_after(std::time::Duration::from_millis(500));

        self.agents_panel(ctx);
        self.processes_panel(ctx);
    }
}