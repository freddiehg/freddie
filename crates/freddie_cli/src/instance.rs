//! Which daemon a command line meant, and where that daemon's files go.

use std::fmt;
use std::path::{Path, PathBuf};

/// Which daemon this is.
///
/// The slug names a lock file and a log file, so it is whatever the filesystem will take. The
/// display name is what a person typed, so `status` says something they recognize. For an app
/// with one global daemon both are its name.
#[derive(Clone, Debug)]
pub struct Instance {
    slug: String,
    display_name: String,
    lock_file: PathBuf,
    log_dir: PathBuf,
    log_file_name: String,
}

impl Instance {
    /// The one daemon of an app that has one, keyed to the app itself.
    ///
    /// # Errors
    ///
    /// [`NoUserDir`] when the environment names no per-user directory.
    pub fn global(app: &str) -> Result<Self, NoUserDir> {
        Self::named(app, app, app)
    }

    /// One of many: `slug` names its files, `display_name` is what the person who asked for it
    /// typed.
    ///
    /// `slug` has to be stable across two invocations that mean the same daemon, and distinct for
    /// two that do not, since it is the whole of what the lock is keyed to. A path that has been
    /// resolved, or a hash of one, is the shape of it. It also has to be a filename, because it
    /// becomes one.
    ///
    /// # Errors
    ///
    /// [`NoUserDir`] when the environment does not say where this user's files go, which is the
    /// one thing about placing a daemon that can fail. Both paths resolve here, once, rather than
    /// at each call that wants one: an instance that exists is one whose files have a place to be.
    pub fn named(
        app: &str,
        slug: impl Into<String>,
        display_name: impl Into<String>,
    ) -> Result<Self, NoUserDir> {
        let slug = slug.into();
        Ok(Self {
            lock_file: freddie_single_instance::lock_path(&slug).ok_or(NoUserDir)?,
            log_dir: log_dir(app)?,
            log_file_name: format!("{slug}.log"),
            display_name: display_name.into(),
            slug,
        })
    }

    /// The file whose exclusive lock is this daemon's claim to being the only one of itself.
    #[must_use]
    pub fn lock_file(&self) -> &Path {
        &self.lock_file
    }

    /// The directory this daemon's log goes in, one per app.
    ///
    /// Kept apart from the file name because that is how `tracing_appender` takes them, and
    /// because the directory is what has to be created before anything opens the file.
    #[must_use]
    pub fn log_dir(&self) -> &Path {
        &self.log_dir
    }

    /// The name of this daemon's log file, one per daemon.
    #[must_use]
    pub fn log_file_name(&self) -> &str {
        &self.log_file_name
    }

    /// The two of them joined, for saying where the log is and for reading it back.
    #[must_use]
    pub fn log_file(&self) -> PathBuf {
        self.log_dir.join(&self.log_file_name)
    }

    /// What a verb calls this daemon when it says something about it.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// What this daemon's files are keyed to.
    #[must_use]
    pub fn slug(&self) -> &str {
        &self.slug
    }
}

/// The environment names no per-user directory to keep this daemon's lock and log in.
///
/// One error for both, because one environment answers for both: the lookup that places the lock
/// is the lookup that places the log, and a daemon missing either was never going to run.
#[derive(Debug)]
pub struct NoUserDir;

impl fmt::Display for NoUserDir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("no per-user directory to keep the lock and the log in; is HOME set?")
    }
}

impl std::error::Error for NoUserDir {}

/// The per-user directory `app`'s logs go in: the platform's place for logs a person is expected
/// to read. It sits beside where `freddie_single_instance` puts the lock, so a daemon that can
/// take its lock can write its log.
fn log_dir(app: &str) -> Result<PathBuf, NoUserDir> {
    let home = std::env::var_os("HOME").ok_or(NoUserDir)?;
    Ok(PathBuf::from(home).join("Library/Logs").join(app))
}

#[cfg(test)]
mod tests {
    use super::Instance;

    #[test]
    fn a_global_instance_is_named_for_its_app() {
        let instance = Instance::global("testapp").expect("HOME is set in a test run");
        assert_eq!(instance.slug(), "testapp");
        assert_eq!(instance.display_name(), "testapp");
        assert_eq!(instance.log_file_name(), "testapp.log");
    }

    // Two ids that name two daemons keep every file of theirs apart, which is the whole of what
    // an instance is for.
    #[test]
    fn two_instances_share_no_file() {
        let a = Instance::named("testapp", "testapp-a", "./a.json").expect("HOME is set");
        let b = Instance::named("testapp", "testapp-b", "./b.json").expect("HOME is set");
        assert_ne!(a.lock_file(), b.lock_file());
        assert_ne!(a.log_file(), b.log_file());
        assert_eq!(a.log_dir(), b.log_dir());
    }

    #[test]
    fn the_display_name_is_what_was_typed() {
        let instance = Instance::named("testapp", "testapp-a", "./a.json").expect("HOME is set");
        assert_eq!(instance.display_name(), "./a.json");
    }
}
