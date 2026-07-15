# launching mercury at login

Running mercury as a real daemon that starts with the machine and stays up, rather than something you `cargo run` and babysit for thirty seconds.

The plist is the easy part. Two things in mercury make autostart actively dangerous today, and they have nothing to do with launchd.

## What has to change in mercury first

- start in typing mode

The keyboard is dead at boot. `Mercury::default()` starts Unpaused in `Layer::Home`, which binds only `n`, `r`, `t`, `i`, `p`, `q` (plus `escape`). Every other key is swallowed. Start that at login and the machine appears to have a broken keyboard until you happen to press one of those. Autostart needs a decision about the boot state: start in typing (everything passes through), or add an explicit off state that the model boots into, or make home pass unbound keys through instead of eating them. This is a model question, not a packaging one, and it blocks everything else here.

Update: the off state now exists, so most of the model-side work is done. `Power::Paused` holds the layer without descending into it, and its active node binds `AnyKey => pass_through`, so every key flows through untouched. That is the login-safe boot state, and re-enabling is already wired three ways: the `cmd-alt-p` chord, the menu-bar Toggle, and `p` from Home. So the boot-state answer is to default the launchd build to `Power::Paused` rather than Unpaused + Home. Paused beats booting into typing, because typing is still a layer you can `escape` out of into Home, which drops you back into the dead-keyboard state, whereas Paused is the deliberate off switch. See `enable-disable.md`.

- killswitch: already removed

