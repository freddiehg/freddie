# the lifecycle verbs, off macOS

Changes 1 through 4 landed; this is here as their record. Change 5, the graceful stop on Windows, is deferred: it needs a daemon listening on the pipe, and no freddie daemon runs on Windows yet, since mercury is macOS-only and is the only one that exists. `--force` is Windows's whole stop until then, and `stop` without it says so. The portable crates compile, test, and clippy-clean on Linux and Windows in CI; a Windows freddie daemon is what makes change 5 worth building.


`freddie_cli` finds a daemon, starts one, stops it, and follows its log. Four things in it are macOS-shaped: the log directory is `~/Library/Logs`, `logs` runs `/usr/bin/tail`, `stop` runs `/bin/kill`, and `start` detaches its child with a Unix process group.

`freddie_single_instance` is not one of them. It locks through `File::try_lock`, which is `std`, and picks its state directory per platform already, so the lock and every probe over it work on all three today.

The app's `run` is the app's, so an app bound to one platform and an app that runs everywhere both use this crate unchanged. What follows is only the verbs.

## The log directory

`Instance::named` resolves the path once and hands out a `&Path`, so this is one function, shaped like `freddie_single_instance::state_dir`.

Before:

```rust
fn log_dir(app: &str) -> Result<PathBuf, NoUserDir> {
    let home = std::env::var_os("HOME").ok_or(NoUserDir)?;
    Ok(PathBuf::from(home).join("Library/Logs").join(app))
}
```

After:

```rust
/// This user's home, which is the only environment variable this crate reads.
#[cfg(unix)]
fn home() -> Result<PathBuf, NoUserDir> {
    std::env::var_os("HOME").map(PathBuf::from).ok_or(NoUserDir)
}

#[cfg(windows)]
fn home() -> Result<PathBuf, NoUserDir> {
    std::env::var_os("USERPROFILE").map(PathBuf::from).ok_or(NoUserDir)
}

/// The per-user directory `app`'s logs go in: the platform's place for logs a person is expected
/// to read. Each sits beside where `freddie_single_instance` puts the lock, so a daemon that can
/// take its lock can write its log.
#[cfg(target_os = "macos")]
fn log_dir(app: &str) -> Result<PathBuf, NoUserDir> {
    Ok(home()?.join("Library/Logs").join(app))
}

/// `$XDG_STATE_HOME`, defaulting to `~/.local/state`. The base directory specification has no log
/// directory of its own and names state as where a log belongs.
#[cfg(all(unix, not(target_os = "macos")))]
fn log_dir(app: &str) -> Result<PathBuf, NoUserDir> {
    let base = match std::env::var_os("XDG_STATE_HOME") {
        Some(base) => PathBuf::from(base),
        None => home()?.join(".local/state"),
    };
    Ok(base.join(app))
}

/// `%LOCALAPPDATA%`, per-machine: a roaming profile must not sync one machine's log onto another.
#[cfg(target_os = "windows")]
fn log_dir(app: &str) -> Result<PathBuf, NoUserDir> {
    let base = std::env::var_os("LOCALAPPDATA").ok_or(NoUserDir)?;
    Ok(PathBuf::from(base).join(app).join("logs"))
}

#[cfg(not(any(unix, target_os = "windows")))]
compile_error!("freddie_cli has no per-user log directory for this platform");
```

## Following the log

`logs` reads the file itself on every platform, and `TAIL`, `TAIL_LINES`, and the `Command` that ran them are deleted.

Before, in `client::logs`:

```rust
    let mut tail = match Command::new(TAIL)
        .args(["-n", TAIL_LINES])
        .arg("-F")
        .arg(&path)
        .stdout(Stdio::piped())
        .spawn()
```

After:

```rust
/// How much of the existing log to show before following.
const BACKLOG_LINES: usize = 50;

/// How long to wait before looking for more, once a read has come up empty.
///
/// A poll, and the exception the "never poll" rule allows: no platform reports a regular file
/// growing through a readiness primitive. `epoll` and `kqueue` both call a regular file always
/// ready and return zero bytes, and `tail -F` polls for the same reason.
const IDLE: Duration = Duration::from_millis(200);

/// Write `path`'s last [`BACKLOG_LINES`] lines to `out`, then whatever is appended to it, until
/// the reader is interrupted or `out` closes.
///
/// The file is opened once and never reopened: `tracing_appender::rolling::never` holds one file
/// for the life of the process and never rotates it, so there is no new inode to follow onto.
fn follow(path: &Path, out: &mut impl Write) -> io::Result<()> {
    let mut file = File::open(path)?;
    let mut reader = BufReader::new(&mut file);

    // The backlog, kept in a ring so the whole file is never held: a log is appended to for the
    // life of a daemon and there is no bound on how long that is.
    let mut backlog = VecDeque::with_capacity(BACKLOG_LINES);
    let mut line = String::new();
    while reader.read_line(&mut line)? != 0 {
        if backlog.len() == BACKLOG_LINES {
            backlog.pop_front();
        }
        backlog.push_back(std::mem::take(&mut line));
    }
    for line in &backlog {
        show(out, line)?;
    }

    // Then whatever arrives. A read that returns nothing is the writer not having written yet,
    // which is the common case: waiting on it is the whole of what this verb does.
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            std::thread::sleep(IDLE);
            continue;
        }
        show(out, &line)?;
    }
}
```

