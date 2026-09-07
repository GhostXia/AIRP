# Run with pwsh -NoProfile -File smoke-cleanup.test.ps1 (also Windows PowerShell 5.1).
# On the maintainer machine set TEMP and TMP to D:\AIRP-Dev\target first.
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw 'These regression tests require Windows file-sharing semantics.'
}
. (Join-Path $PSScriptRoot 'smoke-cleanup.ps1')

function Assert-Test([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw "Assertion failed: $Message" }
}

function Get-TestFailure([scriptblock]$Action) {
    try { & $Action | Out-Null } catch { return $_ }
    throw 'Expected a terminating error, but the operation succeeded.'
}

$script:Passed = 0
function Test-Case([string]$Name, [scriptblock]$Action) {
    & $Action
    $script:Passed++
    Write-Host "PASS: $Name"
}

$script:TempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$script:Fixtures = New-Object 'System.Collections.Generic.List[string]'
function New-TestDirectory([string]$Name = ('airp-desktop-smoke-' + [Guid]::NewGuid().ToString('N'))) {
    $path = Join-Path $script:TempRoot $Name
    Assert-Test (-not [IO.Directory]::Exists($path)) 'Fixture must not already exist.'
    [void][IO.Directory]::CreateDirectory($path)
    $script:Fixtures.Add($path)
    return $path
}

