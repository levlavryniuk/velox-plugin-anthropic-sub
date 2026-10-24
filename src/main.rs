//! The plugin as a subprocess: `velox` starts this for the `[[plugin]]` entry that installed it.

use std::process::ExitCode;

fn main() -> ExitCode {
    velox_plugin_anthropic_sub::run()
}
