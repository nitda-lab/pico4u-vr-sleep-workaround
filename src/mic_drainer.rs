//! Pico Connect の仮想マイク（PicoStreamingMicrophone）を常時 drain し、
//! バッファ滞留によるマイク遅延の累積を防ぐ。
//! 仕組みは PicoMicDrainer (nezumi-tech, MIT) を参考に Rust で再実装したもの。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tauri::{AppHandle, Emitter};

use crate::state::AppState;

/// 録音デバイス名が対象（PicoStreaming）かどうかを大文字小文字無視で判定する。
pub fn device_name_matches(name: &str) -> bool {
    name.to_lowercase().contains("picostreaming")
}

/// f32 サンプル列のピーク（絶対値の最大、0.0–1.0）を返す。空なら 0.0。
pub fn compute_peak_f32(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0_f32, |acc, &s| acc.max(s.abs()))
}

/// i16 サンプル列のピークを 0.0–1.0 に正規化して返す。空なら 0.0。
pub fn compute_peak_i16(samples: &[i16]) -> f32 {
    samples
        .iter()
        .fold(0.0_f32, |acc, &s| acc.max((s as f32 / i16::MAX as f32).abs()))
}

/// 排水スーパーバイザを起動する。既に稼働中なら何もしない。
pub fn start(app: AppHandle, state: &AppState) {
    // 既に有効なら二重起動しない
    if state.mic_drainer_enabled.swap(true, Ordering::SeqCst) {
        return;
    }

    let enabled = state.mic_drainer_enabled.clone();

    let task = tokio::spawn(async move {
        supervisor_loop(app, enabled).await;
    });

    if let Ok(mut lock) = state.mic_drainer_task.lock() {
        *lock = Some(task);
    }
}

/// 排水スーパーバイザを停止する。
pub fn stop(state: &AppState) {
    state.mic_drainer_enabled.store(false, Ordering::SeqCst);
    if let Ok(mut lock) = state.mic_drainer_task.lock() {
        if let Some(task) = lock.take() {
            task.abort();
        }
    }
}

