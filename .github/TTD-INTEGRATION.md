# TTD Debugger Integration

## Overview

The Morgagni, TTD extension bundles Microsoft's Time Travel Debugger (TTD) binaries as part of its distribution. These native debugger DLLs are essential for the extension's differential diagnosis and time-travel debugging capabilities.

## Architecture

### Download Pipeline

The TTD binaries are downloaded **only during Windows builds** and included in the packaged extension:

```
GitHub Actions (build-extension.yml)
    ↓
Windows Runner (x64 or arm64)
    ↓
.github/scripts/download-ttd.ps1
    ↓
Fetch Microsoft's appinstaller metadata
    ↓
Parse XML to find msixbundle URI
    ↓
Download msixbundle (MSIX container)
    ↓
Extract platform-specific MSIX files
    ↓
    Extract recorder + replay binaries from each MSIX
    ↓
    Organize into extension/resources/ttd/{arch}/
    ↓
    Download Replay API SDK (NuGet: Microsoft.TimeTravelDebugging.Apis)
    ↓
    Extract headers + import libs into extension/resources/ttd-sdk/
    ↓
    Package extension VSIX (includes TTD binaries and SDK)
```

Only **x64** and **arm64** are supported. x86 is intentionally not packaged.

### Directory Structure

After download, the TTD runtime binaries and Replay API SDK are organized as follows:

```
extension/resources/
├── ttd/
│   ├── x64/
│   │   ├── TTD.exe              (recorder CLI)
│   │   ├── TTDInject.exe
│   │   ├── TTDLoader.dll
│   │   ├── TTDRecord.dll
│   │   ├── TTDRecordCPU.dll
│   │   ├── TTDRecordUI.dll
│   │   ├── ProcLaunchMon.sys
│   │   ├── TTDReplay.dll        (replay engine)
│   │   └── TTDReplayCPU.dll     (CPU-specific replay support)
│   └── arm64/
│       └── ... (same set of recorder + replay binaries)
└── ttd-sdk/
    ├── include/                 (Replay API C++ headers)
    └── lib/
        ├── x64/                 (import libs for linking the shim)
        └── arm64/
```

The `ttd-sdk/` directory is what the C++ shim in `morgagni-ttd-decoder-sys` compiles and links against. The `ttd/{arch}/` directories provide the runtime recorder and replay binaries that ship with the extension.

### Build Process

1. **Windows x64 Build**:
   - Runs `download-ttd.ps1`
   - Downloads TTD recorder + replay for x64 and arm64
   - Downloads the Replay API SDK (headers + import libs)
   - Extracts to `extension/resources/ttd/` and `extension/resources/ttd-sdk/`
   - Packages VSIX with both architectures and the SDK included

2. **Windows arm64 Build**:
   - Runs `download-ttd.ps1` (downloads both architectures)
   - Packages VSIX for arm64 target

3. **Non-Windows Builds**:
   - TTD download is skipped (`if: runner.os == 'Windows'`)
   - Only Windows runners are used; this note is kept for reference

## Scripts

### `download-ttd.ps1`

PowerShell script that automates the TTD download and extraction process.

**Features:**
- Fetches Microsoft's appinstaller metadata
- Parses XML to find the dynamic msixbundle URI
- Downloads the bundle (MSIX container format)
- Extracts the x64 and arm64 MSIX files (x86 is not supported)
- Copies the full recorder and replay binary set from each MSIX root (`.dll`, `.exe`, `.sys`), including `TTD.exe`, `TTDInject.exe`, `TTDLoader.dll`, `TTDRecord.dll`, `TTDRecordCPU.dll`, `TTDRecordUI.dll`, `ProcLaunchMon.sys`, `TTDReplay.dll`, and `TTDReplayCPU.dll`
- Organizes runtime files by architecture under `extension/resources/ttd/{x64,arm64}/`
- Downloads the `Microsoft.TimeTravelDebugging.Apis` NuGet package and installs the Replay API SDK (headers + x64/arm64 import libs) under `extension/resources/ttd-sdk/`
- Cleans up temporary files
- Provides colored console output with progress

