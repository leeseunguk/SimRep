# Windows process hygiene for WSL-launched tools (conhost.exe leaks).
# Usage (from WSL). Run from a /mnt/c cwd (a \\wsl.localhost UNC cwd can hang) with stdin closed:
#   s="$(wslpath -w tools/win/winproc.ps1)"; (cd /mnt/c && timeout 30 powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$s" status < /dev/null)
#   s="$(wslpath -w tools/win/winproc.ps1)"; (cd /mnt/c && timeout 30 powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$s" cleanup < /dev/null)
param([ValidateSet('status', 'cleanup')][string]$Action = 'status')

$all = Get-CimInstance Win32_Process
$byId = @{}
foreach ($p in $all) { $byId[[int]$p.ProcessId] = $p }

$conhosts = foreach ($c in ($all | Where-Object Name -eq 'conhost.exe')) {
    $parent = $byId[[int]$c.ParentProcessId]
    # A PID can be reused; a "parent" created after the child is not the real parent.
    $alive = $parent -and ($parent.CreationDate -le $c.CreationDate)
    [pscustomobject]@{
        Pid        = [int]$c.ProcessId
        ParentPid  = [int]$c.ParentProcessId
        ParentName = if ($alive) { $parent.Name } else { '<gone>' }
        Orphan     = -not $alive
        Started    = $c.CreationDate
    }
}

$watched = 'cargo.exe', 'rustc.exe', 'simion.exe', 'node.exe', 'cmd.exe', 'powershell.exe', 'hh.exe', 'simrep.exe', 'simrep-app.exe'
Write-Output '== process counts =='
$all | Where-Object { $watched -contains $_.Name -or $_.Name -eq 'conhost.exe' } |
    Group-Object Name | Sort-Object Name | Format-Table Name, Count -AutoSize | Out-String | Write-Output
Write-Output '== conhost by parent =='
$conhosts | Group-Object ParentName | Sort-Object Count -Descending |
    Format-Table Name, Count -AutoSize | Out-String | Write-Output

$orphans = @($conhosts | Where-Object Orphan)
Write-Output ("orphan conhost: {0}" -f $orphans.Count)

# A conhost whose parent is alive can still spin at ~100% of a core (seen with
# long-lived wsl.exe sessions). Report them; never kill them automatically.
$cpu = @{}
foreach ($q in (Get-CimInstance Win32_PerfFormattedData_PerfProc_Process | Where-Object { $_.Name -like 'conhost*' })) {
    $cpu[[int]$q.IDProcess] = [int]$q.PercentProcessorTime
}
$busy = @($conhosts | Where-Object { $cpu[$_.Pid] -gt 50 })
Write-Output ("busy conhost (>50% of a core): {0}" -f $busy.Count)
foreach ($b in $busy) {
    $parent = $byId[$b.ParentPid]
    $cmd = if ($parent) { (($parent.CommandLine + '') -replace '\s+', ' ') } else { '' }
    if ($cmd.Length -gt 100) { $cmd = $cmd.Substring(0, 100) }
    Write-Output ("  pid {0} cpu {1}% parent {2} ({3}) {4}" -f $b.Pid, $cpu[$b.Pid], $b.ParentName, $b.ParentPid, $cmd)
}

if ($Action -eq 'cleanup') {
    $killed = 0
    foreach ($o in $orphans) {
        try { Stop-Process -Id $o.Pid -Force -ErrorAction Stop; $killed++ }
        catch { Write-Output ("failed to stop conhost {0}: {1}" -f $o.Pid, $_.Exception.Message) }
    }
    Write-Output ("stopped orphan conhost: {0}" -f $killed)
}
