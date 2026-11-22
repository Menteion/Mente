// The Store's PowerShell 7 and the local limits, defined once for the capped runs
// (tools/local-limits.mjs) and the raw-build hook (tools/raw-build.mjs). Started inside a job, the
// Store's pwsh takes every process it starts out of it; Windows PowerShell, by its path in System32,
// does not. Norman, 2026-11-22: the Store's PowerShell 7 stays (7.7 ships no MSI, and winget
// installs the same package), and Mente closes the way out from its side: no capped command is
// pwsh, and no capped PATH holds a WindowsApps directory, where it is found by name. A program that
// starts the Store's package by its full path is a known limit. This module starts no process and
// reads nothing, since the workflow policy loads it.

import path from "node:path";

/// Windows PowerShell, by the path a refusal names.
export const SYSTEM_POWERSHELL = "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe";

// A WindowsApps directory, the App Execution Aliases' or the packages', or a path into one.
const WINDOWS_APPS = /[\\/]windowsapps([\\/]|$)/i;

/// Whether `program` is the Store's PowerShell 7, by its name or by any path into a WindowsApps
/// directory.
export function storePowerShell(program) {
  const name = path.win32.basename(String(program)).toLowerCase();
  return name === "pwsh" || name === "pwsh.exe" || WINDOWS_APPS.test(String(program));
}

/// `env` with every WindowsApps directory taken off its PATH, under whatever case the key has.
export function withoutStoreApps(env) {
  const out = { ...env };
  for (const key of Object.keys(out).filter((k) => k.toLowerCase() === "path")) {
    out[key] = String(out[key])
      .split(";")
      .filter((dir) => !WINDOWS_APPS.test(dir.trim()))
      .join(";");
  }
  return out;
}
