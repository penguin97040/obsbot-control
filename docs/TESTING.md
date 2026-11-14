# Hardware test checklist

Automated tests cover protocol encoding and safety helpers, but every camera feature needs
a real test with the OBSBOT plugged in. Run through the Windows list on Windows and the Linux list on
Linux after each release. The Windows camera code was checked on a real Tiny 2 Lite (v0.3). The Linux
version hasn't been tested on a real camera yet, so the Linux list is the first thing to run.

Both versions can be started with `--demo` to check the interface without a camera. That tests only the
interface, not the camera code.

## Windows

### Before you start

1. Close OBSBOT Center, Teams, Zoom, OBS and the Camera app (only one app can
   use the video stream at a time).
2. Download `obsbot-control.exe` from the latest GitHub Release (or the
   **Actions** tab → latest run → *Artifacts*), and run it. Windows SmartScreen
   may warn because the exe isn't code-signed: choose *More info → Run anyway*.

### Checklist

Tick each box or note what happened.

Some checks use the **Developer** tab. Turn it on with *Camera → Advanced → Developer tab*.

#### Connection and preview
- [ ] The header shows "OBSBOT Tiny 2 Lite" with a green dot.
- [ ] The live preview appears. Note what the three pickers at the top left show (it was *1280×720*,
      *NV12* and *30 fps* in v0.3).
