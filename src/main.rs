// Otush — native GNOME (GTK4/libadwaita) speech-to-text application.
//
// The process entry point parses CLI arguments and hands control to the
// application library (`otush::run`). Headless one-shot paths
// (--transcribe-file / --list-devices / --list-models) never start the GTK
// main loop.

use clap::Parser;
use otush::cli::CliArgs;

fn main() {
    otush::silence_alsa_logging();
    let cli_args = CliArgs::parse();
    otush::run(cli_args);
}
