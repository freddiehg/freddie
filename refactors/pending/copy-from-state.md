# copy reads state only (mercury)

The copy bindings copy what the model holds and nothing else. With no reported URL there is nothing to copy, so the key does nothing — the same honest gap as every other unreported mirror. `Copied::FrontTabUrl`, the osascript fallback that asked Chrome at use time, is deleted, and with it mercury's last on-demand read of the outside world. figaro's half is `figaro/refactors/past/copy-from-state.md`, already landed; this is its mercury twin.

## The effect collapses

That leaves `Copied` one variant, `Text(String)`, so the enum is deleted and the effect carries the string. `crates/mercury_model/src/effect.rs`, before:

```rust
/// The text a copy puts on the clipboard, and where it comes from.
#[cfg_attr(feature = "testing", derive(PartialEq, Eq))]
#[derive(Debug)]
pub enum Copied {
    /// Text mercury already holds. The extension reports the front tab's URL as it changes, so
    /// this is the usual case for a copy, and it costs a string.
    Text(String),
    /// The front Chrome tab's URL, read back out of Chrome, and the part of it to keep.
    ///
    /// The fallback for when nothing reported one: no extension connected, or a page it never
    /// sees. It asks the app rather than the model, so it is a subprocess and an Apple Events
    /// permission, which is why it is not the way this normally works.
    FrontTabUrl(UrlPart),
}
```

after:

```rust
    /// Put text on the clipboard, replacing what is there.
    Copy(String),
```

`UrlPart` stays: it is the handlers' vocabulary for whole-versus-host, and they resolve it against state before the effect exists. `Copied` leaves `mercury_model`'s `lib.rs` and `mercury`'s `daemon.rs` import lists.

## The performer stops asking

`crates/mercury/src/daemon.rs`: `front_tab_url` and its osascript are deleted; `copy` takes the text. Before:

```rust
/// Put text on the clipboard, fire-and-forget on its own thread like the rest: `arboard` talks to
/// `NSPasteboard`, and [`Copied::FrontTabUrl`] runs `osascript`, neither of which the effect loop
/// should wait on.
///
/// The pasteboard keeps what it is handed, so the `Clipboard` going out of scope at the end of the
/// thread does not take the text with it.
fn copy(what: Copied) {
    std::thread::spawn(move || {
        let Some(text) = (match what {
            Copied::Text(text) => Some(text),
            Copied::FrontTabUrl(part) => front_tab_url(part),
        }) else {
            return;
        };
        match arboard::Clipboard::new().and_then(|mut board| board.set_text(text.clone())) {
            Ok(()) => debug!(%text, "copied"),
            Err(e) => warn!(%text, error = %e, "copy failed"),
        }
    });
}
```

after:

```rust
/// Put text on the clipboard, fire-and-forget on its own thread like the rest: `arboard` talks
/// to `NSPasteboard`, which the effect loop should not wait on.
///
/// The pasteboard keeps what it is handed, so the `Clipboard` going out of scope at the end of the
/// thread does not take the text with it.
fn copy(text: String) {
    std::thread::spawn(move || {
        match arboard::Clipboard::new().and_then(|mut board| board.set_text(text.clone())) {
            Ok(()) => debug!(%text, "copied"),
            Err(e) => warn!(%text, error = %e, "copy failed"),
        }
    });
}
```

The `perform_effect` arm respells to match: `MercuryEffect::Copy(text) => copy(text),`.

## The handler copies nothing it does not hold

`crates/mercury_model/src/handlers/app.rs`, `copy`'s fallback arm. The doc comment's fallback paragraph becomes one sentence — without a reported URL there is nothing to copy, and the key does nothing until the extension reports — and the body, before:

```rust
    else {
        return vec![MercuryEffect::Copy(Copied::FrontTabUrl(part))];
    };
```

after:

```rust
    else {
        return Vec::new();
    };
```

and the `Copied::Text(...)` construction at the bottom becomes:

```rust
    text.map(|text| MercuryEffect::Copy(text.to_owned()))
        .into_iter()
        .collect()
```

## AGENTS.md

The synced-state section's exception sentence cites `Copied::FrontTabUrl` as the named exception. With it gone mercury has no on-demand read at all, and the sentence tightens:

```
On-demand asking is reserved for a source with no observation channel at all, and each such read is a named exception, not a pattern to extend. None currently exist.
```

## Tests

In `crates/mercury_model/tests/transitions.rs`, the `copies` helper respells:

```rust
fn copies(text: &str) -> MercuryEffect {
    MercuryEffect::Copy(text.to_owned())
}
```

The claude.ai copy table and `copying_the_host_of_a_hostless_url_copies_nothing` pass unchanged through it. `a_copy_with_no_reported_url_asks_chrome` becomes the deletion's own test; the keys are spent in the in-app layer, so the empty answer still carries the return-home timer:

```rust
// With no URL reported there is nothing to copy out of the state, so the key does nothing:
// mercury copies what it holds, and it holds nothing until the extension reports.
#[test]
fn a_copy_with_no_reported_url_copies_nothing() {
    let mut m = home();
    let _ = m.handle(&foreground(App::Chrome, Pid(7)));
    let _ = m.handle(&key(Key::KeyI));
    assert_eq!(
        m.handle(&key_with(Key::KeyL, ModifierFlags::SHIFT)),
        in_app(vec![])
    );
    assert_eq!(
        m.handle(&key_with(Key::KeyL, ModifierFlags::COMMAND)),
        in_app(vec![])
    );
}
```

## Order of changes

One change: the effect collapse, the performer, the handler arm, the AGENTS.md sentence, and the tests land together.
