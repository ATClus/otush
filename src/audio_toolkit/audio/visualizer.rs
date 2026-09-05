use rustfft::{num_complex::Complex32, Fft, FftPlanner};
use std::sync::Arc;

// `db` below is not true dBFS: it's a per-bin average divided by the FFT
// window size, which lands ~20 dB low for speech. So this window is calibrated
// against measured mic audio (dictation ~-32 dBFS, room tone ~-48 dBFS) rather
// than absolute dBFS. The old -55/-8 left speech ~1 px above the overlay's
// floor, which reads as a frozen waveform (#1694). Not lowered past -68: at
// -70 a noisy room starts making the idle waveform twitch.
const DB_MIN: f32 = -68.0;
const DB_MAX: f32 = -30.0;
const GAIN: f32 = 1.3;
const CURVE_POWER: f32 = 0.7;

pub struct AudioVisualiser {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    bucket_ranges: Vec<(usize, usize)>,
    fft_input: Vec<Complex32>,
    noise_floor: Vec<f32>,
    buffer: Vec<f32>,
    window_size: usize,
    buckets: usize,
}

impl AudioVisualiser {
    pub fn new(
        sample_rate: u32,
        window_size: usize,
        buckets: usize,
        freq_min: f32,
        freq_max: f32,
    ) -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(window_size);

