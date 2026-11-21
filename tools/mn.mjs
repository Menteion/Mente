#!/usr/bin/env node
// An ad-hoc run of mn, for a probe or a review: it builds this checkout's mn in one of the
// machine's build slots, under the build limits, then runs it under the probe limits
// (tools/local-limits.mjs): each process at most 4 GB committed, all of them at most 8 GB, at most
// 5 minutes unless --minutes says otherwise. The run itself takes no slot. Paths in the arguments
// are read from the current directory.
//
//   node tools/mn.mjs [--minutes N] <mn arguments>
//   node tools/mn.mjs check examples/01_holographic_module.mn

import path from "node:path";
import { fileURLToPath } from "node:url";
import { BUILD, PROBE, acquireSlot, minutesFrom, runCapped } from "./local-limits.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const { args, limits } = minutesFrom(process.argv.slice(2), PROBE);
const slot = acquireSlot("cargo build --bin mn (tools/mn.mjs)");
const built = runCapped(["cargo", "build", "--quiet", "--bin", "mn"], BUILD, { cwd: ROOT });
slot.release();
if (built !== 0) process.exit(built);
const target = process.env.CARGO_TARGET_DIR ? path.resolve(ROOT, process.env.CARGO_TARGET_DIR) : path.join(ROOT, "target");
const mn = path.join(target, "debug", process.platform === "win32" ? "mn.exe" : "mn");
process.exit(runCapped([mn, ...args], limits));
