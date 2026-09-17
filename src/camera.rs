//! Приём и отображение видеокадров от подключённых камер.
//!
//! Камера отправляет JPEG-кадры в маршрут `/camera/<имя_компьютера>`.
//! Сервер хранит только последний кадр каждого компьютера, не записывая видео на диск.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::IntoResponse;

use crate::protocol::ServerToAgent;
use crate::server::AppState;

/// Последний кадр конкретного компьютера в формате, понятном `egui`.
pub struct CameraFrame {
    image: egui::ColorImage,
}

/// Общая память для видеокадров. В ней хранится ровно один кадр на камеру.
pub type CameraFrames = Arc<Mutex<HashMap<String, CameraFrame>>>;

/// Создаёт пустое хранилище кадров при запуске сервера.
pub fn new_camera_frames() -> CameraFrames {
    Arc::new(Mutex::new(HashMap::new()))
}

/// WebSocket-обработчик для камеры. Имя берётся из URL, например `/camera/PC-12`.
pub async fn camera_ws_handler(
    Path(camera_name): Path<String>,
    ws: WebSocketUpgrade,
    State(state): State<crate::server::AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| receive_frames(socket, camera_name, state.camera_frames))
}

/// Принимает JPEG-кадры. Старый кадр сразу заменяется свежим.
async fn receive_frames(mut socket: WebSocket, camera_name: String, frames: CameraFrames) {
    println!("камера подключена: {camera_name}");
    while let Some(message) = socket.recv().await {
        let Ok(Message::Binary(jpeg)) = message else {
            continue;
        };
        match image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg) {
            Ok(decoded) => {
                let rgb = decoded.to_rgb8();
                let size = [rgb.width() as usize, rgb.height() as usize];
                let image = egui::ColorImage::from_rgb(size, rgb.as_raw());
                frames
                    .lock()
                    .unwrap()
                    .insert(camera_name.clone(), CameraFrame { image });
            }
            Err(error) => eprintln!("не удалось декодировать кадр от {camera_name}: {error}"),
        }
    }
    // Не показываем устаревший стоп-кадр после отключения камеры.
    frames.lock().unwrap().remove(&camera_name);
    println!("камера отключена: {camera_name}");
}

/// UI отдельного окна «Камеры».
pub struct CameraView {
    state: AppState,
    selected_camera: Option<String>,
    /// Камеры, которые были включены именно этим окном и должны быть выключены при закрытии.
    requested_cameras: HashSet<String>,
    texture: Option<egui::TextureHandle>,
    texture_camera: Option<String>,
}

impl CameraView {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            selected_camera: None,
            requested_cameras: HashSet::new(),
            texture: None,
            texture_camera: None,
        }
    }

    /// Рисует выбор компьютера и его последний кадр.
    pub fn ui(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("camera_list")
            .min_width(190.0)
            .show(ctx, |ui| {
                ui.heading("Компьютеры");
                ui.separator();
                let mut names: Vec<String> =
                    self.state.agents.lock().unwrap().keys().cloned().collect();
                names.sort();
                if names.is_empty() {
                    ui.weak("Нет подключённых компьютеров");
                }
                for name in names {
                    let selected = self.selected_camera.as_deref() == Some(name.as_str());
                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(selected, format!("📷  {name}"))
                            .clicked()
                        {
                            self.selected_camera = Some(name.clone());
                        }
                        if self.requested_cameras.contains(&name) {
                            if ui.small_button("Выключить").clicked() {
                                self.stop_camera(&name);
                            }
                        } else if ui.small_button("Включить просмотр").clicked() {
                            self.start_camera(&name);
                        }
                    });
                }
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(camera_name) = self.selected_camera.clone() else {
                ui.centered_and_justified(|ui| ui.weak("Выберите камеру слева"));
                return;
            };
            let frame = self
                .state
                .camera_frames
                .lock()
                .unwrap()
                .get(&camera_name)
                .map(|frame| frame.image.clone());
            let Some(frame) = frame else {
                self.texture = None;
                ui.centered_and_justified(|ui| {
                    ui.weak("Ожидание кадра: нажмите «Включить просмотр»")
                });
                return;
            };

            ui.heading(format!("Камера: {camera_name}"));
            ui.separator();
            if self.texture_camera.as_deref() != Some(camera_name.as_str()) {
                self.texture =
                    Some(ctx.load_texture("camera_stream", frame, egui::TextureOptions::LINEAR));
                self.texture_camera = Some(camera_name);
            } else if let Some(texture) = &mut self.texture {
                texture.set(frame, egui::TextureOptions::LINEAR);
            }
            if let Some(texture) = &self.texture {
                let available = ui.available_size();
                let original = texture.size_vec2();
                let scale = (available.x / original.x)
                    .min(available.y / original.y)
                    .min(1.0);
                ui.image((texture.id(), original * scale));
            }
        });
    }

    /// Останавливает все камеры, которые были включены в этом окне.
    pub fn stop_all(&mut self) {
        let names: Vec<String> = self.requested_cameras.drain().collect();
        for name in names {
            self.send_command(&name, ServerToAgent::StopCamera);
        }
        self.selected_camera = None;
        self.texture = None;
    }

    fn start_camera(&mut self, name: &str) {
        self.send_command(name, ServerToAgent::StartCamera);
        self.requested_cameras.insert(name.to_owned());
        self.selected_camera = Some(name.to_owned());
    }

    fn stop_camera(&mut self, name: &str) {
        self.send_command(name, ServerToAgent::StopCamera);
        self.requested_cameras.remove(name);
    }

    fn send_command(&self, name: &str, command: ServerToAgent) {
        if let Some(agent) = self.state.agents.lock().unwrap().get(name) {
            let _ = agent.cmd_tx.send(command);
        }
    }
}
