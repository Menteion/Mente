# The Windows half of .github/scripts/limits.sh: puts a process in a new job object that bounds
# the memory each of its processes may commit and the memory all of them commit together. Every
# process it starts from then on is in the job too, and stays there after this script exits: a
# test that allocates without end fails its allocation instead of taking the runner down.
#
#   pwsh -NoProfile -NonInteractive -File .github/scripts/job-limits.ps1 -ProcessId <pid>
#        -ProcessBytes <bytes> -JobBytes <bytes>
#
# It reads the limits back from the job and checks that the process is in it before it reports
# them, so a call that sets nothing fails.
param(
  [Parameter(Mandatory = $true)] [int] $ProcessId,
  [Parameter(Mandatory = $true)] [UInt64] $ProcessBytes,
  [Parameter(Mandatory = $true)] [UInt64] $JobBytes
)
$ErrorActionPreference = 'Stop'

# The job object, defined once for CI and for the laptop (tools/capped.ps1).
Add-Type -Path (Join-Path $PSScriptRoot "lib/job-object.cs")

Write-Output ([JobLimits]::Limit($ProcessId, $ProcessBytes, $JobBytes))
