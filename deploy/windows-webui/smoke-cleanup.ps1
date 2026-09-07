# Only pass process objects whose ownership the smoke has already verified.
function Stop-SmokeProcess {
    param(
        [Parameter(Mandatory = $true)][System.Diagnostics.Process]$Process,
        [ValidateRange(0, 60000)][int]$TimeoutMilliseconds = 5000
    )

    if (-not $Process.HasExited) {
        try { $Process.Kill() }
        catch {
            # Exiting between the check and Kill is a successful shutdown.
            if (-not $Process.HasExited) { throw }
        }
    }
    if (-not $Process.WaitForExit($TimeoutMilliseconds)) {
        throw "Owned smoke process $($Process.Id) did not exit within ${TimeoutMilliseconds}ms."
    }
}

# Empty lock contents and a closed HTTP port do not prove every file handle
# has been released. Retry only removal of the exact isolated smoke directory.
function Remove-SmokeScratchDirectory {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [ValidateRange(0, 60000)][int]$TimeoutMilliseconds = 5000
    )

    $scratchPath = [System.IO.Path]::GetFullPath($Path).TrimEnd('\', '/')
    $tempPath = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd('\', '/')
    if (-not [System.IO.Path]::IsPathRooted($Path) -or
        -not [string]::Equals([System.IO.Path]::GetDirectoryName($scratchPath), $tempPath,
            [System.StringComparison]::OrdinalIgnoreCase) -or
        [System.IO.Path]::GetFileName($scratchPath) -cnotmatch '^airp-desktop-smoke-[0-9a-f]{32}$') {
        throw "Refusing to remove unexpected smoke directory: $Path"
    }
    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    while (Test-Path -LiteralPath $scratchPath) {
        $item = Get-Item -LiteralPath $scratchPath -Force -ErrorAction Stop
        if (-not $item.PSIsContainer -or
            ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint)) {
            throw "Refusing to remove non-directory or linked smoke root: $scratchPath"
        }
        try {
            Remove-Item -LiteralPath $scratchPath -Recurse -Force -ErrorAction Stop
            return
        }
        catch {
            if (($_.Exception -isnot [System.IO.IOException] -and
                 $_.Exception -isnot [System.UnauthorizedAccessException]) -or
                $timer.ElapsedMilliseconds -ge $TimeoutMilliseconds) {
                throw
            }
            $remaining = $TimeoutMilliseconds - $timer.ElapsedMilliseconds
            if ($remaining -gt 0) {
                Start-Sleep -Milliseconds ([int][Math]::Min(100, $remaining))
            }
        }
    }
}

# Retain the primary failure and every cleanup failure instead of letting a
# finally-block exception replace the test evidence.
function Assert-SmokeResult {
    param(
        [System.Management.Automation.ErrorRecord]$PrimaryError,
        [System.Exception[]]$CleanupErrors = @()
    )

    if ($CleanupErrors.Count -gt 0) {
        $failures = [System.Collections.Generic.List[System.Exception]]::new()
        if ($PrimaryError) { $failures.Add($PrimaryError.Exception) }
        $failures.AddRange($CleanupErrors)
        $details = ($failures | ForEach-Object { $_.Message }) -join ' | '
        throw [System.AggregateException]::new("Desktop smoke and/or cleanup failed: $details", $failures.ToArray())
    }
    if ($PrimaryError) { throw $PrimaryError }
}
