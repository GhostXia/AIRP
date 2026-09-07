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

# Keep the production shutdown statements, catch/finally and result reporting
# intact. Only startup/recovery/hosting are omitted: this is a lifecycle slice,
# not a packaged UI integration test. UI methods and closed HTTP are simulated;
# engine handles, WaitForExit, empty lock reads and scratch cleanup are real.
function Get-LifecycleSlice {
    param([string]$Mutation = '')
    $tokens = $null
    $errors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseFile($smokeScript, [ref]$tokens, [ref]$errors)
    if ($errors.Count) { throw "Cannot parse smoke script: $errors" }
    $main = @($ast.EndBlock.Statements | Where-Object { $_ -is [System.Management.Automation.Language.TryStatementAst] })
    if ($main.Count -ne 1) { throw 'Expected one main smoke try/finally' }
    $statements = @($main[0].Body.Statements)
    $closures = @()
    foreach ($shell in @('uiProcess', 'secondUiProcess')) {
        $start = @($statements | Where-Object { $_.Extent.Text.StartsWith('if (-not $' + $shell + '.CloseMainWindow())') })
        if ($start.Count -ne 1) { throw "Missing unique $shell closure" }
        $end = @($statements | Where-Object {
            $_.Extent.StartOffset -gt $start[0].Extent.StartOffset -and
            $_.Extent.Text -eq 'Assert-LockHasNoOwner -Path $lock'
        })[0]
        if (-not $end) { throw "Missing $shell lock assertion" }
        $closure = @($statements | Where-Object {
            $_.Extent.StartOffset -ge $start[0].Extent.StartOffset -and
            $_.Extent.EndOffset -le $end.Extent.EndOffset
        })
        if ($Mutation -eq $shell) {
            $wait = @($closure | Where-Object { $_.Extent.Text -match 'ownedEngines.*WaitForExit' })
            if ($wait.Count -ne 1) { throw "Missing unique $shell engine wait for mutation" }
            $closure = @($closure | Where-Object { $_ -ne $wait[0] })
        }
        $closures += ($closure | ForEach-Object { $_.Extent.Text }) -join "`n"
    }
    $lockFunction = $ast.Find({ param($node)
        $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Assert-LockHasNoOwner'
    }, $false).Extent.Text
    $tailStatements = @($ast.EndBlock.Statements | Where-Object {
        $_.Extent.StartOffset -gt $main[0].Extent.EndOffset
    })
    $earlySuccess = ''
    if ($Mutation -eq 'early-success') {
        $success = @($tailStatements | Where-Object { $_.Extent.Text -match '^Write-Host .*Desktop UI smoke passed' })
        if ($success.Count -ne 1) { throw 'Missing unique success statement for mutation' }
        $earlySuccess = $success[0].Extent.Text
        $tailStatements = @($tailStatements | Where-Object { $_ -ne $success[0] })
    }
    $tail = ($tailStatements | ForEach-Object { $_.Extent.Text }) -join "`n"
    return [scriptblock]::Create(@"
$lockFunction
try {
    `$ownedEngines.Add(`$uiProcess.Engine)
    $($closures[0])
    `$ownedEngines.Add(`$secondUiProcess.Engine)
    $($closures[1])
    `$lifecycle.BodyCompleted = `$true
    $earlySuccess
}
$($main[0].CatchClauses.Extent.Text -join "`n")
finally $($main[0].Finally.Extent.Text)
$tail
"@)
}

