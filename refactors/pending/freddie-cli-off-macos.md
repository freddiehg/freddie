# the lifecycle verbs, off macOS

`freddie-cli.md` builds a crate that finds a daemon, starts one, signals it, and tails its log. Nothing in what it does is particular to a platform, and much of how it does it is: it shells out to `/bin/kill`, it shells out to `/usr/bin/tail`, it writes under `~/Library/Logs`, and it detaches a child with a Unix process group.

This doc is what each of those becomes on Linux and on Windows. It is worth doing because the crate's second consumer is a compiler daemon that watches project directories, and those run wherever the projects are. The keyboard remapper stays macOS-only whatever happens here, because a `CGEventTap` has no counterpart to port to.

The split is already drawn where it needs to be: an app's `run` is the app's, so a macOS-only app compiled on macOS and a portable app compiled everywhere both use the same crate. What follows is only about the verbs.

## What is already portable

`freddie_single_instance` is done, and it is the hard part. It locks through `File::try_lock` and `File::try_lock_shared`, which are `std`, and it already picks a per-user state directory three ways:

```rust
#[cfg(target_os = "macos")]
fn state_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
}

#[cfg(target_os = "windows")]
fn state_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn state_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
}

#[cfg(not(any(unix, target_os = "windows")))]
compile_error!("freddie_single_instance has no per-user state directory for this platform");
```

So `status`, and the probe half of `start`, `restart`, and `stop`, work on all three today. What is left is four things.

## The log directory

`Instance::named` resolves the log path once, when the instance is built, and fails with `NoLogDir` when the environment names nowhere to put it. So this is one function with three arms, written the way `state_dir` already is, and every caller downstream just reads a `&Path`.

```rust
/// The per-user directory this app's logs go in, or [`NoLogDir`] when there is none.
///
/// Each is the platform's place for logs a person is expected to read, and each matches where
/// `freddie_single_instance` puts the lock, so a daemon that can take its lock can write its log.
#[cfg(target_os = "macos")]
fn log_dir(app: &str) -> Result<PathBuf, NoLogDir> {
    Ok(home()?.join("Library/Logs").join(app))
}

/// `$XDG_STATE_HOME`, defaulting to `~/.local/state`. The base directory specification has no log
/// directory of its own and names state as where a log belongs.
#[cfg(all(unix, not(target_os = "macos")))]
fn log_dir(app: &str) -> Result<PathBuf, NoLogDir> {
    let base = match std::env::var_os("XDG_STATE_HOME") {
        Some(base) => PathBuf::from(base),
        None => home()?.join(".local/state"),
    };
    Ok(base.join(app))
}

/// `%LOCALAPPDATA%`, per-machine like the lock: a roaming profile must not sync one machine's log
/// onto another.
#[cfg(target_os = "windows")]
fn log_dir(app: &str) -> Result<PathBuf, NoLogDir> {
    let base = std::env::var_os("LOCALAPPDATA").ok_or(NoLogDir)?;
    Ok(PathBuf::from(base).join(app).join("logs"))
}

#[cfg(not(any(unix, target_os = "windows")))]
compile_error!("freddie_cli has no per-user log directory for this platform");
```

`home()` reads `HOME` on Unix and `USERPROFILE` on Windows, and is the only place either is read once this lands. The `compile_error!` arm is copied from `freddie_single_instance` deliberately: a platform this crate has no answer for should not build.

## Following the log

`logs` runs `/usr/bin/tail -n 50 -F`. Windows has no `tail`, and `Get-Content -Wait` through PowerShell is a second shell-out with a second set of quoting rules.

The verb reads the file itself instead, on every platform. It opens the log, seeks back far enough for the last fifty lines, writes what it finds, then blocks reading and writes what arrives. A file that is replaced rather than appended to is not a case that arises: `tracing_appender::rolling::never` holds one file open and never rotates it.

This deletes a subprocess rather than porting one, and it deletes one of the three exceptions to `refactors/past/one-log-many-writers.md`: with no `tail` writing to the terminal, the only things that bypass tracing are clap and the tests.

Reading a file that another process is appending to needs no locking on any of the three: a reader sees bytes once they are written, and a partially written line is a line that has not arrived yet.

## Stopping a daemon

This is the one that does not port, and it is the reason this doc is not a small one.

On Unix, `stop` sends SIGTERM and `--force` sends SIGKILL, both through `/bin/kill` because the workspace forbids `unsafe` and every binding for `kill(2)` is an unsafe extern call. Linux needs nothing new: `/bin/kill` is there, and the two signals mean what they mean.

Windows has neither signal. What it has:

- `taskkill /PID <pid>` posts `WM_CLOSE` to the process's windows. A daemon with no window ignores it, which is every daemon here.
- `taskkill /F /PID <pid>` terminates the process. That is SIGKILL, and it is `--force`.
- `GenerateConsoleCtrlEvent` reaches a process group sharing a console, which a detached daemon does not have.

So there is no Windows equivalent of the graceful stop, and one has to be built. The daemon opens a named event at a path derived from the instance, and waits on it alongside its other work; `stop` opens the same event and sets it. That is the same shape as the Unix path, where the signal arrives and the runtime turns it into the app's own quit, and it keeps `freddie-daemon-runtime.md`'s `on_stop` as the single place an app answers.

The decision this doc does not make is which crate owns that event, because it is the one piece here that needs `unsafe` or a dependency, and `refactors/past/reuse-existing-crates.md` is the precedent for auditing before adding either. Settle it before writing any of this.

`--force` works on Windows the day it is written. The graceful path is the work.

## Detaching the spawned daemon

`start` spawns the child and exits, so the child must outlive the terminal.

```rust
#[cfg(unix)]
fn detach(command: &mut Command) -> &mut Command {
    use std::os::unix::process::CommandExt;
    // Its own process group, so a ctrl-c in the terminal that spawned it does not reach it.
    command.process_group(0)
}

#[cfg(windows)]
fn detach(command: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    // DETACHED_PROCESS: no console at all, which is what the three null stdio streams already say.
    // CREATE_NEW_PROCESS_GROUP: no console control event reaches it.
    command.creation_flags(0x0000_0008 | 0x0000_0200)
}
```

The three null stdio streams and `current_exe` are already portable.

## What CI has to say

`.github/workflows/ci.yml` builds and tests on `macos-latest` only, for the crates that are macOS-bound. A portable `freddie_cli` that is never compiled anywhere else stops being portable within a week.

So the workflow gains a job that builds and tests `freddie_cli` and `freddie_single_instance`, and nothing else, on `ubuntu-latest` and `windows-latest`. Those two crates have no macOS-only dependency, which is what makes the job possible at all.

## The changes, in order

Each is shippable on its own, and the first three change nothing on macOS.

1. **The log directory, three ways.** `Instance::log_dir` with its `cfg` arms and `home()`. macOS keeps the path it has.
2. **`logs` reads the file itself.** The `tail` subprocess is deleted, along with `TAIL`, `TAIL_LINES`, and the exception in `one-log-many-writers.md`.
3. **`detach`, two ways.** The Unix arm is what `spawn_daemon` does today, moved behind a `cfg`.
4. **CI builds the two portable crates on Linux and Windows.** After 1 through 3, this passes; before them, it is what tells you they are needed.
5. **A graceful stop on Windows.** The named event, once the audit above has settled who owns it. Until this lands, Windows has `--force` and nothing gentler, and `stop` says so rather than appearing to work.
