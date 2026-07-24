# a VM an agent can run mercury in

An agent that changes freddie wants to spin up mercury, send it keys, and watch what it does. The unit tests cover the model as a table and the crates in isolation, but running the real daemon, grab and all, is the thing that shows a change works, and there is nowhere to do it: not on the developer's own machine.

This adds no code to mercury. It is the environment around it, a VM the agent owns, and a small driver that runs the daemon in the VM and feeds it input. mercury is what it is; this is how an agent gets to exercise it.

mercury grabs the global keyboard. A daemon started on the host swallows the human's keys for as long as it runs, and a wedged grab locks them out. It also needs Accessibility and Input Monitoring granted to the binary and a GUI session to grab, neither of which an automated run should be toggling on the machine it runs on. And Windows and Linux behavior cannot run on a mac host at all. A VM per platform, that the agent owns and can wedge or reset freely, is the environment. macOS is the first one that matters, because it is what lets an agent run mercury at all without hijacking the machine.

## What needs a real session, and what does not

- Model-level behavior needs no real keyboard. The event socket takes a frame and the dispatch record lands in the log: connect to `127.0.0.1:3883`, send a frame, read the record out of `~/Library/Logs/mercury/mercury.log` (`CLAUDE.md`, "the event socket"). An agent feeds a frame and reads the log, over SSH, headless. But the daemon still holds the keyboard grab while it runs, so it still wants a session that is not the developer's.
- Grab-level behavior, whether the tap swallows and remaps real keys, needs a GUI session, the permissions granted, and synthetic key injection. That is the VM's reason to exist beyond isolation.
- Source-device attribution (`freddie_hid_device`, `intercept_with_source`) is the same bucket. Pure matching is unit-tested (`OnDevice::is_matching`, figaro `categorize`, dispatch with `DeviceKey`). Resolve, `source_of`, and the per-`SourceId` categorize cache are not: they need a live `CGEvent` / IOKit service. See below.

The socket is why most checks are the same three steps on every platform: start the daemon, send a frame, read the record. Only the questions that are specifically about the OS grab need the graphical path.

## Source-device checks that belong in the guest

These cannot be honest unit tests without mocking private SPI and the registry; the guest is where they run. Against a macOS session with Accessibility / Input Monitoring already granted (and with no seizing remapper upstream — Karabiner grabber rewrites source to its virtual keyboard):

- `source_of` on a synthetic `CGEvent` (e.g. `CGEvent::new_keyboard_event`) returns `None`, so categorize sees `None` and the consumer lands on its synthetic / unknown class.
- A real hardware key yields a stable `SourceId` while the device stays attached; `resolve` returns a `DeviceInfo` whose `vendor_id` / `product_id` / `built_in` / `product` match the attached keyboard (built-in Apple, or an external with known ids).
- `Built-In` often lives on a parent registry entry, not the leaf HID service: `resolve` walks parents and still reports `built_in: true` for the internal keyboard.
- Cache: two keys from the same physical source call `categorize` once (second key is a map hit). Two distinct keyboards (or one keyboard plus a synthetic) produce two categorize calls and two classes.
- End-to-end with a consumer that uses `intercept_with_source` (figaro, once it has a daemon): a BuiltIn-only bind fires for the laptop keyboard and not for a second device or a synthetic inject; a bare key bind still matches either.

A guest that only injects synthetic keys covers the `None` path and the grab path, not the two-device or real-`resolve` path. Real multi-keyboard attribution needs either USB pass-through into the guest or a one-off run on hardware with two devices attached; the runbook should say which path a given check uses.

## The macOS guest

mercury runs here, and this is the guest with immediate value. On Apple Silicon a macOS guest runs through Virtualization.framework: `tart` is the least friction, UTM or Anka also work. The loop an agent drives:

- provision the guest, install the pinned toolchain (1.96.0), `cargo build -p mercury`;
- grant Accessibility and Input Monitoring to the binary, once, then snapshot the guest so every later run starts from the granted state rather than re-granting;
- run `mercury` in the guest's session;
- inject input: a frame on the socket for model-level behavior, or a synthetic `CGEvent` (or `osascript` keystroke) into a foreground app for grab-level;
- read the log and assert on the dispatch records;
- `mercury stop`, or reset to the snapshot.

The host keyboard is never touched, and a grab that wedges is a guest reboot rather than a locked-out developer.

## The Windows and Linux guests

For the portable crates, and once they exist the keyboard backends (`refactors/pending/freddie-keyboard-cross-platform.md`). CI compiles and unit-tests those; a guest is for the runtime behavior CI's headless single-process tests miss:

- lock exclusion across two real processes: start two daemons, the second refuses;
- `stop` and `--force` ending a live daemon and freeing the lock;
- the log landing under the platform's own directory, and `logs` following appends;
- a keyboard backend actually grabbing: on Linux, evdev needs `/dev/input`, `/dev/uinput`, and the `input` group; on Windows, a low-level hook needs a real session.

Linux runs under lima, multipass, or UTM; Windows under UTM (ARM64) on Apple Silicon, or a cloud x64 instance to match CI's `x86_64-pc-windows-msvc`. A Linux keyboard backend needs a full VM, not a container: uinput and a session are not things a container gives you.

## What this produces

Two things, once the mechanics are worked out: a runbook and e2e tests. The runbook is instructions an agent follows to bring a guest up, run the daemon, send it input, and read the result. The e2e tests are the same steps automated: boot or reach the guest, start the daemon, send a known frame or key, and assert the dispatch record the log shows. The runbook is what an agent uses to muck around; the tests are what keep the whole path from rotting between uses.

Neither is permanent VM infrastructure. The guest is a `tart` (or UTM) image, snapshotted with the toolchain installed and, on macOS, the permissions granted, so it boots ready; the tests reach it and drive it. What is worth automating is the driving, not a fleet.

## The unknowns to settle first

This is a spike before it is a plan: the mechanics have to be found before they can be written down or tested. On macOS specifically:

- bringing up a macOS guest an agent can reach non-interactively (`tart` over SSH is the likely answer, but the licensing and the Apple-Silicon-only constraint want confirming);
- granting Accessibility and Input Monitoring without a human clicking the dialog, so a fresh guest is usable unattended (writing the TCC database in the guest, or granting once and snapshotting);
- injecting a real key the grab will see, and confirming a synthetic `CGEvent` is treated as a physical key rather than ignored by the tap (and, separately: that synthetic has no HID origin so `source_of` is `None`, while a hardware key has one);
- whether USB keyboard pass-through into a tart/UTM guest is enough for multi-device source checks, or those stay hardware-only.

The socket path (frame in, record out of the log) is the part already known to work from `CLAUDE.md`; these are what a first pass exists to answer.

## The changes, in order

1. **Settle the macOS mechanics.** The three unknowns above, end to end: a guest an agent reaches, the daemon running in it, a key sent, the record read back. The output is the runbook.
2. **E2e tests on macOS.** The runbook's steps as an automated test: bring the guest to a known state, start the daemon, send a frame, assert the record. This is what makes the path repeatable rather than a thing rediscovered each time. Include the synthetic `source_of → None` path once a consumer calls `intercept_with_source`; multi-device resolve stays a documented hardware (or pass-through) check until the guest can do it unattended.
3. **The Linux and Windows guests, and their e2e.** The same for the portable crates and the keyboard backends, added when a non-mac backend or app makes their runtime behavior worth watching. Source attribution as designed is macOS CGEvent SPI; other platforms get their own story when those backends exist.
