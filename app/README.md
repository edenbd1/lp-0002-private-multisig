# LP-0002 Basecamp app

A Logos Basecamp surface for the private multisig: create a member set, bind an
action to a proposal, approve as a member, watch the threshold fill, and build
the execution.

Every button runs an `msig` subcommand through `MultisigBridge`, so the GUI and
the chain compute the same commitments from the same code. There is no second
implementation to drift.

**The app never holds a member secret.** A secret entered in the approval field
is passed to the CLI process and never written to disk by this plugin.

## Building

### With the Logos module builder (produces a loadable `.lgx`)

Inside the `logos-module-builder` dev shell, where
`$LOGOS_MODULE_BUILDER_ROOT` is set:

```bash
cd app
cmake -B build -S .
cmake --build build
```

`LogosModule.cmake` wires up Qt, the SDK, and the `.lgx` packaging. This is the
path that produces an installable module.

### Standalone (for QML iteration)

Plain Qt6, no Logos stack required:

```bash
cd app
cmake -B build -S . && cmake --build build
```

The fallback branch produces `build/lp_0002_multisig.<so|dylib>` with
`metadata.json` and `module.json` copied next to it. Drop that directory into
Basecamp's user-plugins directory to load it.

Requires Qt 6 with `Core Gui Widgets Quick QuickWidgets Qml`.

### Which Qt — this is load-bearing

Build against **Qt 6.9.x or older**, not against whatever Qt is newest.

Qt refuses to load a plugin whose minor version is above the host's, and Logos
Basecamp 0.2.2 ships Qt 6.9.2. A plugin built against Homebrew's current Qt
(6.11.1) is rejected outright, with nothing in the UI to explain it — the app
tile simply does nothing, and the reason appears only on Basecamp's stderr:

```
Failed to load UI module "lp-0002-multisig" :
  "The plugin ... uses incompatible Qt library. (6.11.0) [release]"
```

The committed `darwin-arm64` variant is built against Qt 6.9.2, obtained without
touching the system Qt:

```bash
python3 -m venv /tmp/aqt && /tmp/aqt/bin/pip install aqtinstall
/tmp/aqt/bin/aqt install-qt mac desktop 6.9.2 clang_64 --outputdir /tmp/Qt

cmake -B build -S . -DCMAKE_PREFIX_PATH=/tmp/Qt/6.9.2/macos -DCMAKE_BUILD_TYPE=Release
cmake --build build
```

Official Qt builds reference their frameworks as `@rpath/QtCore.framework/...`,
which resolves against the Qt that Basecamp bundles. A Homebrew-built plugin
instead hardcodes `/opt/homebrew/opt/qtbase/lib/...`, so it would also fail on
any machine without that exact Homebrew layout. The `linux-amd64` variant builds
against Debian bookworm's Qt 6.4.2, which is below 6.9 and therefore fine.

Check a build before shipping it:

```bash
otool -L build/lp_0002_multisig.dylib | grep -c /opt/homebrew   # must be 0
otool -L build/lp_0002_multisig.dylib | grep QtCore             # must say 6.9.x
```

### The plugin interface is ABI-critical

`src/plugin.h` declares `IComponent` locally so the standalone path needs no SDK
header. Two things in that declaration are not free choices:

- the interface string must be `com.logos.component.IComponent`. It is what
  `qobject_cast<IComponent*>` compares across the plugin boundary; a private IID
  makes the cast return null and Basecamp logs *"Plugin does not implement
  IComponent"*.
- the virtual functions must be exactly `~IComponent`, `createWidget`,
  `destroyWidget`, in that order. An extra virtual — a `name()` accessor, say —
  shifts every later vtable slot, so the host calls the wrong function through a
  pointer that cast successfully.

Both were verified against LogosBasecamp 0.2.2 by reading the secondary vtable
its own `main_ui` plugin emits for `IComponent`.

## Using it

1. Point **Multisig folder** at a directory. The `msig` binary is found
   automatically: the plugin resolves its own path with `dladdr` and uses the
   CLI shipped beside it in the package, so a freshly installed `.lgx` works
   without configuration. The second field overrides that, for running the
   plugin out of a build tree.
2. **Create** — pick N and M, press *New multisig*. This writes `multisig.json`
   and `members.json` and prints the member root and the config hash that anchors
   the pair on chain.
3. **Propose** — enter a proposal id and the action text, press *Bind*.
   Re-binding the same id to a different action is refused, and the message
   explains why: the approvals already gathered do not carry over.
4. **Approve** — pick a member index, or paste a member's own secret, and press
   *Build approval*. Submit the emitted `.args` with `spel` on the
   privacy-preserving path.
5. **Status** shows how many approvals have been gathered. It reads the resumable
   state file, so it survives a Basecamp restart.
