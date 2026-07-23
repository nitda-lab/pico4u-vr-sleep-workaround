# マイク排水機能 統合 実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** pico4u に「PicoStreamingMicrophone を常時 drain して Pico Connect 経由のマイク遅延累積を防ぐ」機能を、ON/OFF トグル・ステータス・レベルメーター付きで統合する。

**Architecture:** Rust バックエンドに `cpal` ベースの排水モジュールを追加。ワーカースレッド（cpal `Stream` は `!Send`）が PicoStreaming デバイスを開いてバッファを読み捨て、ピークを共有アトミックに書く。tokio スーパーバイザタスクが 5秒ごとにデバイスの出入りを検出してワーカーを起動/停止し、100ms ごとにピークを Tauri イベントで送出。フロントは既存の Tailwind/@charcoal-ui スタイルのカードで状態表示とトグルを提供する。既存の V睡機能には触れない。

**Tech Stack:** Tauri 2 / Rust (edition 2024, tokio, cpal) / React 19 + TypeScript + Tailwind + @charcoal-ui + react-i18next。

## Global Constraints

- ランタイムは Windows 11 前提（cpal は WASAPI ホストを使用）。
- 対象デバイス判定キーワードは大文字小文字無視の `"picostreaming"`（PicoMicDrainer と同一）。
- 既存の V睡（keep-awake / auto-dim）機能・既存コマンド・設定フィールドは変更しない（追加のみ）。
- 設定は既存 `AppConfig`（`#[serde(default)]` 済み）にフィールド追加。デフォルト `mic_drainer_enabled = true`。
- Tauri イベント名: 状態 `mic-drainer-status`（値 `"draining"` / `"waiting"` / `"off"`）、ピーク `mic-drainer-peak`（f32, 0.0–1.0）。
- フロントは既存コンポーネント（`ConnectionPanel` 等）と同じカード様式（`bg-white dark:bg-gray-800 rounded-lg p-5 border`）・`bg-brand` アクセント・`@charcoal-ui/react` の `Checkbox` を使う。絵文字は使わない。
- 検証コマンド（作業ディレクトリ `C:\MyProject\pico4u-vr-sleep-workaround`）: Rust は `cargo test` / `cargo check`（Cargo.toml のあるルート）、フロントは `frontend/` で `pnpm run build`（tsc 込み）と `pnpm run lint`（oxlint）。
- 参考元 PicoMicDrainer（nezumi-tech, MIT）は仕組みのみ参照。コードコピー禁止。

---

### Task 1: cpal 依存追加とビルド確認

**Files:**
- Modify: `Cargo.toml`（`[dependencies]` に cpal 追加）

**Interfaces:**
- Consumes: なし
- Produces: cpal クレートが利用可能になる（`cpal::default_host()` 等）。

- [ ] **Step 1: Cargo.toml に cpal を追加**

`Cargo.toml` の `[dependencies]` 末尾（`adb_client = "3.2.1"` の次行）に追加:

```toml
cpal = "0.15"
```

- [ ] **Step 2: 依存解決とコンパイルを確認**

作業ディレクトリ `C:\MyProject\pico4u-vr-sleep-workaround` で実行:

