// The job object that bounds the memory of a tree of processes on Windows, one definition for CI
// and for the laptop: .github/scripts/job-limits.ps1 puts a Windows job's step shell in one (Limit),
// and tools/capped.ps1 runs one command in one (Confine, Run, Overrun, Terminate). Each process of the
// job may commit at most so many bytes, and all of them together at most so many: a process that
// allocates past its limit has the allocation refused and fails, alone, instead of taking the
// machine down with it.
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

  [StructLayout(LayoutKind.Sequential)]
  struct AssociateCompletionPort {
    public IntPtr CompletionKey;
    public IntPtr CompletionPort;
  }

  [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
  static extern IntPtr CreateJobObject(IntPtr attributes, string name);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool SetInformationJobObject(IntPtr job, int infoClass, ref ExtendedLimitInformation info, int length);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool SetInformationJobObject(IntPtr job, int infoClass, ref AssociateCompletionPort info, int length);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool QueryInformationJobObject(IntPtr job, int infoClass, out ExtendedLimitInformation info, int length, IntPtr returned);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern IntPtr OpenProcess(uint access, bool inherit, int processId);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool IsProcessInJob(IntPtr process, IntPtr job, out bool result);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool TerminateJobObject(IntPtr job, uint exitCode);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern IntPtr CreateIoCompletionPort(IntPtr file, IntPtr existing, UIntPtr key, uint threads);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool GetQueuedCompletionStatus(IntPtr port, out uint message, out UIntPtr key, out IntPtr detail, uint milliseconds);

  const int ExtendedLimitInformationClass = 9;
  const int AssociateCompletionPortClass = 7;
  const uint LimitProcessMemory = 0x100;
  const uint LimitJobMemory = 0x200;
  const uint LimitDieOnUnhandledException = 0x400;
  const uint LimitKillOnJobClose = 0x2000;
  const uint ProcessTerminate = 0x0001;
  const uint ProcessSetQuota = 0x0100;
  const uint ProcessQueryLimitedInformation = 0x1000;
  const uint Synchronize = 0x00100000;
  const uint MessageProcessMemoryLimit = 9;
  const uint MessageJobMemoryLimit = 10;
  static readonly IntPtr InvalidHandle = new IntPtr(-1);

  static void Check(bool ok, string what) {
    if (!ok) throw new Win32Exception(Marshal.GetLastWin32Error(), what);
  }

  // A new job with the two limits and `flags` beside them, read back so that a call that sets
  // nothing fails.
  static IntPtr Create(ulong processBytes, ulong jobBytes, uint flags) {
    IntPtr job = CreateJobObject(IntPtr.Zero, null);
    Check(job != IntPtr.Zero, "CreateJobObject");
    var info = new ExtendedLimitInformation();
    info.BasicLimitInformation.LimitFlags = LimitProcessMemory | LimitJobMemory | flags;
    info.ProcessMemoryLimit = new UIntPtr(processBytes);
    info.JobMemoryLimit = new UIntPtr(jobBytes);
    int size = Marshal.SizeOf(typeof(ExtendedLimitInformation));
    Check(SetInformationJobObject(job, ExtendedLimitInformationClass, ref info, size), "SetInformationJobObject");
    ExtendedLimitInformation read;
    Check(QueryInformationJobObject(job, ExtendedLimitInformationClass, out read, size, IntPtr.Zero), "QueryInformationJobObject");
    uint want = LimitProcessMemory | LimitJobMemory | flags;
    if ((read.BasicLimitInformation.LimitFlags & want) != want || read.ProcessMemoryLimit.ToUInt64() != processBytes || read.JobMemoryLimit.ToUInt64() != jobBytes) {
      throw new InvalidOperationException("the job does not hold the limits it was given");
    }
    return job;
  }

  static void Assign(IntPtr job, IntPtr process, string who) {
    Check(AssignProcessToJobObject(job, process), "AssignProcessToJobObject " + who);
    bool inJob;
    Check(IsProcessInJob(process, job, out inJob), "IsProcessInJob");
    if (!inJob) throw new InvalidOperationException(who + " is not in the job");
  }

  // CI (job-limits.ps1): the process and every process it starts from then on, in a job that
  // outlives the script. The job's handle is left open on purpose: the job lives as long as a
  // process is in it.
  public static string Limit(int processId, ulong processBytes, ulong jobBytes) {
    IntPtr job = Create(processBytes, jobBytes, 0);
    IntPtr process = OpenProcess(ProcessTerminate | ProcessSetQuota | ProcessQueryLimitedInformation, false, processId);
    Check(process != IntPtr.Zero, "OpenProcess " + processId);
    Assign(job, process, "process " + processId);
    return "process " + processId + " and every process it starts: at most " + processBytes + " bytes committed each, " + jobBytes + " together";
  }

  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
  struct StartupInfo {
    public int Size;
    public string Reserved, Desktop, Title;
    public int X, Y, XSize, YSize, XCountChars, YCountChars, FillAttribute, Flags;
    public short ShowWindow, Reserved2;
    public IntPtr Reserved2Bytes, StdInput, StdOutput, StdError;
  }

  [StructLayout(LayoutKind.Sequential)]
  struct ProcessInformation {
    public IntPtr Process, Thread;
    public int ProcessId, ThreadId;
  }

  [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
  static extern bool CreateProcess(string application, System.Text.StringBuilder commandLine, IntPtr processAttributes, IntPtr threadAttributes, bool inheritHandles, uint flags, IntPtr environment, string directory, ref StartupInfo startup, out ProcessInformation information);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern uint ResumeThread(IntPtr thread);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern uint WaitForMultipleObjects(uint count, IntPtr[] handles, bool all, uint milliseconds);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool GetExitCodeProcess(IntPtr process, out uint code);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern IntPtr GetStdHandle(int which);
  [DllImport("kernel32.dll", SetLastError = true)]
  static extern bool CloseHandle(IntPtr handle);

  const uint CreateSuspended = 0x4;
  const uint CreateUnicodeEnvironment = 0x400;
  const int UseStdHandles = 0x100;
  const uint WaitTimeout = 0x102;

  static IntPtr port = IntPtr.Zero;

  // The laptop (capped.ps1): a job that ends every process in it when its one handle, this
  // process's, closes, as it does when this process ends however it ends; an unhandled exception
  // ends a process at once, with no dialog waiting for an answer. The job reports to a port each
  // process that reached its limit (Overrun). This process stays out of it.
  public static IntPtr Confine(ulong processBytes, ulong jobBytes) {
    IntPtr job = Create(processBytes, jobBytes, LimitKillOnJobClose | LimitDieOnUnhandledException);
    port = CreateIoCompletionPort(InvalidHandle, IntPtr.Zero, UIntPtr.Zero, 1);
    Check(port != IntPtr.Zero, "CreateIoCompletionPort");
    var link = new AssociateCompletionPort { CompletionKey = job, CompletionPort = port };
    Check(SetInformationJobObject(job, AssociateCompletionPortClass, ref link, Marshal.SizeOf(typeof(AssociateCompletionPort))), "SetInformationJobObject (completion port)");
    return job;
  }

  // Runs `commandLine` with `application` as its program, in the job from its first instruction:
  // created suspended, put in the job, then let go, so that every process it starts is born in the
  // job, whose processes may not break away from it, even where this process's own job would let
  // them (a shell of the desktop app is in such a job). It has this process's standard handles,
  // directory and environment. Returns its status, with `end` "exit"; or, with every process of the
  // job ended, `end` "time" when it was still running after `milliseconds`, or "parent" when
  // `parent`, the process that started this one, ended first (the review of 8f03074: a killed slot
  // holder left its build running, its slot already free).
  public static uint Run(IntPtr job, string application, string commandLine, uint milliseconds, int parent, out string end) {
    IntPtr watched = OpenProcess(Synchronize, false, parent);
    if (watched == IntPtr.Zero) {
      end = "parent";
      return 1;
    }
    var startup = new StartupInfo();
    startup.Size = Marshal.SizeOf(typeof(StartupInfo));
    startup.Flags = UseStdHandles;
    startup.StdInput = GetStdHandle(-10);
    startup.StdOutput = GetStdHandle(-11);
    startup.StdError = GetStdHandle(-12);
    ProcessInformation info;
    Check(CreateProcess(application, new System.Text.StringBuilder(commandLine), IntPtr.Zero, IntPtr.Zero, true, CreateSuspended | CreateUnicodeEnvironment, IntPtr.Zero, null, ref startup, out info), "CreateProcess " + application);
    try {
      Assign(job, info.Process, "process " + info.ProcessId);
    } catch {
      TerminateJobObject(job, 1);
      throw;
    }
    Check(ResumeThread(info.Thread) != uint.MaxValue, "ResumeThread");
    CloseHandle(info.Thread);
    uint woke = WaitForMultipleObjects(2, new[] { info.Process, watched }, false, milliseconds);
    if (woke == WaitTimeout || woke == 1) {
      end = woke == 1 ? "parent" : "time";
      Check(TerminateJobObject(job, 1), "TerminateJobObject");
      return 1;
    }
    Check(woke == 0, "WaitForMultipleObjects");
    end = "exit";
    uint code;
    Check(GetExitCodeProcess(info.Process, out code), "GetExitCodeProcess");
    CloseHandle(info.Process);
    return code;
  }

  // An argument as the C runtime reads one back from a command line: quoted when it is empty or
  // holds a space, a tab or a quote, each quote and the backslashes before it escaped.
  public static string Quote(string argument) {
    if (argument.Length > 0 && argument.IndexOfAny(new[] { ' ', '\t', '"' }) < 0) return argument;
    var quoted = new System.Text.StringBuilder("\"");
    int slashes = 0;
    foreach (char c in argument) {
      if (c == '\\') { slashes++; continue; }
      if (c == '"') quoted.Append('\\', 2 * slashes + 1);
      else quoted.Append('\\', slashes);
      quoted.Append(c);
      slashes = 0;
    }
    quoted.Append('\\', 2 * slashes);
    return quoted.Append('"').ToString();
  }

  // Which limit a process of the job reached, from the messages the job has posted: "process",
  // "job", or null when none did.
  public static string Overrun() {
    string reached = null;
    uint message;
    UIntPtr key;
    IntPtr detail;
    while (port != IntPtr.Zero && GetQueuedCompletionStatus(port, out message, out key, out detail, 0)) {
      if (message == MessageProcessMemoryLimit && reached == null) reached = "process";
      if (message == MessageJobMemoryLimit) reached = "job";
    }
    return reached;
  }

  // Ends every process of the job.
  public static void Terminate(IntPtr job, uint exitCode) {
    Check(TerminateJobObject(job, exitCode), "TerminateJobObject");
  }
}
