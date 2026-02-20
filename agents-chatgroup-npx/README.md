# agents-chatgroup

`agents-chatgroup` is a zero-build NPX launcher.

It **does not compile source code** on the user's machine.
Instead, it downloads prebuilt binaries from object storage (OSS preferred, R2 fallback), verifies checksum, extracts, installs, and runs.

## Behavior

When you run:

```bash
npx agents-chatgroup
```

the CLI does:

1. Detect current platform/arch
2. Download prebuilt `agents-chatgroup.zip` from configured object storage
3. Verify SHA256 from manifest
4. Extract to `~/.agents-chatgroup/bin`
5. Add `~/.agents-chatgroup/bin` to PATH (current process + persistent profile)
6. Launch binary immediately

## Commands

- `npx agents-chatgroup` - install (if needed) + run
- `npx agents-chatgroup install` - install only
- `npx agents-chatgroup start [args]` - run binary
- `npx agents-chatgroup update` - force re-download + reinstall
- `npx agents-chatgroup status` - show install status
- `npx agents-chatgroup uninstall` - remove `~/.agents-chatgroup`
- `npx agents-chatgroup --help`

Pass-through args example:

```bash
npx agents-chatgroup -- --port 54321
npx agents-chatgroup start --port 54321
```

## Binary Source

Release workflow injects these values into `bin/download.js` during `npm pack`:

- `__OSS_PUBLIC_URL__` (when OSS is enabled)
- `__R2_PUBLIC_URL__`
- `__BINARY_TAG__`

Runtime fetches:

- `${BASE_URL}/binaries/${BINARY_TAG}/manifest.json`
- `${BASE_URL}/binaries/${BINARY_TAG}/${platform}/agents-chatgroup.zip`

Global latest version check uses:

- `${BASE_URL}/binaries/manifest.json`

Source selection order:

1. `AGENTS_CHATGROUP_OSS_BASE_URL`
2. Injected `__OSS_PUBLIC_URL__`
3. `AGENTS_CHATGROUP_R2_BASE_URL`
4. Injected `__R2_PUBLIC_URL__`

## Installation Paths

- Install root: `~/.agents-chatgroup`
- Binary: `~/.agents-chatgroup/bin/agents-chatgroup` (or `.exe` on Windows)
- Cache: `~/.agents-chatgroup/cache/<tag>/<platform>/`
- Metadata: `~/.agents-chatgroup/install.json`

## Local Dev Mode

If `agents-chatgroup-npx/dist/` exists (or `AGENTS_CHATGROUP_LOCAL=1`), CLI reads binaries from local `dist` instead of remote object storage.

You can also override runtime source manually:

- `AGENTS_CHATGROUP_OSS_BASE_URL`
- `AGENTS_CHATGROUP_R2_BASE_URL`
- `AGENTS_CHATGROUP_BINARY_TAG`

## Requirements

- Node.js >= 18
- Network access to configured OSS/R2 public URL

No Rust/Git/build toolchain is required for end users.
