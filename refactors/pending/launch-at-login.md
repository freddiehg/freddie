# launching mercury at login

Running mercury as a real daemon that starts with the machine and stays up, rather than something you `cargo run` and babysit for thirty seconds.

The plist is the easy part. Two things in mercury make autostart actively dangerous today, and they have nothing to do with launchd.

## What has to change in mercury first

The keyboard is dead at boot. `Mercury::default()` starts in `Layer::Home`, which binds `n`, `t`, `i`, and `q` and nothing else. Every other key is swallowed. Start that at login and the machine appears to have a broken keyboard until you happen to press `t`. Autostart needs a decision about the boot state: start in typing (everything passes through), or add an explicit off state that the model boots into, or make home pass unbound keys through instead of eating them. This is a model question, not a packaging one, and it blocks everything else here.

The killswitch kills it. `spawn_killswitch` sends `Kill` after 30 seconds and hard-exits 5 seconds later. That is a development safety net for a program that swallows the keyboard, and it is the right default while iterating. It cannot ship. It needs to be opt-in, behind a flag or an environment variable, and off by default when launched by launchd.

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

Two things to verify rather than assume. Whether a grant survives re-signing the same identity over new bits. And whether a TCC prompt raised by a launchd agent, which has no foreground UI, is usable, or whether the first grant has to be done by launching mercury from a terminal once.

`tccutil reset Accessibility` and `tccutil reset ListenEvent` reset the grants when testing this, which will be necessary because the failure mode is silent.

## Environment under launchd

An agent does not inherit your shell. It gets a minimal environment, with `PATH` set to something like `/usr/bin:/bin:/usr/sbin:/sbin`. mercury shells out to `open` and `osascript`, both of which live in `/usr/bin`, so they resolve. Worth knowing that this is luck rather than design, and that any future subprocess in `/usr/local/bin` or a homebrew path will not be found.

`HOME` is set for agents, which `logging::log_dir` depends on. If it were not, logs would land in launchd's working directory.

## Single instance

launchd owns this. One `Label` is one job, and `bootstrap` on an already-loaded label fails. That supersedes the pid file in freddie-cli-plan.md, which was designed for `mercury start` run by hand. If mercury is ever launched both ways at once, two processes both install event taps, and the behavior of two taps swallowing the same keys is not something we want to find out. The CLI's `start` should refuse when the agent is loaded, or the pid file should stay as the guard that covers both paths.

## Getting your keyboard back

This is a program whose failure mode is that you cannot type. Autostarting it means a bug that makes it hang or swallow everything is a bug you cannot fix with the keyboard, on a machine that reproduces it on every login.

What exists today: the tap is on its own thread, so a wedged worker still leaves `Interceptor::drop` able to release the grab, and macOS itself disables a tap whose callback stops answering. Neither helps if mercury is running correctly and merely swallowing keys by design.

What is needed before this ships, roughly in order of how much they would save you: a way to disable the agent without the keyboard (ssh from another machine and `launchctl bootout`, which is why the escape hatch above matters); a modifier held at launch that makes mercury start in a passthrough state; and possibly a watchdog that boots the agent out if mercury has not reported healthy within some seconds.

## Bundle or bare binary

A bare executable at `/usr/local/bin/mercury` is the smaller thing, and it works for a keyboard remapper with no UI.

Once menu-bar.md lands and mercury owns an `NSStatusItem`, an app bundle starts looking necessary rather than decorative, because AppKit may want `NSApplication` and a bundle identity. At that point `SMAppService.agent(plistName:)` on macOS 13 and later registers a LaunchAgent embedded in the bundle, which is the modern path and avoids hand-installing a plist into `~/Library/LaunchAgents`. That is a decision to make with the menu bar, not before it.

## Open questions

- What layer does mercury boot into? Nothing here can ship until that is answered.
- Does the tap need Input Monitoring, Accessibility, or both?
- Does a TCC grant survive rebuilding and re-signing with the same certificate?
- Can a launchd agent raise a usable TCC prompt, or must the first grant come from a terminal launch?
- Does the killswitch become a `--dev` flag, an env var, or a debug-assertions-only feature?
- Does the CLI's pid file survive alongside launchd, or does launchd become the only supported way to run it?
- Does mercury need a health signal for a watchdog, and what would count as healthy?
