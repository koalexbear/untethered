# Test an in-place Linux package upgrade

This acceptance test takes two actual `untethered` **Arch x86-64** package
archives (older version first). It runs `pacman -U` twice against the same
temporary root and package database. The executables discover their bundled
gomuks and the default `$XDG_DATA_HOME/untethered-iced` profile. System
libraries come from the host; packages, X11 display, home directories,
accounts, browser data and Synapse are disposable.

Build a candidate with the repository's `PKGBUILD` (`makepkg`); retain an
earlier package from before the upgrade. Use the [native/Element QA
environment](replies-element.md), plus `cc`, Xlib development headers,
`pacman`, `vercmp`, and an unprivileged user/network namespace. Example:

```sh
export ELEMENT_WEB_DIR=/path/to/extracted/element-v1.12.35
export PYTHON=/path/to/qa/venv/bin/python
unshare -Urn sh -ec '
  ip link set lo up
  exec "$PYTHON" scripts/test-media-interop.py --linux-upgrade "$1" "$2"
' sh /path/to/older.pkg.tar.zst /path/to/newer.pkg.tar.zst
```

The older GUI signs in, verifies a new Matrix device and decrypts a first
Element message. The fixture records the device ID using the profile and
`/account/whoami`. It compares the Ed25519 and Curve25519 public keys in
the local crypto store with the keys Synapse returns to the other account.

The old GUI closes normally. While both receiver processes are stopped,
Element sends **Amber lighthouse** using the same Megolm session. The test
checks that this event is encrypted on Synapse, that its key is already
present on the receiver, and that the event is **absent** from its local
database before the upgrade. Thus showing the historical message after
the upgrade requires decrypting ciphertext, rather than rendering a
previously cached body.

The package manager must record an **upgrade**, the executable hash must
change, and the new GUI must open the same profile without sign-in or
recovery input. Both public keys, the device ID, and the account's device
roster must match. The fixture reads the historical plaintext on the new
GUI and independently checks the event in the local database. It also
checks for backup downloads and sender key resends during this step.
Finally Element discards its outbound session and sends **Violet compass**;
the upgraded receiver must decrypt the new event in a different Megolm
session. Output contains the disposable public device keys, not tokens,
recovery secrets or private key material.

## Recorded run: 2026-11-25

Archived package `0.1.0-2` upgraded to `0.1.0-6` built by `makepkg` from
`f6d6d55`. The release build passed 36 tests (3 opt-in tests ignored).
The sender was Element Web 1.12.35 on loopback Synapse 1.166.0.

| Value checked before and after | Result on both builds |
|---|---|
| `/account/whoami` device ID | `VPKQEQZBTB` |
| Ed25519 signing public key (profile and `/keys/query`) | `xB6Za67XoTMM39BEdgNgKisT5balawM76qVYv8/Csrs` |
| Curve25519 identity public key (profile and `/keys/query`) | `5L8a9rIt2bT/TChUe/sr3ubCBSYms5R9xrTAJf337iA` |

Pacman recorded `upgraded untethered (0.1.0-2 -> 0.1.0-6)`. Archive
SHA-256 changed from `ca874d4386f9c5b1457f3849737d95cf8e535da4e47767ddfc3abad907f1e6fa`
to `d86c7cd23d1c5daaee17d9d0bdb37b2c30e1595fc8a326ed07afea70f864a3ed`.
The installed GUI's SHA-256 also changed (`3363f293…` → `a2e128af…`).

| Element event | What the upgraded client displayed |
|---|---|
| `$59U8PYADhiO0CYlF7Pt90aXUnIj0JV5znenbH0r0xqA`, sent before upgrade and absent from the stopped profile | **Amber lighthouse**, using the retained Megolm session |
| `$7S8arlerGZvO6QbHkTwwqKas4hQtaLmEeqhug0Mo33o`, sent after upgrade with a different Megolm session | **Violet compass** |

No new login, recovery input, or device registration was needed. This run
covers a local Arch package upgrade with the pinned gomuks backend; a
backend-version migration and a graphical updater are separate workflows.
