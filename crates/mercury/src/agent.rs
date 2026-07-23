//! `mercury install` and `mercury uninstall`: the launch agent, which is mercury's own.
//!
//! What a launch agent says is an answer about one app rather than about daemons in general: which
//! session type it loads in, when launchd should revive it, and what label it sits under. So this
//! stays here rather than moving to `freddie_cli` with the lifecycle verbs.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use freddie_cli::{App, DAEMON_VERB};
use serde::Serialize;
use tracing::{debug, info, warn};

use crate::Mercury;

/// mercury's own verbs, flattened into its command line beside freddie's.
#[derive(clap::Subcommand, Debug)]
pub(crate) enum MercuryVerb {
    /// Register this binary as a login agent, so mercury starts with the session.
    Install,
    /// Take the login agent back out.
    Uninstall,
}

/// The reverse-DNS prefix mercury's launch agent sits under.
const AGENT_PREFIX: &str = "hg.freddie.";

/// The launchd job's name, keyed to the same name as the lock and the log directory, so a fork
/// that renames the app gets its own job.
fn label() -> String {
    format!("{AGENT_PREFIX}{}", Mercury::NAME)
}

/// Put what these verbs say on the terminal and in mercury's log, the way every lifecycle verb's
/// output goes. `freddie_cli` does this for its own verbs; these are mercury's.
fn log_to_mercurys_file() {
    if let Ok(instance) = Mercury::instance(&freddie_cli::NoArgs) {
        freddie_cli::init_client_logging(&instance);
    }
}

/// Where `launchctl` lives. Absolute, so `PATH` cannot point this at something else.
const LAUNCHCTL: &str = "/bin/launchctl";

/// Where `id` lives, for the one number with no safe route out of std.
const ID: &str = "/usr/bin/id";

/// A binary under this is not somewhere an agent should point for long: `cargo clean` deletes it.
const TRANSIENT: &str = "/target/";

/// The launch agent this app installs, as launchd reads it.
///
/// Serialized rather than written from a template: a program path holds whatever a home directory
/// holds, and `&` in one would have to be escaped by hand on the way into XML. The serializer does
/// that, and cannot emit a malformed plist at all.
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct Agent {
    /// The job's name, keyed to the same name as the lock and the log directory, so a fork that
    /// renames the app gets its own job rather than fighting this one over a shared label.
    label: String,

    /// The daemon verb, never the bare binary: `mercury` spawns a detached daemon and exits, which would
    /// leave launchd watching the job vanish and unable to see the process that actually holds the
    /// keyboard. This is why that verb stays invocable while hidden from `--help`.
    program_arguments: Vec<String>,

    /// Start it with the session.
    run_at_load: bool,

    /// `Aqua`: the session `CGEventTap` needs a window server, `NSWorkspace`, and per-user TCC,
    /// none of which a root daemon at the login window has.
    limit_load_to_session_type: String,

    keep_alive: KeepAlive,

    /// Seconds. A crash loop must not respawn something that eats the keyboard ten times a second.
    throttle_interval: u32,
}

/// When launchd should bring the job back.
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct KeepAlive {
    /// `false`: revive a mercury that died unexpectedly, leave one down that declined to run.
    ///
    /// Both halves are the exit code. Every deliberate way out exits zero: `q` from home, the
    /// menu bar's Quit, `mercury stop`, `launchctl bootout`, all of which reach the model's quit,
    /// and so does every refusal to start, since none of those is fixed by trying again.
    successful_exit: bool,
}

impl Agent {
    /// The agent that runs `program`.
    fn running(program: &Path) -> Self {
        Self {
            label: label(),
            program_arguments: vec![
                program.to_string_lossy().into_owned(),
                DAEMON_VERB.to_owned(),
            ],
            run_at_load: true,
            limit_load_to_session_type: "Aqua".to_owned(),
            keep_alive: KeepAlive {
                successful_exit: false,
            },
            throttle_interval: 10,
        }
    }
}

/// Where the agent's plist goes. launchd reads this directory per user.
fn plist_path() -> Option<PathBuf> {
    Some(
        PathBuf::from(std::env::var_os("HOME")?)
            .join("Library/LaunchAgents")
            .join(format!("{}.plist", label())),
    )
}

/// Why an install or uninstall did not happen.
enum NotInstalled {
    /// The environment names no home directory to put the agent in.
    NoHome,
    /// This binary's own path could not be read.
    NoExe(io::Error),
    /// The plist could not be written or removed.
    Unwritable(io::Error),
    /// The plist could not be serialized.
    Unserializable(plist::Error),
    /// A subprocess could not be run, or refused.
    Refused(io::Error),
}

/// The terminal wording for each, without the `mercury: ` a caller puts in front.
impl fmt::Display for NotInstalled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoHome => {
                f.write_str("no home directory to install the agent into; is HOME set?")
            }
            Self::NoExe(e) => write!(f, "could not read this binary's path: {e}"),
            Self::Unwritable(e) => write!(f, "could not write the agent: {e}"),
            Self::Unserializable(e) => write!(f, "could not build the agent: {e}"),
            Self::Refused(e) => write!(f, "{e}"),
        }
    }
}

/// `mercury install`: register this binary as a login agent.
///
/// Idempotent. A previously loaded job is booted out before the new one is bootstrapped, so
/// re-running this after `cargo install` is how the agent comes to point at a rebuilt binary.
pub(crate) fn install() -> ExitCode {
    log_to_mercurys_file();
    match install_agent() {
        Ok(program) => {
            info!("mercury installed ({})", program.display());
            if program.to_string_lossy().contains(TRANSIENT) {
                warn!(
                    "mercury: that binary is under target/, which `cargo clean` deletes; \
                     `cargo install --path crates/mercury` and install again to point at a lasting one"
                );
            }
            ExitCode::SUCCESS
        }
        Err(failure) => {
            warn!("mercury: {failure}");
            ExitCode::FAILURE
        }
    }
}

