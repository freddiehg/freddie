# launching mercury at login

Run mercury as a per-user LaunchAgent that starts with the session and restarts on a crash, without ever leaving the keyboard dead. The plist is easy and the boot state is already right; whether a launchd agent can take the keyboard at all is the one thing this doc turns on.

## Boot into typing

`Mercury::default()` boots into `Layer::Typing`, which binds nothing at all: every key falls to the root, runs through the `jk` sequence, and passes through. So the keyboard is normal at login, and the launchd build just runs the default. Home, the command layer, swallows every key it does not bind, which is why it cannot be the boot state.

There is no dedicated off state; typing IS the login-safe boot state.

The way from typing into Home is `jk` — `KeyJ` then `KeyK` within `JK_TIMEOUT`, 200ms. Nothing else leaves typing, and in particular `escape` does not: it is bound in Home, not in typing or at the root.

So the hazard at login is typing a literal `jk` fast enough, which lands you in the layer that swallows unbound keys. It takes two deliberate keys inside a 200ms window rather than one stray press, which is why the recovery paths below are worth having without being the only thing standing between you and a dead keyboard.

## LaunchAgent, not daemon

`~/Library/LaunchAgents`, `LimitLoadToSessionType = Aqua`. A daemon runs as root with no window server, no `NSWorkspace`, and no per-user TCC, none of which the session `CGEventTap` works without. An agent also starts after login rather than at the login window, which is what we want.

## The plist

Written by `mercury install` from a serialized struct, not checked in. Neither of the two things that identify a job can be a literal in a checked-out repo — the program path lives under someone's home directory, and the label is keyed to `APP` so a fork gets its own job — and the struct is the reviewable artifact, carrying each key's reason as a doc comment.

Serialized rather than substituted into a template, because a home directory holds whatever a home directory holds. `/Users/a&b/…` written into XML by hand is a plist launchd will not read; `plist::to_file_xml` emits `&amp;` and cannot produce malformed output at all.

`launchctl` by hand still works against an installed agent:

```
launchctl kickstart -k gui/$(id -u)/hg.freddie.mercury   # restart after a rebuild
launchctl bootout   gui/$(id -u)/hg.freddie.mercury      # stop it (the escape hatch)
```

`daemon` rather than the bare binary, and this is the whole reason that verb stays invocable while hidden from `--help`. Bare `mercury` spawns a detached daemon and exits, which launchd must never run: it would watch the job exit at once, and the process actually holding the keyboard would be one launchd knows nothing about and cannot restart.

`KeepAlive`/`SuccessfulExit=false` restarts on a crash but stays dead after a deliberate exit. Every deliberate way out is one: `q` from home, the menu bar's Quit, `mercury stop`, and `launchctl bootout` all reach the model's quit and exit zero, because SIGTERM is routed into the event channel (`refactors/past/mercury-stop.md`).

So does every refusal to start. `mercury daemon` exits zero when another instance holds the lock, when the menu bar cannot be created, and when the keyboard grab is denied. That is the behaviour this plist wants and it must stay that way: none of those is fixed by trying again, and a nonzero exit would have launchd retry a refused daemon every `ThrottleInterval` for as long as the machine is up. launchd revives a mercury that died unexpectedly and leaves one down that declined to run.

It is worth stating because it currently happens by omission rather than by intent — `daemon::run` returns nothing and `main` hands back a literal zero — and because the obvious tidying, returning a code from each failure arm, is the thing that would break it. If those arms ever do report codes, the plist needs `SuccessfulExit` reconsidered in the same change.

`ThrottleInterval` stops a crash loop from respawning a keyboard-eater ten times a second. No `StandardOutPath` and no `LOG_LEVEL`: mercury writes its own log, and `--log-level` governs a terminal a launchd job does not have. `HOME` is set for agents, which `logging::log_dir` needs; `PATH` is minimal but `/bin/kill` and `open`, the only subprocesses, are at absolute paths.

`launchctl kickstart -k` rather than `mercury restart` under the agent. `restart` stops the daemon and spawns a replacement of its own, which launchd did not start and will not keep alive; the old job then looks like a clean exit and stays down. `kickstart` replaces the process launchd is managing, which is the one you want. `mercury stop` and `mercury status` are unaffected, since they only signal and read.

## Installing it

`mercury install` registers the binary that ran it as the agent; `mercury uninstall` takes it back out. Neither copies a binary anywhere. Someone who checked out this repo, changed a binding, and wants their mercury at login runs two commands:

```
cargo install --path crates/mercury    # the binary, into ~/.cargo/bin
mercury install                        # the agent, pointing at it
```