        // Pre-compute Hann window
        let window: Vec<f32> = (0..window_size)
            .map(|i| {
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / window_size as f32).cos())
            })
            .collect();

        // Pre-compute bucket frequency ranges
        let nyquist = sample_rate as f32 / 2.0;
        let freq_min = freq_min.min(nyquist);
        let freq_max = freq_max.min(nyquist);

        let mut bucket_ranges = Vec::with_capacity(buckets);

        for b in 0..buckets {
            // Use logarithmic spacing for better perceptual representation
            let log_start = (b as f32 / buckets as f32).powi(2);
            let log_end = ((b + 1) as f32 / buckets as f32).powi(2);

            let start_hz = freq_min + (freq_max - freq_min) * log_start;
            let end_hz = freq_min + (freq_max - freq_min) * log_end;

            let start_bin = ((start_hz * window_size as f32) / sample_rate as f32) as usize;
            let mut end_bin = ((end_hz * window_size as f32) / sample_rate as f32) as usize;

            // Ensure each bucket has at least one bin
            if end_bin <= start_bin {
                end_bin = start_bin + 1;
            }

            // Clamp to valid range
            let start_bin = start_bin.min(window_size / 2);
            let end_bin = end_bin.min(window_size / 2);

            bucket_ranges.push((start_bin, end_bin));
        }

        Self {
            fft,
            window,
            bucket_ranges,
            fft_input: vec![Complex32::new(0.0, 0.0); window_size],
            noise_floor: vec![-40.0; buckets], // Initialize to reasonable noise floor
            buffer: Vec::with_capacity(window_size * 2),
            window_size,
            buckets,
        }
    }

    pub fn feed(&mut self, samples: &[f32]) -> Option<Vec<f32>> {
        // Add new samples to buffer
        self.buffer.extend_from_slice(samples);

        // Only process if we have enough samples
        if self.buffer.len() < self.window_size {
            return None;
        }

        // Take the required window of samples
        let window_samples = &self.buffer[..self.window_size];

        // Remove DC component
        let mean = window_samples.iter().sum::<f32>() / self.window_size as f32;

        // Apply window function and prepare FFT input
        for (i, &sample) in window_samples.iter().enumerate() {
            let windowed_sample = (sample - mean) * self.window[i];
            self.fft_input[i] = Complex32::new(windowed_sample, 0.0);
        }

        // Perform FFT
        self.fft.process(&mut self.fft_input);

        // Compute power spectrum and bucket levels
        let mut buckets = vec![0.0; self.buckets];

        for (bucket_idx, &(start_bin, end_bin)) in self.bucket_ranges.iter().enumerate() {
            if start_bin >= end_bin || end_bin > self.fft_input.len() / 2 {
                continue;
            }

            // Calculate average power in this frequency range
            let mut power_sum = 0.0;
            for bin_idx in start_bin..end_bin {
                let magnitude = self.fft_input[bin_idx].norm();
                power_sum += magnitude * magnitude;
            }

            let avg_power = power_sum / (end_bin - start_bin) as f32;

            // Convert to dB with proper scaling
            let db = if avg_power > 1e-12 {
                20.0 * (avg_power.sqrt() / self.window_size as f32).log10()
            } else {
                -80.0 // Very low floor for zero power
            };

            // Only update noise floor when signal is quiet (below current floor + 10dB)
            if db < self.noise_floor[bucket_idx] + 10.0 {
                const NOISE_ALPHA: f32 = 0.001; // Very slow adaptation
                self.noise_floor[bucket_idx] =
                    NOISE_ALPHA * db + (1.0 - NOISE_ALPHA) * self.noise_floor[bucket_idx];
            }

            // Map configurable dB range to 0-1 with gain and curve shaping
            let normalized = ((db - DB_MIN) / (DB_MAX - DB_MIN)).clamp(0.0, 1.0);
            buckets[bucket_idx] = (normalized * GAIN).powf(CURVE_POWER).clamp(0.0, 1.0);
        }

        // Apply light smoothing to reduce jitter
        for i in 1..buckets.len() - 1 {
            buckets[i] = buckets[i] * 0.7 + buckets[i - 1] * 0.15 + buckets[i + 1] * 0.15;
        }

        // Clear processed samples from buffer
        self.buffer.clear();

        Some(buckets)
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
        // Reset noise floor to initial values
        self.noise_floor.fill(-40.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn generate_sine(
        sample_rate: u32,
        freq_hz: f32,
        duration_samples: usize,
        amplitude: f32,
    ) -> Vec<f32> {
        (0..duration_samples)
            .map(|i| amplitude * (2.0 * PI * freq_hz * i as f32 / sample_rate as f32).sin())
            .collect()
    }

    #[test]
    fn test_visualizer_creation_and_partial_feed() {
        let mut vis = AudioVisualiser::new(16000, 512, 16, 80.0, 8000.0);
        let partial_samples = vec![0.0f32; 256];
        assert!(vis.feed(&partial_samples).is_none());
    }

    #[test]
    fn test_visualizer_silence_feed() {
        let mut vis = AudioVisualiser::new(16000, 512, 16, 80.0, 8000.0);
        let silence = vec![0.0f32; 512];
        let buckets = vis.feed(&silence).expect("full window produces buckets");
        assert_eq!(buckets.len(), 16);
        for &b in &buckets {
            assert!((0.0..=1.0).contains(&b));
            assert!(b < 0.1, "Silence should produce near-zero levels, got {b}");
        }
    }

    #[test]
    fn test_visualizer_sine_wave_activation() {
        let mut vis = AudioVisualiser::new(16000, 512, 16, 80.0, 8000.0);
        let tone = generate_sine(16000, 1000.0, 512, 0.8);
        let buckets = vis.feed(&tone).expect("full window produces buckets");
        assert_eq!(buckets.len(), 16);
        let max_val = buckets.iter().copied().fold(0.0f32, f32::max);
        assert!(
            max_val > 0.3,
            "Loud tone must produce elevated visualizer response, got {max_val}"
        );
    }

    #[test]
    fn test_visualizer_reset() {
        let mut vis = AudioVisualiser::new(16000, 512, 16, 80.0, 8000.0);
        vis.feed(&[0.5f32; 256]);
        assert_eq!(vis.buffer.len(), 256);
        vis.reset();
        assert_eq!(vis.buffer.len(), 0);
        for &floor in &vis.noise_floor {
            assert_eq!(floor, -40.0);
        }
    }
}
