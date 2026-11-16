// The broker: the one process of a benchmark run that holds the API key (benchmark/README.md, "The
// harness"; docs/design-review.md, untrusted-test-isolation). The harness that starts builds and
// runs agent code holds no key; it sends each request of the Messages API to the broker over a
// socket, and the broker sends it on with the key and answers with what the API answered.
//
//   env -i PATH="$PATH" node <the flags of benchmark/harness/permission.json> \
//     benchmark/harness/broker.ts --socket <path> --budget <dollars>
//
// benchmark/harness/start-broker.sh starts it so, the key on its standard input, which it reads to
// the end at its start, and in no environment variable. What holds is not what this file imports
// or calls, which process.getBuiltinModule could widen, but two things outside it: Node's
// permission model under those flags, which lets the process start no process, worker, addon or
// WASI, read no file but this one and its limits, write none, and open no node:sqlite; and the
// shape of what it sends. It sends on only a request of the shape the harness makes (api.ts,
// requestBody), cut to that shape and written anew, never the bytes it was given: a model it names,
// max_tokens up to its cap, a system prompt and messages of text alone (and, in the model's own
// earlier replies, the thinking blocks it wrote), and the 5-minute cache, which is all it can price.
// It hands back only an answer that names, as the model that wrote it, one the requested model may
// come back as (answeredAs in broker-limits.json), and refuses any other, so that no result is
// recorded for a model that did not write it. After one, or after an answer billed past the most
// its request could cost, it admits nothing more, and the run fails (Anomaly).
//
// It spends within the run's limits, which it reads at its start from broker-limits.json beside
// it: at most maxRequests requests, at most maxInFlight at once over at most maxConnections
// connections, no body past maxBodyBytes, and nothing whose most cost could take the run past its
// budget or maxDollars, whichever is less. A
// process that reaches its socket can spend no more than that, and it never learns the key: the
// broker takes the key out of every byte it writes. The harness reckons its own budget with the
// same two functions, worstCost and usageCost, so that it stops at its budget before the broker
// refuses; and should the broker refuse first, it says the run's limits are reached, and the
// harness stops as at its budget.
import { readFileSync } from "node:fs";
import net from "node:net";

/**
 * A model's prices, in US dollars per million tokens, and, for a model priced by the length of
 * its prompt, the factor every price takes when the prompt holds more than `above` tokens.
 */
export interface Price {
  input: number;
  output: number;
  cacheRead: number;
  longPrompt?: { above: number; factor: number };
}

/** What a run may spend, from broker-limits.json. */
export interface Limits {
  url: string;
  version: string;
  maxTokens: number;
  maxRequests: number;
  maxInFlight: number;
  maxConnections: number;
  maxBodyBytes: number;
  maxDollars: number;
  cacheWrite: number;
  /** Fields of a usage the broker does not price, each with the one value at which it costs nothing more. */
  standardUsage: Map<string, string>;
  models: Map<string, Price>;
  /** For each model, the names an answer to a request for it may carry in its model field. */
  answeredAs: Map<string, string[]>;
}

/** What the API answered: its status, its retry-after header, and its body as text. */
export interface Answer {
  status: number;
  retryAfter: string;
  text: string;
}

/** One POST to the API. */
export type Post = (url: string, headers: Record<string, string>, body: string) => Promise<Answer>;

/** The one address the broker sends anything to. */
const API_URL = "https://api.anthropic.com/v1/messages";

/** Whether a value is a plain object, not null nor an array. */
const plain = (v: unknown): v is object => v !== null && typeof v === "object" && !Array.isArray(v);

/** The keys of an object beyond those allowed. */
const extraKeys = (o: object, allowed: string[]): string[] => Object.keys(o).filter((k) => !allowed.includes(k));