Run: `cargo fetch` の後 `cargo check`
Expected: cpal を含めてダウンロード・コンパイルが成功し、エラーなし（既存コードは無変更なので警告のみ許容）。

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "build: cpal を依存に追加（マイク排水機能用）"
```

---

### Task 2: 排水モジュールの純粋関数（デバイス名マッチ・ピーク計算）を TDD で実装

**Files:**
- Create: `src/mic_drainer.rs`
- Modify: `src/main.rs`（`mod mic_drainer;` を追加してモジュールをツリーに登録）

**Interfaces:**
- Consumes: なし
- Produces:
  - `pub fn device_name_matches(name: &str) -> bool`
  - `pub fn compute_peak_f32(samples: &[f32]) -> f32`
  - `pub fn compute_peak_i16(samples: &[i16]) -> f32`

- [ ] **Step 1: 失敗するテストを書く**

`src/mic_drainer.rs` を新規作成し、以下を記述:

```rust
//! Pico Connect の仮想マイク（PicoStreamingMicrophone）を常時 drain し、
//! バッファ滞留によるマイク遅延の累積を防ぐ。
//! 仕組みは PicoMicDrainer (nezumi-tech, MIT) を参考に Rust で再実装したもの。

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
```

`src/main.rs` のモジュール宣言部（`mod adb_client;` の並び、`mod config;` の後）に追加:

```rust
mod mic_drainer;
```

- [ ] **Step 2: テストが（コンパイルして）通ることを確認**

Run: `cargo test --lib mic_drainer`（バイナリクレートのため `cargo test mic_drainer` でも可）
Expected: `matches_picostreaming_case_insensitive` 他 4 テストが PASS。

※このタスクは実装と同時にテストを満たす形（純粋関数が短いため）。もし赤→緑を厳密に踏むなら、先に関数本体を `todo!()` にしてテスト失敗を確認してから埋める。

- [ ] **Step 3: Commit**

```bash
git add src/mic_drainer.rs src/main.rs
git commit -m "feat(mic-drainer): デバイス名マッチとピーク計算の純粋関数を追加"
```

---

### Task 3: AppState と AppConfig に排水用フィールドを追加

**Files:**
- Modify: `src/state.rs`
- Modify: `src/config.rs`

**Interfaces:**
- Consumes: なし
- Produces:
  - `AppState.mic_drainer_task: Mutex<Option<JoinHandle<()>>>`
  - `AppState.mic_drainer_enabled: Arc<AtomicBool>`
  - `AppConfig.mic_drainer_enabled: bool`（default `true`）

- [ ] **Step 1: AppState にフィールドを追加**

`src/state.rs` を編集。構造体定義に 2 フィールド追加:

```rust
pub struct AppState {
    pub keep_awake_task: Mutex<Option<JoinHandle<()>>>,
    pub dim_task: Mutex<Option<JoinHandle<()>>>,
    pub is_running: Arc<AtomicBool>,
    pub debug_mode: Arc<AtomicBool>,
    pub adb_started_by_us: Arc<AtomicBool>,
    pub mic_drainer_task: Mutex<Option<JoinHandle<()>>>,
    pub mic_drainer_enabled: Arc<AtomicBool>,
}
```

`Default` 実装にも対応する初期化を追加:

```rust
impl Default for AppState {
    fn default() -> Self {
        Self {
            keep_awake_task: Mutex::new(None),
            dim_task: Mutex::new(None),
            is_running: Arc::new(AtomicBool::new(false)),
            debug_mode: Arc::new(AtomicBool::new(false)),
            adb_started_by_us: Arc::new(AtomicBool::new(false)),
            mic_drainer_task: Mutex::new(None),
            mic_drainer_enabled: Arc::new(AtomicBool::new(false)),
        }
    }
}
```

※`mic_drainer_enabled` の初期値は `false`。実際の有効/無効は起動時に config から反映する（Task 6）。

- [ ] **Step 2: AppConfig にフィールドを追加**

`src/config.rs` の `AppConfig` 構造体に追加:

```rust
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct AppConfig {
    pub dim_delay_hours: f64,
    pub ip_address: String,
    pub keep_awake_interval_secs: u64,
    pub last_connection_mode: Option<String>,
    pub mic_drainer_enabled: bool,
}
```

`Default` 実装に追加:

```rust
impl Default for AppConfig {
    fn default() -> Self {
        Self {
            dim_delay_hours: 1.0,
            ip_address: String::new(),
            keep_awake_interval_secs: 3,
            last_connection_mode: None,
            mic_drainer_enabled: true,
        }
    }
}
```

- [ ] **Step 3: コンパイル確認**

Run: `cargo check`
Expected: 成功。`#[serde(default)]` により旧設定ファイル（当該キーなし）を読んでも `Default`（`true`）で埋まる。

- [ ] **Step 4: Commit**

```bash
git add src/state.rs src/config.rs
git commit -m "feat(mic-drainer): AppState/AppConfig に排水用の状態・設定フィールドを追加"
```

