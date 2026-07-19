# take the crate instead, where the crate does what we need

Several pieces of the daemon lifecycle were written by hand because they were small and because the properties they need were discovered while writing them. Some of those properties are unusual, and a crate that does not have them cannot replace ours. This is the audit: for each piece, what would have to be true of a dependency for it to win.

Two of the four are decided against, from reading the crates' APIs. Knowing a dependency does not fit is worth as much as taking one: it is the difference between code written because nothing else does this and code written because nobody looked.

`plist` is already the outcome of this exercise done once: `mercury install` serializes a struct rather than substituting into XML, because the crate escapes what hand-written substitution does not.

## `freddie_single_instance`

The candidate is `single-instance`, and the bar is high, because three properties this crate has were each found by breaking without them.

- **A probe that does not acquire.** `mercury status` asks who holds the lock without becoming the holder. A crate offering only "acquire, or fail" cannot answer it.
- **A probe that takes a *shared* lock.** Two probes must not refuse each other. An exclusive probe makes one caller read the other's refusal, then read a pid left behind by a process that has since died, and report it as running. `probes_do_not_refuse_each_other` is the regression test; it fails against an exclusive probe.
- **A pid that is an address, never evidence.** The pid is read only on the branch where the lock was refused, so a dead process's pid is never reported. A crate that writes a pid file and reads it to decide liveness has the failure this design exists to avoid.

Plus `await_free`, the blocking wait `mercury stop` uses to know the daemon is gone, which is edge-triggered because flock reports a release.

**Decided against.** `single-instance` v0.3.3 is `SingleInstance::new()` and `is_single()`: acquire, and ask whether you got it. There is no probe that does not acquire and no pid, so `status`, `stop`, `restart`, and `await_free` all still have to be written. It would replace `acquire` alone, which is the one part that was never hard.

## Detaching a spawned daemon

The candidate is `daemonize`, and the bar is low, because what we do is small: `current_exe`, `process_group(0)`, null stdio, and no `unsafe`.

The question is whether the crate double-forks. Classic daemonization detaches from the controlling terminal and becomes a session leader, which is more than a launchd agent or a `mercury start` needs, and `setsid` is an unsafe call this workspace forbids. If the crate is unsafe internally that is fine — the workspace forbids `unsafe` in our crates, not in dependencies — but a session leader is a different process shape from the one `mercury logs` and Ctrl-C were designed around.

Still open, and the cheapest of the four to settle: `daemonize` v0.5.0's rustdoc does not say whether `setsid` is mandatory, so this needs a look at its source. Low stakes either way — our detach is four lines and has no `unsafe`.

## Installing the launch agent

The candidate is `service-manager`, which covers launchd, systemd, Windows services, and rc.d behind one interface, and is the closest thing to what `mercury install` does by hand.

**Decided against, for now.** `service-manager` v0.11.0 takes a `ServiceInstallCtx` of `label`, `program`, `args`, `username`, `working_directory`, `environment`, `autostart`, and `restart_policy`, and its launchd `keep_alive` is a plain `bool`. `SuccessfulExit=false` cannot be said, which is the key mercury's exit-code contract rests on, and neither can `LimitLoadToSessionType`, `RunAtLoad`, or `ThrottleInterval`.

It does expose `contents: Option<String>` for a raw plist, and `ServiceLevel` distinguishes a user agent from a system daemon. So the shape that would work is: serialize our own `Agent` to a string, hand it over as `contents`, and let the crate own `install`, `uninstall`, `start`, and `stop` across platforms.

That is worth taking the day freddie targets anything but macOS, and not before: today it would add a dependency whose portable half we cannot use and whose escape hatch hands the plist back to us.

## Following the log

`mercury logs` shells out to `/usr/bin/tail -F`. The candidate is `notify`, which watches the filesystem properly rather than polling a file.

`tail -F` is doing two things worth keeping: it waits for a file that does not exist yet, which is a machine that has never run mercury, and it reopens by name when the file is replaced. A replacement built on `notify` has to do both, plus the truncation case, which is where hand-rolled followers usually break.

The reason to consider it at all is that shelling out costs a process and inherits its stdio, which is what leaked `Boot-out failed` in `mercury install` before that was captured. `logs` already pipes tail's stdout for filtering, so only stderr is still inherited.

Low priority: the subprocess works, and the failure modes of a hand-rolled follower are worse than the cost of a `tail`.

## What is already reused

`tokio::signal` for SIGTERM, `clap` for the command line, `tracing` and `tracing-subscriber` for everything said, `tracing-appender` for the log file, `plist` for the agent. The hand-rolled set is smaller than it looks.

## The finding

Two of the three obvious candidates do less than their names suggest, and the gap is the same in both cases: they model "is one running" and "install a service", while mercury needs "who is running, without becoming it" and "install *this* service with the exit-code policy it depends on". That is worth knowing in itself — the hand-rolled code is here because the alternatives do not fit, not because nobody checked.

## What cannot be a dependency

Stopping means routing a quit through this app's own model, so the modifiers a command layer swallowed are reopened before the process may go. No crate can do that; the most it can offer is a callback, which is what `freddie-daemon-runtime.md` reduces it to with `From<Stop>`.
