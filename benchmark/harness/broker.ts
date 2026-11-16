// The broker: the one process of a benchmark run that holds the API key (benchmark/README.md, "The
// harness"; docs/design-review.md, untrusted-test-isolation). The harness that starts builds and
// runs agent code holds no key; it sends each request of the Messages API to the broker over a
// socket, and the broker sends it on with the key and answers with what the API answered.
//
//   node --disallow-code-generation-from-strings --permission \
//     --allow-fs-read=<this file> --allow-fs-read=<broker-limits.json beside it> \
//     benchmark/harness/broker.ts --socket <path> --budget <dollars>
//
// The key reaches it on its standard input, which it reads to the end at its start, and in no
// environment variable, so that no other process can read it in /proc/<pid>/environ. Node's
// permission model holds the rest, whatever this file says: the process can start no process,
// worker, addon or WASI, read no file but this one and its limits, and write none. The workflow
// policy (.github/scripts/policy.test.mjs) pins the command line above, and the harness's test runs
// planted escapes under the same flags.
//
// It spends within the run's limits, which it reads at its start from broker-limits.json beside
// it: a model it names, at most maxTokens of output, at most maxRequests requests, and nothing
// whose most cost could take the run past its budget or maxDollars, whichever is less. A process
// that reaches its socket can spend no more than that, and it never learns the key: the broker
// takes the key out of every byte it writes. The harness reckons its own budget with the same two
// functions, worstCost and usageCost, so that it stops at its budget before the broker refuses;
// and should the broker refuse first, it says the run's limits are reached, and the harness stops
// as at its budget.
import { readFileSync } from "node:fs";
import net from "node:net";

/** A model's prices, in US dollars per million tokens. */
export interface Price {
  input: number;
  output: number;
  cacheRead: number;
}

/** What a run may spend, from broker-limits.json. */
export interface Limits {
  url: string;
  version: string;
  maxTokens: number;
  maxRequests: number;
  maxDollars: number;
  cacheWrite: number;
  models: Map<string, Price>;
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

/** The limits of broker-limits.json, or an error naming what is wrong with them. */
export function readLimits(text: string): Limits {
  const raw = JSON.parse(text);
  const positive = (value: unknown, what: string): number => {
    if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) throw new Error(`broker-limits.json: ${what} is no positive number`);
    return value;
  };
  if (raw?.url !== API_URL) throw new Error(`broker-limits.json: url is not ${API_URL}`);
  if (typeof raw.version !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(raw.version)) throw new Error("broker-limits.json: version is no API version");
  const models = new Map<string, Price>();
  for (const [id, price] of Object.entries(raw.models ?? {})) {
    const p = price as Price;
    models.set(id, { input: positive(p?.input, `${id}'s input price`), output: positive(p?.output, `${id}'s output price`), cacheRead: positive(p?.cacheRead, `${id}'s cache-read price`) });
  }
  if (models.size === 0) throw new Error("broker-limits.json: no model");
  return {
    url: raw.url,
    version: raw.version,
    maxTokens: positive(raw.maxTokens, "maxTokens"),
    maxRequests: positive(raw.maxRequests, "maxRequests"),
    maxDollars: positive(raw.maxDollars, "maxDollars"),
    cacheWrite: positive(raw.cacheWrite, "cacheWrite"),
    models,
  };
}

/** A text with every occurrence of the key replaced. */
export function scrub(text: string, key: string): string {
  return key === "" ? text : text.split(key).join("[the API key]");
}

/**
 * The most a request can cost: every byte of its body billed as an input token written to the
 * cache, since no token is shorter than a byte, and its most output. A bound, not an estimate, so
 * that what is reserved for the requests in flight covers what they are billed.
 */
export function worstCost(bodyText: string, maxTokens: number, price: Price, cacheWrite: number): number {
  return (new TextEncoder().encode(bodyText).length * price.input * cacheWrite + maxTokens * price.output) / 1e6;
}

/** What a request cost by the usage the API reported, or its worst case when the usage is unreadable. */
export function usageCost(usage: unknown, price: Price, cacheWrite: number, worst: number): number {
  const u = (usage ?? {}) as { input_tokens?: unknown; output_tokens?: unknown; cache_creation_input_tokens?: unknown; cache_read_input_tokens?: unknown };
  const n = (v: unknown) => (typeof v === "number" && Number.isFinite(v) && v >= 0 ? v : Number.NaN);
  const total = (n(u.input_tokens) * price.input + n(u.cache_creation_input_tokens ?? 0) * price.input * cacheWrite + n(u.cache_read_input_tokens ?? 0) * price.cacheRead + n(u.output_tokens) * price.output) / 1e6;
  return Number.isFinite(total) ? total : worst;
}

