# a VM an agent can test freddie's behavior in

The unit tests cover the model as a table and the crates in isolation, and CI runs them on every platform the code targets. What none of them covers is runtime behavior: the keyboard actually grabbed and remapping, the lifecycle verbs against a live daemon, the menu bar, a cross-platform backend on real input. An agent that changes freddie and wants to see the change work has to run it, and running it on the developer's own machine is the problem this doc solves.

mercury grabs the global keyboard. A daemon started on the host swallows the human's keys for as long as it runs, and a wedged grab locks them out. It also needs Accessibility and Input Monitoring granted to the binary and a GUI session to grab, neither of which an automated run should be toggling on the machine it runs on. And Windows and Linux behavior cannot run on a mac host at all. A VM per platform, that the agent owns and can wedge or reset freely, is the environment. macOS is the first one that matters, because it is what lets an agent run mercury at all without hijacking the machine.

## What needs a real session, and what does not

- Model-level behavior needs no real keyboard. The event socket takes a frame and the dispatch record lands in the log: connect to `127.0.0.1:3883`, send a frame, read the record out of `~/Library/Logs/mercury/mercury.log` (`CLAUDE.md`, "the event socket"). An agent feeds a frame and reads the log, over SSH, headless. But the daemon still holds the keyboard grab while it runs, so it still wants a session that is not the developer's.
- Grab-level behavior, whether the tap swallows and remaps real keys, needs a GUI session, the permissions granted, and synthetic key injection. That is the VM's reason to exist beyond isolation.

The socket is why most checks are the same three steps on every platform: start the daemon, send a frame, read the record. Only the questions that are specifically about the OS grab need the graphical path.

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

## The piece worth building

Not permanent VM fleets. The reusable piece is the driver: one script that, given a running guest, pulls, builds, starts the daemon, sends a frame, and returns the dispatch record, because that is the loop an agent repeats and it is nearly identical on all three platforms. The guest images are worth automating (tart or lima snapshots that boot with the toolchain installed and, on macOS, the permissions granted) once grab-level testing is recurring, which the keyboard backends or a non-mac freddie app are what make it.

Until then the one with standing value is the macOS guest, because it is the difference between an agent being able to run mercury and not: on the host it cannot, and in the guest it can, with the developer's keyboard untouched.

## The changes, in order

Each is usable on its own; the driver is what an agent calls, and the guests are what it calls into.

1. **The driver.** A script taking a host to reach (an SSH target) that builds, starts the daemon, sends a frame it is given, waits for the record, and prints it. Platform-agnostic through the socket; the only per-OS part is the log path, which `Instance` already computes.
2. **The macOS guest image.** A `tart` (or UTM) macOS guest with the toolchain and the granted permissions, snapshotted, so the driver has something to reach.
3. **The Linux and Windows guest images.** The same for the portable crates and the keyboard backends, added when a non-mac backend or app makes their runtime behavior worth watching.
