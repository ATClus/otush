fn main() {
    #[cfg(target_os = "linux")]
    {
        // Embed RPATH/RUNPATH into the ELF binary so it can locate libtranscribe.so.0.2
        // and libggml*.so when installed under /usr/bin, /app/bin (Flatpak),
        // or running locally.
        println!(
            "cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/../share/otush/resources/transcribe-libs:$ORIGIN/../lib/otush:$ORIGIN/resources/transcribe-libs:$ORIGIN/../resources/transcribe-libs:$ORIGIN"
        );
    }
}
