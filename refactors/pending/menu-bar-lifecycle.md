# the menu bar can stop and restart mercury

The status item has one entry, Quit, which sends the model the same quit the keyboard's `q` does. It gains a second, Restart, and both gain a forcible variant under Option, so the mouse can do everything `mercury stop` and `mercury restart` can.

This is the mouse-reachable half of the verbs `refactors/past/mercury-stop.md` and `refactors/past/mercury-start.md` landed, and it matters most in the case those verbs are worst at: a daemon that has swallowed the keyboard is one you cannot type `mercury stop` into.

## Option, not Command

Option-click is what macOS already means by "the forcible version of this item": the Dock turns Quit into Force Quit under Option, and the Apple menu does the same. Command-click has no established meaning on a status item.

The modifier is read when the item is chosen rather than shown as a separate alternate item. `muda`'s `MenuEvent` carries the item's id and nothing else, so the handler asks `NSEvent` what is held.

Verified on the pinned 1.96.0: `NSEvent::modifierFlags_class()` is declared `pub fn`, not `pub unsafe fn`, in `objc2-app-kit-0.3.2/src/generated/NSEvent.rs`, and a binary with `#![forbid(unsafe_code)]` calls it and reads the flags. So `freddie_menu_bar` keeps the workspace's `forbid` and gains one dependency it does not have today.

## The menu bar reports what was chosen

`freddie_menu_bar` learns two items and one modifier, and decides nothing about what either means.

`crates/freddie_menu_bar/Cargo.toml`:

```diff
 [dependencies]
 tray-icon = "0.24"
+objc2-app-kit = { version = "0.3", features = ["NSEvent", "NSResponder"] }
 image = { version = "0.25", default-features = false, features = ["png"] }
```

```rust
/// What the user chose from the status item.
///
/// The menu reports the choice; what stopping or restarting means is the app's, which is why this
/// carries no effects and no events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chosen {
    /// Quit, plainly clicked: end mercury the way its own model would.
    Stop,
    /// Quit, Option-clicked: end it now, whatever state it is in.
    StopForcibly,
    /// Restart, plainly clicked.
    Restart,
    /// Restart, Option-clicked.
    RestartForcibly,
}

/// Whether the forcible variant of an item was asked for.
///
/// Option-click, which is what the Dock means by Force Quit. Read when the item is chosen, because
/// `muda` delivers an id and nothing else.
fn option_held() -> bool {
    NSEvent::modifierFlags_class().contains(NSEventModifierFlags::Option)
}
```

`show`'s signature changes from one `Fn()` to one `Fn(Chosen)`:

```rust
pub fn show(
    tooltip: &str,
    icon_png: &[u8],
    on_choice: impl Fn(Chosen) + Send + Sync + 'static,
) -> Result<MenuBar, Box<dyn std::error::Error + Send + Sync>> {
    let restart = MenuItem::new("Restart", true, None);
    let quit = MenuItem::new("Quit", true, None);
    let (restart_id, quit_id) = (restart.id().clone(), quit.id().clone());

    let menu = Menu::new();
    menu.append(&restart)?;
    menu.append(&quit)?;

    // ... the builder, unchanged ...

    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let forcibly = option_held();
        let chosen = if event.id == quit_id {
            if forcibly { Chosen::StopForcibly } else { Chosen::Stop }
        } else if event.id == restart_id {
            if forcibly { Chosen::RestartForcibly } else { Chosen::Restart }
        } else {
            return;
        };
        on_choice(chosen);
    }));

    Ok(MenuBar { tray })
}
```

The module doc's "a single Quit entry" becomes the two.

## What each choice does

Stopping is what the menu already does: send the model its quit event, which opens the modifiers a command layer swallowed, pushes `Kill`, and lets the `Interceptor` release the keyboard on the way out.

The other three cannot go through the model.

`StopForcibly` exists for a daemon whose worker thread is blocked inside a synchronous effect. The menu still works then, because the item is tracked on the main thread and the block is on the worker — but the event channel it would send to is exactly what nothing is draining. So it exits the process from the main thread instead. That runs no `Drop` impls, which is the cost `Signal::Kill` documents: the keyboard comes back because the tap dies with the process, and a swallowed modifier stays down.

Restarting cannot happen inside the daemon at all. Spawning the replacement first fails, because the lock is still held by the process doing the spawning; dying first leaves nobody to spawn. So the daemon shells out to a client and lets that client outlive it:

