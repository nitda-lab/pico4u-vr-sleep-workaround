use crate::adb_client::{run_adb_device_command, run_adb_host_command};
use crate::config::{AppConfig, load_config, save_config};
use crate::state::AppState;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, State};
use tokio::time::{Duration, interval, sleep};

async fn run_adb_command(app: &AppHandle, args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err("No ADB command provided".to_string());
    }

    match args[0].as_str() {
        "devices" => run_adb_host_command(Some(app), "host:devices").await,
        "connect" if args.len() > 1 => {
            run_adb_host_command(Some(app), &format!("host:connect:{}", args[1])).await
        }
        "disconnect" => run_adb_host_command(Some(app), "host:disconnect:").await,
        "tcpip" if args.len() > 1 => {
            run_adb_device_command(Some(app), None, &format!("tcpip:{}", args[1])).await
        }
        "usb" => run_adb_device_command(Some(app), None, "usb:").await,
        "shell" => {
            let cmd = args[1..].join(" ");
            run_adb_device_command(Some(app), None, &format!("shell:{}", cmd)).await
        }
        "kill-server" => run_adb_host_command(Some(app), "host:kill").await,
        "-s" if args.len() > 2 => {
            let serial = &args[1];
            let cmd_type = &args[2];
            match cmd_type.as_str() {
                "shell" => {
                    let cmd = args[3..].join(" ");
                    run_adb_device_command(Some(app), Some(serial), &format!("shell:{}", cmd)).await
                }
                "tcpip" if args.len() > 3 => {
                    run_adb_device_command(Some(app), Some(serial), &format!("tcpip:{}", args[3]))
                        .await
                }
                "usb" => run_adb_device_command(Some(app), Some(serial), "usb:").await,
                _ => Err(format!("Unsupported device command with -s: {}", cmd_type)),
            }
        }
        _ => Err(format!("Unsupported ADB command in wrapper: {}", args[0])),
    }
}

fn format_ip_address(ip: &str) -> String {
    if ip.contains(':') {
        ip.to_string()
    } else {
        format!("{}:5555", ip)
    }
}

fn get_device_serial_args(ip: &str) -> Vec<String> {
    if ip.is_empty() {
        vec![]
    } else {
        vec!["-s".to_string(), format_ip_address(ip)]
    }
}

#[tauri::command]
pub async fn connect_device(app: AppHandle, ip: Option<String>) -> Result<String, String> {
    let args = match ip.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(ip_addr) => {
            let connection_str = format_ip_address(ip_addr);
            vec!["connect".to_string(), connection_str]
        }
        None => vec!["devices".to_string()],
    };

    run_adb_command(&app, &args).await
}

#[tauri::command]
pub async fn enable_tcpip(app: AppHandle) -> Result<String, String> {
    run_adb_command(&app, &vec!["tcpip".to_string(), "5555".to_string()]).await
}

#[tauri::command]
pub async fn set_usb_mode(app: AppHandle) -> Result<String, String> {
    run_adb_command(&app, &vec!["usb".to_string()]).await
}

#[tauri::command]
pub async fn disconnect_all_wireless(app: AppHandle) -> Result<String, String> {
    run_adb_command(&app, &vec!["disconnect".to_string()]).await
}

#[tauri::command]
pub async fn get_device_ip(app: AppHandle) -> Result<String, String> {
    let output = run_adb_command(
        &app,
        &vec![
            "shell".to_string(),
            "ip".to_string(),
            "addr".to_string(),
            "show".to_string(),
            "wlan0".to_string(),
        ],
    )
    .await?;

    // Parse output for inet address
    // Expected format: "    inet 192.168.1.10/24 ..."
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("inet ") {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() > 1 {
                // parts[1] should be IP/CIDR (e.g., 192.168.1.10/24)
                if let Some(ip) = parts[1].split('/').next() {
                    return Ok(ip.to_string());
                }
            }
        }
    }

    Err("Could not find IP address for wlan0. Ensure device is connected via USB.".to_string())
}

#[tauri::command]
pub async fn get_device_model(app: AppHandle) -> Result<String, String> {
    let output = run_adb_command(
        &app,
        &vec![
            "shell".to_string(),
            "getprop".to_string(),
            "ro.product.model".to_string(),
        ],
    )
    .await?;
    Ok(output.trim().to_string())
}