6. **Build execution** emits the execution arguments once the threshold is met.

The approval list shows marker addresses, never member names — because that is
all the chain records, and all the other members can see.

## Packaged asset

`app/lp-0002-multisig.lgx` (2.4 MB, SHA-256 `0b6907f18fa7002a6167653871af0e4da8ff91e4d3e0d447b7df1740e297e0ac`) is the packaged
module. It carries **two variants** — `darwin-arm64` and `linux-amd64` — each
with the plugin library, the QML view, the module metadata, and the `msig` CLI
the bridge drives. Basecamp selects the one matching the host.

Two variants rather than one because the evaluation happens on Linux; a
macOS-only package is one the evaluator cannot open at all.

### Installing it

```bash
lgx extract app/lp-0002-multisig.lgx --variant darwin-arm64 --output /tmp/x
mkdir -p ~/Library/Application\ Support/Logos/LogosBasecamp/plugins/lp-0002-multisig
cp -R /tmp/x/darwin-arm64/. ~/Library/Application\ Support/Logos/LogosBasecamp/plugins/lp-0002-multisig/
printf darwin-arm64 > ~/Library/Application\ Support/Logos/LogosBasecamp/plugins/lp-0002-multisig/variant
tar xzOf app/lp-0002-multisig.lgx manifest.json \
  > ~/Library/Application\ Support/Logos/LogosBasecamp/plugins/lp-0002-multisig/manifest.json
```

On Linux the directory is `~/.local/share/Logos/LogosBasecamp/plugins/`, and the
variant is `linux-amd64`. Restart Basecamp; the module appears in the left rail.

This was run, not assumed. On **LogosBasecamp 0.2.2** (official macOS arm64
release), the package above loads and the surface is usable:

```
App launcher clicked: "lp-0002-multisig"
Loading UI module: "lp-0002-multisig"
MainContainer: Added plugin dock to WorkspaceArea: "lp-0002-multisig"
Successfully loaded UI module: "lp-0002-multisig"
```

Pointing **Multisig folder** at `artifacts/testnet` and pressing **Status**
returns the live deployment's state — `2-of-3`, `2/2 READY TO EXECUTE`, and the
two approval markers — with the `msig` field left empty, because the plugin
resolves the CLI shipped inside the package.

### The `linux-amd64` half, verified on Linux

The Linux variant exists because the evaluator reviews on a Linux VPS, so
shipping it on static checks alone would have been the same inference this
document criticises elsewhere. It is checked against the real thing.

**Basecamp has a Linux build**, which is not obvious: the `logos-co/logos-basecamp`
0.2.2 release carries `LogosBasecamp-Desktop-v0.2.2-d41a72-x86_64.AppImage`
alongside the macOS `.dmg`. Everything below runs in `--platform linux/amd64`
containers on an arm64 Mac — no VM.

Extract it by computing the squashfs offset **from the ELF header**, not by
scanning for the `hsqs` magic, which false-positives inside the payload:

```bash
SHOFF=$(readelf -h "$F" | awk '/Start of section headers/{print $5}')
SHENT=$(readelf -h "$F" | awk '/Size of section headers/{print $5}')
SHNUM=$(readelf -h "$F" | awk '/Number of section headers/{print $5}')
unsquashfs -o $((SHOFF+SHENT*SHNUM)) -d squashfs-root "$F"
```

Basecamp Linux bundles the **same Qt 6.9.2** as macOS, under
`squashfs-root/usr/lib`. The `linux-amd64` plugin is built on Debian bookworm's
Qt 6.4.2, which is under that ceiling.

**The decisive check** reproduces what Basecamp's `PluginLoader` does —
`QPluginLoader::instance()` then `qobject_cast<IComponent*>` — linked against
Basecamp's own Qt rather than the distribution's:

```bash
LD_LIBRARY_PATH=squashfs-root/usr/lib \
QT_PLUGIN_PATH=squashfs-root/usr/lib/qt/plugins \
QT_QPA_PLATFORM=offscreen \
  ./harness lp_0002_multisig.so
```

```
Qt runtime 6.9.2
declared IID: com.logos.component.IComponent
SUCCESS: loaded + cast to IComponent (what Basecamp does)
```

That result is only worth something if the harness can fail, so it was shown to:
one of Basecamp's own Qt platform plugins — a genuine Qt plugin with a different
IID — gives `CAST FAILED`, and a wrong-architecture file gives `LOAD FAILED`.

