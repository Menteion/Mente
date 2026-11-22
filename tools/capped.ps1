# Runs one command on this laptop with its memory and its time bounded, as CI's Windows jobs bound
# theirs (.github/scripts/job-limits.ps1): the command, and every process it starts, run in a job
# object where each process may commit at most `processGB` of private memory and all of them
# together at most `jobGB`. A process that allocates past its limit has the allocation refused and
# fails alone, instead of taking the machine down (the reset of 2026-11-22: a test binary committed
# 107 GB and Windows ran out of memory). Past `minutes` the job is ended, every process of it with
# it; so it is when `parent`, the process that started this one, ends; and when this script ends,
# however it ends, the job ends every process still in it.
#
#   MENTE_CAPPED='{"command":[...],"processGB":8,"jobGB":16,"minutes":30,"parent":<pid>}'
#   C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe -NoProfile -NonInteractive
#       -ExecutionPolicy Bypass -File tools/capped.ps1
#
# tools/local-limits.mjs starts it so, and nothing else should: the command comes as JSON in one
# variable, never as arguments, which PowerShell's parser would change (the review of 8f03074:
# `--skip=a::b` arrived as two arguments, and `--%` vanished). It runs under Windows PowerShell,
# by its path in System32: the Store's pwsh, started in a job, takes every process it starts out of
# it.
#
# Its status is the command's; 124 when the command was stopped at its time limit; 127 when no
# program has the command's name; 2 for a call it does not take. It fails closed: a job it cannot
# set up runs no command.
$ErrorActionPreference = 'Stop'

function Write-Note([string] $line) { [Console]::Error.WriteLine("capped: $line") }

try {
  $call = $env:MENTE_CAPPED | ConvertFrom-Json
} catch {
  $call = $null
}
$command = @()
if ($null -ne $call -and $null -ne $call.command) { $command = @($call.command | ForEach-Object { [string]$_ }) }
$numbers = @('processGB', 'jobGB', 'minutes', 'parent')
$bad = @($numbers | Where-Object { $null -eq $call -or -not ($call.$_ -is [int] -or $call.$_ -is [long] -or $call.$_ -is [double] -or $call.$_ -is [decimal]) -or [double]$call.$_ -le 0 })
if ($command.Count -eq 0 -or $bad.Count -gt 0) {
  Write-Note "MENTE_CAPPED holds no command with processGB, jobGB, minutes and parent above 0 (tools/local-limits.mjs starts this script)"
  exit 2
}
# The command's own environment holds no call of this script.
Remove-Item Env:MENTE_CAPPED

# The command's program, found as a shell finds it: a path, or a name on PATH.
$name = $command[0]
$program = $null
if (Test-Path -LiteralPath $name -PathType Leaf) {
  $program = (Resolve-Path -LiteralPath $name).Path
} else {
  $found = Get-Command -Name $name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
  if ($found) { $program = $found.Source }
}
if (-not $program) { Write-Note "no program $name"; exit 127 }

try {
  Add-Type -Path (Join-Path $PSScriptRoot '../.github/scripts/lib/job-object.cs')
  $job = [JobLimits]::Confine([UInt64]([double]$call.processGB * 1GB), [UInt64]([double]$call.jobGB * 1GB))
} catch {
  Write-Note "the job object could not be set up, so $name did not run: $($_.Exception.Message)"
  exit 1
}
# The command line the program reads back as the command's arguments; a batch file runs through
# cmd, as Windows runs one, with /s so that cmd keeps the quotes inside.
$rest = @($command | Select-Object -Skip 1)
$line = (@($program) + $rest | ForEach-Object { [JobLimits]::Quote($_) }) -join ' '
$application = $program
if ($program -match '\.(cmd|bat)$') {
  $application = $env:ComSpec
  $line = "$([JobLimits]::Quote($application)) /d /s /c `"$line`""
}
$milliseconds = [uint32][math]::Min([uint32]::MaxValue - 1, [math]::Ceiling([double]$call.minutes * 60000))
$end = ''
try {
  $status = [JobLimits]::Run($job, $application, $line, $milliseconds, [int]$call.parent, [ref] $end)
} catch {
  Write-Note "$name could not be started in the job: $($_.Exception.Message)"
  exit 1
}
if ($end -eq 'time') {
  Write-Note "stopped $name at its limit of $($call.minutes) minutes, with every process it started"
  exit 124
}
if ($end -eq 'parent') {
  Write-Note "stopped $name and every process it started: the process that started this wrapper ended"
  exit 1
}
switch ([JobLimits]::Overrun()) {
  'process' { Write-Note "a process of $name reached its limit of $($call.processGB) GB committed, and its allocation was refused" }
  'job' { Write-Note "the processes of $name together reached their limit of $($call.jobGB) GB committed, and an allocation was refused" }
}
# A status past a signed int's range, such as an NTSTATUS a crash ends a process with, as the same
# 32 bits.
$code = [long]$status
if ($code -gt [int]::MaxValue) { $code -= 4294967296 }
exit [int]$code