/** The limits of broker-limits.json, or an error naming what is wrong with them. */
export function readLimits(text: string): Limits {
  const raw = JSON.parse(text);
  const positive = (value: unknown, what: string): number => {
    if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) throw new Error(`broker-limits.json: ${what} is no positive number`);
    return value;
  };
  const whole = (value: unknown, what: string): number => {
    if (!Number.isInteger(positive(value, what))) throw new Error(`broker-limits.json: ${what} is no whole number`);
    return value as number;
  };
  if (raw?.url !== API_URL) throw new Error(`broker-limits.json: url is not ${API_URL}`);
  if (typeof raw.version !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(raw.version)) throw new Error("broker-limits.json: version is no API version");
  // A cache write costs at least an input token, so that pricing every byte of a body as a cache
  // write bounds a request's input, whatever of it is cached.
  const cacheWrite = positive(raw.cacheWrite, "cacheWrite");
  if (cacheWrite < 1) throw new Error("broker-limits.json: cacheWrite is below 1, so a cache write would cost less than an input token");
  const models = new Map<string, Price>();
  const answeredAs = new Map<string, string[]>();
  for (const [id, price] of Object.entries(raw.models ?? {})) {
    const p = price as Price & { answeredAs?: unknown };
    const names = p?.answeredAs;
    if (!Array.isArray(names) || names.length === 0 || !names.every((n) => typeof n === "string" && n !== "") || new Set(names).size !== names.length) {
      throw new Error(`broker-limits.json: ${id}'s answeredAs is no list of the distinct names an answer to it may carry`);
    }
    answeredAs.set(id, names as string[]);
    const read: Price = { input: positive(p?.input, `${id}'s input price`), output: positive(p?.output, `${id}'s output price`), cacheRead: positive(p?.cacheRead, `${id}'s cache-read price`) };
    if (read.cacheRead > read.input * cacheWrite) throw new Error(`broker-limits.json: ${id}'s cache read costs more than a cache write`);
    if (p.longPrompt !== undefined) {
      const factor = positive(p.longPrompt?.factor, `${id}'s long-prompt factor`);
      if (factor < 1) throw new Error(`broker-limits.json: ${id}'s long-prompt factor is below 1`);
      read.longPrompt = { above: whole(p.longPrompt?.above, `${id}'s long-prompt threshold`), factor };
    }
    models.set(id, read);
  }
  if (models.size === 0) throw new Error("broker-limits.json: no model");
  const standardUsage = new Map<string, string>();
  for (const [field, value] of Object.entries(raw.standardUsage ?? {})) {
    if (typeof value !== "string") throw new Error(`broker-limits.json: standardUsage.${field} is no string`);
    standardUsage.set(field, value);
  }
  if (standardUsage.size === 0) throw new Error("broker-limits.json: no standardUsage");
  return {
    url: raw.url,
    version: raw.version,
    maxTokens: whole(raw.maxTokens, "maxTokens"),
    maxRequests: whole(raw.maxRequests, "maxRequests"),
    maxInFlight: whole(raw.maxInFlight, "maxInFlight"),
    maxConnections: whole(raw.maxConnections, "maxConnections"),
    maxBodyBytes: whole(raw.maxBodyBytes, "maxBodyBytes"),
    maxDollars: positive(raw.maxDollars, "maxDollars"),
    cacheWrite,
    standardUsage,
    models,
    answeredAs,
  };
}

/** A text with every occurrence of the key replaced. */
export function scrub(text: string, key: string): string {
  return key === "" ? text : text.split(key).join("[the API key]");
}

/** The most a line of the broker's log may hold, in characters. */
export const LOG_LINE_MAX = 400;

/**
 * The one way a line reaches the broker's log, which the job's last step prints into the Actions
 * log: a template whose words are the broker's own, with every value it did not author written as
 * JSON, a number aside, so that no newline, control character or workflow command a value holds
 * starts a line of its own; with the key taken out; and with at most LOG_LINE_MAX characters.
 *
 *   logLine(key)`the broker cannot listen at ${socket}: ${error}`
 */
export function logLine(key: string): (words: TemplateStringsArray, ...values: unknown[]) => string {
  return (words, ...values) => {
    let text = "";
    for (const [k, w] of words.entries()) {
      text += w;
      if (k < values.length) {
        const value = values.at(k);
        text += typeof value === "number" ? String(value) : JSON.stringify(String(value));
      }
    }
    text = scrub(text, key);
    return text.length > LOG_LINE_MAX ? `${text.slice(0, LOG_LINE_MAX - 3)}...` : text;
  };
}

