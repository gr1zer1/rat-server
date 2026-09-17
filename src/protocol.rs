use serde::{Deserialize, Serialize};

/// Один процесс в списке задач ученика — то же самое, что видно в диспетчере
/// задач Windows на его компьютере.
#[derive(Serialize, Deserialize, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub memory_kb: u64,
}

/// Команды, которые учитель может отправить агенту.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerToAgent {
    ListTasks,
    KillTask {
        pid: u32,
    },
    /// Явно включает передачу камеры на выбранном компьютере.
    StartCamera,
    /// Немедленно останавливает передачу камеры и освобождает устройство.
    StopCamera,
}

/// Сообщения, которые агент отправляет учителю.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentToServer {
    Hello { name: String },
    ProcessList { processes: Vec<ProcessInfo> },
    KillResult { pid: u32, success: bool },
}