/**
 * The broker's state and its one operation: a request line in, an answer line out. It holds the
 * key, the limits and what the run has spent and reserved, and sends each request through `post`.
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

  constructor(key: string, limits: Limits, budget: number, post: Post) {
    // A key of these characters alone is written in JSON as itself, so that taking it out of an
    // answer line, which is JSON, takes it out of every value in it.
    if (!/^[A-Za-z0-9_-]{8,}$/.test(key)) throw new Error("no API key on the broker's standard input: one line of letters, digits, '-' and '_'");
    if (!(budget > 0) || !Number.isFinite(budget)) throw new Error("no budget: pass --budget <dollars>");
    this.key = key;
    this.limits = limits;
    this.post = post;
    this.cap = Math.min(budget, limits.maxDollars);
  }

  /** Why a request's body is refused, or null when it is one the broker sends. */
  refusal(body: unknown): string | null {
    if (body === null || typeof body !== "object" || Array.isArray(body)) return "a request with no body";
    const b = body as { model?: unknown; max_tokens?: unknown; stream?: unknown };
    if (typeof b.model !== "string" || !this.limits.models.has(b.model)) return `the model ${String(b.model)}, which broker-limits.json does not name`;
    if (typeof b.max_tokens !== "number" || !Number.isInteger(b.max_tokens) || b.max_tokens < 1 || b.max_tokens > this.limits.maxTokens) return `max_tokens ${String(b.max_tokens)}, beyond 1 to ${this.limits.maxTokens}`;
    if (b.stream !== undefined) return "a streamed request, which the broker does not send";
    return null;
  }

  /** What an answered request cost, by the usage the API reported; at its most when it reported none. */
  cost(text: string, price: Price, worst: number): number {
    let usage;
    try {
      usage = JSON.parse(text)?.usage;
    } catch {
      return worst;
    }
    return usageCost(usage, price, this.limits.cacheWrite, worst);
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
    const why = this.refusal(request?.body);
    if (why !== null) return answer({ refused: why });
    const body = request.body as { model: string; max_tokens: number };
    const price = this.limits.models.get(body.model) as Price;
    const text = JSON.stringify(body);
    const worst = worstCost(text, body.max_tokens, price, this.limits.cacheWrite);
    // The run's limits reached: the answer says so, and the harness stops as at its budget.
    if (this.requests >= this.limits.maxRequests) return answer({ refused: `a request past the run's ${this.limits.maxRequests}`, exhausted: true });
    if (this.spent + this.reserved + worst > this.cap) return answer({ refused: `a request that could take the run past its $${this.cap}`, exhausted: true });
    this.requests += 1;
    this.reserved += worst;
    let reply: Answer;
    try {
      reply = await this.post(this.limits.url, { "content-type": "application/json", "x-api-key": this.key, "anthropic-version": this.limits.version }, text);
    } catch (e) {
      // A lost answer may still have been billed, so it counts at its most.
      this.reserved -= worst;
      this.spent += worst;
      return answer({ status: -1, retryAfter: "", text: `the request failed: ${String(e)}` });
    }
    this.reserved -= worst;
    // The API bills no answer but a success.
    if (reply.status >= 200 && reply.status < 300) this.spent += this.cost(reply.text, price, worst);
    return answer({ status: reply.status, retryAfter: reply.retryAfter, text: reply.text });
  }
}

/** The one POST the broker makes, to the API. */
async function httpPost(url: string, headers: Record<string, string>, body: string): Promise<Answer> {
  if (url !== API_URL) throw new Error(`no request goes to ${url}`);
  const response = await fetch(url, { method: "POST", headers, body });
  return { status: response.status, retryAfter: response.headers.get("retry-after") ?? "", text: await response.text() };
}

/** The key, once read, so that even an error on the way out never prints it. */
let held = "";

/** The value of `--<name>`, or "". */
function option(name: string): string {
  const at = process.argv.indexOf(`--${name}`);
  return at > 0 ? (process.argv.at(at + 1) ?? "") : "";
}

/** Reads the key, then serves each connection to the socket, one request a line, until stopped. */
async function main(): Promise<void> {
  const socket = option("socket");
  if (socket === "") throw new Error("no socket: pass --socket <path>");
  held = readFileSync(0, "utf8").trim();
  const limits = readLimits(readFileSync(new URL("./broker-limits.json", import.meta.url), "utf8"));
  const broker = new Broker(held, limits, Number(option("budget")), httpPost);
  const server = net.createServer((conn) => {
    let pending = "";
    conn.setEncoding("utf8");
    conn.on("error", () => conn.destroy());
    conn.on("data", (chunk: string) => {
      pending += chunk;
      for (let at = pending.indexOf("\n"); at >= 0; at = pending.indexOf("\n")) {
        const line = pending.slice(0, at);
        pending = pending.slice(at + 1);
        if (line.trim() === "") continue;
        broker
          .handle(line)
          .catch((e) => scrub(JSON.stringify({ id: null, refused: `the broker failed: ${String(e)}` }), held))
          .then((out) => conn.write(`${out}\n`));
      }
    });
  });
  server.on("error", (e) => {
    console.error(scrub(`the broker cannot listen at ${socket}: ${String(e)}`, held));
    process.exit(1);
  });
  server.listen(socket, () => console.error(`the broker listens at ${socket}, within $${broker.cap} and ${limits.maxRequests} requests`));
}

if (import.meta.main) {
  main().catch((e) => {
    console.error(scrub(`the broker stops: ${String(e)}`, held));
    process.exit(1);
  });
}
