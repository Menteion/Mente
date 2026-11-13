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

Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;

public static class JobLimits {
  [StructLayout(LayoutKind.Sequential)]
  struct BasicLimitInformation {
    public long PerProcessUserTimeLimit;
    public long PerJobUserTimeLimit;
    public uint LimitFlags;
    public UIntPtr MinimumWorkingSetSize;
    public UIntPtr MaximumWorkingSetSize;
    public uint ActiveProcessLimit;
    public UIntPtr Affinity;
    public uint PriorityClass;
    public uint SchedulingClass;
  }

  [StructLayout(LayoutKind.Sequential)]
  struct IoCounters {
    public ulong ReadOperationCount, WriteOperationCount, OtherOperationCount;
    public ulong ReadTransferCount, WriteTransferCount, OtherTransferCount;
  }

  [StructLayout(LayoutKind.Sequential)]
  struct ExtendedLimitInformation {
    public BasicLimitInformation BasicLimitInformation;
    public IoCounters IoInfo;
    public UIntPtr ProcessMemoryLimit;
    public UIntPtr JobMemoryLimit;
    public UIntPtr PeakProcessMemoryUsed;
    public UIntPtr PeakJobMemoryUsed;
  }

  [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
  static extern IntPtr CreateJobObject(IntPtr attributes, string name);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool SetInformationJobObject(IntPtr job, int infoClass, ref ExtendedLimitInformation info, int length);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool QueryInformationJobObject(IntPtr job, int infoClass, out ExtendedLimitInformation info, int length, IntPtr returned);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern IntPtr OpenProcess(uint access, bool inherit, int processId);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool IsProcessInJob(IntPtr process, IntPtr job, out bool result);

  const int ExtendedLimitInformationClass = 9;
  const uint LimitProcessMemory = 0x100;
  const uint LimitJobMemory = 0x200;
  const uint ProcessTerminate = 0x0001;
  const uint ProcessSetQuota = 0x0100;
  const uint ProcessQueryLimitedInformation = 0x1000;

  static void Check(bool ok, string what) {
    if (!ok) throw new Win32Exception(Marshal.GetLastWin32Error(), what);
  }

  // The job's handle is left open on purpose: the job lives as long as a process is in it.
  public static string Limit(int processId, ulong processBytes, ulong jobBytes) {
    IntPtr job = CreateJobObject(IntPtr.Zero, null);
    Check(job != IntPtr.Zero, "CreateJobObject");
    var info = new ExtendedLimitInformation();
    info.BasicLimitInformation.LimitFlags = LimitProcessMemory | LimitJobMemory;
    info.ProcessMemoryLimit = new UIntPtr(processBytes);
    info.JobMemoryLimit = new UIntPtr(jobBytes);
    int size = Marshal.SizeOf(typeof(ExtendedLimitInformation));
    Check(SetInformationJobObject(job, ExtendedLimitInformationClass, ref info, size), "SetInformationJobObject");
    IntPtr process = OpenProcess(ProcessTerminate | ProcessSetQuota | ProcessQueryLimitedInformation, false, processId);
    Check(process != IntPtr.Zero, "OpenProcess " + processId);
    Check(AssignProcessToJobObject(job, process), "AssignProcessToJobObject " + processId);
    bool inJob;
    Check(IsProcessInJob(process, job, out inJob), "IsProcessInJob");
    if (!inJob) throw new InvalidOperationException("process " + processId + " is not in the job");
    ExtendedLimitInformation read;
    Check(QueryInformationJobObject(job, ExtendedLimitInformationClass, out read, size, IntPtr.Zero), "QueryInformationJobObject");
    uint flags = read.BasicLimitInformation.LimitFlags & (LimitProcessMemory | LimitJobMemory);
    if (flags != (LimitProcessMemory | LimitJobMemory) || read.ProcessMemoryLimit.ToUInt64() != processBytes || read.JobMemoryLimit.ToUInt64() != jobBytes) {
      throw new InvalidOperationException("the job does not hold the limits it was given");
    }
    return "process " + processId + " and every process it starts: at most " + processBytes + " bytes committed each, " + jobBytes + " together";
  }
}
'@

Write-Output ([JobLimits]::Limit($ProcessId, $ProcessBytes, $JobBytes))