**Basecamp itself also boots on Linux** with the plugin installed under
`~/.local/share/Logos/LogosBasecamp/plugins/`: `LogosBasecamp version 0.2.2`,
`Logos Core started successfully!`, the three core modules loaded, then
`setUserUiPluginsDirectory` and `getInstalledUiPlugins` against the directory
holding this package. Three things are needed to get that far and each fails
confusingly without: glibc ≥ 2.39, so `ubuntu:24.04` and not `debian:bookworm`
(2.36, which dies with `GLIBC_2.38 not found`); a pty, because Basecamp's
`LogRedirector` throws on a regular-file stdout, so `script -qefc "AppRun" log`;
and a current QEMU (`docker run --privileged tonistiigi/binfmt --install amd64`)
or its core module dies on an emulated `eventfd` with `boost::asio: Bad file
descriptor`.

**What is deliberately not claimed.** A UI app does not appear in the startup
module list — it is a tile in the App Manager, and `Successfully loaded UI
module` is printed on click. Headless, there is nothing to click. So the Linux
evidence is: the load contract passes against Basecamp's own Qt, and Basecamp
boots and scans the directory this package is installed in. The click itself was
exercised on macOS, where the same plugin source and the same interface produce
`Successfully loaded UI module` and a working surface.

Verify the package matches its own manifest:

```bash
python3 scripts/package-lgx.py --verify app/lp-0002-multisig.lgx
```

### Rebuilding the Linux variant

The macOS half builds natively (above). The Linux half builds in Docker, so it
does not depend on the host beyond having Docker:

```bash
./scripts/build-linux-variant.sh                                   # ARCH=arm64 for linux-arm64
python3 scripts/package-lgx.py --add-variant linux-amd64:.linux-variant
```

That produces the ELF plugin and a Linux `msig`, then folds both into the
package. The CLI matters: the bridge shells out to `msig`, so a variant carrying
a Linux plugin next to a macOS binary would load and then fail on the first
button press.

### How it was packaged

**With the real `lgx`** from
[`logos-co/logos-package`](https://github.com/logos-co/logos-package) — the same
tool `nix-bundle-lgx` drives inside the module-builder's Nix shell.
`scripts/package-lgx.py` finds it at `$LGX_BIN`, then
`~/logos/src/logos-package/build/lgx`, then on `PATH`, and calls
`lgx create` / `lgx add`. Metadata is folded into the manifest afterwards,
exactly as `nix-bundle-lgx`'s `bundle.sh` does, because `lgx add` never reads
`metadata.json` and would otherwise leave author, description, type and category
empty.

Read it back with the same tool:

```bash
lgx manifest app/lp-0002-multisig.lgx
lgx extract  app/lp-0002-multisig.lgx --variant darwin-arm64 --output /tmp/x
```

**Without `lgx` on the machine**, the script falls back to writing the package
itself, and that fallback is checked rather than trusted: the manifest hash
scheme is transcribed from `logos-package`'s `src/crypto/signing.cpp`
(`computeDirectoryHash` / `computeParentDirectoryHash`), and the script refuses
to write anything unless the transcription still reproduces the manifest of a
package built by the real tool. One such package ships in this repository — the
committed `.lgx` itself — so the check runs from a clean clone rather than only
on the machine that has the sibling submissions lying around. Both paths were
confirmed to produce the
**identical** root hash `9f5158ed4ade78a2ca9f21b8b60f8392fe2ce47248fdf3f7faa38219e7beaeef`
for this module.

```bash
python3 scripts/package-lgx.py --self-test    # check the fallback transcription
python3 scripts/package-lgx.py --verify app/lp-0002-multisig.lgx
```

## Files

| File | What |
|---|---|
| `metadata.json`, `module.json` | The module manifest, kept in sync at configure time. Basecamp's package manager reads the former; the λPrize validator looks for the latter |
| `src/plugin.{h,cpp}` | The Qt plugin: hosts the QML scene, exposes the bridge |
| `src/multisig_bridge.{h,cpp}` | Shells out to `msig`, passes its human-readable refusals straight through |
| `qml/Main.qml` | The surface |


## What only shows up with two modules installed

**Two modules that both register `qrc:/qml/Main.qml` render each other.** Qt's
resource system is process-global, so whichever registers first wins for both:
with two of these installed together, one tile showed the other module's
UI. Each loaded fine on its own, and `QPluginLoader::load()` was happy in both
cases — the collision is invisible until a second module is present. The resource
prefix is now this module's own name, which cannot collide. Verified in Basecamp
0.2.2 with five modules installed at once, each opened in turn.

A sibling module that talks to a sequencer over `QNetworkAccessManager` has a
second, harsher failure on first click — Qt's macOS proxy lookup asks PCRE2 to
JIT-compile a regex, and Basecamp runs hardened without
`com.apple.security.cs.allow-jit`, so the host dies with `SIGTRAP`. This module
shells out to a CLI and never uses Qt networking, so it is not affected.
