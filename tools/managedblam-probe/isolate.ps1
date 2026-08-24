# Probe each tag in its own process.
#
# Slower than one long-lived process, and the only way to get an honest per-tag
# verdict: the engine accumulates state across loads, and a run of several
# hundred reports crashes on tags that load perfectly well on their own. Sixty
# five of sixty nine "crashers" in one sweep turned out to be that.
param(
    [Parameter(Mandatory = $true)][string]$Kit,
    [Parameter(Mandatory = $true)][string]$WorkList,
    [Parameter(Mandatory = $true)][string]$Results
)
$probe = Join-Path $PSScriptRoot "probe.exe"
$one = Join-Path $env:TEMP "mb_one_$PID.txt"
$oneResult = Join-Path $env:TEMP "mb_one_$PID.result.txt"
if (Test-Path $Results) { [System.IO.File]::Delete($Results) }
$work = @(Get-Content $WorkList | Where-Object { $_.Trim() })
$i = 0
foreach ($line in $work) {
    $i++
    if ($i % 100 -eq 0) { Write-Host "  $i / $($work.Count)" }
    $line | Set-Content $one -Encoding ASCII
    if (Test-Path $oneResult) { [System.IO.File]::Delete($oneResult) }
    & $probe $Kit $one $oneResult *> $null
    if (Test-Path $oneResult) {
        $verdict = Get-Content $oneResult | Select-Object -First 1
        if ($verdict) { Add-Content $Results $verdict } else { Add-Content $Results "DIED`t$line`tno verdict" }
    } else {
        Add-Content $Results "DIED`t$line`tthe probe wrote nothing"
    }
}
"answered $((Get-Content $Results).Count) of $($work.Count)"