- [ ] Open the resolution picker: sizes are listed largest first, with the default marked. Pick the
      largest: the preview restarts at that size (the pixel format and frame rate change only if that
      size doesn't offer them).
- [ ] Try the pixel format (NV12/MJPG/YUY2) and frame rate pickers: each only lists what the chosen
      size offers, and the preview restarts each time. Then go back to the default again.
- [ ] Close and reopen the app: the preview starts at the default format again.
- [ ] Text is sharp, not blurry, on your monitor.
- [ ] Unplug the camera: the header goes red within a few seconds. Plug it back in: it reconnects.
- [ ] Turn off the *Live preview* switch above the preview: it stops promptly and controls still work.
- [ ] Toggle the preview off/on several times; each restart shows new frames.
- [ ] Unplug while previewing, then turn the preview off and close the app: neither action freezes.
- [ ] Sleep the camera while previewing, then turn the preview off/on and wake it; the app stays responsive.
- [ ] With two cameras connected, switch cameras during preview; only the selected camera appears.
- [ ] Resize, minimise and restore the window while previewing; the image redraws normally.

#### Title bar
- [ ] There is one title bar: the app's own header, with minimise, maximise and close at the right. Windows
      draws no separate grey bar above it.
- [ ] Drag the window by the empty part of the header and by the "obsbot-control" text. Clicking *Full*,
      *Compact* or *Sleep camera* never moves the window.
- [ ] Double-click the empty header: it maximises; again: it restores. The maximise button switches to the
      restore icon.
- [ ] Hover the maximise button on Windows 11: the Snap Layouts menu appears. Win+arrow keys snap the window.
- [ ] Close turns red on hover and closes the app (settings are saved; check the size on the next start).
- [ ] Maximised, nothing is cut off at the top of the screen.

#### Compact view and pausing the preview
- [ ] With Discord, Zoom or Teams already using the camera, open the app: it opens narrow, without the
      preview, with the header switch on *Compact* and "Camera in use by ..." under the header naming that app.
      Opening is just as quick as before.
- [ ] With the app open in full view showing the preview, click into Discord: the app's preview pauses
      (a dimmed, blurred picture with "Preview paused"). Turn on your video in Discord: it works, and within a
      few seconds the app goes compact. End the call: the app returns to full view at its normal width, and
      the preview starts again when you click back into the app.
- [ ] Click back into the app with no call running: the blurred picture shows "Starting preview..." and the
      live picture replaces it within a second or two.
- [ ] Turn off *Camera → Pause preview*: the preview keeps running while another window is active.
- [ ] Switch the header to *Compact* by hand: the window narrows smoothly while the preview slides under the
      controls and fades out (under a fifth of a second, smooth at the screen's refresh rate). Switch back to *Full*: it widens to its earlier size
      as the preview fades back in. No flicker or jumping.
- [ ] During a call, switch back to *Full*: it stays in full view when the call ends.
- [ ] Maximise the window, then start a call: it un-maximises and goes compact.
- [ ] Move the window near the right edge of the screen in compact view, then go back to full view: the
      window stays on screen.
- [ ] With the camera connected, the header in compact view fits on one line: the status shows as a dot
      (hover for the text) and the sleep button is a moon icon (amber while asleep). With two cameras, the camera picker moves to its
      own line.
- [ ] Turn off *Camera → Auto compact*: starting and ending a call no longer changes the view.
- [ ] Close the app while compact, then open it with no call running: it opens at the normal full width.

#### AI tracking (Control tab)
- [ ] *Normal* starts tracking you. The button stays highlighted.
- [ ] *Upper body*, *Close-up*, *Headless*, *Lower body*: framing changes.
- [ ] *Group*, *Hand*, *Whiteboard*, *Desk*: note which work.
- [ ] *Tracking off* stops tracking.
- [ ] *Sport* tracking speed: does the Log say "camera acknowledged"?

#### Gimbal and zoom
- [ ] *Smooth (vendor)*: hold each arrow; the camera moves and **stops when you let go**.
- [ ] With both *Invert* switches off, every arrow moves the picture the way it points, in both
      *Smooth (vendor)* and *Steps (UVC)*. Note any arrow that's still the wrong way round.
- [ ] *Invert → Horizontal* swaps left and right; *Invert → Vertical* swaps up and down (both methods).
- [ ] The *Speed* slider is only active for *Smooth (vendor)*.
- [ ] *Pan*, *Tilt* and *Zoom* sliders move the camera.
- [ ] *Centre* returns the camera to the middle. What does the Log say?
- [ ] Release a held arrow while changing tracking speed, FOV or sleep; movement stops promptly.
- [ ] Switch cameras while holding an arrow; the old camera stops and the new camera does not inherit movement.
- [ ] Close the app while moving; the camera stops. Unplug during movement, reconnect and confirm it remains stopped.
- [ ] Drag the speed slider to both ends; the value beside it stays within 5–90°/s.
- [ ] Recall a saved preset after changing cameras; positions stay within the selected camera's ranges.

#### Presets (bottom of Gimbal and zoom)
- [ ] With the app at its normal size, the whole Control tab fits without scrolling.
- [ ] Click *+ Save 1*: it becomes *Preset 1*. Move away, click *Preset 1*: the camera returns.
- [ ] Right-click *Preset 1*: *Update to the current position* and *Clear* both work.

#### Image tab
- [ ] Brightness, contrast, saturation, sharpness change the picture.
- [ ] White balance, exposure and focus *Auto* switches toggle, and the slider greys out while on.
- [ ] Double-click a slider: it returns to the camera's default.
- [ ] *Reset to defaults* works.

#### Camera tab
- [ ] *HDR* toggles (the picture changes subtly; the switch stays on).
- [ ] *Field of view* Wide/Medium/Narrow changes the picture. If the wrong one is
      selected, or nothing happens, try *Developer → Protocol variant → Tiny 2 codes* and note it.
- [ ] *Anti-flicker* 50 Hz is selected by default.
- [ ] *Sleep camera* puts it to sleep (the lens usually points down); *Wake camera* wakes it.

## Linux

### Before you start

1. Close Cheese, OBS, browser tabs and anything else using the camera (only one app can use the video
   stream at a time). The Linux section below also tests what happens when one of them is open.
2. Download `obsbot-control-x86_64.AppImage` from the latest GitHub Release, then:

   ```sh
   chmod +x obsbot-control-x86_64.AppImage
   ./obsbot-control-x86_64.AppImage
   ```

   Add `--demo` to use the simulated camera instead. If it won't start and mentions FUSE, see the
   README (or run it with `--appimage-extract-and-run`).
3. The log is at `~/.config/obsbot-control/obsbot-control.log`. The same text is in the **Log** tab.

### Checklist

Tick each box or note what happened. Note your distribution and desktop (Wayland or X11) too.

#### Connection and camera
- [ ] Exactly one camera is listed (not the camera's separate metadata node), with the right name and model.
- [ ] The log says "Found the OBSBOT extension unit on UVC unit N". Note N.
- [ ] AI tracking (each mode, as in the Windows list), HDR, field of view and sleep/wake all work.
- [ ] Every slider on the Image tab has a sensible range and the picture changes as it moves.
- [ ] *Auto* on/off works for exposure, white balance and focus.
- [ ] After turning *Auto* off, moving the slider takes effect.
- [ ] Anti-flicker 50 Hz / 60 Hz works.
- [ ] *Pan*, *Tilt* and *Zoom* sliders move the camera.
- [ ] Both movement methods (*Smooth (vendor)* and *Steps (UVC)*) work. Note which way each arrow moves
      the picture, and whether the *Invert* switches are needed.

#### Preview
- [ ] The resolution, pixel format and frame rate pickers list MJPG, YUYV and NV12 sizes and rates, with
      no H.264.
- [ ] Switching size, format and rate works, and the label above the preview matches what's showing.
- [ ] MJPG at 1080p and at 4K shows no tearing or damage, and CPU use is reasonable (note the figure).
- [ ] With Cheese, OBS or a browser using the camera, the preview says the camera is busy, and compact view
      names that app. Also try Firefox, which may show "PipeWire" instead.
- [ ] Unplug the camera during the preview: "The camera was disconnected." appears quickly and nothing
      freezes. Plug it back in: it reconnects.

#### Title bar
- [ ] On GNOME and KDE (Wayland and X11), the app draws its own title bar with minimise, maximise and close,
      rounded corners and a soft shadow. If the desktop draws its own frame instead, the app's buttons are
      hidden and the desktop's ones work.
- [ ] Dragging the empty header moves the window; double-click maximises; right-click opens the window menu.
- [ ] Resize from every edge and corner. The cursor changes near the edges.
- [ ] Tiled to half the screen, the tiled sides lose their margin and rounded corners and the free side still
      resizes. Maximised, the window fills the screen with square corners.

#### Other
- [ ] Starting a second copy of the app is refused.
- [ ] *Log → Copy diagnostics* shows the distribution and kernel.
- [ ] The AppImage starts on Fedora under both Wayland and X11.

## Reporting results

1. Go to the **Log** tab and press **Copy diagnostics**.
2. Open a [GitHub issue](../../../issues) and paste that, plus your ticked
   checklist. The diagnostics include the camera's raw status block and control
   ranges, which is exactly what's needed to fix the protocol tables in
   `docs/PROTOCOL.md`.