/** A text's length in UTF-8 bytes. */
export function utf8Length(text: string): number {
  return new TextEncoder().encode(text).length;
}

/** The factor a model's prices take for a prompt of `tokens` tokens: its long-prompt factor above its threshold, else 1. */
const factorFor = (tokens: number, price: Price): number => (price.longPrompt !== undefined && tokens > price.longPrompt.above ? price.longPrompt.factor : 1);

/**
 * The thinking blocks of a request's earlier replies. With preserved thinking each is billed as
 * input at the reasoning it stands for, which its signature's bytes do not bound; it came from one
 * reply, so it stands for at most maxTokens tokens.
 */
export function thinkingBlocks(body: unknown): number {
  if (!plain(body)) return 0;
  const { messages } = body as { messages?: unknown };
  if (!Array.isArray(messages)) return 0;
  let n = 0;
  for (const m of messages) {
    const { role, content } = (plain(m) ? m : {}) as { role?: unknown; content?: unknown };
    if (role !== "assistant" || !Array.isArray(content)) continue;
    for (const b of content) {
      const { type } = (plain(b) ? b : {}) as { type?: unknown };
      if (type === "thinking" || type === "redacted_thinking") n += 1;
    }
  }
  return n;
}

/**
 * The most a request can cost: every byte of its body billed as an input token written to the
 * cache, since no token of text is shorter than a byte, and maxTokens more for each thinking block
 * of its earlier replies, at the prices of a prompt that long, and its most output. A bound, not an
 * estimate, so that what is reserved for the requests in flight covers what they are billed.
 */
export function worstCost(bodyText: string, maxTokens: number, price: Price, limits: Pick<Limits, "cacheWrite" | "maxTokens">, thinking: number): number {
  const input = utf8Length(bodyText) + thinking * limits.maxTokens;
  return (factorFor(input, price) * (input * price.input * limits.cacheWrite + maxTokens * price.output)) / 1e6;
}

/**
 * Why the broker refuses an answer the API billed: the model it names is not one the requested
 * model may come back as (answeredAs in broker-limits.json), or it names none. Null when it is one
 * of them. A result is the work of the model it is recorded for, or no result at all: in the second
 * run with the real key (2026-11-16), Haiku's one answer had Sonnet's text and usage to the token,
 * and nothing read or recorded which model had written it.
 */
export type WrongModel = { answered: string } | { noModel: true } | null;

/** Why the broker refuses an answer from `said`, the model the answer names, to a request for `requested`, or null (WrongModel). */
export function wrongModel(said: unknown, requested: string, limits: Pick<Limits, "answeredAs">): WrongModel {
  if (typeof said !== "string") return { noModel: true };
  return (limits.answeredAs.get(requested) ?? []).includes(said) ? null : { answered: said };
}

/**
 * What makes a run fail and the broker admit nothing more for the rest of it: an answer from another
 * model than the one asked (WrongModel), or an answer whose usage cost more than the most its request
 * could, which no honest usage does (the re-check of 2ead2d4: 200,000 output tokens against a
 * max_tokens of 16,000 spent $0.10 of a $0.05 cap).
 */
export type Anomaly = "another model" | "past its worst";

/**
 * Each anomaly as a run's record and the broker's refusals say it, naming no task. Read by
 * comparison, never by a computed key, which the policy lets only benchmark/own.mjs read and the
 * broker's permission flags leave it no import of.
 */
export function anomalyWords(anomaly: Anomaly): string {
  return anomaly === "another model" ? "an answer came from another model than the one asked" : "an answer was billed past the most its request could cost";
}

/** An anomaly named by a value from elsewhere, a refusal line or an error, or null when it names none. */
export function anomalyOf(value: unknown): Anomaly | null {
  return value === "another model" || value === "past its worst" ? value : null;
}

/** The fields of a usage the broker prices. */
const PRICED =["input_tokens", "output_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"];

