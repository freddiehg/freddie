# CGEventTap vs virtual HID

The macOS backend choice for capture and emit. Details live in keyboard-capture.md (tap) and virtual-hid.md (HID); this is the comparison and the call.

The two are not mutually exclusive. Both sit behind the same `Grab` (observe plus emit, synchronous dispatch), so the choice is which to build first, not a permanent commitment. That framing matters: start on the cheap one, keep the other as a known upgrade.

## Correctness and the loop

CGEventTap shares the global event stream. If a stage decides synchronously and returns the event down the chain, it is correct and loop-free, because nothing is re-posted. The moment it re-posts (`CGEventPost`, which any async decision or one-to-many output forces), its output re-enters the stream: the tag stops it re-eating its own output, but not another process feeding that output back. Two remappers with inverse maps loop. So CGEventTap is correct for synchronous, single-process remapping and has a real hole outside that.

HID seizes the physical keyboard (exclusive) and posts to a separate virtual device, so output is never input. One transformation, no loop, cross-process safe by construction. Correct where the tap is not.

## Secure input and level

A session `CGEventTap` is bypassed by secure input: password fields and apps that call `EnableSecureEventInput` stop your tap from seeing keys, so remaps do not apply there.

HID is below that. It reads raw HID reports from the seized device, so it keeps working in password fields, which is a large part of why people run Karabiner. If remapping has to work everywhere, the tap cannot do it and HID is not optional.

## Cost to build and ship

CGEventTap is pure userland: one process, safe Rust over `core-graphics`, no driver, no root. The only gate is Accessibility plus Input Monitoring, which the user grants in System Settings. It ships today.

HID is a project. A DriverKit system extension in C++, code-signed with managed HID entitlements that Apple has to approve on request, notarized, installed as a system extension the user approves, plus a root daemon to seize the device and drive the virtual one, with IPC to a session process. The realistic shortcut is leaning on Karabiner's open-source driver instead of shipping and getting our own approved, but that still means a driver on the machine. The entitlement approval is the gate, and it is outside our control, but only for distribution: locally you develop with system-extension developer mode (or against Karabiner's installed driver) without approval or notarization, so the gate does not block trying it.

## Permissions and reliability

The tap asks for two TCC permissions and is otherwise self-contained. Its failure modes are the OS disabling a slow callback (keep it fast, watch for the disable event) and no control over tap ordering across processes.

HID has more moving parts (extension load state, the driver, the root daemon, macOS updates that touch DriverKit) but more control over the actual remapping, and it is the design that is known to be robust at Karabiner's scale.

## Latency

Both are a single fast hop, microseconds, well under anything perceptible. Not a differentiator.

## The call

Build CGEventTap first, behind `Grab`, with the synchronous-dispatch model. It is cheap, safe, ships now, and is correct for the single-process case that mercury and figaro actually are. The synchronous model is what makes it correct enough to ship, since it removes the re-post that causes the loop.

Move to HID when a hard requirement forces it, not before:

- remapping has to work in password fields or other secure-input contexts;
- multiple independent remappers have to coexist correctly (the cross-process loop);
- you want Karabiner-grade robustness as a product, not a personal tool.

Until one of those is real, HID is a large investment gated on Apple, for correctness in cases a single-process tap does not hit. Because the swap is invisible above `Grab`, starting on the tap costs nothing if we later need the driver.
