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
