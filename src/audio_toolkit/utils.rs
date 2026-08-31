/// Returns the CPAL ALSA host for Linux audio capture and playback.
pub fn get_cpal_host() -> cpal::Host {
    cpal::host_from_id(cpal::HostId::Alsa).unwrap_or_else(|_| cpal::default_host())
}
