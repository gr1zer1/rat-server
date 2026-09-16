mod protocol;
mod server;
mod task_manager;

use server::AppState;
use task_manager::TeacherApp;

fn main() -> eframe::Result<()> {
    let state = AppState::new();

    // Axum-сервер живёт на отдельном потоке со своим tokio-рантаймом — GUI (eframe)
    // должен работать на главном потоке.
    let server_state = state.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("failed to build tokio runtime");
        rt.block_on(server::run_server(server_state));
    });

    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "Мониторинг класса — Диспетчер задач",
        options,
        Box::new(|_cc| Ok(Box::new(TeacherApp::new(state)))),
    )
}