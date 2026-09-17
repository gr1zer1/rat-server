use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;

use crate::camera::{self, CameraFrames};
use crate::protocol::{AgentToServer, ProcessInfo, ServerToAgent};

/// Состояние одного подключённого агента, как его видит GUI.
pub struct AgentEntry {
    /// Канал, через который GUI-поток кладёт команды в websocket-задачу этого агента.
    pub cmd_tx: mpsc::UnboundedSender<ServerToAgent>,
    pub processes: Vec<ProcessInfo>,
    pub last_kill_result: Option<(u32, bool)>,
}

/// Общее состояние сервера — разделяется между async-задачами Axum и GUI-потоком.
#[derive(Clone)]
pub struct AppState {
    pub agents: Arc<Mutex<HashMap<String, AgentEntry>>>,
    /// Последние кадры камер для окна видеонаблюдения.
    pub camera_frames: CameraFrames,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            agents: Arc::new(Mutex::new(HashMap::new())),
            camera_frames: camera::new_camera_frames(),
        }
    }
}

/// Поднимает Axum-сервер и слушает подключения агентов. Вызывается на отдельном
/// потоке со своим tokio-рантаймом — GUI должен жить на главном потоке.
pub async fn run_server(state: AppState) {
    let app = Router::new()
        .route("/ws", get(ws_handler))
        // Бинарные JPEG-кадры камер не смешиваются с JSON-командами обычных агентов.
        // В axum 0.7 параметр пути записывается как `:name`.
        .route("/camera/:name", get(camera::camera_ws_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8000")
        .await
        .expect("failed to bind port 8000");

    println!("сервер слушает ws://0.0.0.0:8000/ws");
    axum::serve(listener, app).await.expect("server error");
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut ws_write, mut ws_read) = socket.split();

    // Первое сообщение от агента обязано быть Hello с именем компьютера
    let name = loop {
        match ws_read.next().await {
            Some(Ok(Message::Text(text))) => match serde_json::from_str::<AgentToServer>(&text) {
                Ok(AgentToServer::Hello { name }) => break name,
                _ => {
                    eprintln!("ожидался Hello от агента, получено что-то другое");
                    return;
                }
            },
            _ => {
                eprintln!("агент отключился до отправки Hello");
                return;
            }
        }
    };

    println!("агент подключён: {name}");

    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<ServerToAgent>();

    state.agents.lock().unwrap().insert(
        name.clone(),
        AgentEntry {
            cmd_tx,
            processes: Vec::new(),
            last_kill_result: None,
        },
    );

    // Отдельная задача: пересылает команды из GUI (через канал) в сокет этого агента
    let writer_task = tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            let text = match serde_json::to_string(&cmd) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("не удалось сериализовать команду: {e}");
                    continue;
                }
            };
            if ws_write.send(Message::Text(text)).await.is_err() {
                break;
            }
        }
    });

    // Читаем ответы от агента и обновляем состояние, которое рисует GUI
    while let Some(msg) = ws_read.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                eprintln!("ошибка чтения от {name}: {e}");
                break;
            }
        };

        let Message::Text(text) = msg else { continue };

        let parsed: AgentToServer = match serde_json::from_str(&text) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("не удалось разобрать сообщение от {name}: {e}");
                continue;
            }
        };

        let mut agents = state.agents.lock().unwrap();
        if let Some(entry) = agents.get_mut(&name) {
            match parsed {
                AgentToServer::ProcessList { processes } => entry.processes = processes,
                AgentToServer::KillResult { pid, success } => {
                    entry.last_kill_result = Some((pid, success))
                }
                AgentToServer::Hello { .. } => {}
            }
        }
    }

    println!("агент отключился: {name}");
    state.agents.lock().unwrap().remove(&name);
    writer_task.abort();
}
