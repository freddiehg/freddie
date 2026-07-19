# launching mercury at login

Run mercury as a per-user LaunchAgent that starts with the session and restarts on a crash, without ever leaving the keyboard dead. The plist is easy; the model's boot state and the TCC grant are the work.

## Boot into typing

`Mercury::default()` boots into `Layer::Typing`, which binds nothing at all: every key falls to the root, runs through the `jk` sequence, and passes through. So the keyboard is normal at login, and the launchd build just runs the default. Home, the command layer, swallows every key it does not bind, which is why it cannot be the boot state.

There is no dedicated off state; typing IS the login-safe boot state.

The way from typing into Home is `jk` — `KeyJ` then `KeyK` within `JK_TIMEOUT`, 200ms. Nothing else leaves typing, and in particular `escape` does not: it is bound in Home, not in typing or at the root.

So the hazard at login is typing a literal `jk` fast enough, which lands you in the layer that swallows unbound keys. It takes two deliberate keys inside a 200ms window rather than one stray press, which is why the recovery paths below are worth having without being the only thing standing between you and a dead keyboard.

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
- The `escape`-out-of-typing-into-Home hole: whether the launch build also needs Home to pass unbound keys through, or boot-into-typing plus the recovery paths is enough.