Resolved, and no longer a blocker. The dev killswitch is gone: `spawn_killswitch` was deleted in commit 6aa9836 ("Remove the 60s dev killswitch"). Current mercury runs no timer at all. The only exits are `MercuryEffect::Kill`, raised by `q` in Home or the menu-bar Quit, both deliberate. So nothing auto-terminates mercury, and there is nothing to gate behind a dev flag. (The doc's earlier "30 seconds" was also wrong; the real timer was 60s before it was removed. The menu-bar Quit, which is mouse-reachable and does not need the grabbed keyboard, is what let the timed net go.)

## Agent, not daemon

mercury must be a LaunchAgent (`~/Library/LaunchAgents`), not a LaunchDaemon (`/Library/LaunchDaemons`). Daemons run as root in the system context with no window server connection and no GUI session. A session `CGEventTap` needs that connection, `NSWorkspace` is per-user, and the TCC permissions mercury depends on are granted per-user. All of that exists only in an Aqua session, which is what agents get and daemons do not.

That also means mercury starts after login, not at boot, which is what we want. Nothing should be swallowing the keyboard at the login window.

## The plist

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>                  <string>hg.freddie.mercury</string>
    <key>ProgramArguments</key>       <array><string>/usr/local/bin/mercury</string></array>
    <key>RunAtLoad</key>              <true/>
    <key>LimitLoadToSessionType</key> <string>Aqua</string>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>     <false/>
    </dict>
    <key>ThrottleInterval</key>       <integer>10</integer>
    <key>EnvironmentVariables</key>
    <dict>
        <key>LOG_LEVEL</key>          <string>info</string>
    </dict>
</dict>
</plist>
```

`KeepAlive` as a dict with `SuccessfulExit` false is the part worth getting right. Plain `KeepAlive` true would restart mercury after you quit it with `q`, since `Kill` exits zero. What we want is restart on a crash, stay dead on a deliberate quit. `ThrottleInterval` keeps a crash loop from respawning a keyboard-swallowing process ten times a second.

Loading and unloading:

```
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/hg.freddie.mercury.plist
launchctl kickstart -k gui/$(id -u)/hg.freddie.mercury   # restart after a rebuild
launchctl bootout gui/$(id -u)/hg.freddie.mercury        # stop it, and this is the escape hatch
```

No `StandardOutPath` or `StandardErrorPath` is needed. mercury already writes `~/Library/Logs/mercury/mercury.log` itself, and that file is the record of a run.

## Permissions are the real work

mercury needs Accessibility, and probably Input Monitoring as well (its own error message claims both; worth confirming which the tap actually requires). Those are TCC grants, and TCC keys a grant to the binary's identity, not merely its path. For an unsigned or ad-hoc-signed binary that identity includes the code hash, so every `cargo build` produces a different binary and the grant does not follow it. This is the thing that will waste an afternoon.

So autostart wants a stable, signed binary at a stable path, granted once. Concretely: build in release, copy to a fixed location, sign it with a stable identity, and grant Accessibility to that. `codesign --force --sign - ` (ad-hoc) still rehashes on every build, so a self-signed certificate in the keychain, or a Developer ID if one exists, is what gives the grant something durable to attach to.

- can we have a wrapper that is stable that executes cargo run?

No, not a wrapper that execs or spawns the build. TCC attributes an Accessibility grant to the process that actually calls the tap/AX API, evaluated by its code identity. A wrapper that `exec`s the freshly-built binary replaces its own image, so the process installing the tap is the rebuilt binary with a new code hash each build, and the grant on the wrapper does not apply. Spawning the build as a child is no better: the child is what TCC evaluates, and `AXIsProcessTrusted` is asked of the child, not the parent. `cargo run` is the worst case, since the binary under `target/debug` is rehashed on every build even though its path is stable. The grant has to live on whoever installs the tap, and today that is mercury itself (`freddie_keyboard::intercept`), so mercury is the thing that needs a stable signed identity.

The one wrapper shape that does work is not exec/spawn but a process split: a stable, signed, long-lived binary that owns the tap and holds the grant, talking over IPC to a frequently-rebuilt child that holds the model and never touches the tap or AX APIs. Then the rebuilt half has no TCC dependency and the granted half never changes. That is a real architecture change, not a shell wrapper, and it is only worth it if re-granting during dev is painful enough to justify it. The simpler answer is to sign the mercury binary with a stable self-signed cert and grant that once.

Two things to verify rather than assume. Whether a grant survives re-signing the same identity over new bits. And whether a TCC prompt raised by a launchd agent, which has no foreground UI, is usable, or whether the first grant has to be done by launching mercury from a terminal once.

`tccutil reset Accessibility` and `tccutil reset ListenEvent` reset the grants when testing this, which will be necessary because the failure mode is silent.

## Environment under launchd

An agent does not inherit your shell. It gets a minimal environment, with `PATH` set to something like `/usr/bin:/bin:/usr/sbin:/sbin`. mercury shells out to `open` as a bare command name, so it is `PATH`-resolved, and `open` lives in `/usr/bin`, so it resolves. (osascript is no longer shelled out; that poll was replaced by the `NSWorkspace` observer, so only `open` matters now.) Worth knowing that this is luck rather than design, and that any future subprocess in `/usr/local/bin` or a homebrew path will not be found.

`HOME` is set for agents, which `logging::log_dir` depends on. If it were not, logs would land in launchd's working directory.

## Single instance

launchd owns this. One `Label` is one job, and `bootstrap` on an already-loaded label fails. That supersedes the pid file in freddie-cli-plan.md, which was designed for `mercury start` run by hand. If mercury is ever launched both ways at once, two processes both install event taps, and the behavior of two taps swallowing the same keys is not something we want to find out. The CLI's `start` should refuse when the agent is loaded, or the pid file should stay as the guard that covers both paths.

## Getting your keyboard back

This is a program whose failure mode is that you cannot type. Autostarting it means a bug that makes it hang or swallow everything is a bug you cannot fix with the keyboard, on a machine that reproduces it on every login.

What exists today: the tap is on its own thread, so a wedged worker still leaves `Interceptor::drop` able to release the grab, and macOS itself disables a tap whose callback stops answering. Neither helps if mercury is running correctly and merely swallowing keys by design.

- can we disable it if booted in safe mode or whatever it's called

Safe Mode already does this for free: macOS does not load third-party launchd agents in Safe Mode (hold Shift at boot), only Apple's own. So a Safe Mode boot comes up with mercury not running and a working keyboard, and you can remove or `bootout` the plist from there. mercury needs no safe-mode detection of its own, because in Safe Mode it is never launched. (Worth a one-line confirmation: boot Safe Mode, check that `launchctl print gui/$(id -u)` does not list the label.)

Safe Mode is the heavy recovery, though. The lighter one, which does need code, is a modifier held at launch that boots mercury into `Power::Paused` instead of the active default. `main.rs` already overrides part of the default state at startup (it sets the real frontmost app), so reading modifier state there and choosing `Power::Paused` when a chord is held is the same shape. That recovers a normal keyboard on the next login without a reboot, and Paused's `cmd-alt-p` turns mercury back on once you have confirmed it is behaving.

What is needed before this ships, roughly in order of how much they would save you: a way to disable the agent without the keyboard (ssh from another machine and `launchctl bootout`, which is why the escape hatch above matters); a modifier held at launch that makes mercury start in a passthrough state; and possibly a watchdog that boots the agent out if mercury has not reported healthy within some seconds.

## Bundle or bare binary

A bare executable at `/usr/local/bin/mercury` is the smaller thing, and it works for a keyboard remapper with no UI.

Once menu-bar.md lands and mercury owns an `NSStatusItem`, an app bundle starts looking necessary rather than decorative, because AppKit may want `NSApplication` and a bundle identity. At that point `SMAppService.agent(plistName:)` on macOS 13 and later registers a LaunchAgent embedded in the bundle, which is the modern path and avoids hand-installing a plist into `~/Library/LaunchAgents`. That is a decision to make with the menu bar, not before it.

## Open questions

- What layer does mercury boot into? Answered above: default the launchd build to `Power::Paused`, the existing off/passthrough state, not a layer. Remaining work is making the call and changing the default; nothing is missing from the model.
- Does the tap need Input Monitoring, Accessibility, or both?
- Does a TCC grant survive rebuilding and re-signing with the same certificate?
- Can a launchd agent raise a usable TCC prompt, or must the first grant come from a terminal launch?
- (Resolved) The killswitch was removed in 6aa9836, so there is nothing to gate; the menu-bar Quit is the replacement.
- Does the CLI's pid file survive alongside launchd, or does launchd become the only supported way to run it?
- Does mercury need a health signal for a watchdog, and what would count as healthy?
