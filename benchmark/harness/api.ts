// Asking a model through the Claude Messages API over HTTP, with no dependency, for the measured
// benchmark run (benchmark/README.md, "The harness").
//
// The request names one model and no fallback, so every program comes from the model it names
// and a refusal is recorded as one. Thinking is adaptive at each model's default effort, the API
// version accepts no sampling parameters, and the conversation is sent as it was, the model's
// earlier replies unchanged. The system prompt carries a cache breakpoint and the request asks for
// automatic caching of the conversation, 5-minute entries both: the document is the same in every
// run of a language, so each later call reads it from the cache. Caching changes what a call
// costs, never what the model is shown.

export const API_VERSION = "2026-10-21";
export const MAX_TOKENS = 16000;
/** A cache write's price, as a multiple of the input price, for 5-minute entries. */
export const CACHE_WRITE = 1.25;

/** A model and its prices, in US dollars per million tokens. */
export interface Model {
  id: string;
  input: number;
  output: number;
  cacheRead: number;
}

// Prices as Anthropic lists them for its first-party API (the claude-api reference, cached
// 2026-10-06): Haiku's are for prompts of 100K tokens or fewer, which every prompt here is.
export const KNOWN_MODELS: Record<string, Model> = {
  "claude-haiku-5-5": { id: "claude-haiku-5-5", input: 0.1, output: 0.5, cacheRead: 0.01 },
  "claude-sonnet-5-5": { id: "claude-sonnet-5-5", input: 2, output: 10, cacheRead: 0.2 },
  "claude-opus-5-5": { id: "claude-opus-5-5", input: 4, output: 20, cacheRead: 0.2 },
  "claude-fable-5-1": { id: "claude-fable-5-1", input: 10, output: 50, cacheRead: 0.25 },
};

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
      if (priced) return { id: priced[1], input: Number(priced[2]), output: Number(priced[3]), cacheRead: Number(priced[4]) };
      const known = KNOWN_MODELS[m];
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

/** What a call cost, in US dollars. */
export function callCost(usage: Usage, model: Model): number {
  return (
    (usage.input_tokens * model.input +
      (usage.cache_creation_input_tokens ?? 0) * model.input * CACHE_WRITE +
      (usage.cache_read_input_tokens ?? 0) * model.cacheRead +
      usage.output_tokens * model.output) /
    1e6
  );
}

/** All the input tokens a call was billed for, cached or not. */
export function inputTokens(usage: Usage): number {
  return usage.input_tokens + (usage.cache_creation_input_tokens ?? 0) + (usage.cache_read_input_tokens ?? 0);
}

/**
 * The most a call can cost: its prompt, estimated at one token per three characters and priced
 * as a cache write, and the most output it may produce.
 */
export function worstCost(promptChars: number, model: Model): number {
  return ((promptChars / 3) * model.input * CACHE_WRITE + MAX_TOKENS * model.output) / 1e6;
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

export type Message = { role: "user" | "assistant"; content: unknown };

export interface Reply {
  content: unknown[];
  text: string;
  stop_reason: string;
  usage: Usage;
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

/**
 * One call, retried on a rate limit or a server error, which are not billed. A connection lost
 * during the call throws NetworkError, so that the caller can count the call as spent.
 */
export async function ask(apiKey: string, model: string, system: string, messages: Message[]): Promise<Reply> {
  for (let attempt = 0; ; attempt++) {
    let response: Response;
    try {
      response = await fetch("https://api.anthropic.com/v1/messages", {
        method: "POST",
        headers: {
          "content-type": "application/json",
          "x-api-key": apiKey,
          "anthropic-version": API_VERSION,
        },
        body: JSON.stringify(requestBody(model, system, messages)),
      });
    } catch (e) {
      throw new NetworkError(`the request failed: ${String(e)}`);
    }
    if (response.ok) {
      const body = await response.json();
      const content = body.content ?? [];
      const text = content
        .filter((b: { type: string }) => b.type === "text")
        .map((b: { text: string }) => b.text)
        .join("\n");
      return { content, text, stop_reason: body.stop_reason, usage: body.usage };
    }
    const retryable = response.status === 429 || response.status >= 500;
    if (!retryable || attempt >= 5) {
      throw new Error(`the API answered ${response.status}: ${await response.text()}`);
    }
    const after = Number(response.headers.get("retry-after") ?? "0");
    await new Promise((done) => setTimeout(done, after > 0 ? after * 1000 : 2000 * 2 ** attempt));
  }
}
