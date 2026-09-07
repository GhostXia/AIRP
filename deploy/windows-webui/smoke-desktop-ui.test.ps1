$ErrorActionPreference = 'Stop'
$smokeScript = Join-Path $PSScriptRoot 'smoke-desktop-ui.ps1'
$fixture = Join-Path ([System.IO.Path]::GetTempPath()) ('airp-smoke-source-test-' + [Guid]::NewGuid().ToString('N'))
$fixtureState = [pscustomobject]@{ injectCleanupFailure = $false; injectCopyFailure = $false; observedScratch = $null }

# Exercise the real script's early-failure/finally path without launching a UI.
function Get-NetTCPConnection { param($LocalPort, $State, $ErrorAction) }
function Start-Process { throw 'fixture primary startup failure' }
function Copy-Item {
    [CmdletBinding()]
    param($LiteralPath, $Destination, [switch]$Recurse)
    $fixtureState.observedScratch = Split-Path $Destination -Parent
    if ($fixtureState.injectCopyFailure) { throw 'fixture primary copy failure' }
    Microsoft.PowerShell.Management\Copy-Item @PSBoundParameters
}
function Remove-Item {
    [CmdletBinding()]
    param($LiteralPath, $Path, [switch]$Recurse, [switch]$Force)
    if ($fixtureState.injectCleanupFailure -and $LiteralPath -eq $fixtureState.observedScratch) {
        throw 'fixture cleanup failure'
    }
    Microsoft.PowerShell.Management\Remove-Item @PSBoundParameters
}

try {
    New-Item -ItemType Directory -Path (Join-Path $fixture 'webui'), (Join-Path $fixture 'data') | Out-Null
    New-Item -ItemType File -Path (Join-Path $fixture 'airp-ui.exe'), (Join-Path $fixture 'airp-core.exe'),
        (Join-Path $fixture 'webui/index.html'), (Join-Path $fixture 'data/keep.txt') | Out-Null
    foreach ($scenario in @('startup', 'copy', 'primary-and-cleanup')) {
        $fixtureState.injectCopyFailure = $scenario -eq 'copy'
        $fixtureState.injectCleanupFailure = $scenario -eq 'primary-and-cleanup'
        $caught = $null
        $messages = [System.Collections.Generic.List[string]]::new()
        try { & $smokeScript -PackageRoot $fixture 6>&1 | ForEach-Object { $messages.Add([string]$_) } }
        catch { $caught = $_ }
        if (-not $caught) { throw "Expected failure for $scenario" }
        if ($messages -match 'Desktop UI smoke passed') { throw 'Success was printed before cleanup completed' }
        $expectedPrimary = if ($fixtureState.injectCopyFailure) { 'copy' } else { 'startup' }
        if ($caught.Exception.Message -notmatch "fixture primary $expectedPrimary failure") {
            throw "Primary failure was lost: $caught"
        }
        if ($fixtureState.injectCleanupFailure) {
            if ($caught.Exception -isnot [System.AggregateException] -or
                $caught.Exception.InnerExceptions.Count -ne 2 -or
                $caught.Exception.Message -notmatch 'fixture cleanup failure') {
                throw "Cleanup failure was lost: $caught"
            }
        }
        elseif (Test-Path -LiteralPath $fixtureState.observedScratch) { throw "Scratch leaked after $scenario" }
        if (-not (Test-Path -LiteralPath (Join-Path $fixture 'data/keep.txt'))) {
            throw 'Source package data was modified'
        }
        $fixtureState.injectCleanupFailure = $false
        . (Join-Path $PSScriptRoot 'smoke-cleanup.ps1')
        Remove-SmokeScratchDirectory -Path $fixtureState.observedScratch
        Write-Host "PASS: real smoke $scenario failure preserves evidence and cleanup scope"
    }
}
finally {
    $fixtureState.injectCleanupFailure = $false
    if ($fixtureState.observedScratch) {
        . (Join-Path $PSScriptRoot 'smoke-cleanup.ps1')
        Remove-SmokeScratchDirectory -Path $fixtureState.observedScratch
    }
    # Exact unique fixture created above; never delete the temp/workspace root.
    if ((Split-Path $fixture -Leaf) -cmatch '^airp-smoke-source-test-[0-9a-f]{32}$' -and
        (Split-Path $fixture -Parent).TrimEnd('\', '/') -eq ([System.IO.Path]::GetTempPath()).TrimEnd('\', '/')) {
        Microsoft.PowerShell.Management\Remove-Item -LiteralPath $fixture -Recurse -Force
    }
}
