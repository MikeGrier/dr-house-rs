# TTD Debugger Integration

## Overview

The Dr House, TTD extension bundles Microsoft's Time Travel Debugger (TTD) binaries as part of its distribution. These native debugger DLLs are essential for the extension's differential diagnosis and time-travel debugging capabilities.

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
Extract TTD DLLs from each MSIX
    ↓
Organize into crates/dr-house-extension/resources/ttd/{arch}/
    ↓
Package extension VSIX (includes TTD binaries)
```

### Directory Structure

After download, the TTD binaries are organized by platform:

```
crates/dr-house-extension/resources/ttd/
├── x64/
│   ├── TTDReplay.dll        (Time Travel Debugger replay engine)
│   └── TTDReplayCPU.dll     (CPU-specific replay support)
├── x86/
│   ├── TTDReplay.dll
│   └── TTDReplayCPU.dll
└── arm64/
    ├── TTDReplay.dll
    └── TTDReplayCPU.dll
```

### Build Process

1. **Windows x64 Build**:
   - Runs `download-ttd.ps1`
   - Downloads TTD for x64, x86, and arm64 (all in one pass)
   - Extracts to `resources/ttd/`
   - Packages VSIX with all three architectures included

2. **Windows arm64 Build**:
   - Runs `download-ttd.ps1` (downloads all platforms)
   - Same extraction as x64
   - Packages VSIX for arm64 target

3. **Non-Windows Builds** (Linux, macOS):
   - TTD download is skipped (`if: runner.os == 'Windows'`)
   - Extension builds without TTD binaries
   - Useful for basic functionality; full TTD features require Windows

## Scripts

### `download-ttd.ps1`

PowerShell script that automates the TTD download and extraction process.

**Features:**
- Fetches Microsoft's appinstaller metadata
- Parses XML to find the dynamic msixbundle URI
- Downloads the bundle (MSIX container format)
- Extracts three platform-specific MSIX files
- Recursively searches for `TTDReplay.dll` and `TTDReplayCPU.dll`
- Organizes files by architecture (x64, x86, arm64)
- Cleans up temporary files
- Provides colored console output with progress

**Usage:**

```powershell
# Use default output location
.\.github\scripts\download-ttd.ps1

# Specify custom output location
.\.github\scripts\download-ttd.ps1 -OutputPath "./my-ttd-folder"
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
- Downloads all three platform architectures in a single invocation
- Organizes files into the extension resources directory

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

The TTD binaries are **not committed** to the repository:

```
crates/dr-house-extension/resources/ttd/
!crates/dr-house-extension/resources/ttd/.gitkeep
```

The `.gitkeep` file ensures the directory structure is preserved in version control while excluding the large binary files.

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
2. **Manual download** — Run `.\github\scripts\download-ttd.ps1` locally
3. **Check directory** — Verify `crates/dr-house-extension/resources/ttd/` contains extracted files

## Future Enhancements

- [ ] Cache TTD binaries across builds (GitHub Actions cache)
- [ ] Pre-build and distribute TTD binaries as separate package
- [ ] Add Linux/macOS debugging support (with different debuggers)
- [ ] Implement background download for extension updates
