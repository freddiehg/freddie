# launching mercury at login

Run mercury as a per-user LaunchAgent that starts with the session and restarts on a crash, without ever leaving the keyboard dead. The plist is easy; the model's boot state and the TCC grant are the work.

## Boot into typing

`Mercury::default()` boots into `Layer::Typing`, the passthrough layer: it binds only `escape` and passes everything else through, so the keyboard is normal at login. Nothing extra is needed; the launchd build just runs the default. (Home, the command layer, swallows every key it does not bind, which is why it cannot be the boot state.)

There is no dedicated off state; typing IS the login-safe boot state.

The hole: `escape` in typing drops to Home, the dead-keyboard state. So the recovery paths below are load-bearing, not nice-to-haves.

## LaunchAgent, not daemon

`~/Library/LaunchAgents`, `LimitLoadToSessionType = Aqua`. A daemon runs as root with no window server, no `NSWorkspace`, and no per-user TCC, none of which the session `CGEventTap` works without. An agent also starts after login rather than at the login window, which is what we want.

## The plist

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>                  <string>hg.freddie.mercury</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/mercury</string>
        <string>daemon</string>
    </array>
    <key>RunAtLoad</key>              <true/>
    <key>LimitLoadToSessionType</key> <string>Aqua</string>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>     <false/>
    </dict>
    <key>ThrottleInterval</key>       <integer>10</integer>
</dict>
</plist>
```

`daemon` rather than the bare binary, and this is the whole reason that verb stays invocable while hidden from `--help`. Bare `mercury` spawns a detached daemon and exits, which launchd must never run: it would watch the job exit at once, and the process actually holding the keyboard would be one launchd knows nothing about and cannot restart.

`KeepAlive`/`SuccessfulExit=false` restarts on a crash but stays dead after a deliberate exit. Every deliberate way out is one: `q` from home, the menu bar's Quit, `mercury stop`, and `launchctl bootout` all reach the model's quit and exit zero, because SIGTERM is routed into the event channel (`refactors/past/mercury-stop.md`).

`ThrottleInterval` stops a crash loop from respawning a keyboard-eater ten times a second. No `StandardOutPath` and no `LOG_LEVEL`: mercury writes its own log, and `--log-level` governs a terminal a launchd job does not have. `HOME` is set for agents, which `logging::log_dir` needs; `PATH` is minimal but `/bin/kill` and `open`, the only subprocesses, are at absolute paths.

```
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/hg.freddie.mercury.plist
launchctl kickstart -k gui/$(id -u)/hg.freddie.mercury   # restart after a rebuild
launchctl bootout   gui/$(id -u)/hg.freddie.mercury      # stop it (the escape hatch)
```

`launchctl kickstart -k` rather than `mercury restart` under the agent. `restart` stops the daemon and spawns a replacement of its own, which launchd did not start and will not keep alive; the old job then looks like a clean exit and stays down. `kickstart` replaces the process launchd is managing, which is the one you want. `mercury stop` and `mercury status` are unaffected, since they only signal and read.

## Permissions: a stable signed binary

mercury needs Accessibility (confirm whether the tap also needs Input Monitoring). TCC keys the grant to the binary's code identity, not its path, so every `cargo build` produces bits the grant does not follow. Ship one stable, signed binary at a fixed path, granted once: build release, copy to `/usr/local/bin/mercury`, sign with a stable self-signed cert in the keychain. Ad-hoc `--sign -` still rehashes every build, so it does not count as stable.

No wrapper that `exec`s or spawns `cargo run` fixes this: TCC evaluates whoever actually calls the tap (the rebuilt child), not the wrapper, and `cargo run` rehashes `target/debug` on every build. The only wrapper that works is a process split, a stable signed binary owning the tap over IPC to a frequently-rebuilt model child, which is a real architecture change and not worth it against signing mercury once.

Reset grants while testing (the failure is silent): `tccutil reset Accessibility` and `tccutil reset ListenEvent`.

## Getting the keyboard back

The failure mode is that you cannot type, reproduced on every login. Three ways out, cheapest first:

- Safe Mode (hold Shift at boot) does not load third-party agents, so it comes up with a working keyboard; `bootout` the plist from there. Free, no code.
- A modifier held at launch makes mercury skip the grab entirely (native keyboard). Same shape as the frontmost-app seed: read modifier state in `daemon.rs`'s `serve` before `intercept`, and don't install the tap if a chord is held.
- ssh from another machine and `launchctl bootout gui/$(id -u)/hg.freddie.mercury`.

## Single instance

launchd owns the label, so a second `bootstrap` fails. Two mercuries are already impossible below that: `refactors/past/single-instance.md`'s lock refuses the second, whoever started it.

Nothing extra is needed for a hand-started mercury meeting the agent's. `mercury start` finds the agent's daemon holding the lock and adopts it, reporting its pid; `mercury status` names it; `mercury stop` ends it, and `KeepAlive`/`SuccessfulExit=false` leaves it stopped because that is a clean exit.

## Bundle vs bare binary

A bare binary at `/usr/local/bin/mercury` is enough for a headless remapper. Once the menu bar owns an `NSStatusItem`, an app bundle with `SMAppService.agent(plistName:)` (macOS 13+) becomes the cleaner path: it registers the agent from the bundle instead of a hand-installed plist. Decide it with the menu bar, not before.

## Open

- Which TCC permission the tap needs: Accessibility, Input Monitoring, or both.
- Whether `launchctl bootout` gives the daemon long enough to finish its quit before SIGKILL follows, since that path now has destructors to run.
- Whether a grant survives re-signing the same cert over new bits, and whether a launchd agent can raise a usable TCC prompt or the first grant must come from a terminal launch.
- The `escape`-out-of-typing-into-Home hole: whether the launch build also needs Home to pass unbound keys through, or boot-into-typing plus the recovery paths is enough.