# A CLR timer releases the actual exclusive handle while PowerShell is blocked
# inside directory cleanup. No PowerShell runspace or mocked filesystem needed.
if (-not ('AirpSmokeCleanupTests.DelayedHandle' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Threading;
namespace AirpSmokeCleanupTests {
    public sealed class DelayedHandle : IDisposable {
        private FileStream stream;
        private Timer timer;
        public DelayedHandle(string path) {
            stream = new FileStream(path, FileMode.CreateNew, FileAccess.ReadWrite, FileShare.None);
        }
        public void ReleaseAfter(int milliseconds) {
            timer = new Timer(delegate(object unused) { Release(); }, null, milliseconds, Timeout.Infinite);
        }
        private void Release() {
            FileStream owned = Interlocked.Exchange(ref stream, null);
            if (owned != null) owned.Dispose();
        }
        public void Dispose() {
            if (timer != null) timer.Dispose();
            Release();
        }
    }
}
'@
}

try {
    Test-Case 'exclusive handle released later allows cleanup to finish' {
        $path = New-TestDirectory
        $file = Join-Path $path 'locked.txt'
        $handle = New-Object AirpSmokeCleanupTests.DelayedHandle($file)
        try {
            # Prove Windows really refuses deletion before scheduling release.
            $failure = Get-TestFailure { [IO.File]::Delete($file) }
            Assert-Test ([IO.File]::Exists($file)) 'Exclusive lock must prevent deletion.'
            $watch = [Diagnostics.Stopwatch]::StartNew()
            $handle.ReleaseAfter(700)
            Remove-SmokeScratchDirectory -Path $path -TimeoutMilliseconds 5000
            $watch.Stop()
            Assert-Test (-not [IO.Directory]::Exists($path)) 'Directory must be gone after retry.'
            Assert-Test ($watch.ElapsedMilliseconds -ge 500) 'Cleanup must wait for delayed release.'
            Assert-Test ($watch.ElapsedMilliseconds -lt 6500) 'Delayed cleanup must remain bounded.'
        } finally { $handle.Dispose() }
    }

    Test-Case 'permanent exclusive contention throws within its deadline' {
        $path = New-TestDirectory
        $file = Join-Path $path 'locked.txt'
        $handle = [IO.File]::Open($file, [IO.FileMode]::CreateNew, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try {
            $watch = [Diagnostics.Stopwatch]::StartNew()
            $failure = Get-TestFailure { Remove-SmokeScratchDirectory -Path $path -TimeoutMilliseconds 400 }
            $watch.Stop()
            Assert-Test ([IO.File]::Exists($file)) 'Locked file must survive failed cleanup.'
            Assert-Test ($watch.ElapsedMilliseconds -ge 300) 'Contention must be retried, not fail immediately.'
            Assert-Test ($watch.ElapsedMilliseconds -lt 3000) 'Cleanup must honor its bounded timeout.'
        } finally { $handle.Dispose() }
        Remove-SmokeScratchDirectory -Path $path
        Assert-Test (-not [IO.Directory]::Exists($path)) 'Cleanup must work after the lock is released.'
    }

    Test-Case 'cleanup is idempotent for an already absent owned directory' {
        $path = New-TestDirectory
        Remove-SmokeScratchDirectory -Path $path
        Remove-SmokeScratchDirectory -Path $path
        Assert-Test (-not [IO.Directory]::Exists($path)) 'Repeated cleanup must leave directory absent.'
    }

    Test-Case 'unexpected names, nested paths and relative paths preserve sentinel data' {
        $outside = New-TestDirectory ('airp-cleanup-test-' + [Guid]::NewGuid().ToString('N'))
        $nested = Join-Path $outside ('airp-desktop-smoke-' + [Guid]::NewGuid().ToString('N'))
        [void][IO.Directory]::CreateDirectory($nested)
        $uppercase = New-TestDirectory ('airp-desktop-smoke-A' + [Guid]::NewGuid().ToString('N').Substring(1).ToUpperInvariant())
        $short = New-TestDirectory ('airp-desktop-smoke-' + [Guid]::NewGuid().ToString('N').Substring(0, 31))
        foreach ($path in @($outside, $nested, $uppercase, $short)) {
            $sentinel = Join-Path $path 'keep.txt'
            [IO.File]::WriteAllText($sentinel, 'not smoke-owned')
            $failure = Get-TestFailure { Remove-SmokeScratchDirectory -Path $path -TimeoutMilliseconds 100 }
            Assert-Test ([IO.File]::Exists($sentinel)) "Rejected path must survive: $path"
            Assert-Test ([IO.File]::ReadAllText($sentinel) -eq 'not smoke-owned') 'Sentinel contents must be unchanged.'
        }
        $relative = New-TestDirectory
        $sentinel = Join-Path $relative 'keep.txt'
        [IO.File]::WriteAllText($sentinel, 'relative path sentinel')
        Push-Location $script:TempRoot
        try {
            $failure = Get-TestFailure { Remove-SmokeScratchDirectory -Path ([IO.Path]::GetFileName($relative)) }
            Assert-Test ([IO.File]::Exists($sentinel)) 'Relative path must not be accepted even when it resolves to an owned name.'
        } finally { Pop-Location }
    }

    Test-Case 'root junction is rejected without deleting its target' {
        $outside = New-TestDirectory ('airp-cleanup-test-' + [Guid]::NewGuid().ToString('N'))
        $sentinel = Join-Path $outside 'keep.txt'
        [IO.File]::WriteAllText($sentinel, 'junction target sentinel')
        $junction = Join-Path $script:TempRoot ('airp-desktop-smoke-' + [Guid]::NewGuid().ToString('N'))
        Assert-Test (-not (Test-Path -LiteralPath $junction)) 'Junction path must be new.'
        [void](New-Item -ItemType Junction -Path $junction -Target $outside)
        try {
            Assert-Test (((Get-Item -LiteralPath $junction).Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) 'Fixture must be a real junction.'
            $failure = Get-TestFailure { Remove-SmokeScratchDirectory -Path $junction -TimeoutMilliseconds 100 }
            Assert-Test ([IO.File]::Exists($sentinel)) 'Junction target must survive.'
            Assert-Test ([IO.File]::ReadAllText($sentinel) -eq 'junction target sentinel') 'Junction target content must survive.'
        } finally {
            # Nonrecursive Delete removes the link, never traverses the target.
            if ([IO.Directory]::Exists($junction)) { [IO.Directory]::Delete($junction, $false) }
        }
    }

    Test-Case 'owned live process is stopped and waited for; exited process is harmless' {
        $executable = (Get-Process -Id $PID).Path
        $start = New-Object Diagnostics.ProcessStartInfo
        $start.FileName = $executable
        $start.Arguments = '-NoProfile -NonInteractive -Command "[Console]::WriteLine(''ready''); [Threading.Thread]::Sleep(30000)"'
        $start.UseShellExecute = $false
        $start.CreateNoWindow = $true
        $start.RedirectStandardOutput = $true
        $child = [Diagnostics.Process]::Start($start)
        try {
            $ready = $child.StandardOutput.ReadLineAsync()
            Assert-Test ($ready.Wait(10000)) 'Child must report readiness within 10 seconds.'
            Assert-Test ($ready.Result -eq 'ready') 'Child readiness handshake must succeed.'
            Assert-Test (-not $child.HasExited) 'Child must be alive before stop.'
            $watch = [Diagnostics.Stopwatch]::StartNew()
            Stop-SmokeProcess -Process $child -TimeoutMilliseconds 2000
            $watch.Stop()
            Assert-Test $child.HasExited 'Stop must return only after the supplied process exits.'
            Assert-Test ($child.WaitForExit(0)) 'Process exit must already be observable without waiting.'
            Assert-Test ($watch.ElapsedMilliseconds -lt 3500) 'Process shutdown must be bounded.'
            Stop-SmokeProcess -Process $child
        } finally {
            if (-not $child.HasExited) {
                $child.Kill()
                if (-not $child.WaitForExit(5000)) { throw 'Fixture process did not exit.' }
            }
            $child.Dispose()
        }
    }

    Test-Case 'result success is silent and primary ErrorRecord is preserved' {
        Assert-Test (@(Assert-SmokeResult -PrimaryError $null).Count -eq 0) 'Success must produce no pipeline output.'
        try { throw [InvalidOperationException]::new('primary smoke failure') } catch { $primary = $_ }
        $failure = Get-TestFailure { Assert-SmokeResult -PrimaryError $primary }
        # PowerShell may wrap an ErrorRecord on rethrow; its exception and
        # diagnostic fields, not wrapper reference identity, are the contract.
        Assert-Test ($failure.FullyQualifiedErrorId -eq $primary.FullyQualifiedErrorId) 'Primary error ID must be retained.'
        Assert-Test ($failure.CategoryInfo.Category -eq $primary.CategoryInfo.Category) 'Primary error category must be retained.'
        Assert-Test ($failure.ScriptStackTrace -eq $primary.ScriptStackTrace) 'Primary error stack must be retained.'
        Assert-Test ([object]::ReferenceEquals($failure.Exception, $primary.Exception)) 'Original primary exception must be retained.'
    }

    Test-Case 'aggregate retains primary plus every cleanup exception, including cleanup-only failures' {
        try { throw [InvalidOperationException]::new('primary smoke failure') } catch { $primary = $_ }
        $first = [IO.IOException]::new('directory cleanup failure')
        $second = [TimeoutException]::new('process cleanup failure')
        foreach ($primaryError in @($primary, $null)) {
            $failure = Get-TestFailure { Assert-SmokeResult -PrimaryError $primaryError -CleanupErrors @($first, $second) }
            $aggregate = $failure.Exception
            Assert-Test ($aggregate -is [AggregateException]) 'Cleanup failure must throw AggregateException.'
            $expected = @($first, $second)
            if ($null -ne $primaryError) { $expected = @($primaryError.Exception) + $expected }
            Assert-Test ($aggregate.InnerExceptions.Count -eq $expected.Count) 'Aggregate must retain every failure, without extra wrappers.'
            foreach ($exception in $expected) {
                $found = @($aggregate.InnerExceptions | Where-Object { [object]::ReferenceEquals($_, $exception) }).Count
                Assert-Test ($found -eq 1) 'Aggregate must retain each original exception exactly once.'
            }
        }
    }
} finally {
    # Only remove exact fixture paths recorded at creation, never TEMP itself.
    foreach ($path in $script:Fixtures) {
        $fullPath = [IO.Path]::GetFullPath($path)
        $parent = [IO.Path]::GetDirectoryName($fullPath).TrimEnd('\')
        if ($parent -ne $script:TempRoot.TrimEnd('\') -or
            [IO.Path]::GetFileName($fullPath) -notmatch '^airp-(desktop-smoke|cleanup-test)-[a-fA-F0-9]{31,32}$') {
            throw "Unsafe fixture teardown path: $fullPath"
        }
        if ([IO.Directory]::Exists($fullPath)) {
            if (([IO.File]::GetAttributes($fullPath) -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "Refusing recursive fixture teardown of reparse point: $fullPath"
            }
            Remove-Item -LiteralPath $fullPath -Recurse -Force
        }
    }
}
Write-Host "All $script:Passed smoke cleanup regression tests passed."
