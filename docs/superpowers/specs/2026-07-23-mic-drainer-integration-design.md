# pico4u × マイク排水機能 統合 設計書

- 日付: 2026-07-23
- 対象リポジトリ: `nitda-lab/pico4u-vr-sleep-workaround`（`ryium/pico4u-vr-sleep-workaround` の公式フォーク）
- 目的: Pico 4 / 4 Ultra を Pico Connect 経由で PC の VRChat に繋ぐと、時間とともにマイク遅延が増大する問題（バッファ蓄積）を解消する「マイク排水」機能を、既存の V睡（スリープ防止）ツールに統合する。

## 背景と問題

Pico Connect が PC 上に作る仮想録音デバイス `PicoStreamingMicrophone` は、PC 側で誰もストリームを読み出さないとバッファにデータが滞留し、遅延が累積していく（クロックドリフト由来）。

先行ツール **PicoMicDrainer**（nezumi-tech, MIT）は「このデバイスのストリームを常時開いてデータを読み捨てる」ことでバッファ滞留を防いでいる。本統合ではこの**仕組み（アイデア）を Rust で再実装**し、pico4u の Tauri アプリに機能として組み込む。コードのコピーはしない。

## 全体像

既存アプリ構成（Tauri 2 + Rust バックエンド / React + TypeScript + Tailwind + @charcoal-ui フロント）を踏襲し、既存の V睡機能には手を触れず、独立した「マイク排水」機能を追加する。

- マイク排水は **ADB 接続や V睡機能とは完全に独立**（PC 側オーディオのみ・ヘッドセット接続不要）。
- アプリ起動時に設定に従って**自動開始**（デフォルト ON）。
- Pico Connect の起動/終了に**自動追従**（デバイスを定期ポーリングして検出したら排水開始、消えたら待機に戻る）。

## バックエンド（Rust）

### 依存追加
- `cpal`（クロスプラットフォーム音声 I/O、Windows では WASAPI）を `Cargo.toml` に追加。

### 新モジュール `src/mic_drainer.rs`

責務: 「PicoStreaming を名前に含む録音デバイスを探し、入力ストリームを開いてバッファを読み捨てる。ピーク値をフロントに通知する。デバイスの出入りに自動追従する」。

純粋関数（`cargo test` 対象）:
- `device_name_matches(name: &str) -> bool` — 名前に `"picostreaming"` を含むか（大文字小文字無視）を判定。
- `compute_peak_f32(samples: &[f32]) -> f32` — 絶対値の最大（0.0–1.0）。
- `compute_peak_i16(samples: &[i16]) -> f32` — i16 を正規化してピーク算出。

並行モデル（cpal の `Stream` は `!Send` のため tokio タスクに載せられない点に対処）:
- **ワーカースレッド**（`std::thread`）: デバイスを検出し、`default_input_config()` でストリームを構築、`play()`。データコールバックはサンプル形式（f32/i16/u16）に応じてピークを計算し、共有アトミック（`Arc<AtomicU32>`、f32 の bits）に格納。バッファは読んだ時点で破棄。停止フラグが立つ、またはストリームエラー時にスレッド終了。「稼働中」を示すアトミックを持つ。
- **スーパーバイザ**（tokio タスク、`start_mic_drainer` で spawn）: 100ms 間隔ループ。
  - 毎 tick: 共有ピークを読み `mic-drainer-peak`（f32）を emit。
  - 5秒ごと: 有効かつワーカー非稼働ならデバイス検出→ワーカースレッド起動を試行。状態変化時に `mic-drainer-status`（`"draining"` / `"waiting"` / `"off"`）を emit。
  - `mic_drainer_enabled` が false になったらワーカー停止フラグを立て、`"off"` を emit してループ終了。

### `src/state.rs` に追加
- `mic_drainer_task: Mutex<Option<JoinHandle<()>>>`（スーパーバイザのハンドル）
- `mic_drainer_enabled: Arc<AtomicBool>`

### `src/config.rs` に追加
- `AppConfig` に `mic_drainer_enabled: bool`（デフォルト `true`）。`#[serde(default)]` 済みなので旧設定ファイルとも後方互換。

### `src/commands.rs` に追加
- `set_mic_drainer_enabled(app, state, enabled: bool)`:
  - `state.mic_drainer_enabled` を更新し、config に永続化。
  - true 化: スーパーバイザ未稼働なら起動。false 化: 停止フラグ→ハンドル abort。
