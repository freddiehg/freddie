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
    <key>ProgramArguments</key>       <array><string>/usr/local/bin/mercury</string></array>
    <key>RunAtLoad</key>              <true/>
    <key>LimitLoadToSessionType</key> <string>Aqua</string>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>     <false/>
    </dict>
    <key>ThrottleInterval</key>       <integer>10</integer>
    <key>EnvironmentVariables</key>
    <dict><key>LOG_LEVEL</key>        <string>info</string></dict>
</dict>
</plist>
```

`KeepAlive`/`SuccessfulExit=false` restarts on a crash but stays dead after a deliberate `q` or menu-bar Quit (both exit zero). `ThrottleInterval` stops a crash loop from respawning a keyboard-eater ten times a second. No `StandardOutPath`; mercury writes its own log. `HOME` is set for agents, which `logging::log_dir` needs; `PATH` is minimal but `open` (the only subprocess) is in `/usr/bin`.

```
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/hg.freddie.mercury.plist
launchctl kickstart -k gui/$(id -u)/hg.freddie.mercury   # restart after a rebuild
launchctl bootout   gui/$(id -u)/hg.freddie.mercury      # stop it (the escape hatch)
```

## Permissions: a stable signed binary

mercury needs Accessibility (confirm whether the tap also needs Input Monitoring). TCC keys the grant to the binary's code identity, not its path, so every `cargo build` produces bits the grant does not follow. Ship one stable, signed binary at a fixed path, granted once: build release, copy to `/usr/local/bin/mercury`, sign with a stable self-signed cert in the keychain. Ad-hoc `--sign -` still rehashes every build, so it does not count as stable.

No wrapper that `exec`s or spawns `cargo run` fixes this: TCC evaluates whoever actually calls the tap (the rebuilt child), not the wrapper, and `cargo run` rehashes `target/debug` on every build. The only wrapper that works is a process split, a stable signed binary owning the tap over IPC to a frequently-rebuilt model child, which is a real architecture change and not worth it against signing mercury once.

Reset grants while testing (the failure is silent): `tccutil reset Accessibility` and `tccutil reset ListenEvent`.

## Getting the keyboard back

The failure mode is that you cannot type, reproduced on every login. Three ways out, cheapest first:

- Safe Mode (hold Shift at boot) does not load third-party agents, so it comes up with a working keyboard; `bootout` the plist from there. Free, no code.
- A modifier held at launch makes mercury skip the grab entirely (native keyboard). Same shape as the frontmost-app seed: read modifier state in `main.rs` before `intercept`, and don't install the tap if a chord is held.
- ssh from another machine and `launchctl bootout gui/$(id -u)/hg.freddie.mercury`.

## Single instance

launchd owns the label; a second `bootstrap` fails. This supersedes the CLI pid file (`freddie-cli-plan.md`): `mercury start` by hand must refuse when the agent is loaded, or two taps fight over the keyboard.

## Bundle vs bare binary

A bare binary at `/usr/local/bin/mercury` is enough for a headless remapper. Once the menu bar owns an `NSStatusItem`, an app bundle with `SMAppService.agent(plistName:)` (macOS 13+) becomes the cleaner path: it registers the agent from the bundle instead of a hand-installed plist. Decide it with the menu bar, not before.

## Open

- Which TCC permission the tap needs: Accessibility, Input Monitoring, or both.
- Whether a grant survives re-signing the same cert over new bits, and whether a launchd agent can raise a usable TCC prompt or the first grant must come from a terminal launch.
- The `escape`-out-of-typing-into-Home hole: whether the launch build also needs Home to pass unbound keys through, or boot-into-typing plus the recovery paths is enough.