/**
 * Whether a field of a usage that the broker does not price costs nothing as the API reported it:
 * empty, the 5-minute cache's breakdown, the share of the output tokens that were thinking, which
 * output_tokens already counts (exactly {thinking_tokens: N}, N a whole number no larger than
 * output_tokens), server tools never used, and the one value of each field of standardUsage in
 * broker-limits.json (the standard tier, speed and place of inference). Anything else, a 1-hour
 * cache write, a web search, fast mode, inference held in one region, or a field the broker does not
 * know, makes it count the request at its worst.
 */
function costsNothing(key: string, value: unknown, standard: Map<string, string>, usage: object): boolean {
  if (value === null || value === 0) return true;
  if (key === "output_tokens_details") {
    if (!plain(value) || extraKeys(value, ["thinking_tokens"]).length > 0) return false;
    const thinking = (value as { thinking_tokens?: unknown }).thinking_tokens;
    const output = (usage as { output_tokens?: unknown }).output_tokens;
    return typeof thinking === "number" && Number.isInteger(thinking) && thinking >= 0 && typeof output === "number" && thinking <= output;
  }
  if (key === "cache_creation") {
    if (!plain(value) || extraKeys(value, ["ephemeral_5m_input_tokens", "ephemeral_1h_input_tokens"]).length > 0) return false;
    const hour = (value as { ephemeral_1h_input_tokens?: unknown }).ephemeral_1h_input_tokens;
    return hour === undefined || hour === null || hour === 0;
  }
  if (key === "server_tool_use") return plain(value) && Object.values(value).every((v) => v === 0 || v === null);
  return standard.has(key) && standard.get(key) === value;
}

/**
 * Why the broker cannot price a usage, which makes it count the request at its worst: the first
 * field it does not know or that the API reported at a value that costs more, whatever its name,
 * the empty one and one named usage among them; or no usage object to read. Null when it can price
 * it all: no name a field can take means that.
 */
export type Unpriced = { field: string } | { noUsage: true } | null;

/** Why the broker cannot price a usage, or null when it can price it all (Unpriced). */
export function unpricedField(usage: unknown, limits: Pick<Limits, "standardUsage">): Unpriced {
  if (!plain(usage)) return { noUsage: true };
  for (const [key, value] of Object.entries(usage)) if (!PRICED.includes(key) && !costsNothing(key, value, limits.standardUsage, usage)) return { field: key };
  return null;
}

/** What a request cost by the usage the API reported, or its worst case when the broker cannot price all of it. */
export function usageCost(usage: unknown, price: Price, limits: Pick<Limits, "cacheWrite" | "standardUsage">, worst: number): number {
  if (unpricedField(usage, limits) !== null) return worst;
  const cacheWrite = limits.cacheWrite;
  const u = usage as { input_tokens?: unknown; output_tokens?: unknown; cache_creation_input_tokens?: unknown; cache_read_input_tokens?: unknown };
  const n = (v: unknown) => (typeof v === "number" && Number.isFinite(v) && v >= 0 ? v : Number.NaN);
  const input = n(u.input_tokens);
  const written = n(u.cache_creation_input_tokens ?? 0);
  const read = n(u.cache_read_input_tokens ?? 0);
  const output = n(u.output_tokens);
  // A prompt's length counts every input token, cached or not.
  const total = (factorFor(input + written + read, price) * (input * price.input + written * price.input * cacheWrite + read * price.cacheRead + output * price.output)) / 1e6;
  return Number.isFinite(total) ? total : worst;
}

/** The one cache the broker prices, the 5-minute one. */
const isEphemeral = (v: unknown): boolean => plain(v) && extraKeys(v, ["type"]).length === 0 && (v as { type?: unknown }).type === "ephemeral";

/** A system prompt of text, written anew, or null. */
function shapeSystem(system: unknown): unknown {
  if (typeof system === "string") return system;
  if (!Array.isArray(system)) return null;
  const blocks: object[] = [];
  for (const b of system) {
    if (!plain(b) || extraKeys(b, ["type", "text", "cache_control"]).length > 0) return null;
    const { type, text, cache_control } = b as { type?: unknown; text?: unknown; cache_control?: unknown };
    if (type !== "text" || typeof text !== "string") return null;
    if (cache_control !== undefined && !isEphemeral(cache_control)) return null;
    blocks.push(cache_control === undefined ? { type: "text", text } : { type: "text", text, cache_control: { type: "ephemeral" } });
  }
  return blocks;
}