---

### Task 4: 排水ワーカー＋スーパーバイザ（start/stop）を実装

**Files:**
- Modify: `src/mic_drainer.rs`（`start` / `stop` と内部ヘルパを追加）

**Interfaces:**
- Consumes:
  - `device_name_matches` / `compute_peak_f32` / `compute_peak_i16`（Task 2）
  - `AppState`（Task 3）
  - `cpal`（Task 1）
- Produces:
  - `pub fn start(app: tauri::AppHandle, state: &AppState)` — 排水スーパーバイザを起動し、ハンドルを `state.mic_drainer_task` に格納。`state.mic_drainer_enabled` を `true` にする。多重起動しない。
  - `pub fn stop(state: &AppState)` — `mic_drainer_enabled` を `false` にし、スーパーバイザタスクを abort。

- [ ] **Step 1: モジュール冒頭の use と start/stop を追加**

`src/mic_drainer.rs` の先頭ドキュメントコメント直後（純粋関数の前）に use を追加:

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tauri::{AppHandle, Emitter};

use crate::state::AppState;
```

同ファイル末尾（`#[cfg(test)]` の前）に排水制御を追加:

```rust
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
                move |data: &[f32], _| {
                    peak_for_cb.store(compute_peak_f32(data).to_bits(), Ordering::Relaxed);
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &config,
                move |data: &[i16], _| {
                    peak_for_cb.store(compute_peak_i16(data).to_bits(), Ordering::Relaxed);
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_input_stream(
                &config,
                move |data: &[u16], _| {
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

        // 停止フラグ or エラーで worker_running が落ちるまで保持（Stream は !Send のためこのスレッドで保持）
        while !worker_stop.load(Ordering::Relaxed) && worker_running.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(100));
        }

        worker_running.store(false, Ordering::SeqCst);
        drop(stream);
    });
}
```

- [ ] **Step 2: コンパイル確認**

Run: `cargo check`
Expected: 成功（既存純粋関数テストも壊れない）。cpal の型（`SampleFormat`, `StreamConfig`, `build_input_stream`）が解決されること。

- [ ] **Step 3: 純粋関数テストが引き続き通ることを確認**

Run: `cargo test mic_drainer`
Expected: Task 2 の 4 テストが PASS（新規ロジックはテスト対象外だがコンパイルに巻き込まれる）。

- [ ] **Step 4: Commit**

```bash
git add src/mic_drainer.rs
git commit -m "feat(mic-drainer): 排水ワーカースレッドとスーパーバイザ(start/stop)を実装"
```

---

### Task 5: set_mic_drainer_enabled コマンドを追加

**Files:**
- Modify: `src/commands.rs`

**Interfaces:**
- Consumes: `mic_drainer::start` / `mic_drainer::stop`（Task 4）、`AppConfig` / `load_config` / `save_config`（既存）、`AppState`（Task 3）
- Produces:
  - `pub async fn set_mic_drainer_enabled(app: AppHandle, state: State<'_, AppState>, enabled: bool) -> Result<(), String>`

- [ ] **Step 1: コマンドを実装**

`src/commands.rs` 末尾（`save_config_cmd` の後）に追加:

```rust
#[tauri::command]
pub async fn set_mic_drainer_enabled(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    // 設定を永続化
    let mut config = load_config(&app);
    config.mic_drainer_enabled = enabled;
    save_config(&app, &config)?;

    // 稼働状態を反映
    if enabled {
        crate::mic_drainer::start(app.clone(), &state);
    } else {
        crate::mic_drainer::stop(&state);
    }

    Ok(())
}
```

ファイル先頭の use に `State` が既にあることを確認（`use tauri::{AppHandle, Emitter, State};` が既存）。`load_config` / `save_config` も既存 import 済み。

- [ ] **Step 2: コンパイル確認**

Run: `cargo check`
Expected: 成功。

- [ ] **Step 3: Commit**

```bash
git add src/commands.rs
git commit -m "feat(mic-drainer): set_mic_drainer_enabled コマンドを追加"
```

