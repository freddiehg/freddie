# running a command

A handler can ask for a key, a window frame, a clipboard, and an app. It cannot ask for a program to be run, so a binding whose whole point is a program — clone this repository, check out this pull request, paste today's date — has nowhere to put it.

`MercuryEffect::Run` is that effect. It carries the program, its arguments, and the directory to run them in, and the effect side runs exactly that.

## The payload carries everything

`perform_effect` decides nothing, so it does not expand `~`, does not read `HOME`, and does not fall back to a working directory. The handler that asks for a command has already worked out an absolute directory, which means the model holds one; `github-site.md` puts it there.

`crates/mercury/src/effect.rs`, added above `MercuryEffect`:

```rust
/// A program to run, and everything running it needs.
///
/// The directory is absolute. The effect side does not expand `~`, does not read the environment
/// to find one, and does not have a default: a command with nowhere to run is a handler that
/// dropped something it had.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub struct Run {
    /// Found on `PATH`, or an absolute path to the program.
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}
```

`MercuryEffect`, before:

```rust
    /// Put text on the clipboard, replacing what is there.
    Copy(Copied),
```

after:

```rust
    /// Put text on the clipboard, replacing what is there.
    Copy(Copied),
    /// Run a program. Nothing waits for it and nothing reads its output; the log is where its
    /// exit status and its stderr go.
    Run(Run),
```

`Eq` survives, because `Run` is strings and a path. `MercuryEffect` keeps `PartialEq` only, for the same reason it already does.

## Performing it

`crates/mercury/src/daemon.rs`, in `perform_effect`, before:

```rust
        MercuryEffect::Copy(what) => copy(what),
```

after:

```rust
        MercuryEffect::Copy(what) => copy(what),
        MercuryEffect::Run(run) => run_command(run),
```

and the performer, next to `copy`:

```rust
/// Run a program, fire-and-forget on its own thread like the rest. A `git clone` takes seconds,
/// which the effect loop cannot spend: a key the model has already decided on would wait behind it.
///
/// Nothing comes back to the model. The exit status and whatever the program said on stderr go to
/// the log, which is where the answer to "did that work" is.
fn run_command(run: Run) {
    std::thread::spawn(move || {
        let Run { program, args, cwd } = run;
        let out = match std::process::Command::new(&program)
            .args(&args)
            .current_dir(&cwd)
            .output()
        {
            Ok(out) => out,
            Err(e) => {
                warn!(%program, ?args, cwd = %cwd.display(), error = %e, "could not run");
                return;
            }
        };
        let stderr = String::from_utf8_lossy(&out.stderr);
        if out.status.success() {
            debug!(%program, ?args, cwd = %cwd.display(), "ran");
        } else {
            warn!(
                %program,
                ?args,
                cwd = %cwd.display(),
                status = ?out.status.code(),
                stderr = %stderr.trim(),
                "command failed"
            );
        }
    });
}
```

`output` rather than `spawn`, because the stderr is the whole point of the failure log: a `spawn` that succeeds tells the log nothing about a `git clone` that then refused.

The program's stdout is read and dropped. A command whose output should land somewhere is a different effect, and `Copy` is the shape it would take.

## The test the effect gets

Effects are inert data, so the model tests assert the payload and never run anything. `crates/mercury/tests/transitions.rs` gains the helper the bindings that use this will assert against:

```rust
fn ran(program: &str, args: &[&str], cwd: &str) -> MercuryEffect {
    MercuryEffect::Run(Run {
        program: program.to_owned(),
        args: args.iter().map(|a| (*a).to_owned()).collect(),
        cwd: PathBuf::from(cwd),
    })
}
```
