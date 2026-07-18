# mercury takes arguments

Not built. mercury parses no arguments today and reads `LOG_LEVEL` straight out of the environment inside `logging::init`. `external-events.md` wants a `--port`, and every configurable thing after it will want the same treatment, so the parser lands first, as a prefactor, with the one setting that already exists.

clap with the `derive` and `env` features. `env` is what makes this worth doing rather than reading `std::env::args` by hand: one `#[arg]` attribute declares the flag, the environment variable, and the default, and clap resolves them in that order. Precedence, `--help`, `--version`, a misspelled flag, and a value that does not parse are all handled by the same declaration.

## The struct

```rust
// crates/mercury/src/cli.rs

use clap::Parser;

/// Everything mercury can be told at startup.
///
/// Each field is a flag, an environment variable, and a default, in that order of precedence,
/// which clap resolves. A flag it does not recognize, or a value that does not parse, exits with a
/// message naming the offender.
#[derive(Parser, Debug)]
#[command(name = "mercury", version, about = "A layered keyboard remapper.")]
pub struct Args {
    /// What the terminal shows. The log file always records `debug`, whatever this says.
    #[arg(long, env = "LOG_LEVEL", default_value = "info")]
    pub log_level: String,
}
```

`log_level` stays a `String` because it is a `tracing_subscriber` `EnvFilter` directive, not a bare level: `info`, and also `mercury=debug,bind=warn`. Parsing it into a `LevelFilter` would refuse the second form, which is the form that is actually useful when chasing one crate's output.

## Change 1: parse arguments, and hand the log level to logging

`crates/mercury/Cargo.toml` gains:

```toml
clap = { version = "4", features = ["derive", "env"] }
```

`crates/mercury/src/main.rs`, before:

```rust
use std::ops::ControlFlow;

use freddie::{AlwaysEqual, TimerEffect};

mod logging;

fn main() {
    let log_path = logging::init();
    println!("mercury: logging to {}", log_path.display());
```

after:

```rust
use std::ops::ControlFlow;

use clap::Parser;
use freddie::{AlwaysEqual, TimerEffect};

mod cli;
mod logging;

fn main() {
    // First, so `--help` prints and a bad flag exits before the lock, the keyboard, or the icon.
    // clap exits the process itself rather than returning an error to handle here.
    let args = cli::Args::parse();

    let log_path = logging::init(&args.log_level);
    println!("mercury: logging to {}", log_path.display());
```

`crates/mercury/src/logging.rs`, before:

```rust
//! Two sinks with independent filters. The file always records [`FILE_LEVEL`], so
//! the record of a run survives however quiet the terminal was asked to be. The
//! terminal shows whatever `LOG_LEVEL` asks for, defaulting to `info`.

/// The environment variable that sets the terminal's log level.
const LOG_LEVEL_ENV: &str = "LOG_LEVEL";

/// What the log file records, always. Deliberately not tied to [`LOG_LEVEL_ENV`]:
/// the file is the record of what happened, so quieting the terminal must never
/// quiet it.
const FILE_LEVEL: LevelFilter = LevelFilter::DEBUG;

/// Send tracing to the log file and the terminal, and return the file's path.
pub fn init() -> PathBuf {
```

after:

```rust
//! Two sinks with independent filters. The file always records [`FILE_LEVEL`], so
//! the record of a run survives however quiet the terminal was asked to be. The
//! terminal shows whatever `--log-level` asks for, defaulting to `info`.

/// What the log file records, always. Deliberately not tied to the terminal's filter: the file is
/// the record of what happened, so quieting the terminal must never quiet it.
const FILE_LEVEL: LevelFilter = LevelFilter::DEBUG;

/// Send tracing to the log file and the terminal, and return the file's path.
///
/// `directives` is a `tracing_subscriber` filter string, so `info` and `mercury=debug,bind=warn`
/// are both accepted. One that does not parse falls back to `info` and says so on the terminal,
/// since the alternative is a run with no logging at all.
pub fn init(directives: &str) -> PathBuf {
```

and, in the same file, before:

```rust
    let terminal = fmt::layer().with_writer(std::io::stderr).with_filter(
        EnvFilter::try_from_env(LOG_LEVEL_ENV).unwrap_or_else(|_| EnvFilter::new("info")),
    );
```

after:

```rust
    let terminal = fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(EnvFilter::try_new(directives).unwrap_or_else(|e| {
            eprintln!("mercury: {directives:?} is not a log filter ({e}); using info");
            EnvFilter::new("info")
        }));
```

`EnvFilter::try_from_env` goes away with the variable it read: clap has already resolved `LOG_LEVEL` into `args.log_level` by the time `init` is called, so reading the environment a second time here would be a second source of truth for the same setting.

Verify by hand:

- `cargo run -p mercury -- --help` lists `--log-level` and names `LOG_LEVEL` as its variable.
- `cargo run -p mercury -- --log-level error` gives a quiet terminal, and `~/Library/Logs/mercury/mercury.log` still records `debug`.
- `LOG_LEVEL=error cargo run -p mercury` does the same, and `--log-level error` beats `LOG_LEVEL=debug`.
- `cargo run -p mercury -- --log-level 'mercury=debug,bind=warn'` filters per crate.
- `cargo run -p mercury -- --log-level nonsense` warns and runs at `info`.
- `cargo run -p mercury -- --prot 9000` exits with clap's unknown-argument message and no menu-bar icon appears.

`CLAUDE.md`'s logging section describes `LOG_LEVEL`; it gains `--log-level` alongside it.

## Change 2: `--port`

Ships with `external-events.md`, which is what gives mercury a port to configure. `Args` gains one field:

```rust
 pub struct Args {
     /// What the terminal shows. The log file always records `debug`, whatever this says.
     #[arg(long, env = "LOG_LEVEL", default_value = "info")]
     pub log_level: String,
+
+    /// The loopback port the event socket listens on.
+    #[arg(long, env = "MERCURY_PORT", default_value_t = mercury::DEFAULT_PORT)]
+    pub port: u16,
 }
```

`u16` is the whole of the validation. Confirmed against clap 4: `--port abc` exits with `invalid value 'abc' for '--port <PORT>': invalid digit found in string`, and `--port 99999` with `99999 is not in 0..=65535`, both before `main` runs a line of its own.

A bad `MERCURY_PORT` produces that same message, naming `--port` rather than the variable that actually carried the value. It is clap's wording and not worth working around, but it is what you will see when the typo is in a shell profile.

The precedence and the parsing were checked by building this `Args` and running it: flags beat environment variables, environment variables beat the defaults, `--log-level 'mercury=debug,bind=warn'` survives intact as a filter string, and `--help` lists both variables and both defaults.

That field is the entire mercury-side story for the port. `external-events.md` keeps `DEFAULT_PORT` and the reasoning behind the number, and drops any parsing of its own.