---

### Task 6: main.rs へコマンド登録・起動時自動開始・終了時停止を追加

**Files:**
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `set_mic_drainer_enabled`（Task 5）、`mic_drainer::start` / `stop`（Task 4）、`load_config`（既存）
- Produces: アプリ起動時に config に従って排水を自動開始し、終了時に停止する挙動。

- [ ] **Step 1: invoke_handler にコマンドを登録**

`src/main.rs` の `tauri::generate_handler![ ... ]` の最後の要素 `save_config_cmd` の後にカンマ区切りで追加:

```rust
            save_config_cmd,
            set_mic_drainer_enabled
```

- [ ] **Step 2: setup で起動時自動開始**

現在の `.setup(|_app| Ok(()))` を以下に置き換える:

```rust
        .setup(|app| {
            let handle = app.handle().clone();
            let config = crate::config::load_config(&handle);
            if config.mic_drainer_enabled {
                let state = handle.state::<AppState>();
                crate::mic_drainer::start(handle.clone(), &state);
            }
            Ok(())
        })
```

※`use tauri::Manager;` は既存（`app_handle.state::<AppState>()` を使うため）。`config` モジュールは `crate::config::load_config` で参照。

- [ ] **Step 3: 終了時にワーカーを停止**

`RunEvent::Exit` のアーム内、既存の adb kill 処理の直前に追加（`let state = app_handle.state::<AppState>();` は既存の行を再利用）:

```rust
            tauri::RunEvent::Exit => {
                let state = app_handle.state::<AppState>();
                crate::mic_drainer::stop(&state);
                if state.adb_started_by_us.load(Ordering::SeqCst) {
                    if let Ok(sidecar) = app_handle.shell().sidecar("adb") {
                        let _ = tauri::async_runtime::block_on(async {
                            let _ = sidecar.args(["kill-server"]).output().await;
                        });
                    }
                }
            }
```

- [ ] **Step 4: コンパイル確認**

Run: `cargo check`
Expected: 成功。

- [ ] **Step 5: Rust 全体のテスト実行**

Run: `cargo test`
Expected: `mic_drainer` の純粋関数テスト 4 件が PASS、既存テストがあれば維持。

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "feat(mic-drainer): コマンド登録・起動時自動開始・終了時停止を追加"
```

---

### Task 7: i18n に排水パネルの文言（日英）を追加

**Files:**
- Modify: `frontend/src/i18n.ts`

**Interfaces:**
- Consumes: なし
- Produces: 翻訳キー `mic_drainer_title`, `mic_drainer_status_draining`, `mic_drainer_status_waiting`, `mic_drainer_status_off`, `mic_drainer_toggle`, `mic_drainer_note`（en / ja 両方）。

- [ ] **Step 1: en 側にキーを追加**

`frontend/src/i18n.ts` の `resources.en.translation` オブジェクト内（既存キー群の適当な位置、例えば `debug_mode` の近く）に追加:

```ts
      mic_drainer_title: 'Mic Delay Fix',
      mic_drainer_status_draining: 'Draining',
      mic_drainer_status_waiting: 'Waiting (device not found)',
      mic_drainer_status_off: 'Stopped',
      mic_drainer_toggle: 'Prevent mic delay buildup (Pico Connect)',
      mic_drainer_note: 'Works only while Pico Connect is running.',
```

- [ ] **Step 2: ja 側にキーを追加**

`resources.ja.translation` オブジェクト内の対応する位置に追加:

```ts
      mic_drainer_title: 'マイク遅延対策',
      mic_drainer_status_draining: '排水中',
      mic_drainer_status_waiting: '待機中（デバイス未検出）',
      mic_drainer_status_off: '停止',
      mic_drainer_toggle: 'マイク遅延の蓄積を防ぐ（Pico Connect）',
      mic_drainer_note: 'Pico Connect 起動中のみ動作します。',
