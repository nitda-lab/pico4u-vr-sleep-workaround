use std::sync::atomic::Ordering;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_shell::ShellExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

const ADB_PORT: u16 = 5037;
const TIMEOUT: Duration = Duration::from_secs(5);

pub async fn run_adb_host_command(
    app: Option<&AppHandle>,
    command: &str,
) -> Result<String, String> {
    let mut stream = connect_adb(app).await?;

    let req = format!("{:04x}{}", command.len(), command);
    stream
        .write_all(req.as_bytes())
        .await
        .map_err(|e| e.to_string())?;

    let mut status = [0u8; 4];
    stream
        .read_exact(&mut status)
        .await
        .map_err(|e| e.to_string())?;

    if &status != b"OKAY" {
        return Err("ADB server rejected request".to_string());
    }

    let mut len_buf = [0u8; 4];
    if stream.read_exact(&mut len_buf).await.is_err() {
        return Ok("".to_string());
    }

    let len_str = std::str::from_utf8(&len_buf).unwrap_or("0");
    let len = usize::from_str_radix(len_str, 16).unwrap_or(0);

    if len > 0 {
        let mut data = vec![0u8; len];
        stream
            .read_exact(&mut data)
            .await
            .map_err(|e| e.to_string())?;
        Ok(String::from_utf8_lossy(&data).to_string())
    } else {
        Ok("".to_string())
    }
}

pub async fn run_adb_device_command(
    app: Option<&AppHandle>,
    serial: Option<&str>,
    command: &str,
) -> Result<String, String> {
    let mut stream = connect_adb(app).await?;

    let transport_req = if let Some(s) = serial {
        format!("host:transport:{}", s)
    } else {
        "host:transport-any".to_string()
    };

    let req = format!("{:04x}{}", transport_req.len(), transport_req);
    stream
        .write_all(req.as_bytes())
        .await
        .map_err(|e| e.to_string())?;

    let mut status = [0u8; 4];
    stream
        .read_exact(&mut status)
        .await
        .map_err(|e| e.to_string())?;
    if &status != b"OKAY" {
        return Err("ADB server rejected transport. Is the device connected?".to_string());
    }

    let req = format!("{:04x}{}", command.len(), command);
    stream
        .write_all(req.as_bytes())
        .await
        .map_err(|e| e.to_string())?;

    stream
        .read_exact(&mut status)
        .await
        .map_err(|e| e.to_string())?;
    if &status != b"OKAY" {
        return Err(format!("ADB server rejected device command '{}'", command));
    }

    let mut output = String::new();
    stream
        .read_to_string(&mut output)
        .await
        .map_err(|e| e.to_string())?;

    Ok(output)
}

async fn connect_adb(app: Option<&AppHandle>) -> Result<TcpStream, String> {
    match timeout(
        TIMEOUT,
        TcpStream::connect(format!("127.0.0.1:{}", ADB_PORT)),
    )
    .await
    {
        Ok(Ok(stream)) => Ok(stream),
        _ => {
            if let Some(app) = app {
                if let Ok(command) = app.shell().sidecar("adb") {
                    let _ = command.args(["start-server"]).output().await;
                    if let Ok(Ok(stream)) = timeout(
                        TIMEOUT,
                        TcpStream::connect(format!("127.0.0.1:{}", ADB_PORT)),
                    )
                    .await
                    {
                        // Mark that we started the server so we can safely kill it on exit
                        let state = app.state::<crate::state::AppState>();
                        state.adb_started_by_us.store(true, Ordering::SeqCst);
                        return Ok(stream);
                    }
                }
            }
            Err("Failed to connect to ADB server and failed to start it.".to_string())
        }
    }
}