/**
 * A block of a message, written anew, or why it is not one the harness sends: text in either
 * role, and in the model's own earlier replies the thinking blocks it wrote, sent back unchanged.
 * A reply's text block may carry `citations: null`, which goes; nothing else does.
 */
function shapeBlock(role: string, block: unknown): object | string {
  if (!plain(block)) return "a block that is no object";
  const { type } = block as { type?: unknown };
  if (type === "text") {
    const extra = extraKeys(block, ["type", "text", "citations"]);
    const { text, citations } = block as { text?: unknown; citations?: unknown };
    if (extra.length > 0 || typeof text !== "string" || (citations !== undefined && citations !== null) || (role === "user" && citations !== undefined)) return "a text block with more than its text";
    return { type: "text", text };
  }
  if (role !== "assistant") return `a user block of type ${String(type)}, which the harness never sends`;
  if (type === "thinking") {
    const { thinking, signature } = block as { thinking?: unknown; signature?: unknown };
    if (extraKeys(block, ["type", "thinking", "signature"]).length > 0 || typeof thinking !== "string" || typeof signature !== "string") return "a thinking block of another shape";
    return { type: "thinking", thinking, signature };
  }
  if (type === "redacted_thinking") {
    const { data } = block as { data?: unknown };
    if (extraKeys(block, ["type", "data"]).length > 0 || typeof data !== "string") return "a redacted thinking block of another shape";
    return { type: "redacted_thinking", data };
  }
  return `an assistant block of type ${String(type)}, which the harness never sends`;
}

/** A message, written anew, or why it is not one the harness sends. */
function shapeMessage(message: unknown): object | string {
  if (!plain(message) || extraKeys(message, ["role", "content"]).length > 0) return "a message with more than its role and content";
  const { role, content } = message as { role?: unknown; content?: unknown };
  if (role !== "user" && role !== "assistant") return `the role ${String(role)}`;
  if (typeof content === "string") return { role, content };
  if (!Array.isArray(content) || content.length === 0) return "no content";
  const blocks: object[] = [];
  for (const b of content) {
    const shaped = shapeBlock(role, b);
    if (typeof shaped === "string") return shaped;
    blocks.push(shaped);
  }
  return { role, content: blocks };
}

/** The keys a request may hold, in the order requestBody writes them. */
const REQUEST_KEYS = ["model", "max_tokens", "system", "cache_control", "messages"];

/** A request cut to the shape the harness sends, and written anew; or why it is not that shape. */
export type Shaped = { text: string; model: string; maxTokens: number; thinking: number } | { refused: string };

/**
 * The request the broker sends for a body it was given: the shape requestBody makes (api.ts),
 * every value checked and written anew in that shape, so that nothing it does not price reaches
 * the API, not fast mode, a 1-hour cache, a tool, a document or image by URL, nor any field it
 * does not know.
 */