```

※`ja.translation` の正確なキー配置は既存ファイルを Read して確認し、`en` と同じキー集合になるように揃える。

- [ ] **Step 3: フロントのビルド確認**

作業ディレクトリ `frontend/` で実行:

Run: `pnpm run build`
Expected: 型エラーなくビルド成功。

- [ ] **Step 4: Commit**

```bash
git add frontend/src/i18n.ts
git commit -m "feat(mic-drainer): 排水パネルの日英文言を i18n に追加"
```

---

### Task 8: useAppLogic に排水の状態・トグル・イベントリスナを追加

**Files:**
- Modify: `frontend/src/hooks/useAppLogic.ts`

**Interfaces:**
- Consumes: バックエンドの `get_config`（`mic_drainer_enabled` を含むよう Task 3 で拡張済み）、`set_mic_drainer_enabled` コマンド（Task 5）、Tauri イベント `mic-drainer-status` / `mic-drainer-peak`（Task 4）。
- Produces: フックの返り値に以下を追加:
  - `micDrainerEnabled: boolean`
  - `micDrainerStatus: 'draining' | 'waiting' | 'off'`
  - `micDrainerPeak: number`
  - `toggleMicDrainer: (enabled: boolean) => Promise<void>`

- [ ] **Step 1: state を追加**

`useAppLogic` 関数内の state 宣言群（`const [dimAfterHours, ...]` 付近）に追加:

```ts
  const [micDrainerEnabled, setMicDrainerEnabled] = useState<boolean>(true)
  const [micDrainerStatus, setMicDrainerStatus] = useState<'draining' | 'waiting' | 'off'>('off')
  const [micDrainerPeak, setMicDrainerPeak] = useState<number>(0)
```

- [ ] **Step 2: 初期化で config から反映**

`initApp` 内、`get_config` の戻り値型に `mic_drainer_enabled` を足し、反映する。まず型注釈を拡張:

```ts
        const config = await invoke<{
          dim_delay_hours: number
          ip_address: string
          keep_awake_interval_secs: number
          last_connection_mode: 'wired' | 'wireless' | null
          mic_drainer_enabled: boolean
        }>('get_config')
```

`setKeepAwakeInterval(config.keep_awake_interval_secs)` の直後に追加:

```ts
        setMicDrainerEnabled(config.mic_drainer_enabled)
```

- [ ] **Step 3: イベントリスナを追加**

既存の `debug-log` リスナ用 `useEffect` の直後に、排水イベント用の `useEffect` を追加:

```ts
  // Mic drainer status / peak listeners
  useEffect(() => {
    const statusPromise = listen<string>('mic-drainer-status', (event) => {
      const v = event.payload
      if (v === 'draining' || v === 'waiting' || v === 'off') {
        setMicDrainerStatus(v)
      }
    }).catch(() => () => {})

    const peakPromise = listen<number>('mic-drainer-peak', (event) => {
      if (typeof event.payload === 'number') {
        setMicDrainerPeak(event.payload)
      }
    }).catch(() => () => {})

    return () => {
      statusPromise.then((fn) => fn && fn())
      peakPromise.then((fn) => fn && fn())
    }
  }, [])
```

- [ ] **Step 4: toggle 関数を追加**

`toggleKeepAwake` の定義付近に追加:

```ts
  const toggleMicDrainer = useCallback(async (enabled: boolean) => {
    setMicDrainerEnabled(enabled)
    try {
      await invoke('set_mic_drainer_enabled', { enabled })
    } catch (e) {
      console.error('Failed to toggle mic drainer:', e)
    }
  }, [])
```

- [ ] **Step 5: 返り値に追加**

`return { ... }` オブジェクトの末尾（`retryAutoConnect,` の後）に追加:

```ts
    micDrainerEnabled,
    micDrainerStatus,
    micDrainerPeak,
    toggleMicDrainer,