`show` is what `client.rs` already has: it reads the record's level out of the line, colours it when the terminal is one, and writes it. It is unchanged, and it is the reason the backlog is kept as lines rather than bytes.

A partial line is a line that has not arrived. `read_line` returns it without its newline only at end of file, and the next read resumes mid-line, so a record written in two writes reaches `show` in two pieces. Read to a newline before showing:

```rust
        if !line.ends_with('\n') {
            // The writer is mid-record. Leave what was read in the buffer and wait for the rest.
            partial.push_str(&line);
            std::thread::sleep(IDLE);
            continue;
        }
```

## Stopping a daemon

Unix keeps `/bin/kill`, which Linux has where macOS has it. Windows has no signal to send, so the two halves split:

```rust
/// Destroy `pid`. `--force`, and the only stop Windows has until the graceful one below lands.
#[cfg(windows)]
fn kill(pid: Pid) -> io::Result<()> {
    run(Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]))
}

#[cfg(unix)]
fn kill(pid: Pid) -> io::Result<()> {
    run(Command::new("/bin/kill").args(["-KILL", &pid.to_string()]))
}
```

A subprocess on both, for the reason `signal_pid` already gives: the workspace forbids `unsafe`, and every binding for `kill(2)` and for `TerminateProcess` is an unsafe extern call.

The graceful stop is a signal on Unix and a named pipe on Windows.

The daemon end belongs to `freddie_daemon`, beside the SIGTERM handler it already installs, and pushes on the same stop channel:

```rust
/// Push an ask to leave when something opens this daemon's pipe and writes to it.
///
/// The Windows counterpart of SIGTERM, over `tokio::net::windows::named_pipe`: tokio is already
/// this crate's runtime, and it keeps the `unsafe` a binding would need out of here.
#[cfg(windows)]
fn forward_pipe(instance: &Instance, stop_tx: &UnboundedSender<()>) {
    let name = format!(r"\\.\pipe\{}", instance.slug());
    // .. serve `name`, and send `()` for each client that connects ..
}
```

The client end is `std::fs`, because Windows opens a named pipe as a file, so `freddie_cli` gains no dependency:

```rust
#[cfg(windows)]
fn ask_to_stop(instance: &Instance) -> io::Result<()> {
    let name = format!(r"\\.\pipe\{}", instance.slug());
    std::fs::OpenOptions::new().write(true).open(name)?.write_all(b"stop")
}

#[cfg(unix)]
fn ask_to_stop(pid: Pid) -> io::Result<()> {
    run(Command::new("/bin/kill").args(["-TERM", &pid.to_string()]))
}
```

Both ends key off `instance.slug()`, which is the one string that already names one daemon.

`stop` then waits for the lock to go free exactly as it does now, on both platforms, and that wait is what tells it the daemon left. Nothing about `stop`'s reporting, its timeout, or its exit codes changes.

## Detaching the spawned daemon

Before, in `spawn_daemon`:

```rust
    use std::os::unix::process::CommandExt;

    let exe = std::env::current_exe()?;
    let child = Command::new(exe)
        .arg(DAEMON_VERB)
        .args(typed.argv::<TApp>())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
```

After:

```rust
    let exe = std::env::current_exe()?;
    let mut command = Command::new(exe);
    command
        .arg(DAEMON_VERB)
        .args(typed.argv::<TApp>())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = detach(&mut command).spawn()?;
```

```rust
/// Put the child where a terminal cannot reach it, so it outlives the one that spawned it.
#[cfg(unix)]
fn detach(command: &mut Command) -> &mut Command {
    use std::os::unix::process::CommandExt;
    // Its own process group, so a ctrl-c in the spawning terminal does not reach it.
    command.process_group(0)
}

#[cfg(windows)]
fn detach(command: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    /// No console at all, which is what the three null stdio streams already say.
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    /// No console control event reaches it.
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
}
```

## CI

`.github/workflows/ci.yml` runs on `macos-latest`. It gains one job:

```yaml
  portable:
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, windows-latest]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@master
        with:
          toolchain: 1.96.0
      # The two crates with no macOS-only dependency. `-p` rather than the workspace, because
      # everything else in it binds to AppKit and will not build here.
      - run: cargo test -p freddie_cli -p freddie_single_instance
```

## The changes, in order

The first three change nothing on macOS, and each is shippable alone.

1. **`log_dir`, three ways**, with `home()` beside it. Landed.
2. **`logs` follows the file itself.** `follow`, and the deletion of `TAIL`, `TAIL_LINES`, and the subprocess. `refactors/past/one-log-many-writers.md` loses its `tail` exception: clap and the tests are then the only things that reach a terminal without going through tracing. Landed. The graceful path split into `forward_pipe`/`ask_to_stop` in the doc above did not land; instead the existing `signal_pid` gained a Windows arm, SIGKILL via `taskkill /F` and SIGTERM returning "use --force" until change 5.
3. **`detach`, and `kill` for `--force`**, both behind `cfg`. Landed.
4. **The CI job.** After 1 through 3 it passes, and it is what keeps them true. Landed, with a clippy step beside the test.
5. **The graceful stop on Windows.** Deferred until a freddie daemon runs on Windows. It is a pipe the daemon listens on and `stop` writes to, replacing the Windows `signal_pid`'s SIGTERM arm. mercury cannot run on Windows, so nothing exercises it yet.
