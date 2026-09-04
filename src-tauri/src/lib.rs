use tauri::{
    Emitter, Manager,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

use port_deck_engine::{ScanSnapshot, TerminateRequest};

#[tauri::command]
async fn scan_services() -> Result<ScanSnapshot, String> {
    tauri::async_runtime::spawn_blocking(|| port_deck_engine::scan_all_excluding("port-deck", 1420))
        .await
        .map_err(|error| format!("扫描任务失败: {error}"))
}

#[tauri::command]
async fn terminate_service(request: TerminateRequest) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || port_deck_engine::terminate(request))
        .await
        .map_err(|error| format!("结束任务失败: {error}"))?
}

#[tauri::command]
fn open_service(port: u16) -> Result<(), String> {
    port_deck_engine::open_local_service(port)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            scan_services,
            terminate_service,
            open_service
        ])
        .setup(|app| {
            let show_item = MenuItem::with_id(app, "show", "显示 Port Deck", true, None::<&str>)?;
            let refresh_item = MenuItem::with_id(app, "refresh", "重新扫描", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &refresh_item, &quit_item])?;

            TrayIconBuilder::new()
                .icon(
                    app.default_window_icon()
                        .expect("application icon is missing")
                        .clone(),
                )
                .tooltip("Port Deck · 本地运行资源")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main_window(app),
                    "refresh" => {
                        show_main_window(app);
                        let _ = app.emit("refresh-requested", ());
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main_window(tray.app_handle());
                    }
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to run Port Deck");
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}
