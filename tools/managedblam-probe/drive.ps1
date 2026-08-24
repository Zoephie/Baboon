# Run the probe until every line of the work list has a verdict.
#
# A tag that takes the engine down takes the probe with it, so the probe writes
# and flushes one line at a time and this restarts it. The line that never got
# written is the one that killed it, and it is recorded as CRASH before the next
# attempt so the run makes progress instead of looping on it.
param(
    [Parameter(Mandatory = $true)][string]$Kit,
    [Parameter(Mandatory = $true)][string]$WorkList,
    [Parameter(Mandatory = $true)][string]$Results,
    [int]$MaxRestarts = 400
)

$probe = Join-Path $PSScriptRoot "probe.exe"
$work = @(Get-Content $WorkList | Where-Object { $_.Trim() })
if (-not (Test-Path $Results)) { New-Item -ItemType File -Path $Results | Out-Null }

for ($restart = 0; $restart -le $MaxRestarts; $restart++) {
    $answered = @{}
    foreach ($line in Get-Content $Results) {
        $parts = $line -split "`t"
        if ($parts.Count -ge 2) { $answered[$parts[1]] = $true }
    }
    $remaining = @($work | Where-Object { -not $answered.ContainsKey($_) })
    if ($remaining.Count -eq 0) { break }

    # The engine prints its assertion to stdout before it halts, and that
    # is the line that names the field. The crash callback only sees the
    # shell's halt, so the console is where the answer actually is.
    $log = "$Results.console.txt"
    & $probe $Kit $WorkList $Results *>> $log
    $code = $LASTEXITCODE

    # Whatever is still unanswered starts with the one that died.
    $answered = @{}
    foreach ($line in Get-Content $Results) {
        $parts = $line -split "`t"
        if ($parts.Count -ge 2) { $answered[$parts[1]] = $true }
    }
    $still = @($work | Where-Object { -not $answered.ContainsKey($_) })
    if ($still.Count -eq 0) { break }
    if ($still.Count -eq $remaining.Count -and $code -ne 0) {
        Add-Content $Results "DIED`t$($still[0])`tthe probe exited $code without answering"
    } elseif ($still.Count -eq $remaining.Count) {
        Add-Content $Results "DIED`t$($still[0])`tthe probe stopped without answering"
    }
}
"answered $((Get-Content $Results).Count) of $($work.Count)"
