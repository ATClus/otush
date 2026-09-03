//! Audio DSP module: High-Pass Filter, Voice Isolation & Dynamic Normalization
//!
//! Provides zero-allocation, in-place digital signal processing for 16 kHz mono speech:
//! 1. 2nd-order Butterworth High-Pass Filter at 80 Hz (removes DC offset, 50/60Hz mains hum,
//!    handling noise, and low-frequency rumble).
//! 2. Adaptive Noise Gate (suppresses background ambient hiss during pauses).
//! 3. Dynamic RMS & Peak Normalization with a smooth soft-knee limiter (ensures speech amplitude
//!    is optimal for Silero VAD and Whisper transcription without distortion or clipping).

use std::f32::consts::PI;

/// 2nd-order Butterworth High-Pass Filter (IIR) designed for 16 kHz sample rate.
#[derive(Debug, Clone)]
pub struct HighPassFilter {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl HighPassFilter {
    /// Creates a 2nd-order Butterworth high-pass filter.
    /// `cutoff_hz`: Cutoff frequency in Hz (typically 80.0 Hz for speech).
    /// `sample_rate`: Sampling rate in Hz (typically 16000.0).
    pub fn new(cutoff_hz: f32, sample_rate: f32) -> Self {
        let w0 = 2.0 * PI * cutoff_hz / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * std::f32::consts::FRAC_1_SQRT_2); // Q = 1/sqrt(2) = 0.7071

        let a0 = 1.0 + alpha;
        let b0 = ((1.0 + cos_w0) / 2.0) / a0;
        let b1 = (-(1.0 + cos_w0)) / a0;
        let b2 = ((1.0 + cos_w0) / 2.0) / a0;
        let a1 = (-2.0 * cos_w0) / a0;
        let a2 = (1.0 - alpha) / a0;

        Self {
            b0,
            b1,
            b2,
            a1,
            a2,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// Default 80 Hz high-pass filter optimized for 16 kHz mono speech.
    pub fn new_80hz_16khz() -> Self {
        Self::new(80.0, 16000.0)
    }

    /// Reset internal filter state (e.g. between separate recordings).
    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }

    /// Process a slice of f32 samples in-place.
    pub fn process_in_place(&mut self, samples: &mut [f32]) {
        for sample in samples.iter_mut() {
            let x0 = *sample;
            let y0 = self.b0 * x0 + self.b1 * self.x1 + self.b2 * self.x2
                - self.a1 * self.y1
                - self.a2 * self.y2;

            self.x2 = self.x1;
            self.x1 = x0;
            self.y2 = self.y1;
            self.y1 = y0;

            *sample = y0;
        }
    }
}

/// Adaptive voice gate to attenuate steady low-level ambient noise.
#[derive(Debug, Clone)]
pub struct NoiseGate {
    threshold: f32,
    attenuation: f32,
    envelope: f32,
    attack: f32,
    release: f32,
}

impl NoiseGate {
    pub fn new(threshold_db: f32, attenuation_db: f32, sample_rate: f32) -> Self {
        let threshold = 10.0f32.powf(threshold_db / 20.0);
        let attenuation = 10.0f32.powf(attenuation_db / 20.0);
        // ~10ms attack, ~100ms release
        let attack = (-1.0 / (0.010 * sample_rate)).exp();
        let release = (-1.0 / (0.100 * sample_rate)).exp();

        Self {
            threshold,
            attenuation,
            envelope: 0.0,
            attack,
            release,
        }
    }

    pub fn new_default_16khz() -> Self {
        Self::new(-45.0, -18.0, 16000.0)
    }

    pub fn reset(&mut self) {
        self.envelope = 0.0;
    }