### Why the binary is cargo's problem and not ours

`cargo install --path crates/mercury` builds release, puts `mercury` in `~/.cargo/bin`, and replaces it on the next run. Anyone forking this repo has a toolchain by definition, and `~/.cargo/bin` is already on their PATH. Verified: a release build and install of this workspace takes 25 seconds and produces one executable.

So `install` does not copy, and `/usr/local/bin` stops being mentioned anywhere. That path needs `sudo`, is root-owned, and is system-wide, while a LaunchAgent is per-user by construction — `~/Library/LaunchAgents`, `LimitLoadToSessionType = Aqua`, one per login session. A per-user agent pointing into a system-wide directory is a mismatch that buys nothing.

### The path comes from the running binary

The plist cannot hold a literal program path, because the one that matters lives under a home directory. `install` writes `std::env::current_exe()` instead, which makes the agent point at whichever binary registered it: `~/.cargo/bin/mercury` after a `cargo install`, or `target/release/mercury` if that is what you ran.

That last case is worth a word rather than a refusal. Registering a binary under `target/` is exactly right while iterating on the agent itself, and exactly wrong afterwards, because `cargo clean` deletes it and launchd then has a job pointing at nothing.

```rust
/// Where a binary under this lives is not somewhere an agent should point for long.
const TRANSIENT: &str = "/target/";
```

### The agent is a struct

```rust
/// The launch agent this app installs, as launchd reads it.
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct Agent {
    /// The job's name, keyed to the same `APP` as the lock and the log directory.
    label: String,
    /// `daemon`, never the bare binary, which spawns a detached daemon and exits.
    program_arguments: Vec<String>,
    run_at_load: bool,
    /// `Aqua`: the session `CGEventTap` needs a window server, `NSWorkspace`, and per-user TCC.
    limit_load_to_session_type: String,
    keep_alive: KeepAlive,
    /// Seconds. A crash loop must not respawn a keyboard-eater ten times a second.
    throttle_interval: u32,
}

/// When launchd should bring the job back.
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct KeepAlive {
    /// `false`: revive a mercury that died unexpectedly, leave one down that declined to run.
    successful_exit: bool,
}
```

`plist = "1.10"` in `crates/mercury/Cargo.toml`; mercury already depends on `serde`.

### The verb

In `client.rs`.

```rust
/// Where `launchctl` lives. Absolute, so `PATH` cannot point this at something else.
const LAUNCHCTL: &str = "/bin/launchctl";

/// Why an install did not happen.
enum NotInstalled {
    /// The environment names no home directory to put the agent in.
    NoHome,
    /// This binary's own path could not be read.
    NoExe(io::Error),
    /// The plist could not be written or removed.
    Unwritable(io::Error),
    /// The plist could not be serialized.
    Unserializable(plist::Error),
    /// `launchctl` could not be run, or refused.
    Launchctl(io::Error),
}

impl fmt::Display for NotInstalled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoHome => f.write_str("no home directory to install the agent into; is HOME set?"),
            Self::NoExe(e) => write!(f, "could not read this binary's path: {e}"),
            Self::Unwritable(e) => write!(f, "could not write the agent: {e}"),
            Self::Launchctl(e) => write!(f, "{LAUNCHCTL}: {e}"),
        }
    }
}

/// `mercury install`: register this binary as a login agent.
///
/// Idempotent. A previously loaded job is booted out before the new one is bootstrapped, so
/// re-running this after `cargo install` is how you point the agent at a rebuilt binary.
pub(crate) fn install() -> i32 {
    logging::init(&Terminal::Client);
    match install_agent() {
        Ok(program) => {
            info!("mercury installed ({})", program.display());
            if program.to_string_lossy().contains(TRANSIENT) {
                warn!(
                    "mercury: that binary is under target/, which `cargo clean` deletes; \
                     `cargo install --path crates/mercury` then `mercury install` again"
                );
            }
            0
        }
        Err(failure) => {
            warn!("mercury: {failure}");
            1
        }
    }
}

/// Write the plist and hand it to launchd.
fn install_agent() -> Result<PathBuf, NotInstalled> {
    let program = std::env::current_exe().map_err(NotInstalled::NoExe)?;
    let path = plist_path().ok_or(NotInstalled::NoHome)?;

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(NotInstalled::Unwritable)?;
    }
    plist::to_file_xml(&path, &Agent::running(&program)).map_err(NotInstalled::Unserializable)?;
    debug!(plist = %path.display(), program = %program.display(), "wrote the agent");

    // Ignored: it fails when nothing was loaded, which is the normal first install.
    let _ = bootout();
    launchctl(&["bootstrap", &domain(), &path.to_string_lossy()])?;
    Ok(program)
}

/// `mercury uninstall`: take the agent back out.
///
/// Exits 0 when nothing was installed, so a teardown script that does not know the state is not
/// wrong to call it.
pub(crate) fn uninstall() -> i32 {
    logging::init(&Terminal::Client);
    match uninstall_agent() {
        Ok(()) => {
            info!("mercury uninstalled");
            0
        }
        Err(failure) => {
            warn!("mercury: {failure}");
            1
        }
    }
}

fn uninstall_agent() -> Result<(), NotInstalled> {
    let path = plist_path().ok_or(NotInstalled::NoHome)?;
    // Before the file goes: launchd is told to forget the job, then the job's description is
    // removed. The other order leaves launchd holding a job whose plist is gone.
    let _ = bootout();
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        // Nothing installed is not a failure to uninstall.
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(NotInstalled::Unwritable(e)),
    }
}

/// The user's GUI domain, which is where a LaunchAgent lives.
fn domain() -> String {
    // `id -u` is what the documented invocation uses; this is the same number without a subprocess.
    format!("gui/{}", users_uid())
}

fn bootout() -> Result<(), NotInstalled> {
    launchctl(&["bootout", &format!("{}/{}", domain(), label())])
}

/// Run `launchctl` with `args`, reporting a refusal as a failure.
fn launchctl(args: &[&str]) -> Result<(), NotInstalled> {
    let status = Command::new(LAUNCHCTL)
        .args(args)
        .status()
        .map_err(NotInstalled::Launchctl)?;
    if status.success() {
        Ok(())
    } else {
        Err(NotInstalled::Launchctl(io::Error::other(format!(
            "{args:?} exited with {status}"
        ))))
    }
}
```

