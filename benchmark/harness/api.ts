// Asking a model through the Claude Messages API over HTTP, with no dependency, for the measured
// benchmark run (benchmark/README.md, "The harness").
//
// The request names one model and no fallback, so every program comes from the model it names
// and a refusal is recorded as one; the broker hands back only an answer that names, as its
// writer, a model the requested one may come back as, and each attempt records that name.
// Thinking is adaptive at each model's default effort, the API version accepts no sampling
// parameters, and the conversation is sent as it was, the model's
// earlier replies unchanged. The system prompt carries a cache breakpoint and the request asks for
// automatic caching of the conversation, 5-minute entries both: the document is the same in every
// run of a language, so each later call reads it from the cache. Caching changes what a call
// costs, never what the model is shown.

import { readFileSync } from "node:fs";
import { anomalyWords, readLimits, thinkingBlocks, usageCost, worstCost as worstOfBody, type Anomaly, type Answer } from "./broker.ts";
import { BrokerRefused } from "./brokered.ts";
import { own } from "../own.mjs";

/**
 * The limits the broker, which holds the key, spends within (benchmark/harness/broker-limits.json):
 * the harness reads its API version, its most output, its cache-write price and its models' prices
 * from the same file, through the broker's own reader, so that the harness's budget and the
 * broker's cannot drift apart.
 */
const LIMITS = readLimits(readFileSync(new URL("./broker-limits.json", import.meta.url), "utf8"));
export const API_VERSION: string = LIMITS.version;
export const MAX_TOKENS: number = LIMITS.maxTokens;
/** The most requests the broker sends at once: a run's concurrency may not pass it. */
export const MAX_IN_FLIGHT: number = LIMITS.maxInFlight;

/** A model and its prices, in US dollars per million tokens. */
export interface Model {
  id: string;
  input: number;
  output: number;
  cacheRead: number;
  /** For a model priced by its prompt's length, the factor every price takes above `above` tokens. */
  longPrompt?: { above: number; factor: number };
}

// The models a run may use, with their prices, from broker-limits.json, whose `about` says where
// they come from. The broker refuses any other.
export const KNOWN_MODELS: Record<string, Model> = Object.fromEntries([...LIMITS.models].map(([id, price]) => [id, { id, ...price }]));

/**
 * Reads a comma-separated list of models: a known id, or `id@input/output/cache-read` with its
 * prices in US dollars per million tokens. An unknown id without prices is an error, so a run
 * never spends at a price it does not know.
 */
export function parseModels(list: string): Model[] {
  return list
    .split(",")
    .map((m) => m.trim())
    .filter((m) => m !== "")
    .map((m) => {
      const priced = /^([\w.-]+)@([\d.]+)\/([\d.]+)\/([\d.]+)$/.exec(m);
      if (priced) return { id: own(priced, 1), input: Number(own(priced, 2)), output: Number(own(priced, 3)), cacheRead: Number(own(priced, 4)) };
      const known = own(KNOWN_MODELS, m);
      if (!known) throw new Error(`no prices for ${m}; name it as ${m}@<input>/<output>/<cache read> in dollars per million tokens`);
      return known;
    });
}

export interface Usage {
  input_tokens: number;
  output_tokens: number;
  cache_creation_input_tokens?: number;
  cache_read_input_tokens?: number;
}

/** What a call cost, in US dollars, as the broker counts it: at its worst when the usage is unreadable. */
export function callCost(usage: Usage, model: Model, worst: number): number {
  return usageCost(usage, model, LIMITS, worst);
}

/** All the input tokens a call was billed for, cached or not. */
export function inputTokens(usage: Usage): number {
  return usage.input_tokens + (usage.cache_creation_input_tokens ?? 0) + (usage.cache_read_input_tokens ?? 0);
}

/**
 * The most a call can cost, as the broker reckons it from the body it is sent: every byte of the
 * body an input token written to the cache, maxTokens more for each thinking block of its earlier
 * replies, and the most output.
 */
export function worstCost(body: object, model: Model): number {
  return worstOfBody(JSON.stringify(body), MAX_TOKENS, model, LIMITS, thinkingBlocks(body));
}

/** The budget of a run of the benchmark, in US dollars. */
export interface Budget {
  limit: number;
  spent: number;
  /** What the calls in flight may still cost at most. */
  reserved: number;
  stopped: boolean;
}

/** Reserves the most a call can cost, or stops the benchmark when that could pass the budget. */
export function reserve(b: Budget, worst: number): boolean {
  if (b.stopped || b.spent + b.reserved + worst > b.limit) {
    b.stopped = true;
    return false;
  }
  b.reserved += worst;
  return true;
}

/** Settles a reservation at what the call cost. */
export function settle(b: Budget, worst: number, cost: number): void {
  b.reserved -= worst;
  b.spent += cost;
}

