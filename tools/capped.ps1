# Runs one command on this laptop with its memory and its time bounded, as CI's Windows jobs bound
# theirs (.github/scripts/job-limits.ps1): the command, and every process it starts, run in a job
# object where each process may commit at most -ProcessGB and all of them together at most -JobGB.
# A process that allocates past its limit has the allocation refused and fails alone, instead of
# taking the machine down (the reset of 2026-11-22: a test binary committed 107 GB and Windows ran
# out of memory). Past -Minutes the job is ended, every process of it with it; and when this script
# ends, however it ends, the job ends every process still in it.
#
#   pwsh -NoProfile -NonInteractive -File tools/capped.ps1 [-ProcessGB n] [-JobGB n] [-Minutes n]
#        [-MenteOnly] -- <command> [argument...]
#
# tools/local-limits.mjs starts it, for tools/quick.mjs, tools/cargo.mjs and tools/mn.mjs, with
# the limits it defines. -MenteOnly bounds the command only when the current directory lies in a
# checkout of this repository, and runs it untouched elsewhere: for this script as cargo's runner
# for every project of the machine.
#
# Its status is the command's; 124 when the command was stopped at its time limit; 127 when no
# program has the command's name; 2 for a call it does not take. It fails closed: a job it cannot
# set up runs no command.
$ErrorActionPreference = 'Stop'

function Write-Note([string] $line) { [Console]::Error.WriteLine("capped: $line") }

$processGB = 8.0
$jobGB = 16.0
$minutes = 30.0
$menteOnly = $false
$k = 0
while ($k -lt $args.Count -and $args[$k] -ne '--') {
  $name = [string]$args[$k]
  if ($name -eq '-MenteOnly') { $menteOnly = $true; $k += 1; continue }
  if ($name -notin @('-ProcessGB', '-JobGB', '-Minutes') -or $k + 1 -ge $args.Count) { Write-Note "no option $name"; exit 2 }
  $value = 0.0
  if (-not [double]::TryParse([string]$args[$k + 1], [Globalization.NumberStyles]::Float, [Globalization.CultureInfo]::InvariantCulture, [ref] $value) -or $value -le 0) {
    Write-Note "$name takes a number above 0, not $($args[$k + 1])"
    exit 2
  }
  switch ($name) {
    '-ProcessGB' { $processGB = $value }
    '-JobGB' { $jobGB = $value }
    '-Minutes' { $minutes = $value }
  }
  $k += 2
}
if ($k + 1 -ge $args.Count) { Write-Note 'usage: capped.ps1 [-ProcessGB n] [-JobGB n] [-Minutes n] [-MenteOnly] -- <command> [argument...]'; exit 2 }
$command = @($args[($k + 1)..($args.Count - 1)] | ForEach-Object { [string]$_ })

# The command's program, found as a shell finds it: a path, or a name on PATH.
function Find-Program([string] $name) {
  if (Test-Path -LiteralPath $name -PathType Leaf) { return (Resolve-Path -LiteralPath $name).Path }
  $found = Get-Command -Name $name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
  if ($found) { return $found.Source }
  Write-Note "no program $name"
  exit 127
}

# Whether the current directory lies in a checkout of this repository: one of it or its parents
# holds Cargo.toml and this very tool.
function Test-InMente {
  $dir = (Get-Location).ProviderPath
  while ($dir) {
    if ((Test-Path -LiteralPath (Join-Path $dir 'Cargo.toml')) -and (Test-Path -LiteralPath (Join-Path $dir 'tools/capped.ps1'))) { return $true }
    $up = Split-Path -Parent $dir
    if ($up -eq $dir) { break }
    $dir = $up
  }
  return $false
}

$program = Find-Program $command[0]
$rest = @($command | Select-Object -Skip 1)
if (-not $IsWindows -or ($menteOnly -and -not (Test-InMente))) {
  if (-not $IsWindows) { Write-Note 'no job object off Windows: the command runs unbounded' }
  & $program @rest
  exit $LASTEXITCODE
}

try {
  Add-Type -Path (Join-Path $PSScriptRoot '../.github/scripts/lib/job-object.cs')
  $job = [JobLimits]::Confine([UInt64]($processGB * 1GB), [UInt64]($jobGB * 1GB))
} catch {
  Write-Note "the job object could not be set up, so $($command[0]) did not run: $($_.Exception.Message)"
  exit 1
}
# The command line the program reads back as the command's arguments; a batch file runs through
# cmd, as Windows runs one, with /s so that cmd keeps the quotes inside.
$line = (@($program) + $rest | ForEach-Object { [JobLimits]::Quote($_) }) -join ' '
$application = $program
if ($program -match '\.(cmd|bat)$') {
  $application = $env:ComSpec
  $line = "$([JobLimits]::Quote($application)) /d /s /c `"$line`""
}
$milliseconds = [uint32][math]::Min([uint32]::MaxValue - 1, [math]::Ceiling($minutes * 60000))
try {
  $status = [JobLimits]::Run($job, $application, $line, $milliseconds)
} catch {
  Write-Note "$($command[0]) could not be started in the job: $($_.Exception.Message)"
  exit 1
}
if ($null -eq $status) {
  Write-Note "stopped $($command[0]) at its limit of $minutes minutes, with every process it started"
  [JobLimits]::Terminate($job, 124)
  exit 124
}
switch ([JobLimits]::Overrun()) {
  'process' { Write-Note "a process of $($command[0]) reached its limit of $processGB GB committed, and its allocation was refused" }
  'job' { Write-Note "the processes of $($command[0]) together reached their limit of $jobGB GB committed, and an allocation was refused" }
}
# A status past a signed int's range, such as an NTSTATUS a crash ends a process with, as the same
# 32 bits.
$code = [long]$status
if ($code -gt [int]::MaxValue) { $code -= 4294967296 }
exit [int]$code