```

- [ ] **Step 6: 型・ビルド確認**

作業ディレクトリ `frontend/`:

Run: `pnpm run build`
Expected: 型エラーなくビルド成功。`AppContext` は `ReturnType<typeof useAppLogic>` なので新フィールドが自動的に露出する。

- [ ] **Step 7: Commit**

```bash
git add frontend/src/hooks/useAppLogic.ts
git commit -m "feat(mic-drainer): useAppLogic に排水の状態・トグル・イベントリスナを追加"
```

---

### Task 9: MicDrainerPanel コンポーネントを作成しホームに配置

**Files:**
- Create: `frontend/src/components/MicDrainerPanel.tsx`
- Modify: `frontend/src/components/ModeSelection.tsx`

**Interfaces:**
- Consumes: `useAppContext`（`t`, `micDrainerEnabled`, `micDrainerStatus`, `micDrainerPeak`, `toggleMicDrainer`）
- Produces: `export function MicDrainerPanel(): JSX.Element`

- [ ] **Step 1: MicDrainerPanel を作成**

`frontend/src/components/MicDrainerPanel.tsx` を新規作成:

```tsx
import { Checkbox } from '@charcoal-ui/react'
import { useAppContext } from '../context/AppContext'

export function MicDrainerPanel() {
  const { t, micDrainerEnabled, micDrainerStatus, micDrainerPeak, toggleMicDrainer } =
    useAppContext()

  // ステータスに応じたドット色
  const dotClass =
    micDrainerStatus === 'draining'
      ? 'bg-brand'
      : micDrainerStatus === 'waiting'
        ? 'bg-amber-400'
        : 'bg-gray-300 dark:bg-gray-600'

  const statusText =
    micDrainerStatus === 'draining'
      ? t('mic_drainer_status_draining')
      : micDrainerStatus === 'waiting'
        ? t('mic_drainer_status_waiting')
        : t('mic_drainer_status_off')

  // レベルメーター幅（0–100%）。off のときは 0。
  const meterPct = micDrainerEnabled ? Math.min(100, Math.round(micDrainerPeak * 100)) : 0

  return (
    <div className='bg-white dark:bg-gray-800 rounded-lg p-5 border border-gray-200 dark:border-gray-700 w-full max-w-sm'>
      {/* ヘッダー: タイトル + ステータス */}
      <div className='flex justify-between items-center mb-3'>
        <span className='font-semibold text-sm text-gray-800 dark:text-gray-200'>
          {t('mic_drainer_title')}
        </span>
        <div className='flex items-center gap-1.5'>
          <span className={`relative inline-flex rounded-full h-2 w-2 ${dotClass}`}>
            {micDrainerStatus === 'draining' && (
              <span className='animate-ping absolute inline-flex h-full w-full rounded-full bg-brand opacity-60' />
            )}
          </span>
          <span className='text-xs text-gray-500 dark:text-gray-400'>{statusText}</span>
        </div>
      </div>

      {/* レベルメーター */}
      <div className='h-2 w-full rounded-full bg-gray-100 dark:bg-gray-700 overflow-hidden mb-3'>
        <div
          className='h-full rounded-full bg-brand transition-[width] duration-100 ease-out'
          style={{ width: `${meterPct}%` }}
        />
      </div>

      {/* トグル */}
      <div className='text-sm text-gray-900 dark:text-gray-100'>
        <Checkbox checked={micDrainerEnabled} onChange={toggleMicDrainer}>
          {t('mic_drainer_toggle')}
        </Checkbox>
      </div>

      {/* 補足 */}
      <p className='mt-2 text-xs text-gray-400 dark:text-gray-500'>{t('mic_drainer_note')}</p>
    </div>
  )
}
```

- [ ] **Step 2: ModeSelection のホーム画面に配置**

`frontend/src/components/ModeSelection.tsx` を編集。import に追加:

```tsx
import { MicDrainerPanel } from './MicDrainerPanel'
```

`homeView === 'main'` 分岐内、モード選択カードを囲む `<div className='flex-1 flex flex-col items-center justify-center px-5'>` の中、モード選択カード（`<div className='bg-white ... max-w-sm text-center'>...</div>`）の**直後**に追加（縦並びにするため親に `gap-4` を付与）:

親 div を次のように変更:

```tsx
          <div className='flex-1 flex flex-col items-center justify-center gap-4 px-5'>
```

モード選択カード閉じタグの直後に:

```tsx
            <MicDrainerPanel />
