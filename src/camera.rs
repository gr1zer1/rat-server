
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Instant;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use minifb::{Key, Window, WindowOptions};

/// Один декодированный кадр, готовый к отрисовке в окне.
struct FrameMsg {
    width: usize,
    height: usize,
    /// Буфер в формате 0x00RRGGBB на пиксель — то, что ожидает minifb
    buffer: Vec<u32>,
}

#[derive(Clone)]
struct AppState {
    frame_tx: Sender<FrameMsg>,
}

fn main() {
    // std::sync::mpsc — канал между async-миром Axum (tokio) и синхронным окном minifb.
    // Окно должно жить и обновляться на главном потоке, поэтому Axum-сервер уезжает
    // на отдельный поток со своим tokio-рантаймом.
    let (frame_tx, frame_rx) = channel::<FrameMsg>();
    let state = AppState { frame_tx };

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("failed to build tokio runtime");
        rt.block_on(run_server(state));
    });

    run_window(frame_rx);
}

async fn run_server(state: AppState) {
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8000")
        .await
        .expect("failed to bind port 8000");

    println!("listening on ws://0.0.0.0:8000/ws");
    axum::serve(listener, app).await.expect("server error");
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    // Диагностика: считаем реальный fps приёма и время декодирования,
    // чтобы понять, где узкое место — сеть/клиент или декодирование/отрисовка.
    let mut frames_received = 0u32;
    let mut window_start = Instant::now();
    let mut decode_time_total = std::time::Duration::ZERO;

    while let Some(msg) = socket.recv().await {
        let recv_at = Instant::now();

        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                eprintln!("websocket recv error: {e}");
                break;
            }
        };

        let data = match msg {
            Message::Binary(bytes) => bytes,
            Message::Close(_) => break,
            _ => continue, // текстовые/ping-сообщения нас тут не интересуют
        };

        let bytes_len = data.len();
        let decode_start = Instant::now();

        match image::load_from_memory_with_format(&data, image::ImageFormat::Jpeg) {
            Ok(img) => {
                let rgb = img.to_rgb8();
                let width = rgb.width() as usize;
                let height = rgb.height() as usize;
                let buffer = rgb_to_minifb_buffer(&rgb);
                decode_time_total += decode_start.elapsed();

                // Если окно ещё не забирает кадры (например, закрывается), просто пропускаем —
                // это не должно рвать соединение с клиентом.
                let _ = state.frame_tx.send(FrameMsg {
                    width,
                    height,
                    buffer,
                });
            }
            Err(e) => eprintln!("failed to decode JPEG frame: {e}"),
        }

        frames_received += 1;
        let _ = recv_at; // зарезервировано, если понадобится точнее замерить время между recv() и получением байт

        if window_start.elapsed().as_secs() >= 1 {
            let fps = frames_received;
            let avg_decode_ms = if fps > 0 {
                decode_time_total.as_secs_f64() * 1000.0 / fps as f64
            } else {
                0.0
            };
            println!(
                "[diag] received fps: {fps}, avg decode: {avg_decode_ms:.2}ms, last frame size: {bytes_len} bytes"
            );
            frames_received = 0;
            decode_time_total = std::time::Duration::ZERO;
            window_start = Instant::now();
        }
    }
}

/// Конвертирует RGB-изображение в плоский буфер u32 (0x00RRGGBB на пиксель),
/// как того требует minifb.
fn rgb_to_minifb_buffer(img: &image::RgbImage) -> Vec<u32> {
    img.pixels()
        .map(|p| {
            let [r, g, b] = p.0;
            ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
        })
        .collect()
}

/// Цикл окна на главном потоке: берёт самый свежий кадр из канала и рисует его.
fn run_window(rx: Receiver<FrameMsg>) {
    let mut width = 640usize;
    let mut height = 480usize;
    let mut buffer: Vec<u32> = vec![0; width * height];

    let mut window = Window::new("Camera Stream", width, height, WindowOptions::default())
        .expect("failed to create window");

    // Ограничиваем частоту перерисовки, чтобы не грузить CPU впустую
    window.set_target_fps(60);

    while window.is_open() && !window.is_key_down(Key::Escape) {
        // Забираем ВСЕ накопившиеся кадры и оставляем только последний —
        // так окно не будет "отставать", даже если кадры приходят быстрее, чем рисуется окно.
        let mut latest: Option<FrameMsg> = None;
        while let Ok(frame) = rx.try_recv() {
            latest = Some(frame);
        }

        if let Some(frame) = latest {
            if frame.width != width || frame.height != height {
                // Разрешение сменилось (например, переподключился другой клиент) —
                // пересоздаём окно под новый размер.
                width = frame.width;
                height = frame.height;
                window = Window::new("Camera Stream", width, height, WindowOptions::default())
                    .expect("failed to recreate window");
                window.set_target_fps(60);
            }
            buffer = frame.buffer;
        }

        window
            .update_with_buffer(&buffer, width, height)
            .expect("failed to update window buffer");
    }
}