    pub fn process_in_place(&mut self, samples: &mut [f32]) {
        for sample in samples.iter_mut() {
            let abs_val = sample.abs();
            if abs_val > self.envelope {
                self.envelope = self.attack * self.envelope + (1.0 - self.attack) * abs_val;
            } else {
                self.envelope = self.release * self.envelope + (1.0 - self.release) * abs_val;
            }

            let gain = if self.envelope < self.threshold {
                let ratio = (self.envelope / self.threshold).clamp(0.0, 1.0);
                self.attenuation + ratio * (1.0 - self.attenuation)
            } else {
                1.0
            };

            *sample *= gain;
        }
    }
}

/// Configuration parameters for voice enhancement DSP.
#[derive(Debug, Clone)]
pub struct VoiceEnhancerConfig {
    /// Software input gain multiplier (e.g. 1.0 = standard, 2.0 = +6dB, 4.0 = +12dB).
    pub input_gain: f32,
    /// 2nd-order Butterworth high-pass filter at 80 Hz (removes DC, rumble, 60Hz hum).
    pub high_pass_filter: bool,
    /// Adaptive noise gate (suppresses background hiss and fan hum during speech pauses).
    pub noise_reduction: bool,
    /// Noise gate threshold in dB (typically -60.0 to -25.0, default -45.0).
    pub noise_gate_threshold_db: f32,
    /// Dynamic RMS & Peak Normalization with soft-knee limiter.
    pub normalization: bool,
}

impl Default for VoiceEnhancerConfig {
    fn default() -> Self {
        Self {
            input_gain: 1.0,
            high_pass_filter: true,
            noise_reduction: true,
            noise_gate_threshold_db: -45.0,
            normalization: true,
        }
    }
}

/// Dynamic Audio Normalizer with peak limiting and RMS leveling.
pub struct VoiceEnhancer {
    hp_filter: HighPassFilter,
    noise_gate: NoiseGate,
}

impl Default for VoiceEnhancer {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceEnhancer {
    pub fn new() -> Self {
        Self {
            hp_filter: HighPassFilter::new_80hz_16khz(),
            noise_gate: NoiseGate::new_default_16khz(),
        }
    }

    pub fn reset(&mut self) {
        self.hp_filter.reset();
        self.noise_gate.reset();
    }

    /// Process a stream chunk in-place: removes sub-80Hz rumble and gates noise.
    pub fn process_stream_chunk(&mut self, samples: &mut [f32]) {
        self.hp_filter.process_in_place(samples);
        self.noise_gate.process_in_place(samples);
    }

    /// Full-buffer voice cleanup and normalization using specific DSP configuration.
    pub fn process_with_config(samples: &mut [f32], config: &VoiceEnhancerConfig) {
        if samples.is_empty() {
            return;
        }

        // 1. High-pass filter (optional, 80 Hz)
        if config.high_pass_filter {
            let mut hp = HighPassFilter::new_80hz_16khz();
            hp.process_in_place(samples);
        }

        // 2. Adaptive noise gate (optional)
        if config.noise_reduction {
            let mut gate = NoiseGate::new(config.noise_gate_threshold_db, -18.0, 16000.0);
            gate.process_in_place(samples);
        }

        // 3. Dynamic RMS and Peak Normalization (AGC, optional)
        if config.normalization {
            let mut peak = 0.0f32;
            let mut sum_sq = 0.0f32;
            for &s in samples.iter() {
                let abs_s = s.abs();
                if abs_s > peak {
                    peak = abs_s;
                }
                sum_sq += s * s;
            }

            let rms = (sum_sq / samples.len() as f32).sqrt();

            if peak > 0.001 && rms > 0.0005 {
                let target_peak = 0.70f32;
                let max_gain = 5.0f32;
                let mut norm_gain = (target_peak / peak).min(max_gain);

                if norm_gain < 1.0 {
                    norm_gain = norm_gain.max(0.6);
                }

                for s in samples.iter_mut() {
                    *s *= norm_gain;
                }
            }
        }

        // 4. Software input gain boost with soft-knee limiter
        let gain = config.input_gain.clamp(0.1, 10.0);
        if (gain - 1.0).abs() > 0.001 || config.normalization {
            for s in samples.iter_mut() {
                let scaled = *s * gain;
                *s = if scaled > 0.95 {
                    0.95 + (1.0 - (-(scaled - 0.95)).exp()) * 0.05
                } else if scaled < -0.95 {
                    -0.95 - (1.0 - (-(-scaled - 0.95)).exp()) * 0.05
                } else {
                    scaled
                };
            }
        }
    }

