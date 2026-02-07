use std::sync::Mutex;

use portpicker::pick_unused_port;
use tauri::{Manager, WindowUrl};

struct BackendState {
    child: Mutex<Option<tauri::api::process::Child>>,
}

fn spawn_backend(port: u16) -> Result<tauri::api::process::Child, Box<dyn std::error::Error>> {
    let mut cmd = tauri::api::process::Command::new_sidecar("server")?;
    cmd = cmd
        .env("BACKEND_PORT", port.to_string())
        .env("HOST", "127.0.0.1")
        .env("RUST_LOG", "info");

    let (_rx, child) = cmd.spawn()?;

    Ok(child)
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let port = pick_unused_port().unwrap_or(3999);
            let child = spawn_backend(port)?;

            app.manage(BackendState {
                child: Mutex::new(Some(child)),
            });

            if let Some(window) = app.get_window("main") {
                let url = format!("http://127.0.0.1:{}", port);
                window.set_url(WindowUrl::External(url.parse()?))?;
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, event| match event {
            tauri::RunEvent::ExitRequested { .. } => {
                if let Some(state) = app.try_state::<BackendState>() {
                    if let Ok(mut guard) = state.child.lock() {
                        if let Some(mut child) = guard.take() {
                            let _ = child.kill();
                        }
                    }
                }
            }
            _ => {}
        });
}
