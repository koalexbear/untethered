# Test an in-place Linux package upgrade

This acceptance test takes two actual `untethered` **Arch x86-64** package
archives (older version first), both on the matrix-sdk engine: `pkgrel` 9
or later, which bundle no gomuks. It runs `pacman -U` twice against the
same temporary root and package database. The executables use the default
`$XDG_DATA_HOME/untethered-iced` profile. System libraries come from the
host; packages, X11 display, home directories, accounts, browser data and
Synapse are disposable.

An older package that bundles gomuks is refused: moving from gomuks to the
matrix-sdk engine takes one new sign-in by design (the sign-in screen
explains it), so there is no device to carry over.

Build a candidate with the repository's `PKGBUILD` (`makepkg`); retain an
earlier matrix-sdk package from before the upgrade. To try it with only one
release, build the older commit and the newer one from copies of the
`PKGBUILD` whose `source` names each commit, and give the newer one a higher
`pkgrel`. Use the [native/Element QA environment](replies-element.md),
plus `cc`, Xlib development headers, `pacman`, `vercmp`, the debug client
(the fixture's `seed()` uses it) and an unprivileged user/network
namespace. Example:

```sh
export ELEMENT_WEB_DIR=/path/to/extracted/element-v1.12.35
export PYTHON=/path/to/qa/venv/bin/python
unshare -Urn sh -ec '
  ip link set lo up
  exec "$PYTHON" scripts/test-media-interop.py --linux-upgrade "$1" "$2"
' sh /path/to/older.pkg.tar.zst /path/to/newer.pkg.tar.zst
```

The older GUI signs in, verifies a new Matrix device with the recovery key
and decrypts a first Element message. The fixture reads the device ID and
access token from the profile's `matrix-sdk/session.json` and checks them
with `/account/whoami`, and records the Ed25519 and Curve25519 public keys
Synapse returns to the other account. matrix-sdk encrypts its stores with
the profile's passphrase, so the fixture doesn't read them; instead it
checks that the crypto store file is the same one (inode) throughout.

The old GUI closes normally. While no client of the profile runs, Element
sends **Amber lighthouse** using the same Megolm session. The test checks
that this event is encrypted on Synapse. The profile can't have cached it,
so showing it after the upgrade requires decrypting its ciphertext with the
retained session.

The package manager must record an **upgrade**, the executable hash must
change, and the new GUI must open the same profile without sign-in or
recovery input. The public keys, the device ID, and the account's device
roster must match. The fixture reads the historical plaintext on the new
GUI. It also checks for backup downloads and sender key resends during
this step. Finally Element discards its outbound session and sends
**Violet compass**; the upgraded receiver must decrypt the new event in a
different Megolm session, without a login or a new device. Output contains
the disposable public device keys, not tokens, recovery secrets or private
key material. Release builds have no QA event log, so the fixture reads
the window.

## Recorded run: 2026-11-26

`0.1.0-9` built by `makepkg --nocheck` from `02367e4` (the first package
without gomuks) upgraded to `0.1.0-10` built the same way from `fb1add0`,
each from a copy of the `PKGBUILD` naming its commit. The sender was Element
Web 1.12.35 on loopback Synapse 1.166.0. The old GUI signed in through its
sign-in card and recovery key, then started once more on the new session:
builds before `f4ea4e5` show the room selected during sign-in only after a
restart.

| Value checked before and after | Result on both builds |
|---|---|
| `/account/whoami` device ID (from `session.json`) | `GQEBAHGWYI` |
| Ed25519 signing public key (`/keys/query`) | `tx+bGEFLB1bYlHi/WpthW/Sae4WWB/BrQh/UZd2wSFo` |
| Curve25519 identity public key (`/keys/query`) | `b4+gyEA2h9lZXsYZcEcIAfExcwYBOzMxkuKMi5VZ/EM` |

Pacman recorded `upgraded untethered (0.1.0-9 -> 0.1.0-10)`; the installed
GUI's SHA-256 changed (`b4d0a093…` → `94040c68…`) and the crypto store kept
its inode. The upgraded GUI showed **Amber lighthouse**, sent while the
profile was closed, with the retained Megolm session (no backup download or
key resend), and then **Violet compass** in a new session, without a login
or a new device.

## Recorded run with gomuks: 2026-11-25

Before the matrix-sdk engine (the fixture then also read gomuks's
database). Archived package `0.1.0-2` upgraded to `0.1.0-6` built by
`makepkg` from `f6d6d55`. The release build passed 36 tests (3 opt-in tests ignored).
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