export function shape(body: unknown, limits: Limits): Shaped {
  if (!plain(body)) return { refused: "a request with no body" };
  const extra = extraKeys(body, REQUEST_KEYS);
  if (extra.length > 0) return { refused: `a request with ${extra.join(", ")}, which the harness never sends` };
  const { model, max_tokens, system, cache_control, messages } = body as { model?: unknown; max_tokens?: unknown; system?: unknown; cache_control?: unknown; messages?: unknown };
  if (typeof model !== "string" || !limits.models.has(model)) return { refused: `the model ${String(model)}, which broker-limits.json does not name` };
  if (typeof max_tokens !== "number" || !Number.isInteger(max_tokens) || max_tokens < 1 || max_tokens > limits.maxTokens) return { refused: `max_tokens ${String(max_tokens)}, beyond 1 to ${limits.maxTokens}` };
  if (cache_control !== undefined && !isEphemeral(cache_control)) return { refused: 'a cache_control other than {"type": "ephemeral"}, the 5-minute cache the broker prices' };
  const sys = system === undefined ? undefined : shapeSystem(system);
  if (sys === null) return { refused: "a system prompt that is not text" };
  if (!Array.isArray(messages) || messages.length === 0) return { refused: "a request with no messages" };
  const shapedMessages: object[] = [];
  for (const [k, m] of messages.entries()) {
    const shaped = shapeMessage(m);
    if (typeof shaped === "string") return { refused: `message ${k}: ${shaped}` };
    shapedMessages.push(shaped);
  }
  const request = {
    model,
    max_tokens,
    ...(sys === undefined ? {} : { system: sys }),
    ...(cache_control === undefined ? {} : { cache_control: { type: "ephemeral" } }),
    messages: shapedMessages,
  };
  return { text: JSON.stringify(request), model, maxTokens: max_tokens, thinking: thinkingBlocks(request) };
}

/**
 * Splits a stream of text into lines, calling `line` for each that is not blank, and `tooLong`
 * once, and nothing more, when a line runs past `max` characters, so that a line with no end never
 * fills the broker's memory.
 */
export function lineReader(max: number, line: (text: string) => void, tooLong: () => void): (chunk: string) => void {
  let pending = "";
  let over = false;
  return (chunk) => {
    if (over) return;
    pending += chunk;
    const stop = () => {
      over = true;
      pending = "";
      tooLong();
    };
    for (let at = pending.indexOf("\n"); at >= 0; at = pending.indexOf("\n")) {
      if (at > max) {
        stop();
        return;
      }
      const text = pending.slice(0, at);
      pending = pending.slice(at + 1);
      if (text.trim() !== "") line(text);
    }
    if (pending.length > max) stop();
  };
}

/**
 * The broker's state and its one operation: a request line in, an answer line out. It holds the
 * key, the limits and what the run has spent, reserved and has in flight, and sends each request
 * through `post`.
 */
export class Broker {
  key: string;
  limits: Limits;
  post: Post;
  /** The run's dollar cap: its budget or the limits' maxDollars, whichever is less. */
  cap: number;
  spent = 0;
  /** What the requests in flight may still cost at most. */
  reserved = 0;
  requests = 0;
  inFlight = 0;
  /** The run's first anomaly, after which the broker admits nothing more, or null while there is none. */
  closed: Anomaly | null = null;

  /**
   * Where the broker says, once a run each, what it could not price and each anomaly it met: its
   * log, by default, each line made by logLine.
   */
  note: (line: string) => void;
  /** What it has said, so that each is said once a run. */
  noted = new Set<string>();

  constructor(key: string, limits: Limits, budget: number, post: Post, note: (line: string) => void = (line) => console.error(line)) {
    // A key of these characters alone is written in JSON as itself, so that taking it out of an
    // answer line, which is JSON, takes it out of every value in it.
    if (!/^[A-Za-z0-9_-]{8,}$/.test(key)) throw new Error("no API key on the broker's standard input: one line of letters, digits, '-' and '_'");
    if (!(budget > 0) || !Number.isFinite(budget)) throw new Error("no budget: pass --budget <dollars>");
    this.key = key;
    this.limits = limits;
    this.post = post;
    this.cap = Math.min(budget, limits.maxDollars);
    this.note = note;
  }

  /** Says a line in the broker's log once a run: `what` names it among the things said. */
  noteOnce(what: unknown, line: string): void {
    const said = JSON.stringify(what);
    if (this.noted.has(said)) return;
    this.noted.add(said);
    this.note(line);
  }

  /** What an answered request cost, by the usage the API reported; at its most when the broker cannot price it. */
  cost(usage: unknown, price: Price, worst: number): number {
    // A field it cannot price makes it count the request at its worst, and its log says which, once
    // a field, so that a run that stops early shows why (the first run with the real key, on
    // 2026-11-16: output_tokens_details, which the broker did not then know).
    const unpriced = unpricedField(usage, this.limits);
    if (unpriced !== null) {
      const line = logLine(this.key);
      this.noteOnce(unpriced, "field" in unpriced ? line`the broker counts a request at its worst: it cannot price the usage field ${unpriced.field}` : line`the broker counts a request at its worst: it cannot price an answer without a usage object`);
    }
    return usageCost(usage, price, this.limits, worst);
  }

