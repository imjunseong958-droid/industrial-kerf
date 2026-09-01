# Native Cut packaging

`cut` is the packaged desktop composition root. Release packaging should put
the matching `cutd` executable beside it and include `assets/icon.svg` as the
source artwork for platform icon conversion.

Signing credentials and certificates stay in the platform CI or local
keychain. They must not be placed in this repository, project directories,
checkpoints, journals, or runtime error payloads.

The native app resolves the sidecar in this order:

1. `CUTD_BIN` or `CUT_RUNTIME_BIN`.
2. A `cutd` sibling next to the `cut` executable.
3. `target/debug/cutd` or `target/release/cutd` in a checkout.
4. `cutd` on `PATH`.