function Test-Lifecycle {
    param([string]$Scenario, [scriptblock]$Slice)
    . (Join-Path $PSScriptRoot 'smoke-cleanup.ps1')
    $realStop = ${function:Stop-SmokeProcess}
    # Production cleanup has typed Process parameters. Do not weaken them:
    # pass real engine handles through, and skip only our simulated UI objects.
    function Stop-SmokeProcess {
        param($Process)
        if ($Process -is [System.Diagnostics.Process]) { & $realStop -Process $Process }
    }
    function Invoke-RestMethod { param($Uri, $TimeoutSec) throw 'fixture HTTP port closed' }
    $scratchRoot = Join-Path ([IO.Path]::GetTempPath()) ('airp-desktop-smoke-' + [Guid]::NewGuid().ToString('N'))
    $fixtureState.observedScratch = $scratchRoot
    $fixtureState.injectCleanupFailure = $Scenario -eq 'cleanup-only'
    $lock = Join-Path $scratchRoot 'engine-instance.lock'
    $restartEvidence = Join-Path $scratchRoot 'unused.json'
    $Port = 18765
    $primaryError = $null
    $cleanupErrors = [System.Collections.Generic.List[System.Exception]]::new()
    $ownedEngines = [System.Collections.Generic.List[System.Diagnostics.Process]]::new()
    $children = [System.Collections.Generic.List[System.Diagnostics.Process]]::new()
    $lifecycleFailure = $null
    $lifecycle = [pscustomobject]@{ BodyCompleted = $false; PrematureSuccess = $false }
    $messages = [System.Collections.Generic.List[string]]::new()
    $caught = $null
    try {
        New-Item -ItemType Directory -Path $scratchRoot | Out-Null
        New-Item -ItemType File -Path $lock | Out-Null
        $shells = foreach ($number in @(1, 2)) {
            $start = New-Object System.Diagnostics.ProcessStartInfo
            $start.FileName = (Get-Process -Id $PID).Path
            $start.Arguments = '-NoProfile -NonInteractive -Command "[Console]::WriteLine(''ready''); [Threading.Thread]::Sleep(120000)"'
            $start.UseShellExecute = $false
            $start.CreateNoWindow = $true
            $start.RedirectStandardOutput = $true
            $child = [System.Diagnostics.Process]::Start($start)
            $children.Add($child)
            $ready = $child.StandardOutput.ReadLineAsync()
            if (-not $ready.Wait(10000) -or $ready.Result -ne 'ready' -or $child.HasExited) {
                throw 'Lifecycle child did not become ready'
            }
            $shell = [pscustomobject]@{
                Engine = $child; KeepAlive = $Scenario -eq "live-$number"; Closed = $false
            }
            $shell | Add-Member ScriptMethod CloseMainWindow {
                $this.Closed = $true
                if (-not $this.KeepAlive) { $this.Engine.Kill() }
                return $true
            }
            $shell | Add-Member ScriptMethod WaitForExit { param($Milliseconds) return $true }
            $shell
        }
        $uiProcess = $shells[0]
        $secondUiProcess = $shells[1]
        try {
            & $Slice 6>&1 | ForEach-Object {
                $messages.Add([string]$_)
                if ([string]$_ -match 'Desktop UI smoke passed' -and (Test-Path -LiteralPath $scratchRoot)) {
                    $lifecycle.PrematureSuccess = $true
                }
            }
        }
        catch { $caught = $_ }
        if ($lifecycle.PrematureSuccess) { throw 'Lifecycle assertion: success preceded scratch cleanup' }
        if ($Scenario -like 'live-*') {
            $expected = if ($Scenario -eq 'live-1') { 'after UI shutdown' } else { 'after second UI shutdown' }
            if (-not $caught -or $caught.Exception.Message -notmatch "engine sidecar process did not exit $expected") {
                throw "Lifecycle assertion: $Scenario accepted a live engine or failed for the wrong reason: $caught"
            }
            if ($lifecycle.BodyCompleted -or $messages -match 'Desktop UI smoke passed') {
                throw "Lifecycle assertion: $Scenario reached success"
            }
        }
        else {
            if (-not $lifecycle.BodyCompleted -or -not $uiProcess.Closed -or -not $secondUiProcess.Closed) {
                throw "Lifecycle assertion: $Scenario did not complete both closures: $caught"
            }
            if ($Scenario -eq 'cleanup-only') {
                if (-not $caught -or $caught.Exception -isnot [AggregateException] -or
                    $caught.Exception.InnerExceptions.Count -ne 1 -or
                    $caught.Exception.InnerExceptions[0].Message -ne 'fixture cleanup failure' -or
                    $messages -match 'Desktop UI smoke passed') {
                    throw "Lifecycle assertion: cleanup-only failure was not reported correctly: $caught"
                }
            }
            elseif ($caught -or @($messages | Where-Object { $_ -match 'Desktop UI smoke passed' }).Count -ne 1) {
                throw "Lifecycle assertion: successful lifecycle did not report success exactly once: $caught"
            }
        }
        foreach ($engineProcess in $ownedEngines) {
            if (-not $engineProcess.HasExited) { throw 'Lifecycle assertion: owned engine escaped finally cleanup' }
        }
        if ($Scenario -ne 'cleanup-only' -and (Test-Path -LiteralPath $scratchRoot)) {
            throw 'Lifecycle assertion: scratch escaped finally cleanup'
        }
    }
    catch { $lifecycleFailure = $_ }
    finally {
        $teardownErrors = [System.Collections.Generic.List[System.Exception]]::new()
        foreach ($child in $children) {
            try { & $realStop -Process $child }
            catch { $teardownErrors.Add($_.Exception) }
            finally { $child.Dispose() }
        }
        $fixtureState.injectCleanupFailure = $false
        try { Remove-SmokeScratchDirectory -Path $scratchRoot }
        catch { $teardownErrors.Add($_.Exception) }
    }
    Assert-SmokeResult -PrimaryError $lifecycleFailure -CleanupErrors $teardownErrors.ToArray()
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
    $slice = Get-LifecycleSlice
    foreach ($scenario in @('success', 'cleanup-only', 'live-1', 'live-2')) {
        Test-Lifecycle -Scenario $scenario -Slice $slice
        Write-Host "PASS: production lifecycle slice $scenario"
    }
    foreach ($mutation in @('uiProcess', 'secondUiProcess', 'early-success')) {
        $scenario = switch ($mutation) { 'uiProcess' { 'live-1' } 'secondUiProcess' { 'live-2' } default { 'cleanup-only' } }
        $mutant = Get-LifecycleSlice -Mutation $mutation
        $rejected = $null
        try { Test-Lifecycle -Scenario $scenario -Slice $mutant } catch { $rejected = $_ }
        if (-not $rejected -or $rejected.Exception.Message -notmatch 'Lifecycle assertion:') {
            throw "Mutation $mutation was not rejected by a lifecycle assertion: $rejected"
        }
        Write-Host "PASS: mutation $mutation rejected: $($rejected.Exception.Message)"
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