  /** The answer to one request line, as one line, with the key taken out of it. */
  async handle(line: string): Promise<string> {
    let id: unknown = null;
    const answer = (fields: object) => scrub(JSON.stringify({ id, ...fields }), this.key);
    let request;
    try {
      request = JSON.parse(line);
    } catch {
      return answer({ refused: "a request the broker cannot read" });
    }
    if (typeof request?.id === "number" || typeof request?.id === "string") id = request.id;
    // After the run's first anomaly nothing more is sent; the refusal names that anomaly, so that a
    // run whose requests race it fails for it, whichever answer the harness reads first.
    if (this.closed !== null) return answer({ refused: `the broker admits nothing more this run: ${anomalyWords(this.closed)}`, anomaly: this.closed });
    const shaped = shape(request?.body, this.limits);
    if ("refused" in shaped) return answer({ refused: shaped.refused });
    const bytes = utf8Length(shaped.text);
    if (bytes > this.limits.maxBodyBytes) return answer({ refused: `a body of ${bytes} bytes, past the ${this.limits.maxBodyBytes} of broker-limits.json` });
    if (this.inFlight >= this.limits.maxInFlight) return answer({ refused: `a request past the ${this.limits.maxInFlight} the broker sends at once` });
    const price = this.limits.models.get(shaped.model) as Price;
    const worst = worstCost(shaped.text, shaped.maxTokens, price, this.limits, shaped.thinking);
    // The run's limits reached: the answer says so, and the harness stops as at its budget.
    if (this.requests >= this.limits.maxRequests) return answer({ refused: `a request past the run's ${this.limits.maxRequests}`, exhausted: true });
    if (this.spent + this.reserved + worst > this.cap) return answer({ refused: `a request that could take the run past its $${this.cap}`, exhausted: true });
    this.requests += 1;
    this.reserved += worst;
    this.inFlight += 1;
    let reply: Answer;
    try {
      reply = await this.post(this.limits.url, { "content-type": "application/json", "x-api-key": this.key, "anthropic-version": this.limits.version }, shaped.text);
    } catch (e) {
      // A lost answer may still have been billed, so it counts at its most.
      this.reserved -= worst;
      this.spent += worst;
      return answer({ status: -1, retryAfter: "", text: `the request failed: ${String(e)}` });
    } finally {
      this.inFlight -= 1;
    }
    this.reserved -= worst;
    // The API bills no answer but a success.
    if (reply.status < 200 || reply.status >= 300) return answer({ status: reply.status, retryAfter: reply.retryAfter, text: reply.text });
    let parsed: unknown;
    try {
      parsed = JSON.parse(reply.text);
    } catch {
      parsed = undefined;
    }
    const { model, usage } = (plain(parsed) ? parsed : {}) as { model?: unknown; usage?: unknown };
    // An answer from a model other than the one asked never reaches the harness: it would record
    // that model's work as the other's. It was billed, and counts at the requested model's worst,
    // which admission reserved, so that the run's cap holds: a mismatch makes the answer unusable,
    // and is no evidence that the API billed it otherwise (the review of b520209: counted at any
    // model's most, a Haiku request answered as Sonnet took $1.05 of a $0.05 cap). The run fails on
    // it, so the broker admits nothing more, and what such answers cost stops at what is in flight.
    // The harness counts it alike, and stops alike.
    const say = logLine(this.key);
    const wrong = wrongModel(model, shaped.model, this.limits);
    if (wrong !== null) {
      this.spent += worst;
      this.closed ??= "another model";
      const allowed = (this.limits.answeredAs.get(shaped.model) ?? []).join(", ");
      const why = "answered" in wrong ? say`the broker refuses an answer from the model ${wrong.answered} to a request for ${shaped.model}, which may come back only as ${allowed}` : say`the broker refuses an answer that names no model to a request for ${shaped.model}`;
      this.noteOnce({ requested: shaped.model, ...wrong }, why);
      return answer({ refused: why, anomaly: "another model", billed: true });
    }
    // A usage that costs more than the most its request could cannot be honest. The money is spent,
    // so the ledger counts what the usage says, and the run's cap may be passed by it alone; the run
    // fails on it, so the broker admits nothing more. The answer goes back, so that the harness
    // reckons the same cost from the same usage, fails alike, and records nothing of it.
    const cost = this.cost(usage, price, worst);
    this.spent += cost;
    if (cost > worst) {
      this.closed ??= "past its worst";
      this.noteOnce({ pastItsWorst: true }, say`the broker counted a request at $${cost}, past the $${worst} it reserved for it: no honest usage costs more, so it admits nothing more this run`);
    }
    return answer({ status: reply.status, retryAfter: reply.retryAfter, text: reply.text });
  }
}

