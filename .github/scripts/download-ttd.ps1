#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Downloads and extracts the Windows Time Travel Debugger (TTD) binaries.
    
.DESCRIPTION
    This script fetches the latest TTD package from Microsoft, extracts the
    necessary DLLs for x64, x86, and ARM64 platforms, and organizes them for
    distribution with the Dr House extension.
    
.PARAMETER OutputPath
    The directory where extracted TTD binaries will be placed.
    Default: './crates/dr-house-extension/resources/ttd'
    
.EXAMPLE
    ./download-ttd.ps1
    ./download-ttd.ps1 -OutputPath './my-ttd-folder'
#>

param(
    [string]$OutputPath = './crates/dr-house-extension/resources/ttd'
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
    
    # Step 5: Extract DLLs from each platform's MSIX
    Write-Host "`nStep 5: Extracting DLLs from MSIX files..." -ForegroundColor $infoColor
    
    $platforms = @(
        @{ name = 'x64'; msix = 'TTD-x64.msix' },
        @{ name = 'x86'; msix = 'TTD-x86.msix' },
        @{ name = 'arm64'; msix = 'TTD-ARM64.msix' }
    )
    
    $outputDir = $OutputPath
    if (-not (Test-Path $outputDir)) {
        New-Item -ItemType Directory -Path $outputDir -Force | Out-Null
    }
    
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
        
        # Find and copy the DLLs
        $dllFiles = @('TTDReplay.dll', 'TTDReplayCPU.dll')
        $platformOutputDir = Join-Path $outputDir $platform.name
        New-Item -ItemType Directory -Path $platformOutputDir -Force | Out-Null
        
        foreach ($dll in $dllFiles) {
            $foundDll = Get-ChildItem -Path $msixExtractDir -Filter $dll -Recurse | Select-Object -First 1
            if ($foundDll) {
                Copy-Item -Path $foundDll.FullName -Destination $platformOutputDir -Force
                Write-Host "    Copied $dll" -ForegroundColor $successColor
            } else {
                Write-Host "    Warning: $dll not found in MSIX" -ForegroundColor 'Yellow'
            }
        }
    }
    
    Write-Host "`n✓ TTD binaries successfully downloaded and extracted" -ForegroundColor $successColor
    Write-Host "Output location: $(Resolve-Path $outputDir)" -ForegroundColor $successColor
    
    # Step 6: Cleanup
    Write-Host "`nStep 6: Cleaning up temporary files..." -ForegroundColor $infoColor
    Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    Write-Host "Cleanup complete" -ForegroundColor $successColor
    
} catch {
    Write-Host "`n✗ Error occurred: $_" -ForegroundColor $errorColor
    Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    exit 1
}
