# an app's own verbs sit beside freddie's

`freddie-cli.md` gives every app the lifecycle verbs: `start`, `restart`, `status`, `logs`, `stop`, `install`, `uninstall`, and the hidden `daemon`. An app has verbs of its own beyond those. isograph v2 takes `isograph watch .` and `isograph add-config ./config.json`; mercury's would be pushing an event into the running daemon the way the extension does.

This doc adds one associated type to `freddie_cli::App`. The app declares a `clap::Subcommand`, freddie flattens it in beside its own verbs, and hands back any variant it does not own.

Flattened, so an app's verb is spelled `isograph watch` rather than `isograph app watch`. An app that wants a namespace nests it inside its own enum, where `#[command(subcommand)]` on one of its variants gives `isograph projects list` without freddie knowing.

## The seam

```rust
pub trait App {
    /// This app's own verbs, beyond the lifecycle ones. [`NoVerbs`] for an app with none.
    ///
    /// Flattened into the command line, so each of its variants is a verb of the binary. A
    /// variant carrying its own `#[command(subcommand)]` nests, which is how an app groups them.
    type Verb: clap::Subcommand + fmt::Debug;

    /// Do what one of this app's own verbs asked, and report the exit code for it.
    ///
    /// Runs as a client, with logging already set up the way every client verb has it: `info!` is
    /// the answer and reaches stdout, `warn!` and above reach stderr. The daemon, if one is
    /// running, is another process, and this one does not hold the lock.
    fn run_verb(verb: Self::Verb) -> i32;
}

/// The verbs of an app that adds none.
///
/// Uninhabited, so it adds nothing to the command line and `run_verb` is unreachable.
#[derive(clap::Subcommand, Debug)]
pub enum NoVerbs {}
```

An app with no verbs of its own says so in two lines, and the compiler agrees the arm cannot be reached:

```rust
    type Verb = NoVerbs;

    fn run_verb(verb: NoVerbs) -> i32 {
        match verb {}
    }
```

## Where it joins the command line

One variant, last, so freddie's verbs come first in `--help`:

```rust
#[derive(Subcommand, Debug)]
pub enum Verb<A: App> {
    Start(StartArgs<A>),
    // .. the rest of freddie-cli.md's verbs, unchanged ..
    #[command(hide = true)]
    Daemon(DaemonArgs<A>),

    /// This app's own verbs, each spelled as though it were declared here.
    #[command(flatten)]
    App(A::Verb),
}
```

No signature changes anywhere else: `Verb` is already generic over the app, so the app's verbs arrive as one more associated type. `dispatch` gains one arm, which sets up logging exactly as the other client verbs do and hands over:

```rust
        Some(Verb::App(verb)) => {
            logging::init(A::NAME, &Terminal::Client);
            A::run_verb(verb)
        }
```

`logging::init` runs here rather than inside the app, so an app verb cannot forget it and every verb of every app writes to the one log file with the pid stamp `one-log-many-writers.md` gives it.

## Finding the daemon from an app verb

An app verb that asks something of the running daemon needs to know there is one. `find_daemon` already does this for `stop` and `status`, so it is exposed rather than rewritten:

```rust
/// The running daemon's pid, or `None` when nothing holds the lock.
///
/// What `status` reports, for an app verb that has something to ask of a daemon and needs to know
/// whether one is there. A daemon that has taken the lock but not yet recorded its pid reads as
/// running with no pid, which is the same window `status` reports.
pub fn running<A: App>() -> Result<Option<Pid>, LockError>;
```

How an app then reaches it is the app's: the socket, its port, and what a frame means are all things `freddie_cli` does not know. An app verb declares whatever it needs to say that, in its own args.

## What the apps become

mercury and figaro take `NoVerbs` and the two-line `run_verb` above.

isograph's, as the worked example:

```rust
#[derive(clap::Subcommand, Debug)]
pub enum IsographVerb {
    /// Watch a project directory.
    Watch { dir: PathBuf },
    /// Add a config to the watched set.
    AddConfig { config: PathBuf },
    /// Inspect and change the watched set.
    #[command(subcommand)]
    Projects(ProjectsVerb),
}
```

which gives `isograph watch .`, `isograph add-config ./config.json`, `isograph projects list`, and `isograph projects remove ./src`, alongside `isograph start` and the rest.

## A name an app cannot use

An app verb spelled the same as one of freddie's is a build-time panic, not a silent shadow. clap's debug asserts catch it: `Command isograph: command name 'status' is duplicated`.

Those asserts run in debug builds, so a test is what makes it certain. `freddie_cli` carries one that every app's own tests can call:

```rust
/// Assert that `A`'s command line builds, which is where clap's debug asserts run.
///
/// A verb an app spells the same as one of freddie's is caught here rather than in whatever
/// release build first runs it. `build` is what `get_matches` would have called.
pub fn assert_verbs_are_unique<A: App>() {
    Args::<A>::command().name(A::NAME).about(A::ABOUT).build();
}
```

## Verified

On the pinned 1.96.0 against clap 4.6.2, against a `Verb<A: App>` holding freddie's verbs and `#[command(flatten)] App(A::Verb)`:

- A flattened app verb parses at the top level and takes positionals: `mercury tab https://claude.ai` and `isograph watch ./src` both reach the app's variant.
- A nested one parses two deep: `isograph projects list` and `isograph projects remove ./src`.
- The app's flags still flatten into freddie's verbs alongside all this: `isograph start --config ./iso.toml`.
- `--help` lists freddie's verbs and then the app's, in declaration order, with `daemon` still hidden.
- `NoVerbs` adds nothing: `figaro start` parses and `figaro watch` is refused as an unrecognized subcommand.
- `assert_verbs_are_unique` passes for three apps whose verbs are distinct, and panics for one that declares `status`, naming the duplicated verb.

## The changes, in order

`freddie-cli.md` lands first, since this adds an associated type to the trait it introduces and an arm to its dispatch.

1. **`App::Verb`, `NoVerbs`, and `run_verb`**, the flattened variant, and the `dispatch` arm. mercury and figaro take `NoVerbs`, so no binary gains a verb and no `--help` changes.
2. **`running::<A>()` and `assert_verbs_are_unique::<A>()`**, exposed for the first app that has a verb to write. `running` rather than `daemon`, which is already the module the daemon verb lives in.
