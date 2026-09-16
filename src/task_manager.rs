use std::collections::HashMap;

use crate::protocol::{ProcessInfo, ServerToAgent};
use crate::server::AppState;

/// Один "свёрнутый" процесс в UI — все запущенные копии с одинаковым именем
/// (например 11 окон VS Code) объединены в одну строку с общей памятью и
/// списком PID, которые можно раскрыть.
struct ProcessGroup {
    name: String,
    pids: Vec<u32>,
    total_memory_kb: u64,
}

/// Режим сортировки списка групп процессов.
#[derive(PartialEq, Clone, Copy)]
enum SortBy {
    Name,
    MemoryDesc,
    CountDesc,
}

/// Окно "Диспетчер задач" — показывает подключённые компьютеры класса
/// и позволяет смотреть/завершать процессы на выбранном.
pub struct TeacherApp {
    state: AppState,
    selected_agent: Option<String>,
    search_query: String,
    sort_by: SortBy,
}

impl TeacherApp {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            selected_agent: None,
            search_query: String::new(),
            sort_by: SortBy::Name,
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

            ui.horizontal(|ui| {
                ui.label("🔎");
                ui.text_edit_singleline(&mut self.search_query)
                    .on_hover_text("Фильтр по названию процесса");

                ui.separator();

                ui.label("Сортировка:");
                egui::ComboBox::from_id_salt("sort_by")
                    .selected_text(match self.sort_by {
                        SortBy::Name => "По имени",
                        SortBy::MemoryDesc => "По памяти",
                        SortBy::CountDesc => "По количеству копий",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.sort_by, SortBy::Name, "По имени");
                        ui.selectable_value(&mut self.sort_by, SortBy::MemoryDesc, "По памяти");
                        ui.selectable_value(
                            &mut self.sort_by,
                            SortBy::CountDesc,
                            "По количеству копий",
                        );
                    });
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
            let groups = Self::group_and_sort(processes, &self.search_query, self.sort_by);

            egui::ScrollArea::vertical().show(ui, |ui| {
                for group in &groups {
                    let count = group.pids.len();
                    // Одна копия процесса — не нагромождаем сворачиваемый заголовок,
                    // рисуем как обычную строку.
                    if count == 1 {
                        ui.horizontal(|ui| {
                            ui.label(&group.name);
                            ui.label(format!("{} МБ", group.total_memory_kb / 1024));
                            if ui.button("Завершить").clicked() {
                                self.send_command(
                                    &selected,
                                    ServerToAgent::KillTask { pid: group.pids[0] },
                                );
                            }
                        });
                        continue;
                    }

                    let id = ui.make_persistent_id(&group.name);
                    let state = egui::collapsing_header::CollapsingState::load_with_default_open(
                        ui.ctx(),
                        id,
                        false,
                    );
                    let header = state.show_header(ui, |ui| {
                        ui.label(format!("{} ({count})", group.name));
                        ui.label(format!("{} МБ суммарно", group.total_memory_kb / 1024));
                        if ui.button("Завершить все").clicked() {
                            for pid in &group.pids {
                                self.send_command(&selected, ServerToAgent::KillTask { pid: *pid });
                            }
                        }
                    });

                    header.body(|ui| {
                        for pid in &group.pids {
                            ui.horizontal(|ui| {
                                ui.label(format!("PID {pid}"));
                                if ui.button("Завершить").clicked() {
                                    self.send_command(
                                        &selected,
                                        ServerToAgent::KillTask { pid: *pid },
                                    );
                                }
                            });
                        }
                    });
                }
            });
        });
    }

    /// Фильтрует по подстроке в имени, группирует одинаковые процессы вместе
    /// и сортирует получившиеся группы.
    fn group_and_sort(
        processes: Vec<ProcessInfo>,
        query: &str,
        sort_by: SortBy,
    ) -> Vec<ProcessGroup> {
        let filtered: Vec<ProcessInfo> = if query.is_empty() {
            processes
        } else {
            let query_lower = query.to_lowercase();
            processes
                .into_iter()
                .filter(|p| p.name.to_lowercase().contains(&query_lower))
                .collect()
        };

        let mut by_name: HashMap<String, ProcessGroup> = HashMap::new();
        for p in filtered {
            let entry = by_name.entry(p.name.clone()).or_insert_with(|| ProcessGroup {
                name: p.name.clone(),
                pids: Vec::new(),
                total_memory_kb: 0,
            });
            entry.pids.push(p.pid);
            entry.total_memory_kb += p.memory_kb;
        }

        let mut groups: Vec<ProcessGroup> = by_name.into_values().collect();
        for g in &mut groups {
            g.pids.sort();
        }

        match sort_by {
            SortBy::Name => groups.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
            SortBy::MemoryDesc => groups.sort_by(|a, b| b.total_memory_kb.cmp(&a.total_memory_kb)),
            SortBy::CountDesc => groups.sort_by(|a, b| b.pids.len().cmp(&a.pids.len())),
        }

        groups
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