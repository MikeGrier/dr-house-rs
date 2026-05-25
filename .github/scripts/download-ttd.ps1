#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Downloads and extracts the Windows Time Travel Debugger (TTD) binaries.
    
.DESCRIPTION
    This script fetches the latest TTD package from Microsoft, extracts the
    full recording and replay binaries for x64 and ARM64 platforms, and
    organizes them for distribution with the Morgagni extension.

    Each platform package includes both the recorder (TTD.exe, TTDInject.exe,
    TTDLoader.dll, TTDRecord.dll, TTDRecordCPU.dll, TTDRecordUI.dll,
    ProcLaunchMon.sys) and the replay engine (TTDReplay.dll, TTDReplayCPU.dll).

    x86 is not supported.
    
.PARAMETER OutputPath
    The directory where extracted TTD binaries will be placed.
    Default: './extension/resources/ttd'
    
.EXAMPLE
    ./download-ttd.ps1
    ./download-ttd.ps1 -OutputPath './my-ttd-folder'
#>

param(
    [string]$OutputPath = './extension/resources/ttd',
    [string]$SdkOutputPath = './extension/resources/ttd-sdk',
    [string]$SdkPackageVersion = '0.9.5'
)

$ErrorActionPreference = 'Stop'

# Colors for output
$infoColor = 'Cyan'
$successColor = 'Green'
$errorColor = 'Red'

Write-Host "TTD Debugger Download Script" -ForegroundColor $infoColor
Write-Host "=============================" -ForegroundColor $infoColor

# Create temp directory
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) "ttd-download-$(Get-Random)"
New-Item -ItemType Directory -Path $tempDir -Force | Out-Null
Write-Host "Created temporary directory: $tempDir" -ForegroundColor $infoColor

