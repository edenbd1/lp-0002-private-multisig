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

Requires Qt 6 with `Widgets Quick QuickWidgets Qml Concurrent Gui Network OpenGL`.

Verified on macOS with Qt 6.11.1 and CMake 4.1.2:

```bash
cmake -B build -S . -DCMAKE_PREFIX_PATH=$(brew --prefix qt)
cmake --build build
```

produces `build/lp_0002_multisig.dylib` with `metadata.json` and `module.json`
beside it, the QML scene compiled in via `rcc`, and both interface IIDs present
in the binary — `com.networkschool.logos.IComponent/1.0` and
`com.networkschool.lp0002.MultisigPlugin/1.0`.

## Using it

1. Point **Multisig folder** at a directory. If `msig` is not on `PATH`, put its
   full path in the second field.
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

`app/lp-0002-multisig.lgx` (493 KB, SHA-256 `d994384ee73f64c7c30b5cff2993a4613aba78aa23a3f3ff56a853654eb8b45f`) is the packaged module,
carrying the `darwin-arm64` variant: the plugin dylib, the QML view, the module
metadata, and the `msig` CLI the bridge drives. Drop it into Basecamp's
user-plugins directory (`~/Library/Application Support/Logos/LogosBasecampDev/plugins/`
on macOS) to load it.

Verify the package matches its own manifest:

```bash
python3 scripts/package-lgx.py --verify app/lp-0002-multisig.lgx
```

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
to write anything unless the transcription still reproduces the manifests of two
packages built by the real tool. Both paths were confirmed to produce the
**identical** root hash `abc370099b4b205cbdd323383490c00ed5bc0e1101bea8bfe0ca689f38dd9cd0`
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
