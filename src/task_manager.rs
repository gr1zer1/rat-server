//! Окно диспетчера задач.
//!
//! Модуль отвечает только за процессы и команды выбранному компьютеру.
//! Навигация между окнами находится в `dashboard.rs`.

use std::collections::HashMap;

use crate::protocol::{ProcessInfo, ServerToAgent};
use crate::server::AppState;

/// Несколько процессов с одним именем объединяются в одну строку интерфейса.
struct ProcessGroup {
    name: String,
    pids: Vec<u32>,
    total_memory_kb: u64,
}

/// Доступные способы сортировки списка процессов.
#[derive(PartialEq, Clone, Copy)]
enum SortBy {
    Name,
    MemoryDesc,
    CountDesc,
}

/// Состояние именно окна диспетчера задач.
///
/// `AppState` содержит общие данные сервера, остальные поля принадлежат этому экрану.
pub struct TaskManager {
    state: AppState,
    selected_agent: Option<String>,
    search_query: String,
    sort_by: SortBy,
}

impl TaskManager {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            selected_agent: None,
            search_query: String::new(),
            sort_by: SortBy::Name,
        }
    }

    /// Рисует всё содержимое отдельного окна диспетчера задач.
    pub fn ui(&mut self, ctx: &egui::Context) {
        self.agents_panel(ctx);
        self.processes_panel(ctx);
    }

    /// Левая панель со всеми подключёнными компьютерами.
    fn agents_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("task_manager_agents")
            .min_width(190.0)
            .show(ctx, |ui| {
                ui.heading("Компьютеры класса");
                ui.separator();

                // Не держим mutex во время отрисовки кнопок.
                let mut names: Vec<String> =
                    self.state.agents.lock().unwrap().keys().cloned().collect();
                names.sort();
                if names.is_empty() {
                    ui.weak("Нет подключённых компьютеров");
                }

                for name in names {
                    let selected = self.selected_agent.as_deref() == Some(name.as_str());
                    if ui
                        .selectable_label(selected, format!("🖥  {name}"))
                        .clicked()
                    {
                        self.selected_agent = Some(name);
                    }
                }
            });
    }

    /// Центральная часть: поиск, сортировка и список процессов выбранного компьютера.
    fn processes_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(selected) = self.selected_agent.clone() else {
                ui.centered_and_justified(|ui| ui.weak("Выберите компьютер слева"));
                return;
            };

            ui.horizontal(|ui| {
                ui.heading(format!("Процессы: {selected}"));
                if ui.button("🔄 Обновить список").clicked() {
                    self.send_command(&selected, ServerToAgent::ListTasks);
                }
            });
            ui.horizontal(|ui| {
                ui.label("🔎");
                ui.add(
                    egui::TextEdit::singleline(&mut self.search_query)
                        .hint_text("Поиск по названию процесса"),
                );
                ui.separator();
                ui.label("Сортировка:");
                egui::ComboBox::from_id_salt("task_manager_sort")
                    .selected_text(match self.sort_by {
                        SortBy::Name => "по имени",
                        SortBy::MemoryDesc => "по памяти",
                        SortBy::CountDesc => "по числу копий",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.sort_by, SortBy::Name, "по имени");
                        ui.selectable_value(&mut self.sort_by, SortBy::MemoryDesc, "по памяти");
                        ui.selectable_value(&mut self.sort_by, SortBy::CountDesc, "по числу копий");
                    });
            });
            ui.separator();

            // Клонируем данные, чтобы не блокировать серверный поток на время рисования UI.
            let processes = match self.state.agents.lock().unwrap().get(&selected) {
                Some(agent) => agent.processes.clone(),
                None => {
                    ui.weak("Компьютер отключился");
                    return;
                }
            };
            let groups = Self::group_and_sort(processes, &self.search_query, self.sort_by);
            egui::ScrollArea::vertical().show(ui, |ui| {
                for group in &groups {
                    self.process_group_row(ui, &selected, group);
                }
            });
        });
    }

    /// Одиночный процесс рисуется строкой, группа — раскрывающимся списком PID.
    fn process_group_row(&self, ui: &mut egui::Ui, agent_name: &str, group: &ProcessGroup) {
        if group.pids.len() == 1 {
            ui.horizontal(|ui| {
                ui.label(&group.name);
                ui.label(format!("{} МБ", group.total_memory_kb / 1024));
                if ui.button("Завершить").clicked() {
                    self.send_command(agent_name, ServerToAgent::KillTask { pid: group.pids[0] });
                }
            });
            return;
        }

        egui::CollapsingHeader::new(format!("{} ({})", group.name, group.pids.len()))
            .id_salt((&group.name, agent_name))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("{} МБ суммарно", group.total_memory_kb / 1024));
                    if ui.button("Завершить все").clicked() {
                        for &pid in &group.pids {
                            self.send_command(agent_name, ServerToAgent::KillTask { pid });
                        }
                    }
                });
                for &pid in &group.pids {
                    ui.horizontal(|ui| {
                        ui.label(format!("PID {pid}"));
                        if ui.button("Завершить").clicked() {
                            self.send_command(agent_name, ServerToAgent::KillTask { pid });
                        }
                    });
                }
            });
    }

    /// Фильтрует процессы, группирует одинаковые имена и сортирует готовые группы.
    fn group_and_sort(
        processes: Vec<ProcessInfo>,
        query: &str,
        sort_by: SortBy,
    ) -> Vec<ProcessGroup> {
        let query = query.to_lowercase();
        let mut by_name: HashMap<String, ProcessGroup> = HashMap::new();
        for process in processes
            .into_iter()
            .filter(|process| query.is_empty() || process.name.to_lowercase().contains(&query))
        {
            let group = by_name
                .entry(process.name.clone())
                .or_insert_with(|| ProcessGroup {
                    name: process.name.clone(),
                    pids: Vec::new(),
                    total_memory_kb: 0,
                });
            group.pids.push(process.pid);
            group.total_memory_kb += process.memory_kb;
        }

        let mut groups: Vec<ProcessGroup> = by_name.into_values().collect();
        for group in &mut groups {
            group.pids.sort_unstable();
        }
        match sort_by {
            SortBy::Name => groups.sort_by_key(|group| group.name.to_lowercase()),
            SortBy::MemoryDesc => groups.sort_by(|a, b| b.total_memory_kb.cmp(&a.total_memory_kb)),
            SortBy::CountDesc => groups.sort_by(|a, b| b.pids.len().cmp(&a.pids.len())),
        }
        groups
    }

    /// Передаёт команду серверной задаче, которая отправит её в WebSocket агента.
    fn send_command(&self, agent_name: &str, command: ServerToAgent) {
        if let Some(agent) = self.state.agents.lock().unwrap().get(agent_name) {
            let _ = agent.cmd_tx.send(command);
        }
    }
}
