Based on the MIT-licensed `smithay-clipboard` 0.7.3 crates.io source.
The Iced clipboard already owns a Wayland data device for the window. Two
small hooks use that same device:

- `FileDrag` and the `DataDeviceHandler` callbacks negotiate copy-only URI
  lists or `application/vnd.portal.filetransfer`. They hand the drop pipe to
  the app immediately and finish the Wayland offer on the worker queue after
  the app consumes it (including portal retrieval for a portal key).
- `composer_offer()` asks the focused seat for the regular selection on an
  explicit paste. It returns a PNG or text pipe; the app reads it off the UI
  thread with a byte limit and timeout. It does not use data-control, observe
  clipboard changes, or intercept paste in password and recovery-key inputs.

Retire the hooks when upstream exposes both Wayland file drops and image paste.