/// スーパーバイザ本体。100ms ごとにピークを emit、5秒ごとにデバイス検出とワーカー管理を行う。
async fn supervisor_loop(app: AppHandle, enabled: Arc<AtomicBool>) {
    // ワーカーに渡す共有状態
    let peak_bits = Arc::new(AtomicU32::new(0)); // f32 のビット表現
    let worker_running = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::new(AtomicBool::new(false));

    let mut last_status = "";
    let mut tick: u64 = 0;

    while enabled.load(Ordering::Relaxed) {
        // 100ms ごと: ピーク emit
        let peak = f32::from_bits(peak_bits.load(Ordering::Relaxed));
        let _ = app.emit("mic-drainer-peak", peak);

        // 5秒ごと（tick 50 回に 1 回）: デバイス管理
        if tick % 50 == 0 {
            let running = worker_running.load(Ordering::Relaxed);
            if !running {
                // ワーカー停止中 → 再検出して起動を試みる
                worker_stop.store(false, Ordering::SeqCst);
                spawn_worker(
                    peak_bits.clone(),
                    worker_running.clone(),
                    worker_stop.clone(),
                );
                // 起動試行直後に稼働状態を再確認して status を決める
                thread::sleep(Duration::from_millis(200));
            }

            let status = if worker_running.load(Ordering::Relaxed) {
                "draining"
            } else {
                "waiting"
            };
            if status != last_status {
                let _ = app.emit("mic-drainer-status", status);
                last_status = status;
            }
        }

        tick = tick.wrapping_add(1);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // 終了処理: ワーカーを止めて off を通知
    worker_stop.store(true, Ordering::SeqCst);
    let _ = app.emit("mic-drainer-peak", 0.0_f32);
    let _ = app.emit("mic-drainer-status", "off");
}

/// PicoStreaming デバイスを探して入力ストリームを開くワーカースレッドを起動する。
/// デバイスが無い・開けない場合は worker_running を false のままにして即終了する。
fn spawn_worker(
    peak_bits: Arc<AtomicU32>,
    worker_running: Arc<AtomicBool>,
    worker_stop: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        let host = cpal::default_host();

        // 対象デバイスを検索
        let device = host.input_devices().ok().and_then(|mut devices| {
            devices.find(|d| d.name().map(|n| device_name_matches(&n)).unwrap_or(false))
        });

        let Some(device) = device else {
            worker_running.store(false, Ordering::SeqCst);
            return;
        };

        let Ok(default_config) = device.default_input_config() else {
            worker_running.store(false, Ordering::SeqCst);
            return;
        };

        let sample_format = default_config.sample_format();
        let config: cpal::StreamConfig = default_config.into();

        let peak_for_cb = peak_bits.clone();
        let err_running = worker_running.clone();

        let err_fn = move |_err| {
            // ストリームエラー時はワーカーを終了させ、スーパーバイザに再検出させる
            err_running.store(false, Ordering::SeqCst);
        };

        // サンプル形式ごとにストリームを構築（バッファは読み捨て、ピークだけ記録）
        let stream = match sample_format {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config,
                move |data: &[f32], _: &_| {
                    peak_for_cb.store(compute_peak_f32(data).to_bits(), Ordering::Relaxed);
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &config,
                move |data: &[i16], _: &_| {
                    peak_for_cb.store(compute_peak_i16(data).to_bits(), Ordering::Relaxed);
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_input_stream(
                &config,
                move |data: &[u16], _: &_| {
                    // u16 を中央 32768 基準で -1.0..1.0 に寄せてピーク算出
                    let peak = data.iter().fold(0.0_f32, |acc, &s| {
                        let v = (s as f32 - 32768.0) / 32768.0;
                        acc.max(v.abs())
                    });
                    peak_for_cb.store(peak.to_bits(), Ordering::Relaxed);
                },
                err_fn,
                None,
            ),
            _ => {
                worker_running.store(false, Ordering::SeqCst);
                return;
            }
        };

        let Ok(stream) = stream else {
            worker_running.store(false, Ordering::SeqCst);
            return;
        };

        if stream.play().is_err() {
            worker_running.store(false, Ordering::SeqCst);
            return;
        }

        worker_running.store(true, Ordering::SeqCst);

        // 停止フラグ or エラーで worker_running が落ちるまで保持
        // （Stream は !Send のためこのスレッドで保持し続ける）
        while !worker_stop.load(Ordering::Relaxed) && worker_running.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(100));
        }

        worker_running.store(false, Ordering::SeqCst);
        drop(stream);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_picostreaming_case_insensitive() {
        assert!(device_name_matches("PicoStreamingMicrophone"));
        assert!(device_name_matches("pico streaming? picostreaming yes"));
        assert!(device_name_matches("PICOSTREAMING"));
    }

    #[test]
    fn does_not_match_other_devices() {
        assert!(!device_name_matches("Realtek High Definition Audio"));
        assert!(!device_name_matches("Microphone Array"));
        assert!(!device_name_matches(""));
    }

    #[test]
    fn peak_f32_returns_max_abs() {
        assert_eq!(compute_peak_f32(&[]), 0.0);
        assert_eq!(compute_peak_f32(&[0.1, -0.5, 0.3]), 0.5);
        assert_eq!(compute_peak_f32(&[-0.9, 0.2]), 0.9);
    }

    #[test]
    fn peak_i16_normalizes() {
        assert_eq!(compute_peak_i16(&[]), 0.0);
        assert_eq!(compute_peak_i16(&[i16::MAX]), 1.0);
        // -16383 ≒ -0.5（i16::MAX=32767 で正規化）
        let p = compute_peak_i16(&[-16383, 100]);
        assert!((p - 0.4999).abs() < 0.01);
    }
}