`users_uid` is the one piece with no safe std route: `std::os::unix` exposes no `getuid`. `launchctl print gui/$UID` cannot be asked for it either, since the answer is the question. `$UID` is not exported by every shell, so read `id -u` once:

```rust
/// This user's numeric id, which names the launchd domain their agents live in.
///
/// A subprocess rather than `getuid(2)`, because the workspace forbids `unsafe` and every binding
/// for it is an unsafe extern call. The same trade `signal_pid` makes with `/bin/kill`.
fn users_uid() -> Result<u32, NotInstalled> {
    let out = Command::new("/usr/bin/id")
        .arg("-u")
        .output()
        .map_err(NotInstalled::Launchctl)?;
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .map_err(|_| NotInstalled::Launchctl(io::Error::other("id -u did not print a number")))
}
```

### Wiring

`Verb` gains two, after the lifecycle verbs and before the hidden `daemon`:

```rust
    /// Register this binary as a login agent, so mercury starts with the session.
    Install,
    /// Take the login agent back out.
    Uninstall,
```

with `main.rs` arms calling `client::install()` and `client::uninstall()`.

### Tests

The parts that are not launchd:

```rust
#[test]
fn the_label_is_keyed_to_the_app() {
    assert_eq!(label(), "hg.freddie.mercury");
}

#[test]
fn the_agent_runs_the_daemon_verb() {
    let xml = agent_xml("/Users/somebody/.cargo/bin/mercury");
    assert!(xml.contains("<string>/Users/somebody/.cargo/bin/mercury</string>"));
    assert!(xml.contains("<string>daemon</string>"));
}

// The reason this is serialized rather than substituted: a home directory can hold an `&`, and
// writing one into XML unescaped makes a plist launchd will not read.
#[test]
fn a_program_path_is_escaped() {
    let xml = agent_xml("/Users/a&b/.cargo/bin/mercury");
    assert!(xml.contains("/Users/a&amp;b/.cargo/bin/mercury"));
    assert!(!xml.contains("/Users/a&b/"));
}

#[test]
fn the_agent_only_revives_an_unclean_exit() {
    let xml = agent_xml("/usr/bin/true");
    assert!(xml.contains("<key>SuccessfulExit</key>"));
    assert!(xml.contains("<false/>"));
}
```

What no test covers is launchd accepting the result, which needs a `bootstrap` and a login.

### Verifying the install

- `cargo install --path crates/mercury`, then `mercury install`, says where it installed from and `launchctl print gui/$(id -u)/hg.freddie.mercury` describes the job.
- Logging out and back in leaves a mercury running, by `mercury status`.
- `mercury install` again replaces the job rather than failing, and `mercury status` reports a pid either way.
- `./target/debug/mercury install` says the binary is under `target/` and installs anyway.
- The written `~/Library/LaunchAgents/hg.freddie.mercury.plist` passes `plutil -lint`.
- `mercury uninstall` removes the job and the plist; running it twice exits 0 both times.
- After `mercury uninstall`, logging out and back in leaves no mercury running.