/// Write the plist and hand it to launchd, returning the binary it now names.
fn install_agent() -> Result<PathBuf, NotInstalled> {
    let program = std::env::current_exe().map_err(NotInstalled::NoExe)?;
    let path = plist_path().ok_or(NotInstalled::NoHome)?;

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(NotInstalled::Unwritable)?;
    }
    plist::to_file_xml(&path, &Agent::running(&program)).map_err(NotInstalled::Unserializable)?;
    debug!(plist = %path.display(), program = %program.display(), "wrote the agent");

    // A failure here is the normal first install: there is nothing loaded to boot out. Traced
    // rather than ignored outright, so the log still says what launchd made of it.
    if let Err(e) = bootout() {
        debug!(%e, "nothing was loaded to boot out");
    }
    launchctl(&["bootstrap", &domain()?, &path.to_string_lossy()])?;
    Ok(program)
}

/// `mercury uninstall`: take the login agent back out.
///
/// Exits 0 when nothing was installed, so a teardown script that does not know the state is not
/// wrong to call it.
pub(crate) fn uninstall() -> ExitCode {
    log_to_mercurys_file();
    match uninstall_agent() {
        Ok(()) => {
            info!("mercury uninstalled");
            ExitCode::SUCCESS
        }
        Err(failure) => {
            warn!("mercury: {failure}");
            ExitCode::FAILURE
        }
    }
}

fn uninstall_agent() -> Result<(), NotInstalled> {
    let path = plist_path().ok_or(NotInstalled::NoHome)?;
    // launchd forgets the job before its description goes, or it is left holding one whose plist
    // no longer exists. A failure is nothing having been loaded, as in `install_agent`.
    if let Err(e) = bootout() {
        debug!(%e, "nothing was loaded to boot out");
    }
    match std::fs::remove_file(&path) {
        Ok(()) => {
            debug!(plist = %path.display(), "removed the agent");
            Ok(())
        }
        // Nothing installed is not a failure to uninstall.
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(NotInstalled::Unwritable(e)),
    }
}

/// The user's GUI domain, which is where a `LaunchAgent` lives.
fn domain() -> Result<String, NotInstalled> {
    Ok(format!("gui/{}", users_uid()?))
}

/// Tell launchd to forget the job, whether or not it has one.
fn bootout() -> Result<(), NotInstalled> {
    launchctl(&["bootout", &format!("{}/{}", domain()?, label())])
}

/// Run `launchctl` with `args`, reporting a refusal as a failure.
///
/// `output` rather than `status`, so launchctl's own stderr is captured instead of inherited. It
/// complains on a `bootout` with nothing loaded, which is the normal first install, and printing
/// that beside "mercury installed" reads like a failure. Its words are kept for the error that
/// does fail, where they say more than an exit code.
fn launchctl(args: &[&str]) -> Result<(), NotInstalled> {
    let out = Command::new(LAUNCHCTL)
        .args(args)
        .output()
        .map_err(NotInstalled::Refused)?;
    if out.status.success() {
        Ok(())
    } else {
        let said = String::from_utf8_lossy(&out.stderr);
        Err(NotInstalled::Refused(io::Error::other(format!(
            "{LAUNCHCTL} {} exited with {}: {}",
            args.join(" "),
            out.status,
            said.trim()
        ))))
    }
}

/// This user's numeric id, which names the launchd domain their agents live in.
///
/// A subprocess rather than `getuid(2)`, because the workspace forbids `unsafe` and every binding
/// for it is an unsafe extern call. The same trade [`signal_pid`] makes with `/bin/kill`.
fn users_uid() -> Result<u32, NotInstalled> {
    let out = Command::new(ID)
        .arg("-u")
        .output()
        .map_err(NotInstalled::Refused)?;
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .map_err(|_| NotInstalled::Refused(io::Error::other(format!("{ID} -u printed no number"))))
}

#[cfg(test)]
mod tests {
    use super::{Agent, label};
    use std::path::Path;

    fn agent_xml(program: &str) -> String {
        let mut xml = Vec::new();
        plist::to_writer_xml(&mut xml, &Agent::running(Path::new(program))).expect("serializing");
        String::from_utf8(xml).expect("plists are utf8")
    }

    // A fork renames `APP` and gets its own launchd job rather than fighting this one over a label.
    #[test]
    fn the_label_is_keyed_to_the_app() {
        assert_eq!(label(), "hg.freddie.mercury");
    }

    #[test]
    fn the_agent_runs_the_daemon_verb() {
        let xml = agent_xml("/Users/somebody/.cargo/bin/mercury");
        assert!(xml.contains("<string>/Users/somebody/.cargo/bin/mercury</string>"));
        assert!(xml.contains("<string>daemon</string>"));
        assert!(xml.contains("<key>Label</key>"));
    }

    // The reason this is serialized rather than substituted into a template: a home directory can
    // hold an `&`, and writing one into XML unescaped makes a plist launchd will not read.
    #[test]
    fn a_program_path_is_escaped() {
        let xml = agent_xml("/Users/a&b/.cargo/bin/mercury");
        assert!(xml.contains("/Users/a&amp;b/.cargo/bin/mercury"));
        assert!(!xml.contains("/Users/a&b/"));
    }

    // launchd revives a mercury that died and leaves one down that declined to run.
    #[test]
    fn the_agent_only_revives_an_unclean_exit() {
        let xml = agent_xml("/usr/bin/true");
        assert!(xml.contains("<key>SuccessfulExit</key>"));
        assert!(xml.contains("<false/>"));
    }
}
