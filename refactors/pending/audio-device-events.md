# audio device events

An event when a microphone is connected or disconnected, and an effect that decides which microphone Wispr Flow uses.

A note. The source is well understood; the sink has an unmeasured assumption sitting under it.

## What we actually want

Not a toggle. A priority order, re-evaluated whenever the set of devices changes. voicemode wrote the policy down and never implemented it (`scripts/actions/select-microphone.sh`):

```
# Priority: Shure MV7 > Bose QC Headphones > Built-in (if not clamshell)
```

Which makes this two pieces. A source reporting the current set of input devices, and a pure function from that set (plus whether the lid is closed) to the device we want. The function is model logic, the sort of thing laserbeam should hold, and it is idempotent: recompute on every change, apply only if the answer differs.

The clamshell condition couples this to display-events.md. "Lid closed" is the same fact as "the built-in panel is gone", which voicemode's display module already computes.

## The source

CoreAudio, not AppKit. `AudioObjectAddPropertyListenerBlock` on `kAudioObjectSystemObject`, listening for `kAudioHardwarePropertyDevices` (the device list changed) and `kAudioHardwarePropertyDefaultInputDevice` (something else changed the default out from under us).

The listener-block variant takes a dispatch queue, so the callback arrives on that queue rather than on the main run loop. Worth noticing, because it is the first source that does not need `freddie_main_loop`. It would be the first source whose callback lands on a queue we choose, which means it can send into the event channel from wherever, and it changes nothing about the model.

Its own `freddie_*` crate, by the rule in the README. figaro would want it identically.

## The sink, and the thing nobody has checked

Setting the system default input is `AudioObjectSetPropertyData` on `kAudioHardwarePropertyDefaultInputDevice`, or shelling out to `SwitchAudioSource`.

**Whether Wispr Flow follows the system default input is unmeasured, and the whole feature rests on it.** If Wispr has its own microphone setting in its preferences, changing the system default does nothing, and controlling Wispr means driving its UI through Accessibility or writing its defaults, both of which are worse. This is the first thing to check, before any of the CoreAudio work, because it decides whether the feature is an afternoon or a project.

## Headphones, in both directions

The same source reports headphones arriving and leaving, because a Bluetooth headset appears and disappears from CoreAudio's device list. So "AirPods connected" is a device-list change with a new output and a new input, and the priority policy above already covers what to do about the microphone.

The output side is new. Connecting headphones should probably move the system output to them, and disconnecting should move it back rather than dumping audio through the laptop speakers in a meeting. `kAudioHardwarePropertyDefaultOutputDevice` is the setting, symmetrical with the input one.

Effects that act on the device rather than the routing are a different animal. Disconnecting AirPods from the keyboard, so they hand back to a phone, means talking to Bluetooth rather than to CoreAudio: `IOBluetoothDevice` and `closeConnection`, or shelling out to `blueutil`. Neither is measured, and objc2 may not cover `IOBluetooth` at all, in which case it is raw FFI or a subprocess.

Worth noticing that this makes the audio crate two crates, or one crate with two halves. Enumerating and routing audio devices is CoreAudio. Connecting and disconnecting a headset is Bluetooth. They meet only in that the same physical act produces both.

Open, in addition to the questions below: does disconnecting a headset from macOS make it reconnect to a phone, or merely make it deaf? And is the sensible binding a toggle, given that connecting from the keyboard is a different mechanism from disconnecting?

## Identity, for the third time

`AudioDeviceID` is an opaque integer that is not stable across reboots or reconnects. `kAudioDevicePropertyDeviceUID` gives a stable string. A priority list naming "Shure MV7" wants to match on something durable, and the device-uid-to-microphone table belongs with the bindings, exactly like the bundle-id table and the display-uuid table.

That is now three sources in a row where the obvious identifier is the unstable one and the durable one is a level down. It is worth writing that rule into the crates as they are built rather than rediscovering it each time.

## Open questions

- Does Wispr Flow use the system default input device? Everything else waits on this.
- Does the priority list live in mercury's bindings, or is it configuration?
- Is "clamshell" the same predicate as "no built-in display", and who owns computing it, this crate or the display one?
- Do we want an event per device change, or one event carrying the current device set? The policy is a function of the set, which argues for the set.
- Does anything else want to observe audio devices, or is Wispr the only consumer? If it is the only one, the crate boundary may be premature.