```rust
/// Replace this daemon by handing the job to a client that outlives it.
///
/// `mercury restart` stops the daemon that spawned it and starts a fresh one. This process cannot
/// do that itself: it holds the lock the replacement needs, so it must be gone before the new one
/// starts, and it cannot start anything once it is gone.
///
/// `process_group(0)` and null stdio for the same reasons `start` uses them: the child outlives
/// this process and has no terminal to write to.
fn spawn_restart(forcibly: bool) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;

    let mut command = Command::new(std::env::current_exe()?);
    command.arg("restart");
    if forcibly {
        command.arg("--force");
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    Ok(())
}
```

The daemon does not wait for it. The client SIGTERMs this process, waits for the lock, and starts the replacement; this process's part is to die when the signal arrives, which the SIGTERM handler already does.

`RestartForcibly` passes `--force`, so the client sends SIGKILL instead. That is the same trade as `StopForcibly`, with a replacement afterwards.

## In mercury

`daemon.rs`, where the menu bar is created:

```rust
    let menu_bar = freddie_menu_bar::show("Mercury", include_bytes!("../assets/mercury.png"), {
        let event_tx = event_tx.clone();
        move |chosen| match chosen {
            // Through the model, so the modifiers a command layer swallowed are reopened.
            Chosen::Stop => {
                let _ = event_tx.send(quit_event());
            }
            // The model is not answering, which is why this was chosen. Exit from this thread,
            // which is the one still running.
            Chosen::StopForcibly => {
                warn!("menu bar: forcible stop; no destructors will run");
                std::process::exit(0);
            }
            Chosen::Restart | Chosen::RestartForcibly => {
                let forcibly = chosen == Chosen::RestartForcibly;
                if let Err(e) = spawn_restart(forcibly) {
                    error!(error = %e, "could not spawn the restart");
                }
            }
        }
    });
```

`spawn_restart` lives in `daemon.rs` rather than `client.rs`: it is the daemon starting a client, not a client verb.

## What launchd changes

The agent from `launch-at-login.md` exists and works, so this is not deferrable: on a machine where mercury runs at login, the menu's Restart as described above is wrong. The spawned `mercury restart` would stop launchd's daemon and start one launchd did not start and will not keep alive, leaving the job down until the next login.

The fix belongs in `mercury restart` rather than in the menu, so the menu, `bacon restart`, and a hand-typed restart all behave the same. `restart` asks whether the agent names the daemon it is about to replace, and hands the job back to launchd when it does:

```rust
/// Whether the running daemon is one launchd is managing on our behalf.
///
/// The plist exists and its program is the binary this daemon is running. `PPID` cannot answer it:
/// `mercury start` also reparents to 1, so a hand-started daemon looks identical.
fn managed_by_launchd(daemon: &Path) -> bool { .. }
```

When it is, `restart` runs `launchctl kickstart -k gui/<uid>/<label>` instead of stopping and spawning, which replaces the process launchd is managing and leaves the job loaded. When it is not, it does what it does today.

That is a change to `client.rs`, not to `freddie_menu_bar`, and it wants its own doc: it needs the label and plist path this doc does not otherwise touch, and it changes a verb that has already shipped.

## Tests

`freddie_menu_bar` has no tests today, and the parts worth testing here are the parts that are not AppKit:

```rust
#[cfg(test)]
mod tests {
    use super::Chosen;

    // Option is what distinguishes them, so the four are two items times one modifier.
    #[test]
    fn forcible_and_plain_are_different_choices() {
        assert_ne!(Chosen::Stop, Chosen::StopForcibly);
        assert_ne!(Chosen::Restart, Chosen::RestartForcibly);
    }
}
```

The rest is verified by hand, because a status item needs a window server and a menu needs a click.

## Verifying

- The status item shows Restart above Quit.
- Quit ends mercury: the log gets `kill: exiting`, and a modifier held as it lands is not left down in the app underneath.
- Option-Quit ends mercury with no `kill: exiting` line, which is the cost of the forcible path.
- Restart ends this daemon and leaves a different pid running, by `mercury status` afterwards. The log shows one client's records between the two daemons' pid stamps.
- Option-Restart does the same with no `kill: exiting` from the old one.
- Restart while the worker is blocked still works, because the spawned client signals from outside.