/** A lost answer is sent again this many times; each try counts as spent at its most. */
export const NETWORK_RETRIES = 3;

/** A call whose answer the run cannot stand on (Anomaly in broker.ts), which fails the run. */
export class RunAnomaly extends Error {
  anomaly: Anomaly;

  constructor(anomaly: Anomaly, message: string) {
    super(message);
    this.anomaly = anomaly;
  }
}

/**
 * One call within the budget, or null when the budget stops it: the harness's own, reckoned as the
 * broker reckons it from the same body, or the broker's limits, which end the run as the budget
 * does. The benchmark, the preliminary run and the syntax probe each make their calls so.
 */
export async function budgetedCall(post: Post, budget: Budget, body: object, model: Model): Promise<{ reply: Reply; cost: number } | null> {
  const worst = worstCost(body, model);
  for (let tries = 0; ; tries++) {
    if (!reserve(budget, worst)) return null;
    let reply: Reply;
    try {
      reply = await send(post, body);
    } catch (e) {
      // A lost answer may still have been billed, so it counts at its most. So does an answer the
      // broker refused as another model's, as the broker counts it. After a refusal for the run's
      // anomaly the budget, like the broker, admits nothing more: the run fails on it.
      const refused = e instanceof BrokerRefused ? e : null;
      settle(budget, worst, e instanceof NetworkError || refused?.billed === true ? worst : 0);
      if (refused?.anomaly) budget.stopped = true;
      if (refused?.exhausted) {
        budget.stopped = true;
        return null;
      }
      if (!(e instanceof NetworkError) || tries >= NETWORK_RETRIES) throw e;
      continue;
    }
    const cost = callCost(reply.usage, model, worst);
    settle(budget, worst, cost);
    // A usage that costs more than the most its request could cannot be honest: it counts, since it
    // was spent, and the run fails on it, its answer unrecorded, as the broker, which closes, has it.
    if (cost > worst) {
      budget.stopped = true;
      throw new RunAnomaly("past its worst", `${anomalyWords("past its worst")}: $${cost}, where the most was $${worst}`);
    }
    return { reply, cost };
  }
}

export type Message = { role: "user" | "assistant"; content: unknown };

export interface Reply {
  /** The model that wrote the answer, as the answer names it: one the requested model may come back as, which the broker checks. */
  model: string;
  content: unknown[];
  text: string;
  stop_reason: string;
  usage: Usage;
}

/**
 * How many thinking blocks a request carried back, in the model's earlier replies among `messages`,
 * and how many its answer held, which the next request carries back, each counted as the broker
 * counts them (thinkingBlocks): so that a record shows whether a run sent the model's thinking back
 * through the broker (the non-core preliminary run of 2026-11-16 could not tell from usage alone).
 */
export function thinkingCarried(messages: Message[], reply: Reply): { sent: number; received: number } {
  return { sent: thinkingBlocks({ messages }), received: thinkingBlocks({ messages: [{ role: "assistant", content: reply.content }] }) };
}

/** The request body of one call. */
export function requestBody(model: string, system: string, messages: Message[]): object {
  return {
    model,
    max_tokens: MAX_TOKENS,
    system: [{ type: "text", text: system, cache_control: { type: "ephemeral" } }],
    cache_control: { type: "ephemeral" },
    messages,
  };
}

/** A call whose answer was lost on the way, which may still have been billed. */
export class NetworkError extends Error {}

/** One request of the Messages API, sent through the broker (benchmark/harness/brokered.ts). */
export type Post = (body: object) => Promise<Answer>;

/**
 * One request, retried on a rate limit or a server error, which are not billed. A connection the
 * broker lost during the call throws NetworkError, so that the caller can count the call as spent.
 */
export async function send(post: Post, body: object): Promise<Reply> {
  for (let attempt = 0; ; attempt++) {
    const answer = await post(body);
    if (answer.status === -1) throw new NetworkError(answer.text);
    if (answer.status >= 200 && answer.status < 300) {
      const parsed = JSON.parse(answer.text);
      if (typeof parsed?.model !== "string") throw new Error("the API answered without naming the model that wrote the answer");
      const content = parsed.content ?? [];
      const text = content
        .filter((b: { type: string }) => b.type === "text")
        .map((b: { text: string }) => b.text)
        .join("\n");
      return { model: parsed.model, content, text, stop_reason: parsed.stop_reason, usage: parsed.usage };
    }
    const retryable = answer.status === 429 || answer.status >= 500;
    if (!retryable || attempt >= 5) {
      throw new Error(`the API answered ${answer.status}: ${answer.text}`);
    }
    const after = Number(answer.retryAfter || "0");
    await new Promise((done) => setTimeout(done, after > 0 ? after * 1000 : 2000 * 2 ** attempt));
  }
}

