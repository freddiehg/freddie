# virtual HID keyboard (the Karabiner route)

The correct way to remap a keyboard on macOS, and what it costs. This is the upgrade path behind the same `intercept` seam as the CGEventTap backend (keyboard-capture.md); nothing above `intercept` changes when we swap.

The implementable plan lives in `hid-backend.md` and the four component docs it maps (`hid-virtual-device-client.md`, `hid-seize.md`, `hidd.md`, `hid-session-backend.md`). This doc is the background: why HID, what it takes, and the shortcut.

## The shape

Two halves, the way Karabiner-Elements does it:

- Seize the physical keyboard at the IOKit HID level, so its events do not reach the system on their own. You read raw HID input from it.
- Create a virtual HID keyboard with a DriverKit driver and post your output through that. The system sees the virtual device as a real keyboard.

You read the real device and write a separate virtual one, so your output is never your input. There is exactly one transformation in the path, no feedback, and no cross-process loop, because you are not sharing an event stream with anyone. That is why it is correct where the CGEventTap chain is not.

## What it takes to build

1. A virtual HID device, as a DriverKit system extension.
   - DriverKit is Apple's userspace driver framework, C++. The extension registers a virtual HID keyboard and exposes an IOKit user client so a userspace process can post HID reports through it.
   - It must be code-signed with the DriverKit HID entitlements (`com.apple.developer.driverkit`, the HID transport entitlement, and the virtual-HID-device entitlement). These are managed entitlements: you request them through your Apple developer account and Apple grants them. This is the real gate, not the code. Verify the exact set; expect an approval step.
   - Shipped inside an app bundle, installed with `OSSystemExtensionRequest`, approved by the user in System Settings, notarized. May need a reboot.

2. Seizing the physical keyboard.
   - `IOHIDManager` / `IOHIDDevice`, opened with `kIOHIDOptionsTypeSeizeDevice` for exclusive access, reading HID input reports (Keyboard/Keypad usage page). This is HID usages, one level below the keycodes the tap gives us.
   - Seizing wants root, and the reads want Input Monitoring. Karabiner runs a root `LaunchDaemon` for this.

3. A privileged daemon plus a session piece.
   - The seize and the driver's user client live in a root daemon; a session agent talks to it. So there is privilege separation and IPC to build, which the single-process tap version does not have.

## The shortcut

Karabiner's driver, `Karabiner-DriverKit-VirtualHIDDevice`, is open source and permissively licensed. Two ways to use it:

- Depend on the already-installed Karabiner driver: skip the entitlement gate entirely, at the cost of requiring the user to have it installed.
- Build and sign that driver ourselves: still needs the entitlements, but not writing the C++ from scratch.

Either beats writing and getting a new driver approved.

## Local development is not gated

The entitlement approval and notarization are for shipping to other machines, not for iterating on your own. Locally you turn on system-extension developer mode (`systemextensionsctl developer on`, usually with SIP relaxed) and load a self-signed build, which is the normal driver dev loop. And reusing Karabiner's installed, already-signed driver skips signing and notarization entirely while developing. So the Apple gate is a distribution problem and can be deferred; it does not block trying the approach.

## How it fits behind `intercept`

`intercept(on_key)` is observe-plus-emit: it hands you each key through `on_key` and returns an `Interceptor` and an `Emitter`, whose `emit`/`tap` post output. The HID backend implements that directly: `on_key` is a seized-device read, `emit` posts a virtual HID report. The CGEventTap backend implements the same by swallowing and re-posting, with the tag and the cross-process loop.

The one thing that would leak is CGEventTap's trick of deciding in the callback and returning the event down the chain. HID has no chain to return into, so we do not use that trick; both backends stay on observe-plus-emit, and the swap is invisible above `intercept`.

## mercury is already shaped for this

mercury swallows every key and re-posts its output through `CGEventPost`, rather than deciding in the tap callback and returning the event down the chain. That looks like a compromise on the tap and it is exactly what HID needs: observe, then emit. Nothing above `intercept` changes on the swap, and nothing in mercury's event loop, effect loop, or model changes either.

The alternative, dispatching synchronously in the tap callback, would have to be undone here, because there is no chain to return into. See synchronous-dispatch.md, where that is decided against for this reason.

## Recommendation

CGEventTap now, behind `intercept`, single-process. HID when the cross-process loop or robustness matters, ideally by leaning on Karabiner's driver rather than shipping our own. The API does not change across the swap, so starting on the tap is not wasted.
