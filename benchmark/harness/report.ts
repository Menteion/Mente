// The totals of a measured benchmark run (benchmark/README.md, "Method", "Scoring" and "Failure
// classes"): per set, language and model, the pass rate, the share of runs whose first program
// passes, the median repairs and tokens of solved runs, the pass rate of each run index and its
// spread, and the failure classes of first programs, last programs of unsolved runs, every program
// and the programs that led to a repair; core tasks apart from the others, and adversarial tasks
// as caught at build, caught at run time or missed.

import type { Verdict } from "./build.ts";
import type { Lang } from "./languages.ts";

export interface ProgramClasses {
  /** The failure classes of one program, distinct; empty when it passed what it was run on. */
  classes: string[];
}

export interface RunRecord {
  model: string;
  language: Lang;
  /** The task's id; a held-out task's is replaced by its place in the set. */
  task: string;
  set: "public" | "held-out";
  kind: "solve" | "adversarial";
  core: boolean;
  run: number;
  /** Whether a solve task's last program builds and passes every example and hidden test. */
  solved: boolean | null;
  /** An adversarial task's verdict on its last program. */
  verdict: Verdict | null;
  repairs: number;
  tokens: { input: number; output: number };
  cost: number;
  /** Each program the run submitted, in order; the last as the hidden tests scored it. */
  programs: ProgramClasses[];
}

export interface Stats {
  runs: number;
  solved: number;
  pass_rate: number | null;
  first_program_pass_rate: number | null;
  median_repairs_solved: number | null;
  median_tokens_solved: number | null;
  /** The pass rate of the runs with each index, 1 to the number of runs per task. */
  pass_rate_by_run: (number | null)[];
  /** The population standard deviation of `pass_rate_by_run`. */
  pass_rate_stddev: number | null;
  first_program_failures: Record<string, number>;
  last_program_failures: Record<string, number>;
  every_program_failures: Record<string, number>;
  repairs_by_class: Record<string, number>;
}

export interface Group {
  set: string;
  language: string;
  model: string;
  solve: { all: Stats; core: Stats; non_core: Stats };
  /** Adversarial runs by verdict; a run whose last reply was a refusal has none. */
  adversarial: { runs: number; caught_at_build: number; caught_at_run_time: number; missed: number; refused: number };
}

export function median(values: number[]): number | null {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 1 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

function count(into: Record<string, number>, classes: string[]): void {
  for (const c of classes) into[c] = (into[c] ?? 0) + 1;
}

function rate(part: number, whole: number): number | null {
  return whole === 0 ? null : part / whole;
}

/** The statistics of a set of solve runs. */
export function stats(runs: RunRecord[], runsPerTask: number): Stats {
  const solved = runs.filter((r) => r.solved);
  const byRun = Array.from({ length: runsPerTask }, (_, k) => {
    const these = runs.filter((r) => r.run === k + 1);
    return rate(these.filter((r) => r.solved).length, these.length);
  });
  const known = byRun.filter((v): v is number => v !== null);
  const mean = known.length > 0 ? known.reduce((a, b) => a + b, 0) / known.length : null;
  const s: Stats = {
    runs: runs.length,
    solved: solved.length,
    pass_rate: rate(solved.length, runs.length),
    first_program_pass_rate: rate(solved.filter((r) => r.repairs === 0).length, runs.length),
    median_repairs_solved: median(solved.map((r) => r.repairs)),
    median_tokens_solved: median(solved.map((r) => r.tokens.input + r.tokens.output)),
    pass_rate_by_run: byRun,
    pass_rate_stddev: mean === null ? null : Math.sqrt(known.reduce((a, v) => a + (v - mean) ** 2, 0) / known.length),
    first_program_failures: {},
    last_program_failures: {},
    every_program_failures: {},
    repairs_by_class: {},
  };
  for (const r of runs) {
    if (r.programs.length === 0) continue;
    count(s.first_program_failures, r.programs[0].classes);
    if (!r.solved) count(s.last_program_failures, r.programs[r.programs.length - 1].classes);
    r.programs.forEach((p, i) => {
      count(s.every_program_failures, p.classes);
      if (i < r.programs.length - 1) count(s.repairs_by_class, p.classes);
    });
  }
  return s;
}

/** The totals of every set, language and model. */
export function summarize(runs: RunRecord[], runsPerTask: number): Group[] {
  const keys = [...new Set(runs.map((r) => `${r.set}\u0000${r.language}\u0000${r.model}`))].sort();
  return keys.map((key) => {
    const [set, language, model] = key.split("\u0000");
    const mine = runs.filter((r) => r.set === set && r.language === language && r.model === model);
    const solve = mine.filter((r) => r.kind === "solve");
    const adversarial = mine.filter((r) => r.kind === "adversarial");
    return {
      set,
      language,
      model,
      solve: {
        all: stats(solve, runsPerTask),
        core: stats(
          solve.filter((r) => r.core),
          runsPerTask,
        ),
        non_core: stats(
          solve.filter((r) => !r.core),
          runsPerTask,
        ),
      },
      adversarial: {
        runs: adversarial.length,
        caught_at_build: adversarial.filter((r) => r.verdict === "caught at build").length,
        caught_at_run_time: adversarial.filter((r) => r.verdict === "caught at run time").length,
        missed: adversarial.filter((r) => r.verdict === "missed").length,
        refused: adversarial.filter((r) => r.verdict === null).length,
      },
    };
  });
}