**Usage:**

```powershell
# Use default output locations
.\.github\scripts\download-ttd.ps1

# Specify custom output locations or a pinned SDK version
.\.github\scripts\download-ttd.ps1 `
    -OutputPath './my-ttd-folder' `
    -SdkOutputPath './my-ttd-sdk' `
    -SdkPackageVersion '0.9.5'
```

### `verify-ttd.sh`

Bash script to verify that TTD binaries are present and accessible.

**Usage:**

```bash
bash ./.github/scripts/verify-ttd.sh
```

## CI/CD Integration

### build-extension.yml

The build workflow includes a TTD download step:

```yaml
- name: Download TTD debuggers (Windows only)
  if: runner.os == 'Windows'
  shell: pwsh
  run: .\.github\scripts\download-ttd.ps1
```

This step:
- Runs only on Windows runners
- Executes before npm dependencies are installed
- Downloads both supported architectures (x64 and arm64) in a single invocation
- Organizes the runtime binaries into `extension/resources/ttd/` and the Replay API SDK into `extension/resources/ttd-sdk/`

### publish-extension.yml

The publish workflow builds the extension (including TTD binaries) and publishes to the VS Code Marketplace.

## Local Development

### Downloading TTD for Local Development

```bash
# PowerShell
.\.github\scripts\download-ttd.ps1

# Or with npm (if configured in package.json)
npm run download-ttd
```

### Verifying Installation

```bash
bash ./.github/scripts/verify-ttd.sh
```

## Repository Management

### .gitignore

The TTD runtime binaries and the Replay API SDK are **not committed** to the repository:

```
extension/resources/ttd/
!extension/resources/ttd/.gitkeep
extension/resources/ttd-sdk/
!extension/resources/ttd-sdk/.gitkeep
```

The `.gitkeep` files ensure the directory structure is preserved in version control while excluding the large binary and SDK files.

### Why Not Commit?

1. **File Size** — TTD DLLs are large (~50-100MB combined)
2. **Licensing** — Microsoft's TTD is licensed and distributed dynamically
3. **Freshness** — Microsoft releases updates; fetching at build time ensures latest versions
4. **Clean History** — Keeps repository size and clone time minimal

## References

- [Microsoft TTD Documentation](https://learn.microsoft.com/en-us/windows-hardware/drivers/debuggercmds/time-travel-debugging-ttd-exe-command-line-util)
- [WinDbg Samples - Get-Ttd.ps1](https://github.com/microsoft/WinDbg-Samples/blob/master/TTD/ReplayApi/GetTtd/Get-Ttd.ps1)
- [MSIX Container Format](https://learn.microsoft.com/en-us/windows/msix/)

## Troubleshooting

### TTD Download Fails

1. **Check network connectivity** — The script needs access to Microsoft's servers
2. **Verify XML parsing** — Ensure `aka.ms/ttd/download` still returns valid appinstaller XML
3. **Check temporary disk space** — At least 500MB required for temporary extraction
4. **Run on Windows** — TTD download only works on Windows runners

### Missing DLLs After Build

1. **Verify the script ran** — Check GitHub Actions logs for download-ttd.ps1 output
2. **Manual download** — Run `.\.github\scripts\download-ttd.ps1` locally
3. **Check directories** — Verify `extension/resources/ttd/{x64,arm64}/` contains the recorder + replay binaries and that `extension/resources/ttd-sdk/{include,lib}/` contains the Replay API SDK

## Future Enhancements

- [ ] Cache TTD binaries across builds (GitHub Actions cache)
- [ ] Pre-build and distribute TTD binaries as separate package
- [ ] Add Linux/macOS debugging support (with different debuggers)
- [ ] Implement background download for extension updates
