<#
.SYNOPSIS
Run the repository's Rust checks, optionally including the release build.
.EXAMPLE
pwsh -NoProfile -File ./scripts/check.ps1 -Release
#>
[CmdletBinding()]
param(
    [switch]$Release
)

$ErrorActionPreference = 'Stop'

function Invoke-CargoCheck {
    param([string[]]$CargoArguments)

    Write-Host "Running: cargo $($CargoArguments -join ' ')"
    & cargo @CargoArguments
    if ($LASTEXITCODE -ne 0) {
        throw "cargo $($CargoArguments -join ' ') failed with exit code $LASTEXITCODE."
    }
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $repositoryRoot
try {
    Invoke-CargoCheck -CargoArguments @('fmt', '--check')
    Invoke-CargoCheck -CargoArguments @('clippy', '--locked', '--all-targets', '--', '-D', 'warnings')
    Invoke-CargoCheck -CargoArguments @('test', '--locked')

    if ($Release) {
        Invoke-CargoCheck -CargoArguments @('build', '--locked', '--release', '--target', 'x86_64-pc-windows-msvc')
    }
}
finally {
    Pop-Location
}