## Permissions

Nothing is granted by hand today and nothing needs to be. Rebuilding mercury and starting a fresh daemon keeps the tap: observed against a daemon at `target/debug/mercury` with PPID 1, started by `mercury start` from a terminal, which went on to dispatch 1678 events. So the grant is not keyed to the rebuilt binary's bits, and no stable signed binary is required for the way mercury is run now.

What it is keyed to is the open question, and it decides whether the agent works. macOS attributes an access request to a *responsible* process rather than always to the immediate caller, which is why a script run from a terminal raises a prompt naming the terminal. If that is what is happening, every mercury started from a shell inherits that grant however often it is rebuilt — and a launchd agent, which has no terminal anywhere in its ancestry, inherits nothing.

So the thing to find out is not how to keep a grant stable across rebuilds. It is whether a launchd-started mercury gets a tap at all:

```
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/hg.freddie.mercury.plist
mercury logs --level warn
```

The daemon takes the lock either way, so `mercury status` proves nothing here. `could not intercept the keyboard` in the log is the answer, and its absence plus dispatch records is the other one.

Only if that fails does any of the signing work matter, and a wrapper is worth considering then rather than now. A shell script needs no compiling and never changes, but what launchd executes is `/bin/sh`, and whether responsibility lands on the shell, on mercury after it `exec`s, or on neither is exactly the thing the experiment above settles. Design it against an answer, not ahead of one.

Reset grants while testing, since the failure is silent: `tccutil reset Accessibility` and `tccutil reset ListenEvent`.

## Getting the keyboard back

The failure mode is that you cannot type, reproduced on every login. Safe Mode is the answer and no code is needed for it: holding Shift at boot loads no third-party agents, so the machine comes up with a working keyboard and the plist can be `bootout`ed from there.

Two others exist if Safe Mode ever proves not to be enough. ssh from another machine and `launchctl bootout gui/$(id -u)/hg.freddie.mercury`. Or a modifier held at launch making mercury skip the grab entirely, the same shape as the frontmost-app seed: read modifier state in `daemon.rs`'s `serve` before `intercept`, and do not install the tap if a chord is held.

## Single instance

launchd owns the label, so a second `bootstrap` fails. Two mercuries are already impossible below that: `refactors/past/single-instance.md`'s lock refuses the second, whoever started it.

Nothing extra is needed for a hand-started mercury meeting the agent's. `mercury start` finds the agent's daemon holding the lock and adopts it, reporting its pid; `mercury status` names it; `mercury stop` ends it, and `KeepAlive`/`SuccessfulExit=false` leaves it stopped because that is a clean exit.

## Bundle vs bare binary

Everything above ships a bare binary: `mercury` at `/usr/local/bin`, a plist written by hand into `~/Library/LaunchAgents`, and `launchctl bootstrap` run once. That works and is what this doc plans.

The alternative is an app bundle. `Mercury.app` holds the binary at `Contents/MacOS/mercury`, an `Info.plist`, and its agent plist at `Contents/Library/LaunchAgents/`; the app calls `SMAppService.agent(plistName:).register()` (macOS 13+) and macOS installs the agent itself. Three things follow from that, and none of them are available to a bare binary:

- the job appears in System Settings under Login Items, with a toggle, instead of existing only as a file the user has to know about
- `LSUIElement` in `Info.plist` makes it an accessory app, replacing the `freddie_main_loop::init_menu_bar_app()` call
- a bundle identifier and one signature over the whole bundle give TCC a stable identity to key a grant to

The third is the reason to decide this after the launchd experiment rather than before it. If an agent gets a tap, the bare binary is enough and the bundle buys only tidiness. If it does not, a signed bundle is the most likely thing that fixes it, and the work belongs there rather than in a wrapper.

## Open

- Whether a launchd-started mercury gets a tap at all, which is the one that decides this doc.
- Which TCC permission the tap needs: Accessibility, Input Monitoring, or both.
- Whether `launchctl bootout` gives the daemon long enough to finish its quit before SIGKILL follows, since that path now has destructors to run.
- Whether a launchd agent can raise a usable TCC prompt, or a first grant must come from a terminal launch.
- Whether the launch build also needs Home to pass unbound keys through, or booting into typing plus the recovery paths is enough given that reaching Home takes a deliberate `jk`.
