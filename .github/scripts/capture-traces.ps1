# Copyright (c) 2026, Michael Grier
<#
.SYNOPSIS
    Capture TTD .run traces for all crash specimens in the zoo.

.DESCRIPTION
    Must be run elevated (TTD recorder requires admin to inject TTDLoader).
    Invokes extension\resources\ttd\x64\TTD.exe against each release-built
    specimen and writes <name>.run + <name>.out into fixtures\.

.PARAMETER Profile
    'release' (default) or 'debug'. Selects which target\<profile>\*.exe is
    traced.

.PARAMETER Specimens
    Override the default list of specimen exe names.
#>
[CmdletBinding()]
param(
    [ValidateSet('release', 'debug')]
    [string]$Profile = 'release',

    [string[]]$Specimens = @(
        'null-deref-x64',
        'use-after-free-x64',
        'uninit-read-x64',
        'stack-overflow-x64',
        'double-free-x64'
    )
)

$ErrorActionPreference = 'Stop'

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$ttd = Join-Path $repoRoot 'extension\resources\ttd\x64\TTD.exe'
$fixtures = Join-Path $repoRoot 'fixtures'
$targetDir = Join-Path $repoRoot "target\$Profile"

if (-not (Test-Path $ttd)) { throw "TTD recorder not found at $ttd" }
if (-not (Test-Path $fixtures)) { New-Item -ItemType Directory -Path $fixtures | Out-Null }

# Confirm elevation; TTD recorder fails without it.
$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "capture-traces.ps1 must be run elevated."
}

foreach ($name in $Specimens) {
    $exe = Join-Path $targetDir "$name.exe"
    if (-not (Test-Path $exe)) {
        Write-Warning "skipping $name : $exe not built"
        continue
    }
    $runFile = Join-Path $fixtures "$name.run"

    if (Test-Path $runFile) { Remove-Item $runFile -Force }

    Write-Host "=== capturing $name ($Profile) ==="
    # TTD.exe writes its own <name>.out beside the .run; do NOT redirect into
    # that path or we'd hold a lock TTD needs.
    & $ttd -accepteula -out $runFile $exe

    if (Test-Path $runFile) {
        $size = (Get-Item $runFile).Length
        Write-Host ("  -> {0} ({1:N0} bytes)" -f $runFile, $size)
    }
    else {
        Write-Warning "  trace not produced for $name"
    }
}

Write-Host "`nDone."