- `get_config` は既存のまま新フィールドを返す（フロントは config から初期値を読む）。

### `src/main.rs`
- `mod mic_drainer;` を追加。
- `set_mic_drainer_enabled` を `invoke_handler` に登録。
- `.setup()` で config を読み、`mic_drainer_enabled` が true なら排水スーパーバイザを起動。
- `RunEvent::Exit` でスーパーバイザ/ワーカーを停止（既存の adb kill 処理に追記）。

## フロントエンド（React / TypeScript）

### `hooks/useAppLogic.ts` に追加
- state: `micDrainerEnabled: boolean`、`micDrainerStatus: 'draining' | 'waiting' | 'off'`、`micDrainerPeak: number`。
- 初期化（既存の config ロード内）: `config.mic_drainer_enabled` を `micDrainerEnabled` に反映。
- イベントリスナ（既存の debug-log リスナと同じ形）: `mic-drainer-status`、`mic-drainer-peak` を `listen` して state 更新。ピークはそのまま格納（描画は CSS 遷移で滑らかに）。
- `toggleMicDrainer(enabled: boolean)`: `invoke('set_mic_drainer_enabled', { enabled })` → state 更新。
- 返り値に上記を追加（`AppContext` は `ReturnType<typeof useAppLogic>` なので自動反映）。

### 新コンポーネント `components/MicDrainerPanel.tsx`
既存 `ConnectionPanel` と同じカード様式（`bg-white dark:bg-gray-800 rounded-lg p-5 border`）。

- ヘッダー: ステータスドット（draining=`bg-brand` + ping アニメ / waiting=amber / off=gray）＋見出しテキスト（排水中 / 待機中（デバイス未検出）/ 停止）。
- トグル: `@charcoal-ui/react` の `Checkbox`（既存 Settings のデバッグトグルと同じ）で ON/OFF。
- レベルメーター: 横バー。`micDrainerPeak`（0–1）を幅 % にマップ、`bg-brand`、`transition-[width] duration-100`。off 時はグレーで固定。
- 補足文: 「Pico Connect 起動中のみ動作します」等の一文。

### `components/ModeSelection.tsx`（ホーム）に配置
- `homeView === 'main'` のとき、モード選択カードの**下**に `<MicDrainerPanel />` を追加。マイク排水は ADB 接続前から使えるべきなので、常時見えるホームに置く。

### `i18n.ts`
- 日英の文言を追加（`mic_drainer_title`, `mic_drainer_status_draining`, `mic_drainer_status_waiting`, `mic_drainer_status_off`, `mic_drainer_toggle`, `mic_drainer_note`）。

## エラー処理

- デバイス未検出: エラー扱いせず `"waiting"` 表示のまま 5秒ごとに再試行（Pico Connect 起動待ち）。
- ストリーム構築失敗 / デバイス切断: ワーカースレッドが終了 → 次のポーリングで `"waiting"` に戻り自動復帰。デバッグモード時は Logs にも記録。
- 設定保存失敗: 既存同様 `console.error` に留め、機能自体は継続。

## ライセンス・クレジット

- `LICENSE` / `LICENSES/`（ADB の Apache 2.0）はそのまま維持。
- `README.md` に追記: 「マイク排水機能は PicoMicDrainer（nezumi-tech, MIT）の仕組みを参考に Rust で再実装した」。
- 本家 pico4u（ryium, MIT）との関係を明記し、非公式フォークである旨を記載。

## 検証

1. `cargo test`（`mic_drainer.rs` の純粋関数: デバイス名マッチ・ピーク計算）。
2. `cargo check`（Rust 全体のコンパイル）。
3. フロント: `pnpm run build` 相当の typecheck（`tsc`）＋ `oxlint`（リポジトリ既存構成）。
4. **実機スモーク（ユーザー担当）**: Pico Connect 接続下でパネルが「排水中」になりレベルメーターが反応、VRChat で遅延が累積しないことを確認。実機がこちら側に無いため最終確認はユーザーが行う。

## スコープ外（YAGNI）

- 排水デバイスの手動選択 UI（自動検出のみ）。
- 遅延量の数値計測・可視化（原理上、排水していれば累積しないため不要）。
- 本家への PR（まずは自分用フォークとして完成させる）。