/**
 * The one POST the broker makes, to the API, through `fetchWith`. It follows no redirect: a fetch
 * that follows one to another origin drops an Authorization header but sends x-api-key on, and with
 * it the key. A redirect fails the request, which the broker answers as lost.
 */
export function httpPost(fetchWith: typeof fetch = fetch): Post {
  return async (url, headers, body) => {
    if (url !== API_URL) throw new Error(`no request goes to ${url}`);
    const response = await fetchWith(url, { method: "POST", headers, body, redirect: "error" });
    return { status: response.status, retryAfter: response.headers.get("retry-after") ?? "", text: await response.text() };
  };
}

/** The key, once read, so that even an error on the way out never prints it. */
let held = "";

/** The value of `--<name>`, or "". */
function option(name: string): string {
  const at = process.argv.indexOf(`--${name}`);
  return at > 0 ? (process.argv.at(at + 1) ?? "") : "";
}

/**
 * A server for the broker at `socket`: one request a line on each connection, a line past the most a
 * request may hold refused and its connection closed, and no more than maxConnections connections
 * at once, so that what the broker holds in memory has a bound.
 */
export function serve(broker: Broker, socket: string, onError: (e: Error) => void): Promise<net.Server> {
  const limits = broker.limits;
  // A request line holds its body and an id: a little more than the most a body may hold.
  const maxLine = limits.maxBodyBytes + 1024;
  const server = net.createServer((conn) => {
    conn.setEncoding("utf8");
    conn.on("error", () => conn.destroy());
    const feed = lineReader(
      maxLine,
      (line) => {
        broker
          .handle(line)
          .catch((e) => scrub(JSON.stringify({ id: null, refused: `the broker failed: ${String(e)}` }), broker.key))
          .then((out) => conn.write(`${out}\n`));
      },
      () => {
        conn.end(`${JSON.stringify({ id: null, refused: `a line past ${maxLine} characters` })}\n`);
        conn.destroySoon();
      },
    );
    conn.on("data", (chunk: string) => feed(chunk));
  });
  server.maxConnections = limits.maxConnections;
  server.on("error", onError);
  return new Promise((done) => server.listen(socket, () => done(server)));
}

/** Reads the key, then serves each connection to the socket, one request a line, until stopped. */
async function main(): Promise<void> {
  const socket = option("socket");
  if (socket === "") throw new Error("no socket: pass --socket <path>");
  held = readFileSync(0, "utf8").trim();
  const limits = readLimits(readFileSync(new URL("./broker-limits.json", import.meta.url), "utf8"));
  const broker = new Broker(held, limits, Number(option("budget")), httpPost());
  await serve(broker, socket, (e) => {
    console.error(logLine(held)`the broker cannot listen at ${socket}: ${String(e)}`);
    process.exit(1);
  });
  console.error(logLine(held)`the broker listens at ${socket}, within $${broker.cap}, ${limits.maxRequests} requests, ${limits.maxInFlight} at once and ${limits.maxConnections} connections`);
}

if (import.meta.main) {
  main().catch((e) => {
    console.error(logLine(held)`the broker stops: ${String(e)}`);
    process.exit(1);
  });
}