```

- [ ] **Step 3: 型・ビルド確認**

作業ディレクトリ `frontend/`:

Run: `pnpm run build`
Expected: 型エラーなくビルド成功。

- [ ] **Step 4: lint 確認**

Run: `pnpm run lint`
Expected: oxlint がエラーなしで完了（既存の警告水準を維持）。

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/MicDrainerPanel.tsx frontend/src/components/ModeSelection.tsx
git commit -m "feat(mic-drainer): 排水パネル UI を追加しホーム画面に配置"
```

---

### Task 10: README にクレジットと機能説明を追記

**Files:**
- Modify: `README.md`

**Interfaces:**
- Consumes: なし
- Produces: マイク排水機能の説明と PicoMicDrainer への謝辞、非公式フォークである旨。

- [ ] **Step 1: README を確認して追記**

`README.md` を Read し、機能一覧セクション（あれば）に「マイク遅延対策（Mic Delay Fix）」の項目を追加。無ければ末尾に節を追加:

```markdown
## マイク遅延対策（Mic Delay Fix）

Pico Connect 経由で PC の VRChat 等を使うと、仮想マイク `PicoStreamingMicrophone` のバッファ滞留により、時間とともにマイク遅延が累積することがあります。本アプリはこのデバイスの音声ストリームを常時開いてデータを読み捨てることで、遅延の蓄積を防ぎます。ホーム画面のトグルで ON/OFF できます（デフォルト ON、Pico Connect 起動中のみ動作）。

この機能の仕組みは [PicoMicDrainer](https://github.com/nezumi-tech/PicoMicDrainer)（nezumi-tech, MIT ライセンス）を参考に Rust で再実装したものです。

---

本リポジトリは [ryium/pico4u-vr-sleep-workaround](https://github.com/ryium/pico4u-vr-sleep-workaround)（MIT ライセンス）の非公式フォークです。
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: マイク遅延対策機能の説明とクレジットを README に追記"
```

---

### Task 11: 最終検証（Rust + フロント一括）

**Files:** なし（検証のみ）

- [ ] **Step 1: Rust 検証**

作業ディレクトリ `C:\MyProject\pico4u-vr-sleep-workaround`:

Run: `cargo test` → その後 `cargo check`
Expected: `mic_drainer` テスト 4 件 PASS、コンパイル成功。

- [ ] **Step 2: フロント検証**

作業ディレクトリ `frontend/`:

Run: `pnpm run build` → その後 `pnpm run lint`
Expected: 型エラーなしでビルド成功、lint エラーなし。

- [ ] **Step 3: 実機スモークの申し送り**

実機（Pico Connect 接続）でのスモークはユーザー担当。確認項目を報告に含める:
1. Pico Connect 起動後、ホームのマイク遅延対策パネルが「排水中」になり、話すとレベルメーターが動く。
2. Pico Connect を終了すると「待機中（デバイス未検出）」に戻る。
3. トグル OFF で「停止」になりメーターが 0 固定、再起動しても OFF が保持される。
4. VRChat で長時間使ってもマイク遅延が累積しない。

---

## Self-Review メモ

- **Spec coverage:** 設計書の各節（cpal 依存 T1、純粋関数 T2、state/config T3、ワーカー/スーパーバイザ T4、コマンド T5、main 統合 T6、i18n T7、useAppLogic T8、パネル/配置 T9、クレジット T10、検証 T11）を全てタスク化済み。
- **型整合:** イベント名（`mic-drainer-status` / `mic-drainer-peak`）、コマンド名（`set_mic_drainer_enabled`）、config キー（`mic_drainer_enabled`）、関数名（`device_name_matches` / `compute_peak_f32` / `compute_peak_i16` / `start` / `stop`）はタスク間で一貫。
- **プレースホルダ:** なし。全コードブロックは実コードを記載。
- **既知の注意:** cpal の `Stream` は `!Send` のため必ず `std::thread` 内で保持している（tokio タスクに載せない）。ワーカーはブロッキング睡眠ループで保持し、`worker_stop`/`worker_running` フラグで終了。