try {
    # Step 1: Download appinstaller metadata
    Write-Host "`nStep 1: Downloading TTD metadata..." -ForegroundColor $infoColor
    $appInstallerPath = Join-Path $tempDir "ttd.appinstaller"
    $appInstallerUri = "https://aka.ms/ttd/download"
    
    Invoke-WebRequest -Uri $appInstallerUri -OutFile $appInstallerPath -ErrorAction Stop
    Write-Host "Downloaded appinstaller metadata" -ForegroundColor $successColor
    
    # Step 2: Parse XML to extract msixbundle URI
    Write-Host "`nStep 2: Parsing metadata for bundle URI..." -ForegroundColor $infoColor
    [xml]$appInstaller = Get-Content $appInstallerPath
    $msixBundleUri = $appInstaller.AppInstaller.MainBundle.Uri
    
    if (-not $msixBundleUri) {
        throw "Could not find MainBundle/Uri in appinstaller metadata"
    }
    Write-Host "Found bundle URI: $msixBundleUri" -ForegroundColor $successColor
    
    # Step 3: Download msixbundle (which is a ZIP)
    Write-Host "`nStep 3: Downloading TTD bundle..." -ForegroundColor $infoColor
    $bundlePath = Join-Path $tempDir "ttd.zip"
    Invoke-WebRequest -Uri $msixBundleUri -OutFile $bundlePath -ErrorAction Stop
    Write-Host "Downloaded TTD bundle" -ForegroundColor $successColor
    
    # Step 4: Extract MSIX files from bundle
    Write-Host "`nStep 4: Extracting MSIX files..." -ForegroundColor $infoColor
    $bundleExtractDir = Join-Path $tempDir "bundle-extracted"
    New-Item -ItemType Directory -Path $bundleExtractDir -Force | Out-Null
    
    Expand-Archive -Path $bundlePath -DestinationPath $bundleExtractDir -Force
    Write-Host "Extracted bundle contents" -ForegroundColor $successColor
    
    # Step 5: Extract binaries from each platform's MSIX
    Write-Host "`nStep 5: Extracting binaries from MSIX files..." -ForegroundColor $infoColor

    $platforms = @(
        @{ name = 'x64';   msix = 'TTD-x64.msix'   },
        @{ name = 'arm64'; msix = 'TTD-ARM64.msix' }
    )

    $outputDir = $OutputPath
    if (-not (Test-Path $outputDir)) {
        New-Item -ItemType Directory -Path $outputDir -Force | Out-Null
    }

    $binaryExtensions = @('.dll', '.exe', '.sys')

    foreach ($platform in $platforms) {
        Write-Host "  Processing $($platform.name)..." -ForegroundColor $infoColor

        $msixPath = Join-Path $bundleExtractDir $platform.msix
        if (-not (Test-Path $msixPath)) {
            Write-Host "    Warning: $($platform.msix) not found, skipping" -ForegroundColor 'Yellow'
            continue
        }

        $msixExtractDir = Join-Path $tempDir "msix-$($platform.name)"
        New-Item -ItemType Directory -Path $msixExtractDir -Force | Out-Null
        Expand-Archive -Path $msixPath -DestinationPath $msixExtractDir -Force

        $platformOutputDir = Join-Path $outputDir $platform.name
        New-Item -ItemType Directory -Path $platformOutputDir -Force | Out-Null

        # Copy all binaries from the root of the MSIX only (skip x86 subdirectory)
        Get-ChildItem -Path $msixExtractDir -File |
            Where-Object { $binaryExtensions -contains $_.Extension.ToLower() } |
            ForEach-Object {
                Copy-Item -Path $_.FullName -Destination $platformOutputDir -Force
                Write-Host "    Copied $($_.Name)" -ForegroundColor $successColor
            }
    }
    

    # Step 6: Download TTD Replay API SDK (headers + import libs) from NuGet.
    # This is what the C++ shim in morgagni-ttd-decoder-sys compiles and links against.
    Write-Host "`nStep 6: Downloading TTD Replay API SDK (NuGet $SdkPackageVersion)..." -ForegroundColor $infoColor
    $nupkgUri = "https://www.nuget.org/api/v2/package/Microsoft.TimeTravelDebugging.Apis/$SdkPackageVersion"
    $nupkgPath = Join-Path $tempDir 'ttd-apis.nupkg'
    Invoke-WebRequest -Uri $nupkgUri -OutFile $nupkgPath -ErrorAction Stop
    Write-Host "Downloaded NuGet package" -ForegroundColor $successColor

    $sdkExtractDir = Join-Path $tempDir 'ttd-apis-extracted'
    New-Item -ItemType Directory -Path $sdkExtractDir -Force | Out-Null
    Expand-Archive -Path $nupkgPath -DestinationPath $sdkExtractDir -Force

    if (-not (Test-Path $SdkOutputPath)) {
        New-Item -ItemType Directory -Path $SdkOutputPath -Force | Out-Null
    }
    # Headers
    $includeSrc = Join-Path $sdkExtractDir 'sdk\include'
    $includeDst = Join-Path $SdkOutputPath 'include'
    if (Test-Path $includeDst) { Remove-Item $includeDst -Recurse -Force }
    Copy-Item -Path $includeSrc -Destination $includeDst -Recurse -Force
    Write-Host "  Copied headers -> $includeDst" -ForegroundColor $successColor
    # Import libraries (skip x86)
    foreach ($arch in @('x64', 'arm64')) {
        $libSrc = Join-Path $sdkExtractDir "sdk\lib\$arch"
        $libDst = Join-Path $SdkOutputPath "lib\$arch"
        if (Test-Path $libSrc) {
            if (Test-Path $libDst) { Remove-Item $libDst -Recurse -Force }
            Copy-Item -Path $libSrc -Destination $libDst -Recurse -Force
            Write-Host "  Copied $arch import libs -> $libDst" -ForegroundColor $successColor
        }
    }
    Write-Host "✓ SDK installed at $(Resolve-Path $SdkOutputPath)" -ForegroundColor $successColor

    # Step 7st "Output location: $(Resolve-Path $outputDir)" -ForegroundColor $successColor
    
    # Step 6: Cleanup
    Write-Host "`nStep 7: Cleaning up temporary files..." -ForegroundColor $infoColor
    Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    Write-Host "Cleanup complete" -ForegroundColor $successColor
    
} catch {
    Write-Host "`n✗ Error occurred: $_" -ForegroundColor $errorColor
    Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    exit 1
}
