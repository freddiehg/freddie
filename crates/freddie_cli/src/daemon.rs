//! The hidden `daemon` verb: be the daemon, in this process.

use tracing::{error, info};

use crate::verb::DaemonVerbArgs;
use crate::{App, Instance};

/// Run the app's daemon in the foreground: take the lock, and hand over.
pub(crate) fn run_in_foreground<TApp: App>(
    instance: &Instance,
    args: &DaemonVerbArgs<TApp::Id, TApp::DaemonArgs>,
) {
    info!(path = %instance.log_file().display(), "logging");

    // Before anything that touches the machine, because two of one instance fight over whatever
    // there is only one of. The binding must outlive the call (`let _held`, never `let _`):
    // dropping it releases the lock.
    let _held = match freddie_single_instance::acquire_at(instance.lock_file()) {
        Ok(held) => held,
        Err(e) => {
            error!(daemon = instance.display_name(), error = %e, "already running; `stop` ends it");
            return;
        }
    };

    TApp::run_daemon(&args.id, &args.app);
}