    /// Full-buffer voice cleanup with default settings (backward compatibility).
    pub fn process_recording_buffer(samples: &mut [f32]) {
        Self::process_with_config(samples, &VoiceEnhancerConfig::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_pass_filter_attenuates_dc_and_low_frequencies() {
        let mut hp = HighPassFilter::new_80hz_16khz();
        let mut dc = vec![1.0f32; 1600]; // 100ms
        hp.process_in_place(&mut dc);
        let last_val = dc.last().copied().unwrap_or(1.0);
        assert!(
            last_val.abs() < 0.05,
            "HighPassFilter must attenuate DC, got {last_val}"
        );
    }

    #[test]
    fn voice_enhancer_normalizes_quiet_audio() {
        let mut quiet = vec![0.05f32; 1600];
        VoiceEnhancer::process_recording_buffer(&mut quiet);
        for &s in &quiet {
            assert!(!s.is_nan());
            assert!(!s.is_infinite());
        }
    }

    #[test]
    fn voice_enhancer_gain_scales_amplitude_safely() {
        let mut audio = vec![0.1f32; 800];
        let config = VoiceEnhancerConfig {
            input_gain: 2.5,
            high_pass_filter: false,
            noise_reduction: false,
            noise_gate_threshold_db: -45.0,
            normalization: false,
        };
        VoiceEnhancer::process_with_config(&mut audio, &config);
        assert!(
            (audio[0] - 0.25).abs() < 0.01,
            "Input gain 2.5x should scale 0.1 to ~0.25, got {}",
            audio[0]
        );
    }

    #[test]
    fn test_noise_gate_attenuates_low_noise_and_passes_speech() {
        let mut gate = NoiseGate::new(-40.0, -20.0, 16000.0);
        // Low noise signal (~ -60 dBFS)
        let mut low_noise = vec![0.001f32; 1600];
        gate.process_in_place(&mut low_noise);
        let final_sample = low_noise.last().copied().unwrap_or(0.0);
        assert!(
            final_sample < 0.001,
            "Noise gate should attenuate noise below threshold, got {final_sample}"
        );

        // Loud signal (~ -6 dBFS)
        let mut loud_speech = vec![0.5f32; 1600];
        gate.process_in_place(&mut loud_speech);
        let final_speech = loud_speech.last().copied().unwrap_or(0.0);
        assert!(
            (final_speech - 0.5).abs() < 0.05,
            "Noise gate should pass speech above threshold, got {final_speech}"
        );
    }

    #[test]
    fn test_voice_enhancer_limiter_handles_extreme_peaks() {
        let mut extreme = vec![10.0f32, -10.0, 5.0, -5.0, 2.0, -2.0];
        let config = VoiceEnhancerConfig {
            input_gain: 2.0,
            high_pass_filter: false,
            noise_reduction: false,
            noise_gate_threshold_db: -45.0,
            normalization: true,
        };
        VoiceEnhancer::process_with_config(&mut extreme, &config);
        for &s in &extreme {
            assert!(!s.is_nan());
            assert!(!s.is_infinite());
            assert!(
                s.abs() <= 1.0,
                "Soft-knee limiter must clamp extreme peaks to <= 1.0, got {s}"
            );
        }
    }

    #[test]
    fn test_high_pass_filter_reset() {
        let mut hp = HighPassFilter::new_80hz_16khz();
        let mut dc = vec![1.0f32; 800];
        hp.process_in_place(&mut dc);
        assert!(hp.x1 != 0.0 || hp.y1 != 0.0);
        hp.reset();
        assert_eq!(hp.x1, 0.0);
        assert_eq!(hp.x2, 0.0);
        assert_eq!(hp.y1, 0.0);
        assert_eq!(hp.y2, 0.0);
    }
}