#[tauri::command]
pub async fn set_debug_mode(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    state.debug_mode.store(enabled, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub async fn start_keep_awake(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    _mode: String,
) -> Result<(), String> {
    if state.is_running.swap(true, Ordering::SeqCst) {
        return Err("Keep awake task is already running.".to_string());
    }

    let is_running = state.is_running.clone();
    let debug_mode = state.debug_mode.clone();
    let app_handle_task = app_handle.clone();
    let config = load_config(&app_handle);
    let interval_secs = config.keep_awake_interval_secs;

    // Main Keep-Awake Task
    let task = tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(interval_secs));

        while is_running.load(Ordering::Relaxed) {
            interval.tick().await;

            // Check device status first (optional but safer)
            let mut status_args = get_device_serial_args(&config.ip_address);
            status_args.extend_from_slice(&[
                "shell".to_string(),
                "dumpsys".to_string(),
                "power".to_string(),
            ]);

            let status_output = run_adb_command(&app_handle_task, &status_args).await;

            let should_wake = match status_output {
                Ok(output) => {
                    // Look for mWakefulness=Awake
                    !output.contains("mWakefulness=Awake")
                }
                Err(_) => true, // If check fails, try to wake up anyway
            };

            if should_wake {
                let mut wakeup_args = get_device_serial_args(&config.ip_address);
                wakeup_args.extend_from_slice(&[
                    "shell".to_string(),
                    "input".to_string(),
                    "keyevent".to_string(),
                    "224".to_string(),
                ]);

                let res = run_adb_command(&app_handle_task, &wakeup_args).await;

                if debug_mode.load(Ordering::Relaxed) {
                    let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
                    match res {
                        Ok(_) => {
                            let _ = app_handle_task
                                .emit("debug-log", format!("[{}] Sent wakeup event", timestamp));
                        }
                        Err(e) => {
                            let _ = app_handle_task
                                .emit("debug-log", format!("[{}] Wakeup error: {}", timestamp, e));
                        }
                    }
                }
            } else if debug_mode.load(Ordering::Relaxed) {
                let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
                let _ = app_handle_task.emit(
                    "debug-log",
                    format!("[{}] Device is already awake, skipping", timestamp),
                );
            }
        }
    });

    // Store task handle
    if let Ok(mut lock) = state.keep_awake_task.lock() {
        *lock = Some(task);
    }

    // Auto-Dim Task
    let config = load_config(&app_handle);
    if config.dim_delay_hours > 0.0 {
        let hours = config.dim_delay_hours;
        let is_running_dim = state.is_running.clone();
        let app_handle_dim = app_handle.clone();

        let dim_task = tokio::spawn(async move {
            let seconds = (hours * 3600.0) as u64;
            if seconds > 0 {
                sleep(Duration::from_secs(seconds)).await;
            }

            // Only execute if still running
            if is_running_dim.load(Ordering::Relaxed) {
                let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
                let _ = app_handle_dim.emit(
                    "debug-log",
                    format!("[{}] Executing auto-dim...", timestamp),
                );

                let mut dim_args = get_device_serial_args(&config.ip_address);
                dim_args.extend_from_slice(&[
                    "shell".to_string(),
                    "settings".to_string(),
                    "put".to_string(),
                    "system".to_string(),
                    "screen_brightness".to_string(),
                    "1".to_string(),
                ]);

                let _ = run_adb_command(&app_handle_dim, &dim_args).await;
            }
        });

        if let Ok(mut lock) = state.dim_task.lock() {
            *lock = Some(dim_task);
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn stop_keep_awake(state: State<'_, AppState>) -> Result<(), String> {
    if !state.is_running.swap(false, Ordering::SeqCst) {
        return Err("Keep awake task is not running.".to_string());
    }

    if let Ok(mut lock) = state.keep_awake_task.lock() {
        if let Some(task) = lock.take() {
            task.abort();
        }
    }

    if let Ok(mut lock) = state.dim_task.lock() {
        if let Some(task) = lock.take() {
            task.abort();
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn try_auto_connect(app: AppHandle, ip: String) -> Result<String, String> {
    let connection_str = format_ip_address(&ip);

    // Attempt adb connect
    let connect_result =
        run_adb_command(&app, &vec!["connect".to_string(), connection_str.clone()]).await;

    match connect_result {
        Ok(output) => {
            // "already connected" or "connected to" means success
            if output.contains("connected to") || output.contains("already connected") {
                // Verify with adb devices
                let devices = run_adb_command(&app, &vec!["devices".to_string()]).await?;
                if devices.contains(&ip) && devices.contains("device") {
                    return Ok(format!("connected to {}", connection_str));
                }
            }
            Err(format!("Connection failed: {}", output.trim()))
        }
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub async fn check_connection(app: AppHandle) -> Result<String, String> {
    run_adb_command(&app, &vec!["devices".to_string()]).await
}

#[tauri::command]
pub async fn kill_adb(app: AppHandle) -> Result<String, String> {
    run_adb_command(&app, &vec!["kill-server".to_string()]).await
}

#[tauri::command]
pub async fn get_config(app: AppHandle) -> Result<AppConfig, String> {
    Ok(load_config(&app))
}

#[tauri::command]
pub async fn save_config_cmd(app: AppHandle, config: AppConfig) -> Result<(), String> {
    save_config(&app, &config)
}
