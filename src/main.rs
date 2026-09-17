mod camera;
mod dashboard;
mod protocol;
mod server;
mod task_manager;

use dashboard::DashboardApp;
use server::AppState;

fn main() -> eframe::Result<()> {
    let state = AppState::new();

    // WebSocket-сервер работает в отдельном потоке, а GUI остаётся на главном.
    let server_state = state.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("failed to build tokio runtime");
        rt.block_on(server::run_server(server_state));
    });

    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "Мониторинг класса",
        options,
        Box::new(|_cc| Ok(Box::new(DashboardApp::new(state)))),
    )
}
